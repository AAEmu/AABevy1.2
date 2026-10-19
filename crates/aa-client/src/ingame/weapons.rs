//! Weapons and shields worn on the body (relaxed stance): hung on the attachment bones of the
//! base skeleton, like the bow slung across the back in the real client's spawn view (the shield is not shown at peace: it
//! comes onto the arm with the sword). While the
//! character fights ([`Drawn`], set by the combat code) the weapon it fights with moves to its hand: the bow for the bow
//! clips, the sword (both swords for the dual wield clips, the two handed sword for the `twohand_` ones) for the weapon clips,
//! the instrument for a song. [`Wield`] says which class of weapon the character holds, which the combat code needs to pick the
//! one handed, two handed or dual wield clip of a skill.

use aa_anim::Animator;
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_net::model::{Equipment, slot};
use bevy::prelude::*;
use bevy::ecs::entity::Entities;

use super::data::Db;

/// Models to hang on a character's bones, resolved when it is built.
#[derive(Component)]
pub struct Carried(pub Vec<CarriedItem>);

pub struct CarriedItem {
    pub path: String,
    pub bone: &'static str,
    pub transform: Transform,
    pub grip: Grip,
    /// Held with both hands.
    pub two_handed: bool,
    /// A staff: held with both hands but in the one handed stance.
    pub staff: bool,
    /// The off hand weapon of a dual wielder (left hand).
    pub offhand: bool,
    /// The bone of the hand that holds it and its pose there (`None`: never leaves the body, a shield).
    pub hand_bone: Option<&'static str>,
    pub hand_transform: Transform,
    /// Out of sight while it is not in the hand: a bow whose place on the back a two handed weapon takes (`holdables.sheathe_priority`
    /// 9 against the bow's 4).
    pub hide_sheathed: bool,
}

/// How a weapon is held, which decides when it comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// A bow: in the left hand for the bow's skills.
    Bow,
    /// A sword, axe, staff ...: in the hand for every other weapon skill.
    Blade,
    /// A lute, drum or pipe: in the hands for a song (out of sight otherwise). The value is the kind: 0 string, 1 percussion,
    /// 2 tube (the order of the `*_instrument_*_anim_id` columns).
    Instrument(usize),
    /// A shield: on the arm while a one handed weapon is out, out of sight otherwise (the real client shows none on a character at
    /// peace, nor with a bow or an instrument in the hands).
    Shield,
}

/// The class of weapon a character holds: which of a skill's one handed, two handed and dual wield clips it plays, and which
/// instrument a song is played on.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wield {
    pub two_handed: bool,
    /// The two handed weapon is a staff (a spell caster's): the one handed clips with a `staff_` name, `two_handed` stays set for
    /// the sheath only.
    pub staff: bool,
    pub dual: bool,
    pub instrument: Option<usize>,
    /// The character carries a sword, axe, staff ... (the basic attack has no clip of its own: the server picks the weapon's).
    pub melee: bool,
}

impl Wield {
    pub fn of(items: &[CarriedItem]) -> Wield {
        Wield {
            two_handed: items.iter().any(|i| i.grip == Grip::Blade && i.two_handed && !i.staff),
            staff: items.iter().any(|i| i.grip == Grip::Blade && i.staff),
            dual: items.iter().any(|i| i.grip == Grip::Blade && i.offhand),
            melee: items.iter().any(|i| i.grip == Grip::Blade),
            instrument: items.iter().find_map(|i| match i.grip {
                Grip::Instrument(kind) => Some(kind),
                _ => None,
            }),
        }
    }
}

/// A carried model that can change between its sheath and the hand.
#[derive(Component)]
pub struct HeldWeapon {
    pub owner: Entity,
    pub grip: Grip,
    pub offhand: bool,
    pub sheath: Entity,
    pub hand: Option<Entity>,
    pub sheath_transform: Transform,
    pub hand_transform: Transform,
    pub in_hand: bool,
    pub hide_sheathed: bool,
}

/// What the character has out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Bow,
    /// The sword (and a second one when `dual`).
    Blade { dual: bool },
    Instrument,
}

/// The character has its weapon out until `until` (seconds on the app clock).
#[derive(Component, Clone, Copy, Debug)]
pub struct Drawn {
    pub until: f64,
    pub out: Out,
    /// The holster has begun: `until` is the moment the weapon leaves the hand, and the combat stance is over.
    pub sheathing: bool,
}

impl Drawn {
    /// The weapon comes out for an action that ends at `ends`: it stays out for a while after it.
    pub fn after_action(ends: f64, out: Out) -> Drawn {
        Drawn { until: ends + stay(out), out, sheathing: false }
    }

