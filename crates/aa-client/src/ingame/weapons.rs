//! Weapons and shields worn on the body (relaxed stance): hung on the attachment bones of the
//! base skeleton, like the bow slung across the back in the real client's spawn view. While the
//! character fights ([`Drawn`], set by the combat code) the weapon it fights with moves to its hand.

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
    pub grip: Grip,
    /// The bone of the hand that holds it and its pose there (`None`: never leaves the body, a shield).
    pub hand_bone: Option<&'static str>,
    pub hand_transform: Transform,
}

/// How a weapon is held, which decides when it comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// A bow: in the left hand for the bow's skills.
    Bow,
    /// A sword, axe, staff ...: in the hand for every other weapon skill.
    Blade,
    /// A shield stays on the arm.
    Shield,
}

/// A carried model that can change between its sheath and the hand.
#[derive(Component)]
pub struct HeldWeapon {
    pub owner: Entity,
    pub grip: Grip,
    pub sheath: Entity,
    pub hand: Option<Entity>,
    pub sheath_transform: Transform,
    pub hand_transform: Transform,
    pub in_hand: bool,
}

/// The character has its weapon out until `until` (seconds on the app clock): the bow when `bow`, else the blades.
#[derive(Component, Clone, Copy, Debug)]
pub struct Drawn {
    pub until: f64,
    pub bow: bool,
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
        let (grip, hand_bone) = match (w.code.as_str(), s) {
            ("bow", _) => (Grip::Bow, Some("item_hand_l")),
            ("shield", _) => (Grip::Shield, None),
            (_, slot::OFFHAND) => (Grip::Blade, Some("item_hand_l")),
            _ => (Grip::Blade, Some("item_hand_r")),
        };
        out.push(CarriedItem {
            bone: sheath_bone(&w.code, s),
            transform: Transform {
                translation: Vec3::from(w.offset),
                rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
                scale: Vec3::splat(if w.worn_scale > 0.0 { w.worn_scale } else { 1.0 }),
            },
            grip,
            hand_bone,
            hand_transform: {
                let mut t = Transform::from_scale(Vec3::splat(if w.drawn_scale > 0.0 { w.drawn_scale } else { 1.0 }));
                if grip == Grip::Bow {
                    // the bow's limbs run vertically when the left hand holds it with the arm out (the hand bone turns it flat);
                    // a blade needs nothing: the hand bone holds it as it is modelled
                    t.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
                }
                t
            },
            path: w.path,
        });
    }
    out
}

/// Spawns the carried models under their bones once the character's skeleton exists.
pub fn attach_carried(mut commands: Commands, mut cry: CryAssets, characters: Query<(Entity, &Animator, &Carried)>) {
    for (entity, animator, carried) in &characters {
        let bone_of = |name: &str| animator.rig.skeleton.bones.iter().position(|b| b.name == name).and_then(|i| animator.bones.get(i).copied());
        for item in &carried.0 {
            let Some(bone) = bone_of(item.bone) else { continue };
            // Full shading bakes the Illum specular map into metal / roughness: wood stays wood, blades shine
            let options = ModelOptions { cry_space: true, shading: Shading::Full, ..default() };
            let Ok(loaded) = cry.load_model(&item.path, &options) else { continue };
            let model = spawn_loaded(&mut commands, &item.path, &loaded, &options);
            commands.entity(model).insert((item.transform, ChildOf(bone)));
            if let Some(hand) = item.hand_bone.and_then(bone_of) {
                commands.entity(model).insert(HeldWeapon {
                    owner: entity,
                    grip: item.grip,
                    sheath: bone,
                    hand: Some(hand),
                    sheath_transform: item.transform,
                    hand_transform: item.hand_transform,
                    in_hand: false,
                });
            }
        }
        commands.entity(entity).remove::<Carried>();
    }
}

/// Moves the weapons between the sheath and the hand as their owners start and stop fighting.
pub fn hold_weapons(time: Res<Time>, mut commands: Commands, owners: Query<&Drawn>, mut weapons: Query<(Entity, &mut HeldWeapon)>) {
    let now = time.elapsed_secs_f64();
    for (entity, mut weapon) in &mut weapons {
        let drawn = owners.get(weapon.owner).ok().filter(|d| now < d.until);
        let want = match (weapon.grip, drawn) {
            (Grip::Bow, Some(d)) => d.bow,
            (Grip::Blade, Some(d)) => !d.bow,
            _ => false,
        };
        if want == weapon.in_hand {
            continue;
        }
        weapon.in_hand = want;
        let (bone, transform) = match (want, weapon.hand) {
            (true, Some(hand)) => (hand, weapon.hand_transform),
            _ => (weapon.sheath, weapon.sheath_transform),
        };
        commands.entity(entity).insert((transform, ChildOf(bone)));
    }
}
