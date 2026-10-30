//! Planting: the items that place a doodad (seeds, saplings, livestock, ladders, flags ... every item with a row in
//! `item_spawn_doodads`) are aimed at the ground like a ground skill and send `CSCreateDoodad` instead of a cast.
//!
//! The 1.2 client enters a placement mode when such an item is used: the doodad's model (its start phase) follows the cursor over
//! the ground, a red ring and the refusal text (`SIM_DOODAD_MSG`, the `doodad_sim_*` strings) show where it cannot stand, a left
//! click places it (`CSCreateDoodad`: template, position, rotation, scale, the item's id), the right click or `Escape` cancels.
//! This is the same aiming as a ground skill's ([`super::Aiming`]: spot under the cursor, ring, click, cancel), so it only adds
//! what is specific to a doodad: which items it is for ([`begin`]), the ghost model ([`ghost`]), whether the spot is free
//! ([`check`]) and the packet ([`place`]).
//!
//! The placement rules are the 1.2 seed builder's (`FUN_393a8fa0` and its helpers, see `docs/research/porting` notes):
//! the spot must be within 30 m in 3D of the player (d^2 <= 900; F1 puts the boundary near 30 m, the decompile sets
//! it exactly), the doodad keeps
//! `max(sim_radius / 1000, 0.1)` m of room, round doodads (crops, trees, herbs, livestock, marine crops, group 38,
//! template 1219) overlap as spheres on the sum of their radii, marine crops need water while every other group
//! refuses it, and every refusal has its own `ui_texts` line. Groups 2, 3 and 12 get a random yaw (F3/F4); group 40
//! is in the decompiled set but was not captured. The others keep the previous facing+pi yaw (the decompile only
//! says the builder's rotation, which we do not model: F4/F14).
//!
//! The 30 s planting reuse lock (`_DAT_39ac7748` = 30,000 ms, `FUN_393a6de0`) starts at the end of a world load
//! (`FUN_39426520`) and on `SCTeleportUnit` arrival (F3); the next use shows "Can't place this for N second(s)."
//! (`ui_texts` 4598). A send does not start it (F3: two sends 13 s apart); the cancel path has no raw lock row and
//! is capture-pending. Still open: F2 (2D vs 3D boundary), group 5/24/40 rotations, F5 water
//! site, F6 exact radii, F7 terrain spread, F8-F12 house/farm/zone/labor, F15-F18 harvest and growth. The non-round
//! shape keeps the flat own radius until F6 decodes the oriented boxes; the water 0.3 m depth test is an assumption.

use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_net::farm::CreateDoodad;
use aa_net::state::{Doodads, Units};
use aa_net::{Command, Event, NetCommand, NetEvent, Pos};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::{WaterQuery, cry_to_bevy};
use bevy::prelude::*;

use super::{Aim, Aiming, GroundSkill, Mode, SkillPressed};
use crate::ingame::data::Db;
use crate::ingame::player::{LocalPlayer, Mover};

/// How far from the character a doodad may be placed, metres (3D). `FUN_393a8fa0` refuses only when `d^2 > 900`, so
/// exactly 30 m is allowed; F1's projected sweep only brackets the boundary around 30 m.
pub(crate) const MAX_RANGE: f32 = 30.0;
/// The planting reuse lock: 30,000 ms `_DAT_39ac7748`, set by `FUN_393a6de0` at world-load end/teleport arrival.
pub(crate) const PLANT_REUSE_LOCK_SECS: f64 = 30.0;
/// The groups whose doodads overlap as spheres (`FUN_3983b360`), plus template 1219 and group 38.
pub(crate) const ROUND_GROUPS: [u32; 6] = [2, 3, 5, 12, 40, 80];
const SPHERE_TEMPLATE: u32 = 1219;
const SPHERE_GROUP: u32 = 38;
/// Crops, trees, herbs and marine crops get a random yaw; other groups keep the previous facing+pi yaw (`FUN_393a9340`).
pub(crate) const RANDOM_YAW_GROUPS: [u32; 4] = [2, 3, 12, 40];
const MARINE_CROPS: u32 = 40;
/// The all-clear line (`ui_texts` 281).
pub(crate) const CAN_BE_PLACED: &str = "Can be placed.";