    /// The weapon in hand while the character is in its combat stance at `now` (`None` at peace, or while it is put away).
    pub fn stance(&self, now: f64) -> Option<Out> {
        (!self.sheathing && now < self.until).then_some(self.out)
    }
}

/// Seconds a weapon stays out after the last action with it: a fight is not over with the blow (the real client's captures show the
/// shield still up 7 s after the kill); an instrument goes away sooner.
fn stay(out: Out) -> f64 {
    match out {
        Out::Instrument => 4.0,
        Out::Bow | Out::Blade { .. } => 10.0,
    }
}

/// The holster clip of the weapon a character puts away (the bow and the instruments have none): the weapon leaves the hand at the
/// clip's `aw_none` moment.
pub fn holster_clip(out: Out, wield: Wield) -> Option<&'static str> {
    match out {
        Out::Blade { .. } if wield.staff => Some("staff_ac_holster_staff_l"),
        Out::Blade { .. } if wield.two_handed => Some("twohand_ac_holster_back_r"),
        Out::Blade { .. } => Some("onehand_ac_holster_side_l"),
        Out::Bow | Out::Instrument => None,
    }
}

/// Which weapon the clip `name` is for (`None` for a clip that needs none: fists, the generic `all_` clips, a hit reaction).
pub fn weapon_for_clip(name: &str) -> Option<Out> {
    if name.starts_with("bow_") {
        Some(Out::Bow)
    } else if name.starts_with("music_") {
        Some(Out::Instrument)
    } else if name.starts_with("twoswords_") {
        Some(Out::Blade { dual: true })
    } else if name.starts_with("onehand_") || name.starts_with("twohand_") {
        Some(Out::Blade { dual: false })
    } else {
        None
    }
}

/// Where a holdable hangs while it is not in the hand (bones of the base skeleton: `item_bow_l`
/// slings a bow diagonally across the back, `item_side_l / r` at the left and right hip, `item_arm_l` on the left
/// forearm, `item_back_c` the back: the two handed weapons; a staff hangs like the bow, diagonally from the left shoulder: the
/// real client's capture).
fn sheath_bone(code: &str, slot: usize, two_handed: bool) -> &'static str {
    match (code, slot) {
        ("bow", _) => "item_bow_l",
        ("shield", _) => "item_arm_l",
        _ if two_handed => "item_back_c",
        // the real client hangs the main sword on the left hip (`onehand_ac_holster_side_l` puts it there) and an off hand weapon
        // on the right one (`onehand_ac_holster_side_r`)
        (_, slot::OFFHAND) => "item_side_r",
        (_, slot::MAINHAND) => "item_side_l",
        _ => "item_back_c",
    }
}

/// `AA_DEV_MAINHAND` / `AA_DEV_OFFHAND` / `AA_DEV_MUSICAL`: the item template (0 for none) a character holds in that slot instead
/// of its own, for looking at the two handed, dual wield and instrument clips with the one handed test character.
fn dev_item(slot: usize) -> Option<u32> {
    let name = match slot {
        slot::MAINHAND => "AA_DEV_MAINHAND",
        slot::OFFHAND => "AA_DEV_OFFHAND",
        slot::MUSICAL => "AA_DEV_MUSICAL",
        _ => return None,
    };
    std::env::var(name).ok()?.parse().ok()
}

/// The weapons, shield and instrument in an equipment list.
pub fn carried(db: &Db, equipment: &Equipment) -> Vec<CarriedItem> {
    let mut out = Vec::new();
    for s in [slot::MAINHAND, slot::OFFHAND, slot::RANGED, slot::MUSICAL] {
        let template = dev_item(s).or_else(|| equipment.get(s).map(|i| i.template_id));
        let Some(w) = template.filter(|t| *t != 0).and_then(|t| db.weapon(t)) else { continue };
        let [rx, ry, rz] = w.rotation.map(f32::to_radians);
        let instrument = match w.code.as_str() {
            "string_instrument" => Some(0),
            "percussion_instrument" => Some(1),
            "tube_instrument" => Some(2),
            _ => None,
        };
        let offhand = s == slot::OFFHAND && w.code != "shield";
        let (grip, hand_bone) = match (w.code.as_str(), s) {
            _ if instrument.is_some() => (Grip::Instrument(instrument.unwrap_or(0)), Some("item_hand_r")),
            ("bow", _) => (Grip::Bow, Some("item_hand_l")),
            ("shield", _) => (Grip::Shield, None),
            _ if offhand => (Grip::Blade, Some("item_hand_l")),
            _ => (Grip::Blade, Some("item_hand_r")),
        };
        out.push(CarriedItem {
            bone: if w.staff { "item_bow_l" } else { sheath_bone(&w.code, s, w.two_handed) },
            transform: Transform {
                translation: Vec3::from(w.offset),
                rotation: Quat::from_euler(EulerRot::XYZ, rx, ry, rz),
                scale: Vec3::splat(if w.worn_scale > 0.0 { w.worn_scale } else { 1.0 }),
            },
            grip,
            two_handed: w.two_handed,
            staff: w.staff,
            offhand,
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
            hide_sheathed: false,
        });
    }
    // a two handed weapon takes the bow's place on the back
    if out.iter().any(|i| i.grip == Grip::Blade && i.two_handed) {
        for i in out.iter_mut().filter(|i| i.grip == Grip::Bow) {
            i.hide_sheathed = true;
        }
    }
    out
}

