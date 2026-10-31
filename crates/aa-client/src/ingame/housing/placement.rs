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
/// `CSCreateHouse` at most this often (the original's 5 s, `CANNOT_LOCATE_DOODAD_NOW`).
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
            signals.write(BuilderSignal::Refused("CANNOT_LOCATE_DOODAD_NOW"));
            return;
        }
        self.last_create = Some(now);
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
            z: b.pos.z,
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

/// Why `design` cannot stand at `pos` (`ERROR_MSG` name), or `None`.
///
/// `others` are the other houses (position, garden radius), `units`/`doodads` the points of the
/// units and doodads near the spot, and `terrain` the heights to test the footprint over. The
/// position is the raw ghost position; the checks use its snapped one.
#[allow(clippy::too_many_arguments)]
pub fn check(
    data: &HousingData,
    design: u32,
    pos: Vec2,
    own_categories: &[u32],
    others: impl Iterator<Item = (Vec2, f32)>,
    units: &[Vec2],
    doodads: &[Vec2],
    terrain: Option<&TerrainQuery>,
) -> Option<&'static str> {
    let Some(d) = data.designs.get(&design) else { return Some("INVALID_HOUSE_INFO") };
    let n = footprint_cells(d.garden_radius);
    if !(1..12).contains(&n) {
        // `FUN_39897170` logs "house too big" and returns no footprint; the builder refuses it
        // before any area check (capture-pending: no client text or case was captured).
        return Some("INVALID_HOUSE_INFO");
    }
    let half = footprint_half(d.garden_radius);
    let pos = snap_spot(pos, half);
    let groups: Vec<_> = data.groups_at(pos).collect();
    let allowed = groups.iter().find(|(_, group)| group.categories.contains(&d.category_id));
    if restricted_category(d.category_id) {
        // a centre outside every polygon, or one whose group has no row for the design's
        // category, is 0x244 ("Building doesn't fit in this area.")
        if allowed.is_none() {
            return Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA");
        }
    } else if groups.is_empty() {
        return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
    } else if allowed.is_none() {
        return Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA");
    }
    let (_, group) = allowed?;
    if group.houseless && !own_categories.is_empty() {
        return Some("HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION");
    }
    if group.existing_category_id != 0 && own_categories.contains(&group.existing_category_id) {
        return Some("HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION");
    }
    let lower = pos - Vec2::splat(half);
    let upper = pos + Vec2::splat(half);
    let candidates: Vec<_> = data.land.iter().filter(|land| land.overlaps_box(lower, upper)).collect();
    for cy in 0..n {
        for cx in 0..n {
            let corner = lower + Vec2::new(cx as f32 * CELL, cy as f32 * CELL);
            let cell = [corner, corner + Vec2::X * CELL, corner + Vec2::Y * CELL, corner + Vec2::splat(CELL)];
            let Some(area) = candidates.iter().find(|land| land.contains(cell[0])) else {
                return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
            };
            if cell[1..].iter().any(|p| !area.contains(*p)) {
                return Some("HOUSE_CANNOT_LOCATE_INVALID_AREA");
            }
        }
    }
    for (other, radius) in others {
        let other_n = footprint_cells(radius);
        if other_n <= 0 {
            continue;
        }
        let other_half = footprint_half(radius);
        let other = snap_spot(other, other_half);
        if lower.x < other.x + other_half
            && other.x - other_half < upper.x
            && lower.y < other.y + other_half
            && other.y - other_half < upper.y
        {
            return Some("HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE");
        }
    }
    if units.iter().chain(doodads).any(|p| lower.x < p.x && p.x < upper.x && lower.y < p.y && p.y < upper.y) {
        return Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT");
    }
    if terrain.is_some_and(|terrain| terrain_too_steep(lower, upper, |x, y| terrain.height_at(x, y))) {
        return Some("HOUSE_CANNOT_LOCATE_TERRAIN_TOO_LOW");
    }
    None
}