/// Why a spot cannot hold a doodad, with the 1.2 error code and `ui_texts` row it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `DOODAD_SIM_AREA` (0x114): water, and the house/farm/zone rules (capture-pending).
    Area,
    /// `DOODAD_SIM_TOO_CLOSE_TO_OTHER_DOODAD` (0x115).
    Doodad,
    /// `DOODAD_SIM_TOO_CLOSE_TO_OTHER_UNIT` (0x116).
    Unit,
    /// `DOODAD_SIM_LOW_TERRAIN` (0x117): reserved for the capture-pending 6 m spread.
    #[allow(dead_code)]
    Terrain,
    /// `DOODAD_NOT_ON_GROUND` (0x145): no ground under the cursor.
    NoGround,
    /// `TOO_FAR_AWAY` (0x9a).
    TooFar,
}

impl Refusal {
    #[cfg(test)]
    pub(crate) const fn error(self) -> u16 {
        match self {
            Self::Area => 0x114,
            Self::Doodad => 0x115,
            Self::Unit => 0x116,
            Self::Terrain => 0x117,
            Self::NoGround => 0x145,
            Self::TooFar => 0x9a,
        }
    }

    pub(crate) const fn ui_texts(self) -> u32 {
        match self {
            Self::Area => 277,
            Self::Doodad => 278,
            Self::Unit => 279,
            Self::Terrain => 280,
            Self::NoGround => 2206,
            Self::TooFar => 155,
        }
    }

    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Area => "Can't be installed here.",
            Self::Doodad | Self::Unit => "Too close to other objects.",
            Self::Terrain => "Terrain is too low.",
            Self::NoGround => "Can only be placed aboveground.",
            Self::TooFar => "Too far away.",
        }
    }
}

/// The doodad group and the room it keeps around it (`max(sim_radius / 1000, 0.1)` m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    pub template: u32,
    pub group: u32,
    pub radius: f32,
}

impl Shape {
    /// Whether the 1.2 client tests this doodad with a sphere (else with an oriented box, undecoded: F6).
    pub(crate) fn round(self) -> bool {
        ROUND_GROUPS.contains(&self.group)
            || self.template == SPHERE_TEMPLATE
            || self.group == SPHERE_GROUP
    }
}

/// What an aimed item places.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plant {
    /// The item's id and the doodad template it places.
    pub item_id: u64,
    pub doodad: u32,
    /// The model of the doodad's start phase and the room it keeps around it.
    pub model: Option<String>,
    pub shape: Shape,
    /// Why the spot under the cursor cannot be used (`None`: it can).
    pub blocked: Option<Refusal>,
}

impl Plant {
    /// The `SIM_DOODAD_MSG` line for the spot (the 1.2 all-clear once every rule passed).
    pub(crate) fn text(&self) -> &'static str {
        self.blocked.map_or(CAN_BE_PLACED, Refusal::text)
    }
}

/// The 30 s planting reuse lock: world-load end (`FUN_39426520`) and teleport arrival (F3) start it; a send does not
/// (two sends 13 s apart succeeded). Cancel and other trigger events are capture-pending.
#[derive(Resource, Default)]
pub(crate) struct PlantLock {
    until: f64,
}

impl PlantLock {
    pub(crate) fn refused(&self, now: f64) -> bool {
        now < self.until
    }

    /// Seconds left on the lock (`0` when it is over).
    pub(crate) fn remaining(&self, now: f64) -> f64 {
        (self.until - now).max(0.0)
    }

    pub(crate) fn hold(&mut self, now: f64) {
        self.until = self.until.max(now + PLANT_REUSE_LOCK_SECS);
    }
}

/// The captured `CANNOT_LOCATE_DOODAD_NOW` line with the whole seconds left (`FUN_393a8840`'s `%u`).
pub(crate) fn lock_text(remaining_secs: u64) -> String {
    format!("Can't place this for {remaining_secs} second(s).")
}

/// The end of a world load (`FUN_39426520`, where `FUN_393a6de0` writes `now + 30000`). The game screen's OnEnter is
/// the Bevy equivalent; the Teleport resource guard keeps test/example harnesses that start in-game from locking.
pub(crate) fn lock_on_world_enter(mut lock: ResMut<PlantLock>, time: Res<Time>) {
    lock.hold(f64::from(time.elapsed_secs()));
    info!("plant: world load finished, reuse locked for {PLANT_REUSE_LOCK_SECS} s");
}

