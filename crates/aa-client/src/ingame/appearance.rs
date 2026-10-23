//! What the server tells about a character's looks -> an aa-anim [`CharacterDesc`].
//!
//! The server sends race / gender, the 28 equipment slots (face, hair and the worn armour as item
//! template ids) and the creator's `ModelParams` (hair colour, skin colour, face sliders, decals,
//! colours). aa-anim wants the same things as database ids, which they are one to one.

use aa_anim::custom::Customization;
use aa_anim::db::{Decal, GameDb, ItemVisual};
use aa_anim::{CharacterAssets, CharacterDesc, Costume, Gender, PartAsset, PartKind, Race};
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
    if let Some(item) = desc.hair.filter(|item| Some(*item) != base.hair && !hair_drawable(db, model, *item)) {
        // the server's database is newer than the 1.2 client: its npc-only / beauty-shop styles (nu_m_hair029 ...)
        // have meshes and materials here but no textures, and drew as flat grey sheets without the alpha cut
        debug!("appearance: hair style {item} has no textures in this client, the preset's style stands in");
        desc.hair_color = desc.hair_color.and_then(|id| same_colour(db, model, id, base.hair)).or(base.hair_color);
        desc.hair = base.hair;
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

/// Whether the hair style `item` can be drawn by this client: the colour maps of its material (that of its first
/// `hair_colors` row, else the mesh's own `.mtl`) are in the pak.
pub(crate) fn hair_drawable(db: &GameDb, model: u32, item: u32) -> bool {
    let Ok(part) = db.body_part(item) else { return false };
    let material = db
        .hair_colors_for(model, part.asset_ids[0])
        .ok()
        .and_then(|c| c.into_iter().next())
        .map(|c| c.material)
        .unwrap_or_else(|| part.path.rsplit_once('.').map_or(part.path.clone(), |(stem, _)| format!("{stem}.mtl")));
    let Ok(mtl) = aa_cry::material::Material::load(&material) else { return true };
    let pak = aa_pak::Pak::global();
    let subs = if mtl.sub_materials.is_empty() { std::slice::from_ref(&mtl) } else { &mtl.sub_materials[..] };
    subs.iter()
        .filter_map(|m| m.texture(&aa_cry::material::TextureMap::Diffuse))
        .all(|t| pak.exists(&t.pak_path()))
}

/// The `hair_colors` row of the hair style `hair` with the same colour (`comment`, `customNN`) as the row `id`.
fn same_colour(db: &GameDb, model: u32, id: u32, hair: Option<u32>) -> Option<u32> {
    let name = db.hair_color(id).ok()?.name;
    let asset = db.body_part(hair?).ok()?.asset_ids[0];
    db.hair_colors_for(model, asset).ok()?.into_iter().find(|c| c.name == name).map(|c| c.id)
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
    let mut assets = match headgear_detail(db, desc, equipment) {
        Some(detail) => desc.resolve_with_headgear(db, Some(detail)),
        None => desc.resolve(db),
    }?;
    if let Some(proto) = assets.parts.first().cloned() {
        assets.parts.extend(extra_body_parts(db, desc.model_id(), equipment, &proto));
    }
    Ok(assets)
}

/// The body part slots `aa-anim` does not know: the ferre tail and the beard are skin attachments of their own
/// (`item_body_parts`, `.chr` on the base skeleton like the hair). Glasses (slot 21) have no data in the database.
/// `proto` is any part of the character (a body part has no equipment slot): the new parts copy what is not named here.
fn extra_body_parts(db: &GameDb, model: u32, equipment: &Equipment, proto: &PartAsset) -> Vec<PartAsset> {
    let pak = aa_pak::Pak::global();
    [(slot::TAIL, "tail"), (slot::BEARD, "beard")]
        .into_iter()
        .filter_map(|(s, name)| {
            let template = equipment.get(s)?.template_id;
            let part = db.body_part(template).ok().filter(|p| p.model_id == model && pak.exists(&p.path))?;
            let (path, material, attachments) = if part.path.ends_with(".cdf") {
                let cdf = aa_anim::Cdf::load(&part.path).ok()?;
                (cdf.model, cdf.material, cdf.attachments)
            } else {
                (part.path, None, Vec::new())
            };
            Some(PartAsset { kind: PartKind::Costume, slot: name.into(), path, material, dye: None, attachments, ..proto.clone() })
        })
        .collect()
}

#[cfg(test)]
mod tests;
