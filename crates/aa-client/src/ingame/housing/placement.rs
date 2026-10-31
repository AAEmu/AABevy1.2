//! The builder: what using a house design does (`X2::GameClient::HousingManager`, `x2game.dll` `3932A3B0` .. `39334460`).
//!
//! ## The original
//!
//! The design item is not cast: the client enters a placement mode with the finished house as a ghost model.
//!
//! 1. **Position** (`housing_position.dds` cursor): the ghost stands on the terrain under the cursor (a terrain pick of
//!    up to 200 m, `394298C0`). The wheel turns it (`builder_rotate_mode` of `default_binding.g`: the wheel
//!    `builder_rotate_left/right_normal`, Shift `_large`, Alt `_small`, Ctrl zooms). A left click fixes the spot.
//! 2. **Rotate** (`housing_rotation.dds`): the house turns towards the cursor. A left click asks the server for the
//!    tax (`39325F60`: `CSConstructHouseTax` with the design and the spot), unless the spot is refused: then the refusal
//!    is shown (`39428370`, an `ERROR_MSG`) and nothing is sent.
//! 3. The answer (`SCConstructHouseTax`) raises `HOUSE_BUILD_INFO`; the build window (`x2ui/housing/build_window.lua`)
//!    shows the tax and the materials. Its Start button calls `X2House:NextStepFromBuildCheck` (`3932FE60`:
//!    `CSCreateHouse`, at most once per 5 s, else `CANNOT_LOCATE_DOODAD_NOW`) and the builder ends; Cancel calls
//!    `PrevStepFromBuildCheck` and the ghost follows the cursor again.
//!
//! A right click goes back a step (Rotate -> Position, Position -> end), `BUILDER_END` (`0x225`) is raised when the
//! builder ends (the build window closes with it), `BUILDER_STEP` (`0x224`) while it runs.
//!
//! ## Where the ghost may stand
//!
//! The builder's checks (`39333E20`, `39334060`, `39335860`) set the error the ghost shows: the refused ghost is
//! drawn with `materials/special/housing_build_invalid` (dark red, `housing_build_normal` otherwise). The spot is
//! snapped to the 4 m grid first (`39325e30`: the lower left footprint corner to a whole cell, `x` and `y`).
//!
//! * the centre's area group allows the design's category (restricted categories 1 and 7..=13, 15..=18;
//!   `HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA`, also when the centre is outside every polygon);
//! * every 4 m cell of the `n x n` footprint (`n = int((2r + 3) / 4)`, `r` the garden radius) lies inside one housing
//!   polygon (`HOUSE_CANNOT_LOCATE_INVALID_AREA`);
//! * the footprint does not overlap another house's snapped axis-aligned footprint (`HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE`)
//!   or a unit/doodad standing in it (`HOUSE_CANNOT_LOCATE_OVERLAP_UNIT`);
//! * the terrain under the footprint changes by at most 6 m (`HOUSE_CANNOT_LOCATE_TERRAIN_TOO_LOW`).
//!
//! The account's other houses do not break the group's rules (`houseless`:
//! `HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION`, `existing_category_id`:
//! `HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION`) as the server checks them. The dominion rules (siege zones,
//! castle land) and the exact collision shapes of units and doodads (mask `0x1f`) are left to the server / marked
//! capture-pending.

use aa_cry::scene::{CryAssets, ModelOptions};
use aa_cry::shade::illum::IllumMaterial;
use aa_cry::workers::Workers;
use aa_net::Houses;
use aa_net::housing::HouseCommand;
use aa_net::model::UnitKind;
use aa_net::state::{Doodads, Units};
use aa_net::{Command, NetCommand};
use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::data::{self, HousingData};
use super::render::{self, PreparedPart};
use crate::hud::HudPointerOverUi;
use crate::ingame::ground_target::SkillPressed;
use crate::ingame::player::LocalPlayer;

/// How far the terrain pick reaches (the original's 200 m).
const REACH: f32 = 200.0;
/// A press and release further apart than this (logical pixels) is a drag, not a click.
const CLICK_SLOP: f32 = 6.0;
/// The wheel's turns: `builder_rotate_*_small`, `_normal`, `_large` (radians).
const TURN: [f32; 3] = [std::f32::consts::PI / 180.0, std::f32::consts::PI / 36.0, std::f32::consts::PI / 4.0];
/// `CSCreateHouse` at most this often (`FUN_3932FE60`: `DAT_3a167ea0 + 5000` ms).
const CREATE_INTERVAL: f32 = 5.0;
/// `materials/special/housing_build_invalid.mtl`: `Diffuse`, glowing.
const INVALID: Color = Color::srgb(0.737, 0.013, 0.013);

/// Original39017220 enables rotation mode only in state2; it preserves the
/// other active input modes and reassertion promotes this one.
pub(crate) fn rotation_mode(bridge: Option<&crate::hud::api::HudBridge>, on: bool) {
    if let Some(bridge) = bridge {
        bridge.0.borrow_mut().input_modes.set(2, u32::from(on));

}
}

/// The builder's steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The ghost follows the cursor.
    Position,
    /// The spot is fixed; the ghost turns towards the cursor.
    Rotate,
    /// `CSConstructHouseTax` went out; waiting for `SCConstructHouseTax`.
    Quote,
    /// The build window is up.
    Confirm,
}

/// One house being placed.
#[derive(Debug)]
pub struct Build {
    pub design: u32,
    /// The design item (`CSCreateHouse` names it; the server takes it).
    pub item_id: u64,
    pub item_template: u32,
    pub stage: Stage,
    /// The spot (game coordinates) and the yaw (radians, the units' convention).
    pub pos: Vec3,
    pub yaw: f32,
    /// Why the spot is refused (`ERROR_MSG` name), `None` when the server should take it.
    pub error: Option<&'static str>,
    /// The quote's total tax (copper), sent back with `CSCreateHouse`.
    pub total_tax: i32,
    /// The z sent with `CSConstructHouseTax`; `CSCreateHouse` reuses it so both carry one z
    /// (the capture's 0x56 and 0x57 both show 173.05162, `37 0d 2d 43`).
    quote_z: Option<f32>,
    /// The finished model's local AABB (cry space), once the ghost's parts are prepared
    /// (`FUN_3908d400`; the terrain check and the spot's z use it).
    pub(crate) model_box: Option<(Vec3, Vec3)>,
    ghost: Option<Entity>,
    /// The ghost's meshes and their own materials (swapped for [`INVALID`] while refused).
    meshes: Vec<(Entity, GhostMaterial)>,
    shown_error: Option<Option<&'static str>>,
}

impl Build {
    /// A builder already at its confirm step (dev hooks, tests and examples).
    pub(crate) fn confirmed(design: u32, item_id: u64, item_template: u32, pos: Vec3, yaw: f32, total_tax: i32) -> Build {
        Build {
            design,
            item_id,
            item_template,
            stage: Stage::Confirm,
            pos,
            yaw,
            error: None,
            total_tax,
            quote_z: None,
            model_box: None,
            ghost: None,
            meshes: Vec::new(),
            shown_error: None,
        }
    }
}

