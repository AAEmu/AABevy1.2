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
//! The builder's checks (`39333E20`, `39334060`) set the error the ghost shows: the refused ghost is drawn with
//! `materials/special/housing_build_invalid` (dark red, `housing_build_normal` otherwise). Here, as the server checks
//! (`systems/housing/placement.rs`):
//!
//! * the spot lies on housing land (`HOUSE_CANNOT_LOCATE_INVALID_AREA`) whose group allows the design's category
//!   (`HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA`), and the account's other houses do not break the group's rules
//!   (`houseless`: `HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION`, `existing_category_id`:
//!   `HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION`);
//! * the house's square of land (`garden_radius`) does not overlap another house's (`HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE`).
//!
//! The dominion rules (siege zones, castle land) are left to the server.

use std::sync::atomic::{AtomicBool, Ordering};

use aa_cry::scene::{CryAssets, ModelOptions};
use aa_cry::shade::illum::IllumMaterial;
use aa_cry::workers::Workers;
use aa_net::Houses;
use aa_net::housing::HouseCommand;
use aa_net::model::UnitKind;
use aa_net::state::Units;
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

static BUILDING: AtomicBool = AtomicBool::new(false);

/// The builder runs: the wheel's keys are `builder_rotate_mode`'s.
pub fn building() -> bool {
    BUILDING.load(Ordering::Relaxed)
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
        info!(
            "housing: build design {} at ({:.1}, {:.1}, {:.1}) yaw {:.2} with item {} (tax {})",
            b.design, b.pos.x, b.pos.y, b.pos.z, b.yaw, b.item_id, b.total_tax
        );
        net.write(NetCommand(Command::House(HouseCommand::CreateHouse {
            design_id: b.design,
            x: b.pos.x,
            y: b.pos.y,
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

/// Why `design` cannot stand at `pos` (`ERROR_MSG` name), or `None`.
pub fn check(data: &HousingData, design: u32, pos: Vec2, own_categories: &[u32], others: impl Iterator<Item = (Vec2, f32)>) -> Option<&'static str> {
    let Some(d) = data.designs.get(&design) else { return Some("INVALID_HOUSE_INFO") };
    let mut error = "HOUSE_CANNOT_LOCATE_INVALID_AREA";
    let mut allowed = false;
    for (_, group) in data.groups_at(pos) {
        if !group.categories.contains(&d.category_id) {
            if error == "HOUSE_CANNOT_LOCATE_INVALID_AREA" {
                error = "HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA";
            }
            continue;
        }
        if group.houseless && !own_categories.is_empty() {
            error = "HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION";
            continue;
        }
        if group.existing_category_id != 0 && own_categories.contains(&group.existing_category_id) {
            error = "HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION";
            continue;
        }
        allowed = true;
        break;
    }
    if !allowed {
        return Some(error);
    }
    for (other, radius) in others {
        let extent = d.garden_radius + radius;
        if (pos.x - other.x).abs() < extent && (pos.y - other.y).abs() < extent {
            return Some("HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE");
        }
    }
    None
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
pub fn start(mut pressed: MessageReader<SkillPressed>, mut builder: ResMut<Builder>, player: Query<&Transform, With<LocalPlayer>>, mut signals: MessageWriter<BuilderSignal>) {
    for press in pressed.read() {
        let Some((item_id, template)) = press.item else { continue };
        let Some(design) = data::get().and_then(|d| d.design_by_item.get(&template).copied()) else { continue };
        // the same item again lets go
        if builder.build.as_ref().is_some_and(|b| b.item_id == item_id) {
            builder.end(&mut signals);
            continue;
        }
        builder.end(&mut signals);
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
) {
    BUILDING.store(builder.build.is_some(), Ordering::Relaxed);
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
                signals.write(BuilderSignal::Step);
            }
            Stage::Position => builder.end(&mut signals),
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
            signals.write(BuilderSignal::Step);
        }
        Stage::Rotate => {
            if let Some(error) = b.error {
                signals.write(BuilderSignal::Refused(error));
                return;
            }
            info!("housing: tax of design {} at ({:.1}, {:.1}, {:.1})", b.design, b.pos.x, b.pos.y, b.pos.z);
            net.write(NetCommand(Command::House(HouseCommand::ConstructHouseTax { design_id: b.design, x: b.pos.x, y: b.pos.y, z: b.pos.z })));
            b.stage = Stage::Quote;
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
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    terrain: Option<Res<TerrainQuery>>,
    units: Res<Units>,
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
    match b.stage {
        Stage::Position => {
            if let Some(spot) = spot {
                b.pos = spot;
            }
        }
        Stage::Rotate if pinned.is_none() => {
            if let Some(yaw) = spot.and_then(|s| yaw_towards(b.pos, s)) {
                b.yaw = yaw;
            }
        }
        _ => {}
    }
    let Some(data) = data::get() else { return };
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
    b.error = check(data, b.design, b.pos.truncate(), &own, others);
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
pub fn clear(mut builder: ResMut<Builder>, mut signals: MessageWriter<BuilderSignal>) {
    builder.end(&mut signals);
    BUILDING.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn the_land_its_categories_and_other_houses_decide_where_a_house_may_stand() {
        let d = data();
        let none = std::iter::empty;
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[], none()), None);
        assert_eq!(check(&d, 171, Vec2::new(150.0, 50.0), &[], none()), Some("HOUSE_CANNOT_LOCATE_INVALID_AREA"));
        assert_eq!(check(&d, 172, Vec2::new(50.0, 50.0), &[], none()), Some("HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA"));
        // garden squares: 11 + 7.5 m apart on both axes is needed
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[], [(Vec2::new(68.0, 50.0), 7.5)].into_iter()), Some("HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE"));
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[], [(Vec2::new(69.0, 50.0), 7.5)].into_iter()), None);
        let mut d = d;
        d.groups.get_mut(&15).unwrap().houseless = true;
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[1], none()), Some("HOUSE_CANNOT_OWN_MORE_HOUSELESS_CONDITION"));
        d.groups.get_mut(&15).unwrap().houseless = false;
        d.groups.get_mut(&15).unwrap().existing_category_id = 18;
        assert_eq!(check(&d, 171, Vec2::new(50.0, 50.0), &[18], none()), Some("HOUSE_CANNOT_OWN_MORE_EXISTING_CATEGORY_CONDITION"));
        assert_eq!(check(&d, 999, Vec2::new(50.0, 50.0), &[], none()), Some("INVALID_HOUSE_INFO"));
    }

    #[test]
    fn the_house_faces_the_cursor_in_the_units_yaw() {
        // north is +Y and yaw 0; west (-X) is a quarter turn counter-clockwise
        assert!(yaw_towards(Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0)).unwrap().abs() < 1e-6);
        assert!((yaw_towards(Vec3::ZERO, Vec3::new(-5.0, 0.0, 0.0)).unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(yaw_towards(Vec3::ZERO, Vec3::new(0.1, 0.1, 0.0)).is_none());
    }
}