/// What the character select screen shows of a character's weapons: the sword in its scabbard; the bow only for a character that
/// has no other weapon (`reference/character_select`: Refone carries a sword and a bow, and only the sword is on the model).
pub fn select_weapons(mut items: Vec<CarriedItem>) -> Vec<CarriedItem> {
    if items.iter().any(|i| i.grip == Grip::Blade) {
        items.retain(|i| i.grip != Grip::Bow);
    }
    items
}

/// Spawns the carried models under their bones once the character's skeleton exists.
pub fn attach_carried(mut commands: Commands, mut cry: CryAssets, characters: Query<(Entity, &Animator, &Carried)>) {
    for (entity, animator, carried) in &characters {
        let bone_of = |name: &str| animator.rig.skeleton.bones.iter().position(|b| b.name == name).and_then(|i| animator.bones.get(i).copied());
        debug!("weapons: {entity:?} carries {:?}", carried.0.iter().map(|i| (i.path.rsplit('/').next().unwrap_or(""), i.grip, i.bone)).collect::<Vec<_>>());
        for item in &carried.0 {
            let Some(bone) = bone_of(item.bone) else { continue };
            // Full shading bakes the Illum specular map into metal / roughness: wood stays wood, blades shine
            let options = ModelOptions { cry_space: true, shading: Shading::Full, ..default() };
            let Ok(loaded) = cry.load_model(&item.path, &options) else { continue };
            let model = spawn_loaded(&mut commands, &item.path, &loaded, &options);
            commands.entity(model).insert((item.transform, ChildOf(bone)));
            // an instrument is only there while it is played, a shield while the sword is out
            if matches!(item.grip, Grip::Instrument(_) | Grip::Shield) || item.hide_sheathed {
                commands.entity(model).insert(Visibility::Hidden);
            }
            if item.grip == Grip::Shield {
                commands.entity(model).insert(HeldWeapon {
                    owner: entity,
                    grip: item.grip,
                    offhand: false,
                    sheath: bone,
                    hand: None,
                    sheath_transform: item.transform,
                    hand_transform: item.transform,
                    in_hand: false,
                    hide_sheathed: false,
                });
            } else if let Some(hand) = item.hand_bone.and_then(bone_of) {
                commands.entity(model).insert(HeldWeapon {
                    owner: entity,
                    grip: item.grip,
                    offhand: item.offhand,
                    sheath: bone,
                    hand: Some(hand),
                    sheath_transform: item.transform,
                    hand_transform: item.hand_transform,
                    in_hand: false,
                    hide_sheathed: item.hide_sheathed,
                });
            }
        }
        // the character can go in the same frame (a login stage closing while its model came in): no panic then
        commands.entity(entity).try_insert(Wield::of(&carried.0)).try_remove::<Carried>();
    }
}