/// The original material must survive a refused (red) placement, including Illum meshes.
#[derive(Debug)]
enum GhostMaterial {
    Standard(Handle<StandardMaterial>),
    Illum(Handle<IllumMaterial>),
}

impl GhostMaterial {
    fn show(&self, commands: &mut Commands, entity: Entity, refused: Option<&Handle<StandardMaterial>>) {
        let mut entity = commands.entity(entity);
        if let Some(red) = refused {
            entity.remove::<MeshMaterial3d<IllumMaterial>>().insert(MeshMaterial3d(red.clone()));
        } else {
            match self {
                Self::Standard(material) => { entity.insert(MeshMaterial3d(material.clone())); }
                Self::Illum(material) => { entity.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(material.clone())); }
            }
        }
    }
}

/// What the builder tells the HUD.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum BuilderSignal {
    /// `BUILDER_STEP`: the builder runs (sent when it starts and at each stage).
    Step,
    /// `BUILDER_END`.
    End,
    /// A click on a refused spot: the `ERROR_MSG` to show.
    Refused(&'static str),
    /// `CANNOT_LOCATE_DOODAD_NOW` with its remaining seconds (`$1`).
    Locked(u32),
}

/// The 30 s placement lock is the farm's [`PlantLock`] (`FUN_393a6de0`); both read one timer, so
/// a house and a seed can never disagree. This module arms it only on the world-load points;
/// upstream's `plant::lock_on_arrival` still arms the shared lock on `Event::Teleported`, which the
/// separate farm-plant PR fixes.
pub(crate) use crate::ingame::ground_target::plant::PlantLock;

/// Arms the placement lock when the game screen starts (`FUN_39426520`'s end-of-load point).
pub fn arm_on_entry(mut lock: ResMut<PlantLock>, time: Res<Time>) {
    lock.hold(f64::from(time.elapsed_secs()));
}

/// Arms the placement lock when a world load or instance load finishes
/// (`FUN_39426520` calls `FUN_393a6de0`); a short teleport that needs no load does not.
pub fn arm_on_world_load(
    mut lock: ResMut<PlantLock>,
    time: Res<Time>,
    mut loads: MessageReader<crate::ingame::teleport::WorldLoadFinished>,
) {
    if loads.read().next().is_some() {
        lock.hold(f64::from(time.elapsed_secs()));
    }
}

#[derive(Resource, Default)]
pub struct Builder {
    pub build: Option<Build>,
    /// A dev hook: pin the ghost to this spot (game x, y) and yaw instead of the cursor.
    pub pinned: Option<(Vec2, f32)>,
    /// Dev hook clicks (left) waiting.
    pub clicks: u32,
    last_create: Option<f32>,
}

impl Builder {
    /// `X2House:NextStepFromBuildCheck`: build here (`CSCreateHouse`) and end the builder.
    pub fn commit(&mut self, now: f32, net: &mut MessageWriter<NetCommand>, signals: &mut MessageWriter<BuilderSignal>) {
        let Some(b) = self.build.as_ref().filter(|b| b.stage == Stage::Confirm) else { return };
        if self.last_create.is_some_and(|t| now - t < CREATE_INTERVAL) {
            let seconds = self.last_create.map_or(0, |t| (CREATE_INTERVAL - (now - t)).max(0.0) as u32);
            signals.write(BuilderSignal::Locked(seconds));
            return;
        }
        self.last_create = Some(now);
        let z = b.quote_z.unwrap_or(b.pos.z);
        let pos = data::get()
            .and_then(|d| d.designs.get(&b.design))
            .map_or(b.pos.truncate(), |d| snap_spot(b.pos.truncate(), footprint_half(d.garden_radius)));
        info!(
            "housing: build design {} at ({:.1}, {:.1}, {:.1}) yaw {:.2} with item {} (tax {})",
            b.design, pos.x, pos.y, b.pos.z, b.yaw, b.item_id, b.total_tax
        );
        net.write(NetCommand(Command::House(HouseCommand::CreateHouse {
            design_id: b.design,
            x: pos.x,
            y: pos.y,
            z,
            z_rot: b.yaw,
            item_id: b.item_id,
            money_amount: b.total_tax,
            ht: 0,
            auto_use_aa_point: false,
        })));
        self.end(signals);
    }

    /// `X2House:PrevStepFromBuildCheck`: the build window was cancelled, the ghost follows the cursor again.
    pub fn back_to_position(&mut self, signals: &mut MessageWriter<BuilderSignal>) {
        if let Some(b) = self.build.as_mut().filter(|b| matches!(b.stage, Stage::Quote | Stage::Confirm)) {
            b.stage = Stage::Position;
            signals.write(BuilderSignal::Step);
        }
    }

    /// The tax quote of the design being placed arrived: the build window comes up.
    pub fn quoted(&mut self, design: u32, total_tax: i32) -> bool {
        match self.build.as_mut() {
            Some(b) if b.stage == Stage::Quote && b.design == design => {
                b.stage = Stage::Confirm;
                b.total_tax = total_tax;
                true
            }
            _ => false,
        }
    }

    pub fn end(&mut self, signals: &mut MessageWriter<BuilderSignal>) {
        if self.build.take().is_some() {
            signals.write(BuilderSignal::End);
        }
        self.pinned = None;
        self.clicks = 0;
    }
}

/// The original's footprint cell size (m).
const CELL: f32 = 4.0;
/// `FUN_3999ecf4`: a house needs terrain that changes by at most this over its footprint (m).
const MAX_SLOPE: f32 = 6.0;

/// The side of the footprint in 4 m cells (`FUN_39897170`: `int((garden_radius * 2 + 3) * 0.25)`).
pub fn footprint_cells(garden_radius: f32) -> i32 {
    ((garden_radius * 2.0 + 3.0) * 0.25) as i32
}

/// Half the footprint's side (m).
pub fn footprint_half(garden_radius: f32) -> f32 {
    (footprint_cells(garden_radius) * 4) as f32 / 2.0
}

/// `FUN_39325e30`: the footprint's lower left corner to a whole 4 m cell (`C` truncation).
pub fn snap_spot(pos: Vec2, half: f32) -> Vec2 {
    let snap = |v: f32| ((v - half + CELL * 0.5) * 0.25) as i32 as f32 * CELL + half;
    Vec2::new(snap(pos.x), snap(pos.y))
}

/// `FUN_39897070`: the designs whose centre area must belong to their housing group.
pub fn restricted_category(category_id: u32) -> bool {
    matches!(category_id, 1 | 7..=13 | 15..=18)
}

/// The ghost's spot: the snapped position, its yaw and the finished model's local box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Site {
    pub pos: Vec2,
    pub yaw: f32,
    /// The finished model's local AABB (cry space), `None` before the ghost's parts arrive.
    pub model_box: Option<(Vec3, Vec3)>,
}