/// A teleport arrival starts the same 30 s reuse lock: `SCTeleportUnit` (`Event::Teleported`, F3 at 19:49:27) or the
/// end of a long load (`WorldLoadFinished`, `FUN_39426520`). Other events stay capture-pending.
pub(crate) fn lock_on_arrival(
    mut teleports: MessageReader<NetEvent>,
    mut loads: MessageReader<crate::ingame::teleport::WorldLoadFinished>,
    mut lock: ResMut<PlantLock>,
    time: Res<Time>,
) {
    let mut arrived = loads.read().next().is_some();
    for NetEvent(event) in teleports.read() {
        if matches!(event, Event::Teleported { .. }) {
            arrived = true;
        }
    }
    if arrived {
        lock.hold(f64::from(time.elapsed_secs()));
        info!("plant: teleport/world load finished, reuse locked for {PLANT_REUSE_LOCK_SECS} s");
    }
}

/// The aiming of an item that places a doodad; `None` for every other press.
pub(crate) fn begin(press: &SkillPressed, db: &Db) -> Option<(GroundSkill, Plant)> {
    let (item_id, template) = press.item?;
    let doodad = db.item_doodad(template)?;
    let (model, group, radius) = db.doodad_start(doodad);
    let shape = Shape {
        template: doodad,
        group,
        radius,
    };
    let skill = GroundSkill {
        mode: Mode::Ground,
        radius: radius.max(0.5),
        min_range: 0.0,
        max_range: MAX_RANGE,
    };
    info!(
        "plant: item {template} places doodad {doodad} (group {group}, {model:?}, room {radius:.1} m)"
    );
    Some((
        skill,
        Plant {
            item_id,
            doodad,
            model,
            shape,
            blocked: None,
        },
    ))
}

/// Whether `spot` is within 30 m of `player` in 3D (`FUN_393a8fa0` step 3; the z counts).
pub(crate) fn in_range(player: Vec3, spot: Vec3) -> bool {
    player.distance_squared(spot) <= MAX_RANGE * MAX_RANGE
}

/// Whether a doodad of `group` gets the random yaw `FUN_393a9340` gives groups 2, 3, 12 and 40.
pub(crate) fn random_yaw(group: u32) -> bool {
    RANDOM_YAW_GROUPS.contains(&group)
}

/// The yaw the packet carries: `random` in `0..1` makes the 1.2 random turn, otherwise the previous facing+pi yaw.
/// The decompile says only that non-random groups keep the builder's rotation; we do not model that rotation, so this
/// stays the pre-existing behaviour until F4/F14 (capture-pending).
pub(crate) fn placement_yaw(group: u32, facing: f32, random: f32) -> f32 {
    if random_yaw(group) {
        random.rem_euclid(1.0) * std::f32::consts::TAU
    } else {
        facing + std::f32::consts::PI
    }
}

/// A random number in `0..1` for [`placement_yaw`] (no extra dependency; seeded once from the clock).
fn random_unit() -> f32 {
    use std::cell::Cell;
    thread_local! {
        static STATE: Cell<u64> = Cell::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9e37_79b9_7f4a_7c15)
                | 1,
        );
    }
    STATE.with(|state| {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        ((x >> 40) as u32 as f32) / ((1u32 << 24) as f32)
    })
}

/// Why `spot` cannot hold `shape`, or `None` (the 1.2 order: water, range, doodads, units).
///
/// `wet` is the water test at a spot, `shape_of` the group/radius of a template and `obstacles` the live unit positions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check(
    spot: Vec3,
    player: Vec3,
    shape: Shape,
    doodads: &Doodads,
    units: &Units,
    obstacles: impl Iterator<Item = Vec3>,
    wet: impl Fn(Vec3) -> bool,
    shape_of: impl Fn(u32) -> Shape,
) -> Option<Refusal> {
    if shape.group == MARINE_CROPS {
        if !wet(spot) {
            return Some(Refusal::Area);
        }
    } else if wet(spot) {
        return Some(Refusal::Area);
    }
    if !in_range(player, spot) {
        return Some(Refusal::TooFar);
    }
    let doodad_near = doodads
        .by_id
        .values()
        .filter_map(|d| super::super::units::doodad_world(d, units).map(|pos| (pos, d.template_id)))
        .any(|(other, template)| overlaps(spot, shape, other, shape_of(template)));
    if doodad_near {
        return Some(Refusal::Doodad);
    }
    if obstacles
        .into_iter()
        .any(|other| overlaps_unit(spot, shape, other))
    {
        return Some(Refusal::Unit);
    }
    None
}