/// Moves the weapons between the sheath and the hand as their owners start and stop fighting.
pub fn hold_weapons(time: Res<Time>, mut commands: Commands, entities: &Entities, owners: Query<&Drawn>, mut weapons: Query<(Entity, &mut HeldWeapon)>) {
    let now = time.elapsed_secs_f64();
    for (entity, mut weapon) in &mut weapons {
        // a weapon whose owner went in the frame it was attached lost its bone: it goes too
        if !entities.contains(weapon.owner) {
            commands.entity(entity).despawn();
            continue;
        }
        let drawn = owners.get(weapon.owner).ok().filter(|d| now < d.until).map(|d| d.out);
        let want = match (weapon.grip, drawn) {
            (Grip::Bow, Some(Out::Bow)) | (Grip::Instrument(_), Some(Out::Instrument)) | (Grip::Shield, Some(Out::Blade { .. })) => true,
            // the second sword only comes out for the dual wield clips
            (Grip::Blade, Some(Out::Blade { dual })) => dual || !weapon.offhand,
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
        commands.entity(entity).try_insert((transform, ChildOf(bone)));
        if matches!(weapon.grip, Grip::Instrument(_) | Grip::Shield) || weapon.hide_sheathed {
            commands.entity(entity).try_insert(if want { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(grip: Grip, two_handed: bool, offhand: bool) -> CarriedItem {
        CarriedItem { path: String::new(), bone: "", transform: Transform::IDENTITY, grip, two_handed, staff: false, offhand, hand_bone: None, hand_transform: Transform::IDENTITY, hide_sheathed: false }
    }

    #[test]
    fn the_class_of_what_is_held() {
        assert_eq!(Wield::of(&[]), Wield::default());
        let sword_and_shield = [item(Grip::Blade, false, false), item(Grip::Shield, false, false), item(Grip::Bow, false, false)];
        assert_eq!(Wield::of(&sword_and_shield), Wield { melee: true, ..Default::default() });
        assert_eq!(Wield::of(&[item(Grip::Bow, false, false)]), Wield::default());
        assert_eq!(Wield::of(&[item(Grip::Blade, true, false)]), Wield { two_handed: true, melee: true, ..Default::default() });
        assert_eq!(Wield::of(&[item(Grip::Blade, false, false), item(Grip::Blade, false, true)]), Wield { dual: true, melee: true, ..Default::default() });
        assert_eq!(Wield::of(&[item(Grip::Instrument(2), false, false)]).instrument, Some(2));
        let staff = CarriedItem { staff: true, ..item(Grip::Blade, true, false) };
        let wield = Wield::of(&[staff]);
        assert!(wield.staff && wield.melee && !wield.two_handed, "a staff is held with both hands but fights in the one handed stance");
        assert_eq!(holster_clip(Out::Blade { dual: false }, wield), Some("staff_ac_holster_staff_l"));
    }

    #[test]
    fn clips_say_which_weapon_comes_out() {
        assert_eq!(weapon_for_clip("bow_co_sk_launch"), Some(Out::Bow));
        assert_eq!(weapon_for_clip("onehand_co_sk_streakattack_1"), Some(Out::Blade { dual: false }));
        assert_eq!(weapon_for_clip("twohand_co_sk_weapon_slash"), Some(Out::Blade { dual: false }));
        assert_eq!(weapon_for_clip("twoswords_co_sk_fastattack_1"), Some(Out::Blade { dual: true }));
        assert_eq!(weapon_for_clip("music_co_sk_lute_cast"), Some(Out::Instrument));
        assert_eq!(weapon_for_clip("fist_co_attack_r"), None);
        assert_eq!(weapon_for_clip("all_co_sk_spell_cast_fireball"), None);
    }

    #[test]
    fn a_weapon_stays_out_after_the_action_and_goes_back_with_the_holster() {
        let drawn = Drawn::after_action(5.0, Out::Blade { dual: false });
        assert_eq!(drawn.stance(14.9), Some(Out::Blade { dual: false }), "the stance lasts after the swing");
        assert_eq!(drawn.stance(15.1), None);
        // once the holster began the stance is over though the weapon is still in the hand until `until`
        let sheathing = Drawn { sheathing: true, until: 20.0, ..drawn };
        assert_eq!(sheathing.stance(16.0), None);
        // a song is over sooner than a fight
        assert!(Drawn::after_action(0.0, Out::Instrument).until < drawn.until - 5.0);
        let two_handed = Wield { two_handed: true, ..Default::default() };
        assert_eq!(holster_clip(Out::Blade { dual: false }, Wield::default()), Some("onehand_ac_holster_side_l"));
        assert_eq!(holster_clip(Out::Blade { dual: false }, two_handed), Some("twohand_ac_holster_back_r"));
        assert_eq!(holster_clip(Out::Bow, Wield::default()), None);
    }

    #[test]
    fn the_select_screen_shows_the_sword_or_else_the_bow() {
        let both = vec![item(Grip::Blade, false, false), item(Grip::Bow, false, false)];
        let shown = select_weapons(both);
        assert_eq!(shown.iter().map(|i| i.grip).collect::<Vec<_>>(), [Grip::Blade]);
        assert_eq!(select_weapons(vec![item(Grip::Bow, false, false)]).len(), 1);
        assert!(select_weapons(Vec::new()).is_empty());
    }

    #[test]
    fn the_real_weapons_have_a_class() {
        let Some(db) = Db::open() else { return };
        // item 5660 is the first two handed sword, 5569 the first one handed one, 6177 the first lute
        assert!(db.weapon(5660).is_some_and(|w| w.two_handed));
        assert!(db.weapon(5569).is_some_and(|w| !w.two_handed));
        assert!(db.weapon(6177).is_some_and(|w| w.code == "string_instrument"));
        // item 6102 is the first two handed staff: two handed for the sheath, the one handed stance for the clips
        assert!(db.weapon(6102).is_some_and(|w| w.two_handed && w.staff));
        assert!(db.weapon(5660).is_some_and(|w| w.two_handed && !w.staff));
    }
}