/// The other things the builder tests the spot against.
#[derive(Clone, Copy, Default)]
pub struct Around<'a> {
    /// Categories of the account's houses (`houseless` / `existing_category_id` rules).
    pub own_categories: &'a [u32],
    /// The other houses: position and garden radius.
    pub houses: &'a [(Vec2, f32)],
    /// Units that are not the player's (mounts, mates and slaves of the player are left out).
    pub units: &'a [Vec2],
    /// Doodads that are not attached to a house.
    pub doodads: &'a [Vec2],
    pub terrain: Option<&'a TerrainQuery>,
}

/// Why `design` cannot stand at `site.pos` (`ERROR_MSG` name), or `None`.
#[allow(clippy::too_many_arguments)]
pub fn check(
    data: &HousingData,
    design: u32,
    site: Site,
    around: Around<'_>,
) -> Option<&'static str> {
    let Some(d) = data.designs.get(&design) else {
        return Some("INVALID_HOUSE_INFO");
    };
    let n = footprint_cells(d.garden_radius);
    if n >= 12 {
        // `FUN_39897170` logs "house too big" and returns no footprint; the builder refuses it
        // before any area check (capture-pending: no client text or case was captured).
        return Some("INVALID_HOUSE_INFO");
    }
    let half = footprint_half(d.garden_radius);
    let site = Site {
        pos: snap_spot(site.pos, half),
        ..site
    };
    let groups: Vec<_> = data.groups_at(site.pos).collect();
    let allowed = groups
        .iter()
        .find(|(_, group)| group.categories.contains(&d.category_id));
    if restricted_category(d.category_id) && allowed.is_none() {
        // a centre outside every polygon, or one whose group has no row for the design's
        // category, is 0x244 ("Building doesn't fit in this area."). Unrestricted designs skip
        // this check (the client and the server both do).
            return Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA");
        }
    if let Some((_, group)) = allowed {
        if group.houseless && !around.own_categories.is_empty() {
        return Some("HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION");
    }
        if group.existing_category_id != 0
            && around.own_categories.contains(&group.existing_category_id)
        {
        return Some("HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION");
    }
    }
    let lower = site.pos - Vec2::splat(half);
    let upper = site.pos + Vec2::splat(half);
    let house_boxes: Vec<(Vec2, f32)> = around
        .houses
        .iter()
        .filter_map(|(p, radius)| {
            let n = footprint_cells(*radius);
            let half = footprint_half(*radius);
            (n > 0).then(|| (snap_spot(*p, half), half))
        })
        .collect();
    if n > 0 {
        let candidates: Vec<_> = data
            .land
            .iter()
            .filter(|land| land.overlaps_box(lower, upper))
            .collect();
    for cy in 0..n {
        for cx in 0..n {
            let corner = lower + Vec2::new(cx as f32 * CELL, cy as f32 * CELL);
                let cell = [
                    corner,
                    corner + Vec2::X * CELL,
                    corner + Vec2::Y * CELL,
                    corner + Vec2::splat(CELL),
                ];
            let Some(area) = candidates.iter().find(|land| land.contains(cell[0])) else {
                return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
            };
            if cell[1..].iter().any(|p| !area.contains(*p)) {
                return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
            }
                // `FUN_39333e20`'s cell callback asks the housing manager whether the cell can be
                // built on; a house already standing there is not buildable, and the client
                // answers 0x70 (recapture2), before the 0x71 house-overlap test.
                let centre = corner + Vec2::splat(CELL * 0.5);
                if house_boxes.iter().any(|(c, h)| {
                    centre.x > c.x - h
                        && centre.x < c.x + h
                        && centre.y > c.y - h
                        && centre.y < c.y + h
                }) {
                    return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
        }
    }
        }
    }
    if around
        .units
        .iter()
        .chain(around.doodads)
        .any(|p| lower.x < p.x && p.x < upper.x && lower.y < p.y && p.y < upper.y)
    {
        return Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT");
    }
    if around
        .terrain
        .is_some_and(|terrain| terrain_too_steep(site, half, |x, y| terrain.height_at(x, y)))
    {
        return Some("HOUSE_CANNOT_LOCATE_TERRAIN_TOO_LOW");
    }
    None
}