/// Whether the new doodad overlaps `other`, testing round pairs as spheres on the sum of radii and the rest flat on
/// the new radius (the oriented boxes are not decoded: F6).
fn overlaps(spot: Vec3, shape: Shape, other: Vec3, other_shape: Shape) -> bool {
    if shape.round() && other_shape.round() {
        spot.distance(other) < shape.radius + other_shape.radius
    } else {
        spot.truncate().distance(other.truncate()) < shape.radius
    }
}

/// Whether a unit at `other` is inside the new doodad's room.
fn overlaps_unit(spot: Vec3, shape: Shape, other: Vec3) -> bool {
    if shape.round() {
        spot.distance(other) < shape.radius
    } else {
        spot.truncate().distance(other.truncate()) < shape.radius
    }
}

/// Keeps `Plant::blocked` up to date and tells the HUD (`SIM_DOODAD_MSG`) what the spot under the cursor lets the player do.
#[allow(clippy::too_many_arguments)]
pub(crate) fn validate(
    mut aiming: ResMut<Aiming>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    water: Option<Res<WaterQuery>>,
    player: Res<aa_net::state::Player>,
    mover: Query<&Mover, With<LocalPlayer>>,
    db: Option<Res<Db>>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    mut shown: Local<Option<&'static str>>,
) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let player_pos = mover.single().map_or(Vec3::ZERO, |m| m.pos);
    // Assumption: water counts when its surface is more than 0.3 m above the spot. The decompile's water test
    // (`worldvtbl+0xf4`) was not decoded; F5 must confirm the depth/shoreline rule.
    let wet = |spot: Vec3| {
        water
            .as_deref()
            .and_then(|w| w.water_at(spot))
            .is_some_and(|(height, _)| height > spot.z + 0.3)
    };
    let shape_of = |template: u32| {
        let (group, radius) = db
            .as_deref()
            .map_or((0, 0.1), |db| db.doodad_shape(template));
        Shape {
            template,
            group,
            radius,
        }
    };
    let text = match aiming.0.as_mut() {
        Some(Aim {
            plant: Some(plant),
            spot,
            ..
        }) => {
            match spot {
                Some((spot, ok)) => {
                    let obstacles = units
                        .iter()
                        .filter(|u| u.obj_id != own && !u.dead)
                        .map(|u| Vec3::new(u.pos.x, u.pos.y, u.pos.z));
                    plant.blocked = check(
                        *spot,
                        player_pos,
                        plant.shape,
                        &doodads,
                        &units,
                        obstacles,
                        wet,
                        shape_of,
                    );
                    // the ring is red for a spot that is not free
                    *ok &= plant.blocked.is_none();
                }
                None => {
                    plant.blocked = Some(Refusal::NoGround);
                }
            }
            Some(plant.text())
        }
        _ => None,
    };
    if *shown == text {
        return;
    }
    *shown = text;
    if let Some(rt) = rt {
        match text {
            Some(text) => rt.fire_event("SIM_DOODAD_MSG", text),
            None => rt.fire_event("SIM_DOODAD_MSG", ()),
        }
    }
}

/// The left click: `CSCreateDoodad` at the spot. `false` (nothing sent, the mode goes on) when the spot is not free.
pub(crate) fn place(
    plant: &Plant,
    spot: Vec3,
    mover: &Mover,
    net: &mut MessageWriter<NetCommand>,
) -> bool {
    if let Some(why) = plant.blocked {
        info!(
            "plant: ({:.1}, {:.1}, {:.1}) is not free: {why:?} ({}: {})",
            spot.x,
            spot.y,
            spot.z,
            why.ui_texts(),
            why.text()
        );
        return false;
    }
    if !in_range(mover.pos, spot) {
        info!(
            "plant: ({:.1}, {:.1}, {:.1}) is {:.1} m away, beyond the 30 m: not sent",
            spot.x,
            spot.y,
            spot.z,
            mover.pos.distance(spot)
        );
        return false;
    }
    // crops, trees, herbs and marine crops turn at random; everything else keeps the player's yaw
    let z_rot = placement_yaw(plant.shape.group, mover.facing, random_unit());
    net.write(NetCommand(Command::CreateDoodad(CreateDoodad {
        id: plant.doodad,
        pos: Pos::new(spot.x, spot.y, spot.z),
        z_rot,
        scale: 1.0,
        item_id: plant.item_id,
    })));
    info!(
        "plant: place doodad {} with item {} at ({:.1}, {:.1}, {:.1}), {:.1} m away, yaw {z_rot:.3}",
        plant.doodad,
        plant.item_id,
        spot.x,
        spot.y,
        spot.z,
        mover.pos.distance(spot)
    );
    true
}