/// `FUN_39332180`/`FUN_39326c10`: the terrain over the footprint's four corners changes by more than
/// [`MAX_SLOPE`] (`FUN_3999ecf4`, 6 m). A corner without terrain leaves the check to the server.
fn terrain_too_steep(lower: Vec2, upper: Vec2, height: impl Fn(f32, f32) -> Option<f32>) -> bool {
    let heights: Option<Vec<f32>> = [lower, Vec2::new(upper.x, lower.y), Vec2::new(lower.x, upper.y), upper]
        .into_iter()
        .map(|p| height(p.x, p.y))
        .collect();
    heights.is_some_and(|h| h.iter().copied().fold(f32::NEG_INFINITY, f32::max) - h.iter().copied().fold(f32::INFINITY, f32::min) > MAX_SLOPE)
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
pub fn start(mut pressed: MessageReader<SkillPressed>, mut builder: ResMut<Builder>, player: Query<&Transform, With<LocalPlayer>>, mut signals: MessageWriter<BuilderSignal>,bridge: Option<NonSend<crate::hud::api::HudBridge>>,
) {
    for press in pressed.read() {
        let Some((item_id, template)) = press.item else { continue };
        let Some(design) = data::get().and_then(|d| d.design_by_item.get(&template).copied()) else { continue };
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
    cameras: Query<(&Camera, &GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    terrain: Option<Res<TerrainQuery>>,
    units: Res<Units>,
    doodads: Res<Doodads>,
    houses: Res<Houses>,
    player: Res<aa_net::state::Player>,
    mut builder: ResMut<Builder>,
) {
    let pinned = builder.pinned;
    let Some(b) = builder.build.as_mut() else { return };
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
            terrain.as_deref().and_then(|t| t.height_at(p.x, p.y)).map(|z| Vec3::new(p.x, p.y, z))
        }
        None => {
            let cursor = windows.single().ok().and_then(Window::cursor_position);
            let camera = cameras.iter().find(|(c, _)| c.is_active);
            match (cursor, camera, terrain.as_deref()) {
                (Some(cursor), Some((camera, transform)), Some(terrain)) => terrain_under(camera, transform, cursor, terrain),
                _ => None,
            }
        }
    };
    let Some(data) = data::get() else { return };
    let half = data.designs.get(&b.design).map_or(0.0, |d| footprint_half(d.garden_radius));
    match b.stage {
        Stage::Position => {
            if let Some(spot) = spot {
                let snapped = snap_spot(spot.truncate(), half);
                let z = terrain.as_deref().and_then(|t| t.height_at(snapped.x, snapped.y)).unwrap_or(spot.z);
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
    let me = player.spawn.as_ref().map(|s| s.character().id);
    let own: Vec<u32> = houses
        .by_tl_id
        .values()
        .filter(|h| !h.demolished && Some(h.snapshot.owner_id) == me)
        .filter_map(|h| data.designs.get(&h.snapshot.template_id).map(|d| d.category_id))
        .collect();
    let others = units.iter().filter_map(|u| match u.kind {
        UnitKind::Housing { template_id, .. } => Some((Vec2::new(u.pos.x, u.pos.y), data.designs.get(&template_id).map_or(0.0, |d| d.garden_radius))),
        _ => None,
    });
    let own_obj = player.spawn.as_ref().map(|s| s.obj_id);
    let obstacles: Vec<Vec2> = units
        .iter()
        .filter(|u| !matches!(u.kind, UnitKind::Housing { .. }) && Some(u.obj_id) != own_obj)
        .map(|u| Vec2::new(u.pos.x, u.pos.y))
        .collect();
    let props: Vec<Vec2> = doodads.by_id.values().map(|d| Vec2::new(d.pos.x, d.pos.y)).collect();
    b.error = check(data, b.design, b.pos.truncate(), &own, others, &obstacles, &props, terrain.as_deref());
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

    fn at(d: &HousingData, design: u32, pos: Vec2) -> Option<&'static str> {
        check(d, design, pos, &[], std::iter::empty(), &[], &[], None)
    }

    fn with_others(d: &HousingData, design: u32, pos: Vec2, others: &[(Vec2, f32)]) -> Option<&'static str> {
        check(d, design, pos, &[], others.iter().copied(), &[], &[], None)
    }

    #[test]
    fn the_land_its_categories_and_other_houses_decide_where_a_house_may_stand() {
        let d = data();
        assert_eq!(at(&d, 171, Vec2::new(50.0, 50.0)), None);
        // a centre outside every polygon is 0x244 for a restricted category, not 0x70
        assert_eq!(at(&d, 171, Vec2::new(150.0, 50.0)), Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA"));
        assert_eq!(at(&d, 172, Vec2::new(50.0, 50.0)), Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA"));
        // snapped footprints: design 171 is 24 m, design 172 16 m
        assert_eq!(at(&d, 171, Vec2::new(50.0, 50.0)), None);
        assert_eq!(with_others(&d, 171, Vec2::new(50.0, 50.0), &[(Vec2::new(68.0, 50.0), 7.5)]), Some("HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE"));
        assert_eq!(with_others(&d, 171, Vec2::new(50.0, 50.0), &[(Vec2::new(70.0, 50.0), 7.5)]), None);
        let mut d = d;
        d.groups.get_mut(&15).unwrap().houseless = true;
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[1], std::iter::empty(), &[], &[], None), Some("HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION"));
        d.groups.get_mut(&15).unwrap().houseless = false;
        d.groups.get_mut(&15).unwrap().existing_category_id = 18;
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[18], std::iter::empty(), &[], &[], None), Some("HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION"));
        assert_eq!(at(&d, 999, Vec2::new(50.0, 50.0)), Some("INVALID_HOUSE_INFO"));
    }

    #[test]
    fn every_footprint_cell_must_lie_in_one_polygon() {
        let mut d = data();
        d.land.clear();
        d.land.push(Land::new(338, vec![Vec2::new(-1.0, -1.0), Vec2::new(25.0, -1.0), Vec2::new(25.0, 25.0), Vec2::new(-1.0, 25.0)]));
        // design 171: n = 6, 24 m footprint 0..24, all cells inside
        assert_eq!(at(&d, 171, Vec2::new(12.0, 12.0)), None);
        // footprint 4..28: the cells past x = 25 leave the polygon
        assert_eq!(at(&d, 171, Vec2::new(16.0, 12.0)), Some("HOUSE_CANNOT_LOCATE_INVALID_AREA"));
        // the snap puts the lower left corner on a 4 m cell: 16.4 -> 16.0
        assert_eq!(snap_spot(Vec2::new(16.4, 12.4), 12.0), Vec2::new(16.0, 12.0));
        assert_eq!(snap_spot(Vec2::new(15328.4, 14632.33), 12.0), Vec2::new(15328.0, 14632.0));
    }

    #[test]
    fn a_unit_or_doodad_in_the_footprint_refuses_it() {
        let d = data();
        let spot = Vec2::new(50.0, 50.0);
        let refused = |units: &[Vec2], doodads: &[Vec2]| check(&d, 171, spot, &[], std::iter::empty(), units, doodads, None);
        assert_eq!(refused(&[], &[]), None);
        assert_eq!(refused(&[Vec2::new(49.0, 49.0)], &[]), Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT"));
        assert_eq!(refused(&[], &[Vec2::new(49.0, 49.0)]), Some("HOUSE_CANNOT_LOCATE_OVERLAP_UNIT"));
        // the footprint is 38..62 on both axes; its edge does not overlap
        assert_eq!(refused(&[Vec2::new(64.0, 50.0)], &[]), None);
    }

    #[test]
    fn the_terrain_spread_over_six_metres_refuses_the_spot() {
        let (lower, upper) = (Vec2::ZERO, Vec2::splat(24.0));
        assert!(!terrain_too_steep(lower, upper, |_, _| Some(1.0)));
        assert!(!terrain_too_steep(lower, upper, |x, _| Some(if x > 0.0 { 6.0 } else { 0.0 })));
        assert!(terrain_too_steep(lower, upper, |x, _| Some(if x > 0.0 { 6.1 } else { 0.0 })));
        assert!(!terrain_too_steep(lower, upper, |_, _| None));
    }

    #[test]
    fn the_area_338_polygon_refuses_the_old_spot_and_accepts_the_captured_one() {
        let Ok(_) = std::env::var("AA_DB") else { return };
        if !aa_cry::pak_available() {
            return;
        }
        let d = data::load_now("main_world");
        if !d.land.iter().any(|l| l.area_id == 338) || !d.designs.contains_key(&171) {
            return;
        }
        // 2026-10-31-house-spot.md: 24 m footprint across the area's south east notch
        assert_eq!(at(&d, 171, Vec2::new(15338.53, 14632.33)), Some("HOUSE_CANNOT_LOCATE_INVALID_AREA"));
        // 2026-10-31-recapture.md: the ghost snapped to (15328, 14632) and the client accepted it
        let terrain = TerrainQuery::new("main_world");
        assert_eq!(
            check(&d, 171, Vec2::new(15328.4, 14631.8), &[], std::iter::empty(), &[], &[], Some(&terrain)),
            None
        );
    }

    #[test]
    fn the_house_faces_the_cursor_in_the_units_yaw() {
        // north is +Y and yaw 0; west (-X) is a quarter turn counter-clockwise
        assert!(yaw_towards(Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0)).unwrap().abs() < 1e-6);
        assert!((yaw_towards(Vec3::ZERO, Vec3::new(-5.0, 0.0, 0.0)).unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(yaw_towards(Vec3::ZERO, Vec3::new(0.1, 0.1, 0.0)).is_none());
    }
}