/// The terrain over the ghost's rotated model box: its four corners and centre
/// (`FUN_39326c10`), or the footprint square before the model is loaded.
fn terrain_samples(
    site: Site,
    half: f32,
    height: impl Fn(f32, f32) -> Option<f32>,
) -> Option<(f32, f32)> {
    let (lo, hi) = site
        .model_box
        .map(|(lo, hi)| (lo.truncate(), hi.truncate()))
        .unwrap_or((-Vec2::splat(half), Vec2::splat(half)));
    let (sin, cos) = site.yaw.sin_cos();
    let rotate = |p: Vec2| Vec2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
    let centre = (lo + hi) * 0.5;
    let heights: Option<Vec<f32>> = [
        Vec2::new(lo.x, lo.y),
        Vec2::new(hi.x, lo.y),
        Vec2::new(lo.x, hi.y),
        Vec2::new(hi.x, hi.y),
        centre,
    ]
        .into_iter()
    .map(|p| {
        let p = rotate(p) + site.pos;
        height(p.x, p.y)
    })
        .collect();
    heights.map(|h| {
        (
            h.iter().copied().fold(f32::INFINITY, f32::min),
            h.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    })
}

/// The spot's z: the lowest terrain sample of the rotated model box (`FUN_39332180` stores the
/// minimum before `39325160` moves the ghost).
fn lowest_sample(site: Site, half: f32, height: impl Fn(f32, f32) -> Option<f32>) -> Option<f32> {
    terrain_samples(site, half, height).map(|(min, _)| min)
}

/// `FUN_39332180`: terrain over the rotated model box changes by more than [`MAX_SLOPE`] (6 m).
fn terrain_too_steep(site: Site, half: f32, height: impl Fn(f32, f32) -> Option<f32>) -> bool {
    terrain_samples(site, half, height).is_some_and(|(min, max)| max - min > MAX_SLOPE)
}

/// The terrain under the view ray through `cursor` (game coordinates).
fn terrain_under(camera: &Camera, transform: &GlobalTransform, cursor: Vec2, terrain: &TerrainQuery) -> Option<Vec3> {
    let ray = camera.viewport_to_world(transform, cursor).ok()?;
    let origin = bevy_to_cry(ray.origin);
    let dir = bevy_to_cry(*ray.direction);
    let (mut prev, mut t) = (0.0f32, 0.0f32);
    while t < REACH {
        t += 1.0;
        let p = origin + dir * t;
        let Some(h) = terrain.height_at(p.x, p.y) else { continue };
        if p.z <= h {
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                let q = origin + dir * mid;
                if terrain.height_at(q.x, q.y).is_some_and(|g| q.z <= g) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let q = origin + dir * hi;
            return Some(Vec3::new(q.x, q.y, terrain.height_at(q.x, q.y).unwrap_or(q.z)));
        }
        prev = t;
    }
    None
}

/// The yaw (units' convention: 0 north, counter-clockwise) that faces from `from` to `to`.
fn yaw_towards(from: Vec3, to: Vec3) -> Option<f32> {
    let d = to - from;
    (d.truncate().length() > 0.3).then(|| (-d.x).atan2(d.y))
}

/// A design item used from the bag or a slot starts the builder (`ground_target::route` leaves these items alone).
pub fn start(
    mut pressed: MessageReader<SkillPressed>,
    mut builder: ResMut<Builder>,
    player: Query<&Transform, With<LocalPlayer>>,
    time: Res<Time>,
    lock: Res<PlantLock>,
    mut signals: MessageWriter<BuilderSignal>,
    bridge: Option<NonSend<crate::hud::api::HudBridge>>,
) {
    for press in pressed.read() {
        let Some((item_id, template)) = press.item else { continue };
        let Some(design) = data::get().and_then(|d| d.design_by_item.get(&template).copied()) else { continue };
        let now = f64::from(time.elapsed_secs());
        if lock.refused(now) {
            signals.write(BuilderSignal::Locked(lock.remaining(now) as u32));
            continue;
        }
        // the same item again lets go
        if builder.build.as_ref().is_some_and(|b| b.item_id == item_id) {
            builder.end(&mut signals);
            rotation_mode(bridge.as_deref(), false);
            continue;
        }
        builder.end(&mut signals);
        rotation_mode(bridge.as_deref(), false);
        let pos = player.single().map(|t| bevy_to_cry(t.translation)).unwrap_or_default();
        info!("housing: builder for design {design} (item {template})");
        builder.build = Some(Build {
            design,
            item_id,
            item_template: template,
            stage: Stage::Position,
            pos,
            yaw: 0.0,
            error: None,
            total_tax: 0,
            quote_z: None,
            model_box: None,
            ghost: None,
            meshes: Vec::new(),
            shown_error: None,
        });
        signals.write(BuilderSignal::Step);
    }
}

/// Clicks and the wheel. Runs before the HUD's targeting so the left click does not also pick a unit.
#[allow(clippy::too_many_arguments)]
pub fn input(
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    over_ui: Res<HudPointerOverUi>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut builder: ResMut<Builder>,
    mut net: MessageWriter<NetCommand>,
    mut signals: MessageWriter<BuilderSignal>,
    mut pressed_at: Local<[Option<Vec2>; 2]>,
bridge: Option<NonSend<crate::hud::api::HudBridge>>,
) {
    if builder.build.is_none() {
        *pressed_at = [None; 2];
        return;
    }
    let cursor = windows.single().ok().and_then(Window::cursor_position);
    let mut click = std::mem::take(&mut builder.clicks) > 0;
    let mut back = false;
    let stage = builder.build.as_ref().map(|b| b.stage);
    // Escape lets go while the ghost moves (the build window takes its own Escape)
    if matches!(stage, Some(Stage::Position | Stage::Rotate)) && keys.just_pressed(KeyCode::Escape) {
        keys.clear_just_pressed(KeyCode::Escape);
        builder.end(&mut signals);
        rotation_mode(bridge.as_deref(), false);
        return;
    }
    for (i, button) in [MouseButton::Left, MouseButton::Right].into_iter().enumerate() {
        if mouse.just_pressed(button) {
            pressed_at[i] = if over_ui.0 { None } else { cursor };
            if !over_ui.0 {
                mouse.clear_just_pressed(button);
            }
        }
        if mouse.just_released(button) {
            let start = pressed_at[i].take();
            if let (Some(start), Some(end)) = (start, cursor)
                && start.distance(end) <= CLICK_SLOP
                && !over_ui.0
            {
                if i == 0 { click = true } else { back = true }
            }
            if start.is_some() {
                mouse.clear_just_released(button);
            }
        }
    }
    let Some(b) = builder.build.as_mut() else { return };
    if back {
        match b.stage {
            Stage::Rotate => {
                b.stage = Stage::Position;
                rotation_mode(bridge.as_deref(), false);
                signals.write(BuilderSignal::Step);
            }
            Stage::Position => {
                builder.end(&mut signals);
                rotation_mode(bridge.as_deref(),
            false);
            }
            Stage::Quote | Stage::Confirm => {}
        }
        return;
    }
    if !click {
        return;
    }
    match b.stage {
        Stage::Position => {
            b.stage = Stage::Rotate;
            rotation_mode(bridge.as_deref(), true);
            signals.write(BuilderSignal::Step);
        }
        Stage::Rotate => {
            if let Some(error) = b.error {
                rotation_mode(bridge.as_deref(), true);
                signals.write(BuilderSignal::Refused(error));
                return;
            }
            info!("housing: tax of design {} at ({:.1}, {:.1}, {:.1})", b.design, b.pos.x, b.pos.y, b.pos.z);
            b.quote_z = Some(b.pos.z);
            net.write(NetCommand(Command::House(HouseCommand::ConstructHouseTax { design_id: b.design, x: b.pos.x, y: b.pos.y, z: b.pos.z })));
            b.stage = Stage::Quote;
            rotation_mode(bridge.as_deref(), false);
            signals.write(BuilderSignal::Step);
        }
        Stage::Quote | Stage::Confirm => {}
    }
}

/// The wheel turns the ghost (`builder_rotate_*`), the cursor moves or turns it, the checks colour it.
#[allow(clippy::too_many_arguments)]
pub fn follow(
    actions: Res<crate::settings::Actions>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<
        (&Camera, &GlobalTransform),
        (With<Camera3d>, Without<crate::hud::ModelViewCamera>),
    >,
    terrain: Option<Res<TerrainQuery>>,
    units: Res<Units>,
    doodads: Res<Doodads>,
    houses: Res<Houses>,
    player: Res<aa_net::state::Player>,
    mut builder: ResMut<Builder>,
) {
    let pinned = builder.pinned;
    let Some(b) = builder.build.as_mut() else {
        return;
    };
    for (sign, side) in [(1.0, "left"), (-1.0, "right")] {
        for (turn, size) in TURN.iter().zip(["small", "normal", "large"]) {
            if actions.just_pressed(&format!("builder_rotate_{side}_{size}"), 0) {
                b.yaw = (b.yaw + sign * turn).rem_euclid(std::f32::consts::TAU);
            }
        }
    }
    let spot = match pinned {
        Some((p, yaw)) => {
            if b.stage == Stage::Position {
                b.yaw = yaw;
            }
            terrain
                .as_deref()
                .and_then(|t| t.height_at(p.x, p.y))
                .map(|z| Vec3::new(p.x, p.y, z))
        }
        None => {
            let cursor = windows.single().ok().and_then(Window::cursor_position);
            let camera = cameras.iter().find(|(c, _)| c.is_active);
            match (cursor, camera, terrain.as_deref()) {
                (Some(cursor), Some((camera, transform)), Some(terrain)) => {
                    terrain_under(camera, transform, cursor, terrain)
                }
                _ => None,
            }
        }
    };
    let Some(data) = data::get() else { return };
    let half = data
        .designs
        .get(&b.design)
        .map_or(0.0, |d| footprint_half(d.garden_radius));
    match b.stage {
        Stage::Position => {
            if let Some(spot) = spot {
                let snapped = snap_spot(spot.truncate(), half);
                let site = Site {
                    pos: snapped,
                    yaw: b.yaw,
                    model_box: b.model_box,
                };
                // `FUN_39332180`: the spot's z is the lowest terrain sample of the rotated model
                // box (four corners and centre).
                let z = terrain
                    .as_deref()
                    .and_then(|t| lowest_sample(site, half, |x, y| t.height_at(x, y)))
                    .unwrap_or(spot.z);
                b.pos = snapped.extend(z);
            }
        }
        Stage::Rotate if pinned.is_none() => {
            if let Some(yaw) = spot.and_then(|s| yaw_towards(b.pos, s)) {
                b.yaw = yaw;
            }
        }
        _ => {}
    }
    // `FUN_39332180` runs on every builder update: the spot's z follows the lowest sample of
    // the current rotated model box (also after a wheel turn or when the model box arrives).
    if let Some(terrain) = terrain.as_deref() {
        let site = Site {
            pos: b.pos.truncate(),
            yaw: b.yaw,
            model_box: b.model_box,
        };
        if let Some(z) = lowest_sample(site, half, |x, y| terrain.height_at(x, y)) {
            b.pos.z = z;
        }
    }
    let me = player.spawn.as_ref().map(|s| s.character().id);
    let own: Vec<u32> = houses
        .by_tl_id
        .values()
        .filter(|h| !h.demolished && Some(h.snapshot.owner_id) == me)
        .filter_map(|h| {
            data.designs
                .get(&h.snapshot.template_id)
                .map(|d| d.category_id)
        })
        .collect();
    let others: Vec<(Vec2, f32)> = units
        .iter()
        .filter_map(|u| match u.kind {
            UnitKind::Housing { template_id, .. } => Some((
                Vec2::new(u.pos.x, u.pos.y),
                data.designs
                    .get(&template_id)
                    .map_or(0.0, |d| d.garden_radius),
            )),
        _ => None,
        })
        .collect();
    let own_obj = player.spawn.as_ref().map(|s| s.obj_id);
    let (obstacles, props) = obstacles(&units, &doodads, own_obj, me);
    b.error = check(
        data,
        b.design,
        Site {
            pos: b.pos.truncate(),
            yaw: b.yaw,
            model_box: b.model_box,
        },
        Around {
            own_categories: &own,
            houses: &others,
            units: &obstacles,
            doodads: &props,
            terrain: terrain.as_deref(),
        },
    );
}

/// The unit and doodad points the footprint tests: the player's own object, mount, mate and
/// slave are left out (the server's `player_owned`), and so is every doodad attached to a house
/// (`FUN_39335860`'s skip list).
fn obstacles(
    units: &Units,
    doodads: &Doodads,
    own_obj: Option<u32>,
    character: Option<u32>,
) -> (Vec<Vec2>, Vec<Vec2>) {
    let points = units
        .iter()
        .filter(|u| {
            !matches!(u.kind, UnitKind::Housing { .. }) && !player_owned(u, own_obj, character)
        })
        .map(|u| Vec2::new(u.pos.x, u.pos.y))
        .collect();
    let props = doodads
        .by_id
        .values()
        .filter(|d| {
            !units
                .get(d.parent_obj_id)
                .is_some_and(|u| matches!(u.kind, UnitKind::Housing { .. }))
        })
        .map(|d| Vec2::new(d.pos.x, d.pos.y))
        .collect();
    (points, props)
}

/// The player's own object, mount, mate or slave (the server's `player_owned`).
fn player_owned(unit: &aa_net::model::Unit, own_obj: Option<u32>, character: Option<u32>) -> bool {
    if Some(unit.obj_id) == own_obj {
        return true;
    }
    let Some(character) = character else {
        return false;
    };
    match unit.kind {
        UnitKind::Slave { summoner_id, .. } => summoner_id == character,
        UnitKind::Mate { owner_id, .. } => owner_id == character,
        _ => false,
    }
}

/// Prepared ghost parts arriving from the workers.
#[derive(Resource)]
pub struct GhostJobs {
    tx: std::sync::mpsc::Sender<(u32, Vec<PreparedPart>)>,
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<(u32, Vec<PreparedPart>)>>,
    asked: Option<u32>,
    red: Option<Handle<StandardMaterial>>,
}

impl Default for GhostJobs {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        GhostJobs { tx, rx: std::sync::Mutex::new(rx), asked: None, red: None }
    }
}

