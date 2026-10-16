//! Weapons and shields worn on the body (relaxed stance): hung on the attachment bones of the
//! base skeleton, like the bow slung across the back in the real client's spawn view.

use aa_anim::Animator;
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_net::model::{Equipment, slot};
use bevy::prelude::*;

use super::data::Db;

/// Models to hang on a character's bones, resolved when it is built.
#[derive(Component)]
pub struct Carried(pub Vec<CarriedItem>);

pub struct CarriedItem {
    pub path: String,
    pub bone: &'static str,
    pub transform: Transform,
}

/// Where a holdable hangs while it is not in the hand (bones of the base skeleton: `item_bow_l`
/// slings a bow diagonally across the back, `item_side_l / r` at the hips, `item_arm_l` on the left
/// forearm).
fn sheath_bone(code: &str, slot: usize) -> &'static str {
    match (code, slot) {
        ("bow", _) => "item_bow_l",
        ("shield", _) => "item_arm_l",
        (_, slot::OFFHAND) => "item_side_l",
        (_, slot::MAINHAND) => "item_side_r",
        _ => "item_back_c",
    }
}

/// The weapons and shield in an equipment list.
pub fn carried(db: &Db, equipment: &Equipment) -> Vec<CarriedItem> {
    let mut out = Vec::new();
    for s in [slot::MAINHAND, slot::OFFHAND, slot::RANGED] {
        let Some(item) = equipment.get(s) else { continue };
        let Some(w) = db.weapon(item.template_id) else { continue };
        let [rx, ry, rz] = w.rotation.map(f32::to_radians);
        out.push(CarriedItem {
            bone: sheath_bone(&w.code, s),
            transform: Transform {
                translation: Vec3::from(w.offset),
                rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
                scale: Vec3::splat(if w.worn_scale > 0.0 { w.worn_scale } else { 1.0 }),
            },
            path: w.path,
        });
    }
    out
}

/// Spawns the carried models under their bones once the character's skeleton exists.
pub fn attach_carried(mut commands: Commands, mut cry: CryAssets, characters: Query<(Entity, &Animator, &Carried)>) {
    for (entity, animator, carried) in &characters {
        for item in &carried.0 {
            let Some(index) = animator.rig.skeleton.bones.iter().position(|b| b.name == item.bone) else { continue };
            let Some(&bone) = animator.bones.get(index) else { continue };
            // Full shading bakes the Illum specular map into metal / roughness: wood stays wood, blades shine
            let options = ModelOptions { cry_space: true, shading: Shading::Full, ..default() };
            let Ok(loaded) = cry.load_model(&item.path, &options) else { continue };
            let model = spawn_loaded(&mut commands, &item.path, &loaded, &options);
            commands.entity(model).insert((item.transform, ChildOf(bone)));
        }
        commands.entity(entity).remove::<Carried>();
    }
}