/// The model that follows the cursor.
#[derive(Component)]
pub(crate) struct Ghost;

/// Keeps the ghost model on the spot under the cursor while an item is aimed, and removes it after.
#[allow(clippy::type_complexity)]
pub(crate) fn ghost(
    mut commands: Commands,
    aiming: Res<Aiming>,
    mut cry: CryAssets,
    mut ghosts: Query<(Entity, &mut Transform, &mut Visibility), With<Ghost>>,
    player: Query<&Mover, With<LocalPlayer>>,
    mut drawn: Local<Option<(u32, Option<Entity>)>>,
) {
    let wanted = aiming
        .0
        .as_ref()
        .and_then(|a| a.plant.as_ref().map(|p| (p, a.spot)));
    let Some((plant, spot)) = wanted else {
        if drawn.take().is_some() {
            for (e, ..) in &ghosts {
                commands.entity(e).despawn();
            }
        }
        return;
    };
    // the model is built once per doodad
    if drawn.is_none_or(|(d, _)| d != plant.doodad) {
        for (e, ..) in &ghosts {
            commands.entity(e).despawn();
        }
        let root = plant.model.as_deref().and_then(|path| {
            let options = ModelOptions::default();
            let prepared = cry.loader().prepare_model(path, &options).ok()?;
            let loaded = cry.finish_model(prepared);
            let model = spawn_loaded(&mut commands, path, &loaded, &options);
            let root = commands
                .spawn((
                    Name::new("placing doodad"),
                    Ghost,
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ))
                .id();
            commands.entity(model).insert(ChildOf(root));
            Some(root)
        });
        *drawn = Some((plant.doodad, root));
        return;
    }
    let Some((_, Some(_))) = *drawn else { return };
    let facing = player
        .single()
        .map_or(0.0, |m| m.facing + std::f32::consts::PI);
    for (_, mut transform, mut visibility) in &mut ghosts {
        match spot {
            Some((spot, _)) => {
                transform.translation = cry_to_bevy(spot);
                transform.rotation = cry_rot_to_bevy(Quat::from_rotation_z(facing));
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_check(spot: Vec3, player: Vec3, shape: Shape) -> Option<Refusal> {
        check(
            spot,
            player,
            shape,
            &Doodads::default(),
            &Units::default(),
            std::iter::empty(),
            |_| false,
            |_| shape,
        )
    }

    fn rice() -> Shape {
        Shape {
            template: 2246,
            group: 12,
            radius: 0.5,
        }
    }

    #[test]
    fn range_is_30_metres_in_3d() {
        let player = Vec3::ZERO;
        assert_eq!(empty_check(Vec3::new(29.9, 0.0, 0.0), player, rice()), None);
        assert_eq!(
            empty_check(Vec3::new(30.0, 0.0, 0.0), player, rice()),
            None,
            "the decompile allows exactly 30 m (d^2 <= 900)"
        );
        assert_eq!(
            empty_check(Vec3::new(30.1, 0.0, 0.0), player, rice()),
            Some(Refusal::TooFar)
        );
        // 29 m flat but 8 m up is 30.08 m away: the decompile counts z
        assert_eq!(
            empty_check(Vec3::new(0.0, 29.0, 8.0), player, rice()),
            Some(Refusal::TooFar)
        );
        assert_eq!(empty_check(Vec3::new(0.0, 29.0, 7.0), player, rice()), None);
    }

    #[test]
    fn round_doodads_overlap_as_spheres_on_the_sum_of_radii() {
        let mut doodads = Doodads::default();
        let mut neighbour = test_doodad();
        neighbour.obj_id = 1;
        neighbour.template_id = 2246;
        neighbour.pos = Pos::new(0.9, 0.0, 0.0);
        doodads.by_id.insert(1, neighbour.clone());
        let shape_of = |template: u32| match template {
            2246 => Shape {
                template,
                group: 12,
                radius: 0.5,
            },
            _ => Shape {
                template,
                group: 24,
                radius: 0.5,
            },
        };
        let check_at = |doodads: &Doodads, spot: Vec3| {
            check(
                spot,
                Vec3::ZERO,
                rice(),
                doodads,
                &Units::default(),
                std::iter::empty(),
                |_| false,
                shape_of,
            )
        };
        assert_eq!(
            check_at(&doodads, Vec3::ZERO),
            Some(Refusal::Doodad),
            "0.9 < 0.5 + 0.5"
        );
        neighbour.pos = Pos::new(1.1, 0.0, 0.0);
        doodads.by_id.insert(1, neighbour.clone());
        assert_eq!(check_at(&doodads, Vec3::ZERO), None, "1.1 > 0.5 + 0.5");
        // 0.9 across and 0.6 up is 1.08 m in 3D: outside the spheres (the old flat test refused it)
        neighbour.pos = Pos::new(0.9, 0.0, 0.6);
        doodads.by_id.insert(1, neighbour);
        assert_eq!(check_at(&doodads, Vec3::ZERO), None);
    }

    #[test]
    fn non_round_doodads_keep_the_flat_radius_until_the_box_is_decoded() {
        let mut doodads = Doodads::default();
        let mut neighbour = test_doodad();
        neighbour.obj_id = 1;
        neighbour.template_id = 1034;
        neighbour.pos = Pos::new(0.4, 0.0, 2.0);
        doodads.by_id.insert(1, neighbour.clone());
        let sign = Shape {
            template: 1034,
            group: 24,
            radius: 0.5,
        };
        let shape_of = |template: u32| Shape {
            template,
            group: 24,
            radius: 0.5,
        };
        let check_at = |doodads: &Doodads, spot: Vec3| {
            check(
                spot,
                Vec3::ZERO,
                sign,
                doodads,
                &Units::default(),
                std::iter::empty(),
                |_| false,
                shape_of,
            )
        };
        assert_eq!(
            check_at(&doodads, Vec3::ZERO),
            Some(Refusal::Doodad),
            "flat 0.4 < 0.5, the height is ignored"
        );
        neighbour.pos = Pos::new(0.6, 0.0, 2.0);
        doodads.by_id.insert(1, neighbour);
        assert_eq!(check_at(&doodads, Vec3::ZERO), None);
    }

    #[test]
    fn units_keep_the_doodad_radius() {
        let unit = Vec3::new(0.4, 0.0, 0.0);
        let check_at = |spot: Vec3| {
            check(
                spot,
                Vec3::ZERO,
                rice(),
                &Doodads::default(),
                &Units::default(),
                [unit].into_iter(),
                |_| false,
                |_| rice(),
            )
        };
        assert_eq!(check_at(Vec3::ZERO), Some(Refusal::Unit));
        let far = Vec3::new(0.6, 0.0, 0.0);
        let check_at = |spot: Vec3| {
            check(
                spot,
                Vec3::ZERO,
                rice(),
                &Doodads::default(),
                &Units::default(),
                [far].into_iter(),
                |_| false,
                |_| rice(),
            )
        };
        assert_eq!(check_at(Vec3::ZERO), None);
    }

    #[test]
    fn water_follows_the_doodad_group() {
        let wet = |_: Vec3| true;
        let dry = |_: Vec3| false;
        let check_with = |shape: Shape, wet: &dyn Fn(Vec3) -> bool| {
            check(
                Vec3::ZERO,
                Vec3::ZERO,
                shape,
                &Doodads::default(),
                &Units::default(),
                std::iter::empty(),
                wet,
                |_| shape,
            )
        };
        assert_eq!(
            check_with(rice(), &wet),
            Some(Refusal::Area),
            "a crop refuses water"
        );
        assert_eq!(check_with(rice(), &dry), None);
        let marine = Shape {
            template: 411,
            group: 40,
            radius: 0.5,
        };
        assert_eq!(check_with(marine, &wet), None, "marine crops need water");
        assert_eq!(check_with(marine, &dry), Some(Refusal::Area));
        let platform = Shape {
            template: 1035,
            group: 37,
            radius: 0.5,
        };
        assert_eq!(
            check_with(platform, &wet),
            Some(Refusal::Area),
            "only group 40 may stand in water"
        );
    }

    #[test]
    fn only_crops_trees_herbs_and_marine_crops_get_a_random_yaw() {
        for group in [2, 3, 12, 40] {
            assert!(random_yaw(group), "group {group}");
            assert!((placement_yaw(group, 1.0, 0.25) - std::f32::consts::TAU * 0.25).abs() < 1e-6);
        }
        for group in [5, 24, 37, 38, 80] {
            assert!(!random_yaw(group), "group {group}");
            assert_eq!(
                placement_yaw(group, 1.0, 0.25),
                1.0 + std::f32::consts::PI,
                "group {group} keeps the previous facing+pi yaw"
            );
        }
    }

    #[test]
    fn refusals_use_the_1_2_error_codes_and_texts() {
        for (refusal, error, ui_texts, text) in [
            (Refusal::Area, 0x114, 277, "Can't be installed here."),
            (Refusal::Doodad, 0x115, 278, "Too close to other objects."),
            (Refusal::Unit, 0x116, 279, "Too close to other objects."),
            (Refusal::Terrain, 0x117, 280, "Terrain is too low."),
            (
                Refusal::NoGround,
                0x145,
                2206,
                "Can only be placed aboveground.",
            ),
            (Refusal::TooFar, 0x9a, 155, "Too far away."),
        ] {
            assert_eq!(
                (refusal.error(), refusal.ui_texts(), refusal.text()),
                (error, ui_texts, text),
                "{refusal:?}"
            );
        }
        assert_eq!(
            Plant {
                item_id: 1,
                doodad: 2246,
                model: None,
                shape: rice(),
                blocked: None
            }
            .text(),
            CAN_BE_PLACED
        );
    }

    #[test]
    fn a_teleport_or_world_load_starts_the_30s_lock_and_the_text() {
        use std::time::Duration;
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .add_message::<crate::ingame::teleport::WorldLoadFinished>()
            .init_resource::<PlantLock>()
            .init_resource::<Time>()
            .add_systems(Update, lock_on_arrival);

        app.world_mut().write_message(NetEvent(Event::Teleported {
            pos: Pos::new(1.0, 2.0, 3.0),
            yaw: 0.0,
            reason: 14,
        }));
        app.update();
        let lock = app.world().resource::<PlantLock>();
        assert!(
            lock.refused(29.999),
            "F3: the teleport arrival locks for 30 s"
        );
        assert!(!lock.refused(30.0));
        assert_eq!(
            lock_text(lock.remaining(0.1) as u64),
            "Can't place this for 29 second(s)."
        );

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(31));
        app.update();
        app.world_mut()
            .write_message(crate::ingame::teleport::WorldLoadFinished);
        app.update();
        let lock = app.world().resource::<PlantLock>();
        assert!(
            lock.refused(31.0 + 29.999),
            "the end of a world load locks too"
        );
    }

    #[test]
    fn the_planting_reuse_lock_is_30s_and_shows_the_captured_text() {
        let mut lock = PlantLock::default();
        assert!(!lock.refused(0.0));
        lock.hold(10.0);
        assert!(lock.refused(39.999));
        assert!(!lock.refused(40.0));
        lock.hold(11.0);
        assert!(
            lock.refused(40.5),
            "a shorter lock never shortens the old one"
        );
        assert!(!lock.refused(41.0));
        assert_eq!(
            lock_text(lock.remaining(17.2) as u64),
            "Can't place this for 23 second(s)."
        );
    }

    #[test]
    fn a_doodad_keeps_its_room() {
        let mut doodads = Doodads::default();
        doodads.by_id.insert(
            1,
            aa_net::model::Doodad {
                obj_id: 1,
                template_id: 2246,
                pos: Pos::new(10.0, 10.0, 5.0),
                ..test_doodad()
            },
        );
        let units = Units::default();
        let shape_of = |template: u32| Shape {
            template,
            group: 12,
            radius: 0.5,
        };
        let check_at = |spot: Vec3| {
            check(
                spot,
                Vec3::ZERO,
                rice(),
                &doodads,
                &units,
                std::iter::empty(),
                |_| false,
                shape_of,
            )
        };
        assert_eq!(check_at(Vec3::new(10.3, 10.0, 5.0)), Some(Refusal::Doodad));
        assert_eq!(check_at(Vec3::new(11.0, 10.0, 5.0)), None);
        // another floor 1.2 m up is outside the spheres (the old |dz| < 3 refused it)
        assert_eq!(check_at(Vec3::new(10.3, 10.0, 6.2)), None);
    }

    fn test_doodad() -> aa_net::model::Doodad {
        aa_net::model::Doodad {
            obj_id: 0,
            template_id: 0,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 0,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        }
    }
}