/// Marks the ghost's entities.
#[derive(Component)]
pub struct Ghost;

/// Draws the ghost: the design's finished model at the spot, its own materials while the spot is good and the red of
/// `housing_build_invalid` while it is refused.
#[allow(clippy::too_many_arguments)]
pub fn ghost(
    mut commands: Commands,
    mut cry: CryAssets,
    mut jobs: ResMut<GhostJobs>,
    mut builder: ResMut<Builder>,
    ghosts: Query<Entity, With<Ghost>>,
    children: Query<&Children>,
    meshes: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<IllumMaterial>>), With<Mesh3d>>,
    mut transforms: Query<&mut Transform, With<Ghost>>,
) {
    let Some(b) = builder.build.as_mut() else {
        for e in &ghosts {
            commands.entity(e).despawn();
        }
        jobs.asked = None;
        return;
    };
    let model = data::get().and_then(|d| d.designs.get(&b.design)).map(|d| d.main_model);
    if b.ghost.is_none() && jobs.asked != model {
        jobs.asked = model;
        if let Some(model) = model {
            let (loader, tx) = (cry.loader(), jobs.tx.clone());
            Workers::global().spawn(0.0, move || {
                let parts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render::prepare(&loader, model, &ModelOptions::default()))).unwrap_or_default();
                let _ = tx.send((model, parts));
            });
        }
    }
    let arrived: Vec<_> = {
        let rx = jobs.rx.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    };
    for (m, parts) in arrived {
        if Some(m) != model || b.ghost.is_some() {
            continue;
        }
        b.model_box = render::bounds(&parts);
        let root = commands.spawn((Name::new("house ghost"), Ghost, Transform::default(), Visibility::default())).id();
        render::spawn_parts(&mut commands, &mut cry, root, parts, &ModelOptions::default());
        b.ghost = Some(root);
        b.meshes.clear();
        b.shown_error = None;
    }
    let Some(root) = b.ghost else { return };
    if let Ok(mut t) = transforms.get_mut(root) {
        *t = Transform { translation: cry_to_bevy(b.pos), rotation: Quat::from_rotation_y(b.yaw), scale: Vec3::ONE };
    }
    // the meshes appear a frame after the spawn commands ran
    if b.meshes.is_empty() {
        b.meshes = children.iter_descendants(root).filter_map(|e| {
            let (standard, illum) = meshes.get(e).ok()?;
            let own = illum.map(|m| GhostMaterial::Illum(m.0.clone()))
                .or_else(|| standard.map(|m| GhostMaterial::Standard(m.0.clone())))?;
            Some((e, own))
        }).collect();
        b.shown_error = None;
    }
    if b.shown_error == Some(b.error) || b.meshes.is_empty() {
        return;
    }
    b.shown_error = Some(b.error);
    let red = jobs
        .red
        .get_or_insert_with(|| cry.materials.add(StandardMaterial { base_color: INVALID, emissive: INVALID.to_linear() * 2.0, perceptual_roughness: 0.9, ..default() }))
        .clone();
    for (e, own) in &b.meshes {
        own.show(&mut commands, *e, b.error.is_some().then_some(&red));
    }
}

