//! What the server tells about a character's looks -> an aa-anim [`CharacterDesc`].
//!
//! The server sends race / gender, the 28 equipment slots (face, hair and the worn armour as item
//! template ids) and the creator's `ModelParams` (hair colour, skin colour, face sliders, decals,
//! colours). aa-anim wants the same things as database ids, which they are one to one.

use aa_anim::custom::Customization;
use aa_anim::db::{Decal, GameDb, ItemVisual};
use aa_anim::{CharacterAssets, CharacterDesc, Costume, Gender, Race};
use aa_net::model::{Equipment, Item, ItemDetail, ModelParams, race, slot};
use bevy::prelude::*;

/// Armour slots in the order they are drawn (underwear first, outer layers last).
const WORN: [usize; 11] = [
    slot::UNDERPANTS,
    slot::UNDERSHIRT,
    slot::LEGS,
    slot::CHEST,
    slot::WAIST,
    slot::HANDS,
    slot::FEET,
    slot::ARMS,
    slot::NECK,
    slot::HEAD,
    slot::BACK,
];

/// Race and gender of a unit: its model id (`models.id`, 10 / 11 nuian ... 20 / 21 ferre), else the
/// race / gender bytes of the packet.
pub fn race_gender(model_id: u32, race_id: u8, gender_id: u8) -> Option<(Race, Gender)> {
    Race::from_model_id(model_id).or_else(|| {
        let r = match race_id {
            race::NUIAN => Race::Nuian,
            race::ELF => Race::Elf,
            race::HARIHARAN => Race::Hariharan,
            race::FERRE => Race::Ferre,
            _ => return None,
        };
        Some((
            r,
            if gender_id == aa_net::model::gender::FEMALE {
                Gender::Female
            } else {
                Gender::Male
            },
        ))
    })
}

/// The look of a unit that is drawn like a player character.
pub fn describe(db: &GameDb, race: Race, gender: Gender, equipment: &Equipment, params: &ModelParams) -> Option<CharacterDesc> {
    let base = CharacterDesc::default_for(db, race, gender).ok()?;
    let mut desc = base.clone();
    let model = desc.model_id();

    // face: the customisable base face unless the item names another one
    desc.face = equipment
        .get(slot::FACE)
        .map(|i| i.template_id)
        .filter(|id| db.body_part(*id).is_ok());
    if let Some(item) = equipment.get(slot::HAIR).map(|i| i.template_id) {
        desc.hair = db.body_part(item).is_ok().then_some(item);
    }
    if params.kind >= ModelParams::HAIR && params.hair_color != 0 && db.hair_color(params.hair_color).is_ok() {
        desc.hair_color = Some(params.hair_color);
    } else if desc.hair != base.hair {
        // a different hair style than the preset's: its own first colour
        desc.hair_color = None;
    }
    if params.kind >= ModelParams::SKIN && params.skin_color != 0 {
        match db.skin_color(params.skin_color) {
            Ok(s) if s.model_id == model => desc.skin = params.skin_color,
            Ok(s) => warn!("skin colour {} belongs to model {}, not {model}", s.id, s.model_id),
            Err(e) => warn!("{e}"),
        }
    }
    if let Some(face) = params.face.as_ref().filter(|_| params.kind >= ModelParams::FACE) {
        let mut decals = Vec::new();
        if face.movable_decal_asset_id != 0 {
            decals.push(Decal {
                asset_id: face.movable_decal_asset_id,
                weight: face.movable_decal_weight,
                scale: face.movable_decal_scale,
                rotate: face.movable_decal_rotate,
                move_x: i32::from(face.movable_decal_move_x),
                move_y: i32::from(face.movable_decal_move_y),
            });
        }
        decals.extend(
            face.fixed_decals
                .iter()
                .filter(|(id, _)| *id != 0)
                .map(|&(asset_id, weight)| Decal {
                    asset_id,
                    weight,
                    ..Default::default()
                }),
        );
        desc.customization = Customization {
            modifier: face.modifier.clone(),
            normal_map: face.normal_map_id,
            normal_map_weight: face.normal_map_weight,
            lip_color: face.lip_color,
            left_pupil_color: face.left_pupil_color,
            right_pupil_color: face.right_pupil_color,
            eyebrow_color: face.eyebrow_color,
            deco_color: face.deco_color,
            decals,
        };
    }
    desc.costume = Costume::Items(worn_items(db, model, equipment));
    Some(desc)
}

/// Template ids of the worn armour that have meshes for this model (a look-change item stands in for
/// the item it covers).
fn worn_items(db: &GameDb, model: u32, equipment: &Equipment) -> Vec<u32> {
    WORN.into_iter()
        .chain([slot::COSPLAY])
        .filter_map(|s| selected_visuals(db, model, equipment.get(s)?).map(|(id, _)| id))
        .collect()
}

/// The exact visual already admitted by the costume assembler, including image-item fallback.
fn selected_visuals(db: &GameDb, model: u32, item: &Item) -> Option<(u32, Vec<ItemVisual>)> {
    let pak = aa_pak::Pak::global();
    let look = match &item.detail {
        ItemDetail::Equip(d) if d.image_item_template_id != 0 => d.image_item_template_id,
        _ => item.template_id,
    };
    for id in [look, item.template_id] {
        let Ok(visuals) = db.item_visuals(id, model) else { continue };
        if !visuals.is_empty() && visuals.iter().all(|visual| pak.exists(&visual.path)) {
            return Some((id, visuals));
        }
    }
    None
}

/// Original39315820 uses the rendered cosplay descriptor before the head descriptor,
/// even when the former selects ordinary hair. Never infer flags from item names/classes.
fn headgear_detail(db: &GameDb, desc: &CharacterDesc, equipment: &Equipment) -> Option<u32> {
    let Costume::Items(rendered) = &desc.costume else { return None };
    for s in [slot::COSPLAY, slot::HEAD] {
        let Some((id, visuals)) = equipment.get(s).and_then(|item| selected_visuals(db, desc.model_id(), item)) else {
            continue;
        };
        if !rendered.contains(&id) {
            continue;
        }
        let detail = visuals[0].detail;
        // All original armor/model groups have one shared detail. Refuse an
        // ambiguous replacement catalog rather than guessing which descriptor won.
        if visuals.iter().any(|visual| visual.detail != detail) {
            debug!("appearance: ambiguous headgear detail for item {id}, model {}", desc.model_id());
            return None;
        }
        return Some(detail);
    }
    None
}

/// Resolve received appearance with the hair variant of its actual rendered headgear.
/// Equipment visibility policy remains the costume assembler's responsibility.
pub fn resolve(db: &GameDb, desc: &CharacterDesc, equipment: &Equipment) -> aa_anim::Result<CharacterAssets> {
    match headgear_detail(db, desc, equipment) {
        Some(detail) => desc.resolve_with_headgear(db, Some(detail)),
        None => desc.resolve(db),
    }
}

#[cfg(test)]
mod tests;
