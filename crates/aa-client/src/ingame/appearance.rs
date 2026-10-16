//! What the server tells about a character's looks -> an aa-anim [`CharacterDesc`].
//!
//! The server sends race / gender, the 28 equipment slots (face, hair and the worn armour as item
//! template ids) and the creator's `ModelParams` (hair colour, skin colour, face sliders, decals,
//! colours). aa-anim wants the same things as database ids, which they are one to one.

use aa_anim::custom::Customization;
use aa_anim::db::{Decal, GameDb};
use aa_anim::{CharacterDesc, Costume, Gender, Race};
use aa_net::model::{Equipment, ItemDetail, ModelParams, race, slot};
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
        Some((r, if gender_id == aa_net::model::gender::FEMALE { Gender::Female } else { Gender::Male }))
    })
}

/// The look of a unit that is drawn like a player character.
pub fn describe(db: &GameDb, race: Race, gender: Gender, equipment: &Equipment, params: &ModelParams) -> Option<CharacterDesc> {
    let base = CharacterDesc::default_for(db, race, gender).ok()?;
    let mut desc = base.clone();
    let model = desc.model_id();

    // face: the customisable base face unless the item names another one
    desc.face = equipment.get(slot::FACE).map(|i| i.template_id).filter(|id| db.body_part(*id).is_ok());
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
        decals.extend(face.fixed_decals.iter().filter(|(id, _)| *id != 0).map(|&(asset_id, weight)| Decal {
            asset_id,
            weight,
            ..Default::default()
        }));
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
    let pak = aa_pak::Pak::global();
    let mut items = Vec::new();
    for s in WORN.into_iter().chain([slot::COSPLAY]) {
        let Some(item) = equipment.get(s) else { continue };
        let look = match &item.detail {
            ItemDetail::Equip(d) if d.image_item_template_id != 0 => d.image_item_template_id,
            _ => item.template_id,
        };
        for id in [look, item.template_id] {
            let Ok(paths) = db.item_meshes(id, model) else { continue };
            if !paths.is_empty() && paths.iter().all(|p| pak.exists(p)) {
                items.push(id);
                break;
            }
        }
    }
    items
}

/// How far (depth buffer steps) the nude body is pushed behind worn armour.
const BODY_DEPTH_BIAS: f32 = -30_000.0;

/// The nude body pokes through the armour in places (the engine hides it behind the armour's
/// `Cut` hull, which aa-cry does not draw): push it back in the depth buffer so the armour wins.
/// Applies to the per character material copies aa-anim makes, not to shared ones.
pub fn layer_body_under_armour(
    characters: Query<&aa_anim::Character, Added<aa_anim::Character>>,
    children: Query<&Children>,
    materials_of: Query<&MeshMaterial3d<StandardMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use aa_anim::PartKind;
    for c in &characters {
        if c.parts.len() != c.assets.parts.len() || !c.assets.parts.iter().any(|p| p.kind == PartKind::Costume) {
            continue;
        }
        for (&entity, part) in c.parts.iter().zip(&c.assets.parts) {
            if part.kind != PartKind::Body {
                continue;
            }
            for e in children.iter_descendants(entity) {
                if let Some(mut m) = materials_of.get(e).ok().and_then(|m| materials.get_mut(&m.0)) {
                    m.depth_bias = BODY_DEPTH_BIAS;
                }
            }
        }
    }
}