/// Leaving the game ends the builder.
pub fn clear(mut builder: ResMut<Builder>, mut signals: MessageWriter<BuilderSignal>,
    bridge: Option<NonSend<crate::hud::api::HudBridge>>,
) {
    builder.end(&mut signals);
    rotation_mode(bridge.as_deref(), false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::{Event, NetEvent};
    #[test]
    fn refused_rotate_click_reasserts_builder_after_mode_bar_promotion() {
        use crate::hud::api::{HudBridge, HudData};
        use std::{cell::RefCell, rc::Rc};
        let shared = Rc::new(RefCell::new(HudData::default()));
        shared.borrow_mut().input_modes.set(2, 1);
        shared.borrow_mut().input_modes.set(1, 1);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_non_send(HudBridge(shared.clone()))
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HudPointerOverUi>()
            .add_message::<NetCommand>()
            .add_message::<BuilderSignal>()
            .insert_resource(Builder {
                build: Some(Build {
                    design: 171,
                    item_id: 1,
                    item_template: 1,
                    stage: Stage::Rotate,
                    pos: Vec3::ZERO,
                    yaw: 0.,
                    error: Some("HOUSE_CANNOT_LOCATE_INVALID_AREA"),
                    total_tax: 0,
                    quote_z: None,
                    model_box: None,
                    ghost: None,
                    meshes: Vec::new(),
                    shown_error: None,
                }),
                clicks: 1,
                ..default()
            })
            .add_systems(Update, input);
        app.update();
        assert_eq!(
            shared.borrow().input_modes.priority().collect::<Vec<_>>(),
            ["builder_rotate_mode", "mode_action_mode"]
        );
        assert_eq!(
            app.world()
                .resource::<Builder>()
                .build
                .as_ref()
                .unwrap()
                .stage,
            Stage::Rotate
        );
        assert!(
            app.world_mut()
                .resource_mut::<Messages<NetCommand>>()
                .drain()
                .next()
                .is_none()
        );
        assert!(matches!(
            app.world_mut()
                .resource_mut::<Messages<BuilderSignal>>()
                .drain()
                .next(),
            Some(BuilderSignal::Refused("HOUSE_CANNOT_LOCATE_INVALID_AREA"))
        ));
    }
    use crate::ingame::housing::data::{Design, Group, Land};

    #[test]
    fn refusing_placement_replaces_illum_and_restores_it_when_the_spot_becomes_valid() {
        let mut world = World::new();
        let original = Handle::<IllumMaterial>::default();
        let red = Handle::<StandardMaterial>::default();
        let mesh = world.spawn(MeshMaterial3d(original.clone())).id();
        let own = GhostMaterial::Illum(original.clone());
        let mut state = bevy::ecs::system::SystemState::<Commands>::new(&mut world);
        own.show(&mut state.get_mut(&mut world).unwrap(), mesh, Some(&red));
        state.apply(&mut world);
        assert!(world.get::<MeshMaterial3d<IllumMaterial>>(mesh).is_none());
        assert_eq!(world.get::<MeshMaterial3d<StandardMaterial>>(mesh).unwrap().0, red);
        own.show(&mut state.get_mut(&mut world).unwrap(), mesh, None);
        state.apply(&mut world);
        assert!(world.get::<MeshMaterial3d<StandardMaterial>>(mesh).is_none());
        assert_eq!(world.get::<MeshMaterial3d<IllumMaterial>>(mesh).unwrap().0, original);
    }

    fn data() -> HousingData {
        let mut d = HousingData::default();
        d.designs.insert(171, Design { id: 171, category_id: 18, garden_radius: 11.0, main_model: 1082, steps: vec![] });
        d.designs.insert(172, Design { id: 172, category_id: 1, garden_radius: 7.5, main_model: 1116, steps: vec![] });
        d.land.push(Land::new(338, vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0), Vec2::new(0.0, 100.0)]));
        d.area_group.insert(338, 15);
        d.groups.insert(15, Group { houseless: false, existing_category_id: 0, categories: vec![18, 10, 17] });
        d
    }

    fn site(pos: Vec2) -> Site {
        Site { pos, ..default() }
    }

    fn at(d: &HousingData, design: u32, pos: Vec2) -> Option<&'static str> {
        check(d, design, site(pos), Around::default())
    }

    fn with_others(
        d: &HousingData,
        design: u32,
        pos: Vec2,
        houses: &[(Vec2, f32)],
    ) -> Option<&'static str> {
        check(
            d,
            design,
            site(pos),
            Around {
                houses,
                ..default()
            },
        )
    }

    #[test]
    fn the_land_its_categories_and_other_houses_decide_where_a_house_may_stand() {
        let d = data();
        assert_eq!(at(&d, 171, Vec2::new(50.0, 50.0)), None);
        // a centre outside every polygon is 0x244 for a restricted category, not 0x70
        assert_eq!(
            at(&d, 171, Vec2::new(150.0, 50.0)),
            Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA")
        );
        assert_eq!(
            at(&d, 172, Vec2::new(50.0, 50.0)),
            Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA")
        );
        // an existing house on the spot is 0x70 (recapture2), not 0x71: the placement-mode cell
        // query answers it before the 0x71 house-overlap test; touching edges do not overlap
        assert_eq!(
            with_others(
                &d,
                171,
                Vec2::new(50.0, 50.0),
                &[(Vec2::new(50.0, 50.0), 11.0)]
            ),
            Some("HOUSE_CANNOT_LOCATE_INVALID_AREA")
        );
        assert_eq!(
            with_others(
                &d,
                171,
                Vec2::new(50.0, 50.0),
                &[(Vec2::new(70.0, 50.0), 7.5)]
            ),
            None
        );
        let mut d = d;
        d.groups.get_mut(&15).unwrap().houseless = true;
        assert_eq!(
            check(
                &d,
                171,
                site(Vec2::new(50.0, 50.0)),
                Around {
                    own_categories: &[1],
                    ..default()
                }
            ),
            Some("HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION")
        );
        d.groups.get_mut(&15).unwrap().houseless = false;
        d.groups.get_mut(&15).unwrap().existing_category_id = 18;
        assert_eq!(
            check(
                &d,
                171,
                site(Vec2::new(50.0, 50.0)),
                Around {
                    own_categories: &[18],
                    ..default()
                }
            ),
            Some("HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION")
        );
        assert_eq!(
            at(&d, 999, Vec2::new(50.0, 50.0)),
            Some("INVALID_HOUSE_INFO")
        );
    }

    #[test]
    fn unrestricted_categories_skip_the_area_category_error_and_no_footprint_designs_pass() {
        let mut d = data();
        // category 2 is outside FUN_39897070's restricted set; the group has no row for it
        d.designs.insert(
            173,
            Design {
                id: 173,
                category_id: 2,
                garden_radius: 11.0,
                main_model: 1082,
                steps: vec![],
            },
        );
        assert_eq!(at(&d, 173, Vec2::new(50.0, 50.0)), None);
        // n = int((2 * 0 + 3) * 0.25) = 0: FUN_39897170 returns no footprint and the original
        // lets the design through
        d.designs.insert(
            174,
            Design {
                id: 174,
                category_id: 18,
                garden_radius: 0.0,
                main_model: 1082,
                steps: vec![],
            },
        );
        assert_eq!(at(&d, 174, Vec2::new(50.0, 50.0)), None);
    }

    #[test]
    fn every_footprint_cell_must_lie_in_one_polygon() {
        let mut d = data();
        d.land.clear();
        d.land.push(Land::new(
            338,
            vec![
                Vec2::new(-1.0, -1.0),
                Vec2::new(25.0, -1.0),
                Vec2::new(25.0, 25.0),
                Vec2::new(-1.0, 25.0),
            ],
        ));
        // design 171: n = 6, 24 m footprint 0..24, all cells inside
        assert_eq!(at(&d, 171, Vec2::new(12.0, 12.0)), None);
        // footprint 4..28: the cells past x = 25 leave the polygon
        assert_eq!(
            at(&d, 171, Vec2::new(16.0, 12.0)),
            Some("HOUSE_CANNOT_LOCATE_INVALID_AREA")
        );
        // the snap puts the lower left corner on a 4 m cell: 16.4 -> 16.0
        assert_eq!(
            snap_spot(Vec2::new(16.4, 12.4), 12.0),
            Vec2::new(16.0, 12.0)
        );
        assert_eq!(
            snap_spot(Vec2::new(15328.4, 14632.33), 12.0),
            Vec2::new(15328.0, 14632.0)
        );
    }

    #[test]
    fn a_unit_or_doodad_in_the_footprint_refuses_it() {
        let d = data();
        let spot = Vec2::new(50.0, 50.0);
        let refused = |units: &[Vec2], doodads: &[Vec2]| {
            check(
                &d,
                171,
                site(spot),
                Around {
                    units,
                    doodads,
                    ..default()
                },
            )
        };
        assert_eq!(refused(&[], &[]), None);
        assert_eq!(
            refused(&[Vec2::new(49.0, 49.0)], &[]),
            Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT")
        );
        assert_eq!(
            refused(&[], &[Vec2::new(49.0, 49.0)]),
            Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT")
        );
        // the footprint is 38..62 on both axes; its edge does not overlap
        assert_eq!(refused(&[Vec2::new(64.0, 50.0)], &[]), None);
    }

    #[test]
    fn the_terrain_check_uses_the_rotated_model_box_and_returns_the_lowest_sample() {
        // asymmetric box: a sign error in the yaw would swap the two quarter turns
        let base = Site {
            pos: Vec2::splat(50.0),
            yaw: 0.0,
            model_box: Some((Vec3::new(-5.0, -1.0, -5.0), Vec3::new(3.0, 4.0, 10.0))),
        };
        let height = |x: f32, _y: f32| Some((x * x + x) * 0.01224);
        assert!(terrain_too_steep(base, 14.0, height), "unrotated");
        let turned = Site { yaw: std::f32::consts::FRAC_PI_2, ..base };
        assert!(!terrain_too_steep(turned, 14.0, height), "+90 degrees stays under 6 m");
        let mirrored = Site { yaw: -std::f32::consts::FRAC_PI_2, ..base };
        assert!(terrain_too_steep(mirrored, 14.0, height), "-90 degrees goes over 6 m");
        let (min, max) = terrain_samples(turned, 14.0, height).unwrap();
        assert!((min - 26.462_88).abs() < 0.001 && (max - 32.460_48).abs() < 0.001);
        assert!((lowest_sample(turned, 14.0, height).unwrap() - 26.462_88).abs() < 0.001);
        // without a model box the footprint (28 m here) is sampled as before
        assert!(terrain_too_steep(Site { model_box: None, ..turned }, 14.0, height));
        // a corner without terrain leaves the check to the server
        assert!(!terrain_too_steep(base, 14.0, |_, _| None));
    }

    fn unit(obj_id: u32, kind: UnitKind) -> aa_net::model::Unit {
        aa_net::model::Unit {
            obj_id,
            name: String::new(),
            kind,
            owner_name: String::new(),
            pos: aa_net::wire::Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 1,
            model_id: 0,
            equipment: Default::default(),
            model_params: Default::default(),
            hp: 1,
            mp: 0,
            max_hp: 1,
            max_mp: 0,
            dead: false,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            skills: Vec::new(),
            passive_buffs: Vec::new(),
            ability_exp: Vec::new(),
            active_abilities: Vec::new(),
            buffs: Vec::new(),
            anim_action: 0,
            house_posture: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        }
    }

    fn doodad(obj_id: u32, parent_obj_id: u32, x: f32) -> aa_net::model::Doodad {
        aa_net::model::Doodad {
            obj_id,
            template_id: 1,
            owner_obj_id: 0,
            parent_obj_id,
            attach_point: 0,
            pos: aa_net::wire::Pos::new(x, 0.0, 0.0),
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

    #[test]
    fn the_players_own_units_and_house_doodads_are_not_obstacles() {
        let mut units = Units::default();
        units
            .by_id
            .insert(8615, unit(8615, UnitKind::Character { char_id: 42 }));
        units.by_id.insert(
            9001,
            unit(
                9001,
                UnitKind::Slave {
                    id: 1,
                    tl_id: 1,
                    template_id: 60,
                    summoner_id: 42,
                },
            ),
        );
        units.by_id.insert(
            9002,
            unit(
                9002,
                UnitKind::Mate {
                    tl_id: 2,
                    template_id: 1,
                    owner_id: 42,
                },
            ),
        );
        units.by_id.insert(
            9003,
            unit(
                9003,
                UnitKind::Slave {
                    id: 3,
                    tl_id: 3,
                    template_id: 60,
                    summoner_id: 99,
                },
            ),
        );
        units.by_id.insert(
            9004,
            unit(
                9004,
                UnitKind::Housing {
                    tl_id: 4,
                    template_id: 171,
                    build_step: -1,
                },
            ),
        );
        let mut doodads = Doodads::default();
        doodads.by_id.insert(9100, doodad(9100, 0, 1.0));
        doodads.by_id.insert(9101, doodad(9101, 9004, 2.0));
        let (points, props) = obstacles(&units, &doodads, Some(8615), Some(42));
        assert_eq!(
            points,
            vec![Vec2::new(0.0, 0.0)],
            "only the foreign slave remains"
        );
        assert_eq!(
            props,
            vec![Vec2::new(1.0, 0.0)],
            "only the free doodad remains"
        );
    }

    #[test]
    fn the_tax_and_create_use_one_z() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<HudPointerOverUi>()
            .add_message::<NetCommand>()
            .add_message::<BuilderSignal>()
            .insert_resource(Builder {
                build: Some(Build {
                    design: 171,
                    item_id: 1,
                    item_template: 21_165,
                    stage: Stage::Rotate,
                    pos: Vec3::new(15328.0, 14632.0, 173.05162),
                    yaw: 0.0,
                    error: None,
                    total_tax: 0,
                    quote_z: None,
                    model_box: None,
                    ghost: None,
                    meshes: Vec::new(),
                    shown_error: None,
                }),
                clicks: 1,
                ..default()
            })
            .add_systems(Update, input);
        app.update();
        let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
        let NetCommand(Command::House(HouseCommand::ConstructHouseTax { z, .. })) = &commands[0] else {
            panic!("not a tax: {commands:?}");
        };
        assert_eq!(*z, 173.05162);
        assert_eq!(app.world().resource::<Builder>().build.as_ref().unwrap().quote_z, Some(173.05162));
        // a later z rewrite (the model box arriving) must not change the create's z
        app.add_systems(
            Update,
            |mut builder: ResMut<Builder>, mut net: MessageWriter<NetCommand>, mut signals: MessageWriter<BuilderSignal>| {
                let Some(build) = builder.build.as_mut() else { return };
                build.stage = Stage::Confirm;
                build.pos.z = 100.0;
                builder.commit(10.0, &mut net, &mut signals);
            },
        );
        app.update();
        let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
        let NetCommand(Command::House(HouseCommand::CreateHouse { z, .. })) = &commands[0] else {
            panic!("not a create: {commands:?}");
        };
        assert_eq!(*z, 173.05162, "CSCreateHouse reuses the tax z");
    }

    #[test]
    fn the_placement_and_create_locks_report_their_remaining_seconds() {
        let mut lock = PlantLock::default();
        assert!(!lock.refused(0.0));
        lock.hold(10.0);
        assert_eq!(lock.remaining(10.0), 30.0);
        assert_eq!(lock.remaining(39.0), 1.0);
        assert!(!lock.refused(40.0));
        assert_eq!(lock.remaining(40.0), 0.0);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<NetCommand>()
            .add_message::<BuilderSignal>()
            .insert_resource(Builder {
                build: Some(Build::confirmed(171, 1, 21_165, Vec3::ZERO, 0.0, 0)),
                ..default()
            })
            .add_systems(
                Update,
                |mut builder: ResMut<Builder>,
                 mut net: MessageWriter<NetCommand>,
                 mut signals: MessageWriter<BuilderSignal>| {
                    builder.commit(1.0, &mut net, &mut signals);
                    builder.build = Some(Build::confirmed(171, 1, 21_165, Vec3::ZERO, 0.0, 0));
                    builder.commit(1.0, &mut net, &mut signals);
                },
            );
        app.update();
        let signals: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<BuilderSignal>>()
            .drain()
            .collect();
        assert_eq!(signals, vec![BuilderSignal::End, BuilderSignal::Locked(5)]);
    }

    #[test]
    fn a_short_teleport_does_not_lock_but_a_world_load_does() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<NetEvent>()
            .add_message::<crate::ingame::teleport::WorldLoadFinished>()
            .init_resource::<PlantLock>()
            .add_systems(Update, arm_on_world_load);
        app.world_mut().resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::Teleported {
            pos: aa_net::wire::Pos::default(),
            yaw: 0.0,
            reason: 0,
        }));
        app.update();
        let now = f64::from(app.world().resource::<Time>().elapsed_secs());
        assert_eq!(app.world().resource::<PlantLock>().remaining(now), 0.0, "a short teleport must not lock");
        app.world_mut()
            .resource_mut::<Messages<crate::ingame::teleport::WorldLoadFinished>>()
            .write(crate::ingame::teleport::WorldLoadFinished);
        app.update();
        let remaining = app.world().resource::<PlantLock>().remaining(now);
        assert!(remaining > 29.9 && remaining < 30.1, "a world load must lock ({remaining})");
    }

    #[test]
    fn the_world_load_lock_blocks_starting_the_builder() {
        let Ok(_) = std::env::var("AA_DB") else {
            return;
        };
        if !aa_cry::pak_available() {
            return;
        }
        let data = data::get().unwrap_or_else(|| {
            let d = data::load_now("main_world");
            let _ = data::set(d);
            data::get().expect("housing data")
        });
        if !data.design_by_item.contains_key(&21_165) {
            return;
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<SkillPressed>()
            .add_message::<BuilderSignal>()
            .init_resource::<Builder>()
            .init_resource::<PlantLock>()
            .add_systems(Update, start);
        app.world_mut().resource_mut::<PlantLock>().hold(0.0);
        app.world_mut()
            .resource_mut::<Messages<SkillPressed>>()
            .write(SkillPressed {
                skill: 0,
                item: Some((1, 21_165)),
                target_obj_id: 0,
                gcd: false,
            });
        app.update();
        assert!(app.world().resource::<Builder>().build.is_none());
        let signals: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<BuilderSignal>>()
            .drain()
            .collect();
        assert_eq!(signals, vec![BuilderSignal::Locked(30)]);
    }

    #[test]
    fn the_area_338_polygon_refuses_the_old_spot_and_accepts_the_captured_one() {
        let Ok(_) = std::env::var("AA_DB") else {
            return;
        };
        if !aa_cry::pak_available() {
            return;
        }
        let d = data::load_now("main_world");
        if !d.land.iter().any(|l| l.area_id == 338) || !d.designs.contains_key(&171) {
            return;
        }
        // 2026-10-31-house-spot.md: 24 m footprint across the area's south east notch
        assert_eq!(
            at(&d, 171, Vec2::new(15338.53, 14632.33)),
            Some("HOUSE_CANNOT_LOCATE_INVALID_AREA")
        );
        // 2026-10-31-recapture.md: the ghost snapped to (15328, 14632) and the client accepted it
        let terrain = TerrainQuery::new("main_world");
        assert_eq!(
            check(
                &d,
                171,
                Site {
                    pos: Vec2::new(15328.4, 14631.8),
                    yaw: 0.0,
                    model_box: None
                },
                Around {
                    terrain: Some(&terrain),
                    ..default()
                },
            ),
            None
        );
        // recapture2: an existing house on that spot is 0x70, before the 0x71 overlap test
        assert_eq!(
            check(
                &d,
                171,
                Site {
                    pos: Vec2::new(15328.4, 14631.8),
                    yaw: 0.0,
                    model_box: None
                },
                Around {
                    houses: &[(Vec2::new(15328.0, 14632.0), 11.0)],
                    terrain: Some(&terrain),
                    ..default()
                },
            ),
            Some("HOUSE_CANNOT_LOCATE_INVALID_AREA")
        );
    }

    #[test]
    fn the_house_faces_the_cursor_in_the_units_yaw() {
        // north is +Y and yaw 0; west (-X) is a quarter turn counter-clockwise
        assert!(
            yaw_towards(Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0))
                .unwrap()
                .abs()
                < 1e-6
        );
        assert!(
            (yaw_towards(Vec3::ZERO, Vec3::new(-5.0, 0.0, 0.0)).unwrap()
                - std::f32::consts::FRAC_PI_2)
                .abs()
                < 1e-6
        );
        assert!(yaw_towards(Vec3::ZERO, Vec3::new(0.1, 0.1, 0.0)).is_none());
    }
}
