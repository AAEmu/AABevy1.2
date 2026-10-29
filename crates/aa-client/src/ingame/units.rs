//! Other units and doodads: spawned when the server announces them and they are near, moved and
//! turned smoothly as the server reports, despawned when removed or far away.
//!
//! The server sends units within its view range (hundreds of metres); rendering all of them would
//! cost more than the town is worth, so a unit gets a model only within [`UNIT_RADIUS`] of the
//! player (doodads: [`DOODAD_RADIUS`]) and loses it again beyond a margin. Models are loaded
//! nearest first under a per frame time budget; the cached assets make the second NPC of a kind
//! cheap.

mod face;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use web_time::Instant;

use aa_anim::{Animator, ClipLibrary, spawn_character};
use aa_cry::scene::CryAssets;
use aa_cry::workers::Workers;
use aa_net::model::{Movement, Unit, actor_flags, alertness, move_flags, stance};
use aa_net::state::{Doodads, Units};
use aa_net::{Event, NetEvent};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion};
use super::creature;
use super::data::Db;
use super::doodad_visual::DoodadVisual;
use super::idle::IdleClips;
use super::mount::{Driven, Rider};
use super::player::{LocalPlayer, Mover, approach_angle, placement};
use super::prepare::{DoodadBuild, UnitBuild, build_unit, plan_unit, prepare_doodad};
use super::remote::{self, Report, Track};
use super::weapons;

/// Units are drawn within this many metres of the player, and dropped again beyond `* 1.25`.
pub const UNIT_RADIUS: f32 = 140.0;
pub const DOODAD_RADIUS: f32 = 160.0;
/// Main thread time per frame for building models (ms); one model is always built.
const SPAWN_BUDGET_MS: f32 = 10.0;
/// While the last frame took longer than this (s), builds wait (up to `SPAWN_MAX_WAIT`) so the game
/// stays playable while a town fills up.
const SPAWN_SLOW_FRAME: f32 = 0.025;
const SPAWN_MAX_WAIT: f32 = 0.2;
/// Unit and doodad preparations on the workers at once; the nearest are started first.
const MAX_JOBS: usize = 16;
/// Units are drawn (and animated) within this distance of the camera (m) and inside the view.
const DRAW_RADIUS: f32 = 110.0;
const ANIMATE_RADIUS: f32 = 60.0;
/// Characters beyond these distances (m) are animated at a lower rate (seconds between updates): 30 Hz, then 15 Hz. Sampling,
/// bone transform propagation, skin extraction and the draw preparation all scale with the number of poses that change.
const ANIMATION_LOD: [(f32, f32); 2] = [(25.0, 1.0 / 30.0), (45.0, 1.0 / 15.0)];
/// Units farther than this (m) do not cast shadows: every shadow cascade draws every skinned part again.
const SHADOW_RADIUS: f32 = 45.0;

/// `AA_ANIM_LOD=0` / `AA_UNIT_SHADOW_RADIUS=<m>`: switches for performance experiments.
fn tuning() -> (bool, f32) {
    static T: std::sync::OnceLock<(bool, f32)> = std::sync::OnceLock::new();
    *T.get_or_init(|| {
        let lod = std::env::var("AA_ANIM_LOD").map_or(true, |v| v != "0");
        let shadow = std::env::var("AA_UNIT_SHADOW_RADIUS").ok().and_then(|v| v.parse().ok()).unwrap_or(SHADOW_RADIUS);
        (lod, shadow)
    })
}

/// A spawned unit, played back from the movement packets the server sent ([`remote::Track`]).
#[derive(Component, Clone)]
pub struct RemoteUnit {
    pub obj_id: u32,
    /// The buffered reports and the playback clock.
    track: Track,
    /// Displayed position / facing (game coordinates, radians).
    pub(super) pos: Vec3,
    pub(super) facing: f32,
    scale: f32,
    /// A player character (8 way run clips, jump and fall) rather than an NPC or a creature (always faces where it goes).
    character: bool,
    /// Its movement says it is in combat (see [`remote::Report::combat`]).
    pub combat: bool,
    /// Its meshes cast shadows (see [`SHADOW_RADIUS`]).
    casting: bool,
}

impl RemoteUnit {
    fn announced(unit: &Unit) -> Self {
        let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
        Self { obj_id: unit.obj_id, track: Track::new(pos, unit.yaw()), pos, facing: unit.yaw(),
            scale: if unit.scale > 0.0 { unit.scale } else { 1.0 }, character: unit.is_character(), combat: false, casting: true }
    }

    /// The same committed movement clock is used with and without a render tree.
    fn advance(&mut self, dt: f32, terrain: &TerrainQuery) -> (remote::Pose, bool) {
        let pose = self.track.sample(dt);
        let mut pos = pose.pos;
        let ground = terrain.ground_at(pos.x, pos.y).map(|ground| ground.height);
        let airborne = pose.airborne && ground.is_none_or(|g| pos.z - g > 0.25);
        if !airborne && let Some(g) = ground.filter(|g| (pos.z - g).abs() < GROUND_SNAP) { pos.z = g; }
        self.pos = pos;
        self.combat = pose.combat;
        self.facing = approach_angle(self.facing, pose.yaw, TURN_RATE * dt);
        (pose, airborne)
    }

    pub(super) fn casts_shadow(&self) -> bool { self.casting }
    #[cfg(test)]
    pub(super) fn fixture(obj_id: u32, pos: Vec3) -> Self {
        Self { obj_id, track: Track::new(pos, 0.0), pos, facing: 0.0, scale: 1.0, character: true, combat: false, casting: false }
    }

    pub(super) fn scale(&self) -> f32 {
        self.scale
    }

    /// A player character, not an NPC or a creature.
    pub(super) fn is_character(&self) -> bool {
        self.character
    }

    /// Where the newest movement report put the unit.
    pub(super) fn latest(&self) -> Vec3 {
        self.track.latest()
    }

    /// Starts the buffered movement over at `pos` (a rider got off).
    pub(super) fn reset_track(&mut self, pos: Vec3, facing: f32) {
        self.track = Track::new(pos, facing);
    }
}

#[derive(Component)]
pub struct RemoteDoodad {
    pub obj_id: u32,
}
/// Apply the current remote distance policy to newly created render trees.
/// The cached actor policy can stay false across a visual replacement.
pub(super) fn no_shadows(world: &mut World, root: Entity) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
        if world.get::<Mesh3d>(entity).is_some() { world.entity_mut(entity).insert(NotShadowCaster); }
    }
}

/// Which server objects have entities. `None` = known but not drawn (too far, or no model).
#[derive(Resource)]
pub struct Views {
    units: HashMap<u32, View>,
    doodads: HashMap<u32, View>,
    source_generation: u64,
    tx: Sender<Built>,
    rx: Mutex<Receiver<Built>>,
    /// Prepared objects waiting for the main thread.
    ready: Vec<Built>,
    /// Jobs on the workers.
    jobs: usize,
    /// In range, wanted, and not started because the workers are busy (last sync).
    waiting: usize,
    /// Counters for the log.
    pub spawned_units: u32,
    pub spawned_doodads: u32,
    pub failed: u32,
}

impl Default for Views {
    fn default() -> Self {
        let (tx, rx) = channel();
        Views { units: HashMap::new(), doodads: HashMap::new(), source_generation: 0, tx, rx: Mutex::new(rx), ready: Vec::new(), jobs: 0, waiting: 0, spawned_units: 0, spawned_doodads: 0, failed: 0 }
    }
}

/// What a worker delivers.
enum Built {
    Unit(u32, u64, UnitBuild),
    Doodad(u32, u64, DoodadBuild),
}

#[derive(Default)]
struct View {
    entity: Option<Entity>,
    /// The model failed to build or the object has none: do not try again.
    no_model: bool,
    /// Why there is no model (for the report).
    why: String,
    /// A worker is preparing it.
    preparing: bool,
    /// `Time::elapsed_secs` when the server last announced the doodad's `time_left` (spawn, phase change).
    announced_at: f32,
    source_generation: u64,
    doodad_source: Option<aa_net::model::Doodad>,
    unit_identity: Option<(aa_net::model::UnitKind, u32)>,
    /// Owns playback only while no RemoteUnit component owns it. Render policy
    /// (distance, missing model or failed preparation) does not retire an actor.
    pending: Option<RemoteUnit>,
    /// Receipt which created this incarnation, not a same-identity refresh.
    announced_after: Option<usize>,
}

impl View {
    fn admit(&mut self, commands: &mut Commands, entity: Entity, source: &Unit) {
        if let Some(mut actor) = self.pending.take() {
            // Playback survives culling, but a freshly built tree has the current
            // source scale and default shadow policy, not the evicted mesh cache.
            actor.scale = if source.scale > 0.0 { source.scale } else { 1.0 };
            actor.character = source.is_character();
            actor.casting = true;
            let placed = placement(actor.pos, actor.facing, actor.scale);
            commands.entity(entity).insert((actor, placed));
        }
        self.entity = Some(entity);
    }

    fn evict(&mut self, commands: &mut Commands, actor: Option<(&RemoteUnit, &Transform)>) {
        if let Some((actor, root)) = actor {
            let mut pending = actor.clone();
            pending.pos = aa_world::bevy_to_cry(root.translation);
            pending.facing = root.rotation.to_euler(EulerRot::YXZ).0;
            self.pending = Some(pending);
        }
        if let Some(entity) = self.entity.take() { commands.entity(entity).despawn(); }
    }
}

/// The existing visual-source identity, shared by receipt-owned presentation
/// state so an ordinary snapshot refresh does not invent an actor replacement.
pub(crate) fn source_identity(unit: &Unit) -> (aa_net::model::UnitKind, u32) {
    (unit.kind.clone(), unit.model_id)
}

impl Views {
    fn announce_unit(&mut self, commands: &mut Commands, unit: &Unit) {
        let identity = source_identity(unit);
        if self.units.get(&unit.obj_id).and_then(|view| view.unit_identity.as_ref()) == Some(&identity) { return; }
        if let Some(entity) = self.units.remove(&unit.obj_id).and_then(|view| view.entity) {
            commands.entity(entity).try_despawn();
        }
        self.source_generation += 1;
        self.units.insert(unit.obj_id, View { source_generation: self.source_generation, unit_identity: Some(identity), pending: Some(RemoteUnit::announced(unit)), ..default() });
    }

    fn finish_unit(&mut self, id: u32, generation: u64) -> Option<&mut View> {
        let view = self.units.get_mut(&id).filter(|view| view.source_generation == generation)?;
        view.preparing = false;
        Some(view)
    }

    fn announce_doodad(&mut self, commands: &mut Commands, doodad: &aa_net::model::Doodad, now: f32) {
        let replace = self.doodads.get(&doodad.obj_id)
            .and_then(|view|view.doodad_source.as_ref())
            .is_none_or(|old| !same_doodad_source(old,doodad));
        if replace {
            if let Some(entity) = self.doodads.remove(&doodad.obj_id).and_then(|view|view.entity) {
                commands.entity(entity).try_despawn();
            }
            self.source_generation += 1;
            self.doodads.insert(doodad.obj_id,View { announced_at:now, source_generation:self.source_generation, doodad_source:Some(doodad.clone()), ..default() });
        } else if let Some(view) = self.doodads.get_mut(&doodad.obj_id) {
            view.announced_at = now;
            view.doodad_source = Some(doodad.clone());
        }
    }

    fn finish_doodad(&mut self, id: u32, generation: u64) -> Option<&mut View> {
        let view = self.doodads.get_mut(&id).filter(|view|view.source_generation==generation)?;
        view.preparing = false;
        Some(view)
    }

    /// The entity drawing the unit `obj_id`, when it is drawn.
    pub fn unit_entity(&self, obj_id: u32) -> Option<Entity> {
        self.units.get(&obj_id).and_then(|v| v.entity)
    }

    pub(crate) fn unit_current(&self, unit: &Unit) -> bool {
        self.units.get(&unit.obj_id).and_then(|view| view.unit_identity.as_ref()) == Some(&source_identity(unit))
    }

    /// Current logical point when the model is absent. The caller still checks
    /// received membership/identity, and a live render root takes precedence.
    pub(crate) fn unit_point(&self, unit: &Unit) -> Option<Vec3> {
        let view = self.units.get(&unit.obj_id)?;
        if view.unit_identity.as_ref()? != &source_identity(unit) { return None; }
        view.pending.as_ref().map(|actor| cry_to_bevy(actor.pos))
    }

    pub fn unit_entities(&self) -> usize {
        self.units.values().filter(|v| v.entity.is_some()).count()
    }

    pub fn doodad_entities(&self) -> usize {
        self.doodads.values().filter(|v| v.entity.is_some()).count()
    }

    /// The entity drawing the doodad `obj_id`, when it is drawn.
    pub fn doodad_entity(&self, obj_id: u32) -> Option<Entity> {
        self.doodads.get(&obj_id).and_then(|v| v.entity)
    }

    /// Source lifetime of the currently rendered doodad root.
    pub(crate) fn doodad_generation(&self, obj_id: u32) -> Option<u64> {
        self.doodads.get(&obj_id).map(|view| view.source_generation)
    }

    #[cfg(test)]
    pub(crate) fn fixture_unit(&mut self, obj_id: u32, entity: Entity) {
        self.units.insert(obj_id, View { entity: Some(entity), ..default() });
    }

    #[cfg(test)]
    pub(crate) fn fixture_actor(&mut self, unit: &Unit, entity: Option<Entity>) {
        self.source_generation += 1;
        self.units.insert(unit.obj_id, View { entity, source_generation: self.source_generation,
            unit_identity: Some(source_identity(unit)),
            pending: entity.is_none().then(|| RemoteUnit::announced(unit)), ..default() });
    }

    #[cfg(test)]
    pub(super) fn fixture_doodad(&mut self, obj_id: u32, entity: Entity) {
        self.source_generation += 1;
        self.doodads.insert(obj_id, View { entity: Some(entity), source_generation: self.source_generation, ..default() });
    }

    /// Lets the doodad `obj_id` be built again: a phase change may give it a model it did not have.
    pub fn retry_doodad(&mut self, obj_id: u32) {
        if let Some(v) = self.doodads.get_mut(&obj_id) {
            v.no_model = false;
        }
    }

    /// The server announced the doodad's `time_left` at `now` (`Time::elapsed_secs`).
    pub fn doodad_announced(&mut self, obj_id: u32, now: f32) {
        if let Some(v) = self.doodads.get_mut(&obj_id) {
            v.announced_at = now;
        }
    }

    /// Why the units the server reports are not drawn: `(beyond the draw radius, model failed, still being built)`, and the
    /// failures as `(object id, reason)`. A unit within range that is none of these has not been looked at yet.
    pub fn census(&self, units: &Units, me: Vec3) -> (usize, usize, usize, Vec<(u32, &str)>) {
        let (mut far, mut failed, mut pending) = (0, 0, 0);
        let mut reasons = Vec::new();
        for (id, unit) in &units.by_id {
            let view = self.units.get(id);
            if view.is_some_and(|v| v.entity.is_some()) {
                continue;
            }
            if view.is_some_and(|v| v.no_model) {
                failed += 1;
                reasons.push((*id, view.map_or("", |v| v.why.as_str())));
            } else if flat_distance(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), me) > UNIT_RADIUS {
                far += 1;
            } else {
                pending += 1;
            }
        }
        (far, failed, pending, reasons)
    }

    /// Objects being prepared or waiting to be spawned.
    pub fn pending(&self) -> usize {
        self.jobs + self.ready.len()
    }

    /// [`Self::pending`] plus the ones in range that no worker has been given yet.
    pub fn outstanding(&self) -> usize {
        self.pending() + self.waiting
    }
}

// Phase/countdown/interaction-state refreshes do not change the source geometry identity.
fn same_doodad_source(a: &aa_net::model::Doodad, b: &aa_net::model::Doodad) -> bool {
    let source = |d: &aa_net::model::Doodad| {
        let mut d = d.clone();
        d.func_group_id=0; d.time_left=0; d.has_loot_item=false; d.quest_glow=0;
        d
    };
    source(a)==source(b)
}


fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.y - b.y).length()
}

/// The root transform of a doodad. The server writes the packet's position in the parent unit's frame when it has a
/// `parent_obj_id` (`aaemu-game` `models/doodad.rs`: `attach_point > 0 || parent_obj_id > 0`), so a parented doodad keeps
/// this as its local transform under the parent's root.
fn doodad_root_transform(doodad: &aa_net::model::Doodad, scale: f32) -> Transform {
    let [x, y, z, w] = doodad.rotation;
    Transform {
        translation: cry_to_bevy(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z)),
        rotation: cry_rot_to_bevy(Quat::from_xyzw(x, y, z, w).normalize()),
        scale: Vec3::splat(scale),
    }
}

/// Where a doodad stands in the world (game coordinates): the packet position for an ordinary doodad, or the parent
/// root's placement applied to the parent-local packet position. A unit parent carries its scale exactly as its
/// `ChildOf` root does; a house is drawn at scale 1 (`housing::render`) and a house whose unit is gone is still placed
/// by [`doodad_world_in`] through the drawn root. `None` while a parented doodad's parent is not announced.
pub(crate) fn doodad_world(doodad: &aa_net::model::Doodad, units: &Units) -> Option<Vec3> {
    doodad_world_in(doodad, units, None)
}

/// [`doodad_world`] with the housing system's drawn roots as the fallback for a house parent that is not in `Units`.
pub(crate) fn doodad_world_in(
    doodad: &aa_net::model::Doodad,
    units: &Units,
    houses: Option<&super::housing::render::DrawnHouses>,
) -> Option<Vec3> {
    if doodad.parent_obj_id == 0 {
        return Some(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z));
    }
    let root = parent_placement(doodad.parent_obj_id, units, houses)?;
    let local = doodad_root_transform(doodad, 1.0).translation;
    Some(bevy_to_cry(root.transform_point(local)))
}

/// The root transform the doodad is parented to. A housing unit's scale is not the house root's: `housing::render`
/// draws the house at its unit's position and yaw with scale 1, and its drawn root is preferred when it exists.
fn parent_placement(
    parent_obj_id: u32,
    units: &Units,
    houses: Option<&super::housing::render::DrawnHouses>,
) -> Option<Transform> {
    if let Some(parent) = units.by_id.get(&parent_obj_id) {
        let pos = Vec3::new(parent.pos.x, parent.pos.y, parent.pos.z);
        if matches!(parent.kind, aa_net::model::UnitKind::Housing { .. }) {
            return Some(
                houses
                    .and_then(|h| h.transform(parent_obj_id))
                    .unwrap_or_else(|| placement(pos, parent.yaw(), 1.0)),
            );
        }
        let scale = if parent.scale > 0.0 {
            parent.scale
        } else {
            1.0
        };
        return Some(placement(pos, parent.yaw(), scale));
    }
    houses?.transform(parent_obj_id)
}

/// The entity drawing a doodad's parent: its unit root, or a drawn house (housing render keeps its own roots).
fn parent_entity(
    doodad: &aa_net::model::Doodad,
    views: &Views,
    houses: Option<&super::housing::render::DrawnHouses>,
) -> Option<Entity> {
    if doodad.parent_obj_id == 0 {
        return None;
    }
    views
        .unit_entity(doodad.parent_obj_id)
        .or_else(|| houses.and_then(|h| h.entity(doodad.parent_obj_id)))
}

/// Creates and removes entities for the units and doodads the server reports. The models are prepared on the worker
/// threads ([`super::prepare`]); this system only sends the jobs and spawns what came back, a few per frame.
#[allow(clippy::too_many_arguments)]
pub fn sync_views(
    mut commands: Commands,
    units: Res<Units>,
    doodads: Res<Doodads>,
    db: Option<Res<Db>>,
    mut views: ResMut<Views>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    player: Query<&Mover, With<LocalPlayer>>,
    remote: Query<(&RemoteUnit, &Transform)>,
    time: Res<Time>,
    mut last_spawn: Local<f32>,
    camera: Query<&GlobalTransform, (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    timers: Option<Res<aa_world::profile::Timers>>,
    roots: Query<(), With<RemoteDoodad>>,
    houses: Option<Res<super::housing::render::DrawnHouses>>,
) {
    let (Some(db), Ok(me)) = (db, player.single()) else { return };
    let _t = timers.as_deref().map(|t| t.time("units.sync"));
    let me = me.pos;
    let started = Instant::now();
    *last_spawn += time.delta_secs();
    // build only when the frame rate can take it
    let may_build = time.delta_secs() < SPAWN_SLOW_FRAME || *last_spawn > SPAWN_MAX_WAIT;
    // units behind the camera can wait
    let facing = camera.single().map(flat_dir).unwrap_or(Vec2::ZERO);

    // what the workers finished
    {
        let arrived: Vec<Built> = {
            let rx = views.rx.lock().unwrap();
            std::iter::from_fn(|| rx.try_recv().ok()).collect()
        };
        views.jobs = views.jobs.saturating_sub(arrived.len());
        views.ready.extend(arrived);
    }

    // A doodad under a parent unit goes away with the parent (Bevy despawns its children). Forget the dead root before
    // the removals below so no command is sent to it, and so it is built again when the parent comes back.
    let orphans: Vec<u32> = views
        .doodads
        .iter()
        .filter_map(|(id, view)| Some((*id, view.entity?)))
        .filter(|(_, entity)| roots.get(*entity).is_err())
        .map(|(id, _)| id)
        .collect();
    for id in orphans {
        if let Some(view) = views.doodads.get_mut(&id) {
            view.entity = None;
        }
    }
    // units and doodads that are gone; `try_despawn` because a parented doodad may already be gone with its unit
    let gone: Vec<u32> = views.units.keys().filter(|id| !units.by_id.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(v) = views.units.remove(&id)
            && let Some(e) = v.entity
        {
            commands.entity(e).try_despawn();
        }
    }
    let gone: Vec<u32> = views.doodads.keys().filter(|id| !doodads.by_id.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(v) = views.doodads.remove(&id)
            && let Some(e) = v.entity
        {
            commands.entity(e).try_despawn();
        }
    }

    // units: follow the server, drop the far ones, collect the ones to prepare
    let mut wanted: Vec<(f32, u32)> = Vec::new();
    for (id, unit) in &units.by_id {
        // houses are drawn by `housing::render` (a prefab per construction stage)
        if matches!(unit.kind, aa_net::model::UnitKind::Housing { .. }) {
            continue;
        }
        let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
        let d = flat_distance(pos, me);
        if !views.units.contains_key(id) { views.announce_unit(&mut commands, unit); }
        let view = views.units.get_mut(id).expect("announced unit source");
        match view.entity {
            Some(e) => {
                if d > UNIT_RADIUS * 1.25 {
                    view.evict(&mut commands, remote.get(e).ok());
                }
            }
            None if !view.no_model && !view.preparing && d <= UNIT_RADIUS => {
                let ahead = Vec2::new(pos.x - me.x, pos.y - me.y).normalize_or_zero().dot(facing_game(facing));
                wanted.push((d + if ahead < -0.2 { 50.0 } else { 0.0 }, *id));
            }
            None => {}
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let loader = cry.loader();
    let mut waiting = wanted.len();
    for (priority, id) in wanted {
        if views.jobs >= MAX_JOBS {
            break;
        }
        let Some(unit) = units.by_id.get(&id) else { continue };
        waiting -= 1;
        let view = views.units.get_mut(&id).expect("wanted unit source");
        view.preparing = true;
        let generation = view.source_generation;
        views.jobs += 1;
        let (unit, db, loader, clips, tx) = (unit.clone(), db.clone(), loader.clone(), clips.clone(), views.tx.clone());
        // a panic must not leave the unit "preparing" for ever
        let panicked = |_| UnitBuild::None("the preparation panicked".into());
        Workers::global().spawn(priority, move || {
            let plan = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plan_unit(&db, &unit))) {
                Ok(plan) => plan,
                Err(e) => return drop(tx.send(Built::Unit(unit.obj_id, generation, panicked(e)))),
            };
            // the browser downloads the models, then their textures, before reading them; natively this runs straight on
            let files = if cfg!(target_arch = "wasm32") { plan.files(&db, &unit) } else { Vec::new() };
            Workers::global().then_after(priority, files.clone(), move || {
                let textures = aa_cry::deps::texture_files(&files);
                Workers::global().then_after(priority, textures, move || {
                    let build = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_unit(&db, &loader, &clips, &unit, plan)))
                        .unwrap_or_else(panicked);
                    let _ = tx.send(Built::Unit(unit.obj_id, generation, build));
                });
            });
        });
    }

    // doodads: drop the far ones, collect the ones to prepare
    let mut wanted: Vec<(f32, u32)> = Vec::new();
    for (id, doodad) in &doodads.by_id {
        let d = doodad_world_in(doodad, &units, houses.as_deref()).map_or(f32::MAX, |p| flat_distance(p, me));
        if views.doodads.get(id).and_then(|view|view.doodad_source.as_ref()).is_none_or(|old|!same_doodad_source(old,doodad)) {
            views.announce_doodad(&mut commands,doodad,time.elapsed_secs());
        }
        let parented = doodad.parent_obj_id != 0;
        // a doodad attached to a unit waits for the parent root: its packet position is parent-local
        let parent_waiting = parented && parent_entity(doodad, &views, houses.as_deref()).is_none();
        let view = views.doodads.get_mut(id).expect("announced doodad source");
        match view.entity {
            Some(e) if d > DOODAD_RADIUS * 1.25 || (parented && parent_waiting) => {
                // its parent root may have been despawned this frame, taking the doodad with it
                commands.entity(e).try_despawn();
                view.entity = None;
            }
            Some(_) => {}
            None if !view.no_model && !view.preparing && !parent_waiting && d <= DOODAD_RADIUS => {
                wanted.push((d, *id))
            }
            None => {}
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    waiting += wanted.len();
    for (priority, id) in wanted {
        if views.jobs >= MAX_JOBS {
            break;
        }
        let Some(doodad) = doodads.by_id.get(&id) else { continue };
        waiting -= 1;
        let view = views.doodads.get_mut(&id).expect("wanted doodad source");
        view.preparing = true;
        let generation = view.source_generation;
        views.jobs += 1;
        let (template, group, db, loader, tx) = (doodad.template_id, doodad.func_group_id, db.clone(), loader.clone(), views.tx.clone());
        Workers::global().spawn(priority, move || {
            // the browser downloads the model, then its textures, first (see the units above)
            let files = match db.doodad_phase_visual(template, group) {
                Some(path) if cfg!(target_arch = "wasm32") => aa_cry::deps::model_files(&path, None),
                _ => Vec::new(),
            };
            Workers::global().then_after(priority, files.clone(), move || {
                let textures = aa_cry::deps::texture_files(&files);
                Workers::global().then_after(priority, textures, move || {
                    let build = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_doodad(&db, &loader, template, group)))
                        .unwrap_or(DoodadBuild { path: String::new(), model: None, func_group_id: group, growth: None });
                    let _ = tx.send(Built::Doodad(id, generation, build));
                });
            });
        });
    }

    views.waiting = waiting;

    // spawn what is ready, nearest first, while the frame can take it
    let mut ready = std::mem::take(&mut views.ready);
    let distance = |b: &Built| match b {
        Built::Unit(id, _, _) => units.by_id.get(id).map_or(f32::MAX, |u| flat_distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z), me)),
        Built::Doodad(id, _, _) => doodads
            .by_id
            .get(id)
            .and_then(|d| doodad_world_in(d, &units, houses.as_deref()))
            .map_or(f32::MAX, |p| flat_distance(p, me)),
    };
    ready.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    let mut later = Vec::new();
    for built in ready {
        if !may_build || started.elapsed().as_secs_f32() * 1000.0 > SPAWN_BUDGET_MS {
            later.push(built);
            continue;
        }
        match built {
            Built::Unit(id, generation, build) => {
                let Some(view) = views.finish_unit(id, generation) else { continue };
                let Some(unit) = units.by_id.get(&id) else { continue };
                if view.entity.is_some() || flat_distance(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z), me) > UNIT_RADIUS * 1.25 {
                    continue;
                }
                *last_spawn = 0.0;
                let t0 = Instant::now();
                let t_spawn = timers.as_deref().map(|t| t.time("units.spawn_unit"));
                let entity = spawn_unit(&mut commands, &mut cry, &clips, unit, build);
                drop(t_spawn);
                let entity = entity.inspect_err(|why| {
                    warn!("unit {} {:?} {:?} (model {}) is not drawn: {why}", unit.obj_id, unit.name, unit.kind, unit.model_id);
                });
                let ms = t0.elapsed().as_secs_f32() * 1000.0;
                if ms > 30.0 {
                    debug!("spawning unit {} ({:?}) took {ms:.0} ms", unit.obj_id, unit.kind);
                }
                let view = views.units.entry(id).or_default();
                match entity {
                    Ok(e) => {
                        view.admit(&mut commands, e, unit);
                        views.spawned_units += 1;
                    }
                    Err(why) => {
                        view.no_model = true;
                        view.why = why;
                        views.failed += 1;
                    }
                }
            }
            Built::Doodad(id, generation, build) => {
                let Some(doodad) = doodads.by_id.get(&id) else { continue };
                let parent = parent_entity(doodad, &views, houses.as_deref());
                // Reject old incarnations before touching the current view's preparing flag.
                let Some(view) = views.finish_doodad(id,generation) else { continue };
                // a phase change while the worker ran: prepare it again with the new phase
                if build.func_group_id != doodad.func_group_id {
                    continue;
                }
                let world = doodad_world_in(doodad, &units, houses.as_deref());
                if view.entity.is_some() || world.is_none_or(|p| flat_distance(p, me) > DOODAD_RADIUS * 1.25) {
                    continue;
                }
                // the parent unit is not drawn yet: keep the prepared model and wait (its packet position is local)
                if doodad.parent_obj_id != 0 && parent.is_none() {
                    view.preparing = true;
                    later.push(Built::Doodad(id, generation, build));
                    continue;
                }
                let t_doodad = timers.as_deref().map(|t| t.time("units.spawn_doodad"));
                let DoodadBuild { path, model: prepared, func_group_id, growth } = build;
                // the announced `time_left` has been running down since it was announced, also while the doodad was not drawn
                let since_announced_ms = (time.elapsed_secs() - view.announced_at) * 1000.0;
                let entity = prepared.map(|prepared| {
                    let spawned = super::doodad_model::spawn(&mut commands, &mut cry, prepared);
                    let model = spawned.entity;
                    let base_scale = if doodad.scale > 0.0 { doodad.scale } else { 1.0 };
                    let remaining_ms = (doodad.time_left as f32 - since_announced_ms).max(0.0);
                    let total_ms = growth.map_or(0.0, |g| g.total_ms(remaining_ms, true));
                    let scale = base_scale * growth.map_or(1.0, |g| g.scale_at(remaining_ms, total_ms));
                    let transform = doodad_root_transform(doodad, scale);
                    let visual = DoodadVisual { path: path.clone(), model: Some(model), func_group_id, rendered_func_group_id: func_group_id, bond_helpers: spawned.helpers, growth, remaining_ms, total_ms, base_scale };
                    let mut root = commands.spawn((Name::new(format!("doodad {}", doodad.template_id)), RemoteDoodad { obj_id: id }, visual, transform, Visibility::default()));
                    if let Some(parent) = parent {
                        root.insert(ChildOf(parent));
                    }
                    let root = root.id();
                    commands.entity(model).insert(ChildOf(root));
                    root
                });
                drop(t_doodad);
                let view = views.doodads.entry(id).or_default();
                match entity {
                    Some(e) => {
                        view.entity = Some(e);
                        views.spawned_doodads += 1;
                    }
                    None => view.no_model = true,
                }
            }
        }
    }
    views.ready = later;
}

/// Builds the entity of one unit from what the worker prepared. `None` when there is nothing we can draw.
pub(super) fn spawn_unit(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, unit: &Unit, build: UnitBuild) -> Result<Entity, String> {
    let scale = if unit.scale > 0.0 { unit.scale } else { 1.0 };
    let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let transform = placement(pos, unit.yaw(), scale);
    debug!("unit {} {:?} {:?} model {} at ({:.1}, {:.1}, {:.1}) scale {:.2}", unit.obj_id, unit.name, unit.kind, unit.model_id, pos.x, pos.y, pos.z, scale);
    let entity = match build {
        UnitBuild::Character { assets, models, carried, applied, wield, clips: set, pose, tools } => {
            for m in models {
                cry.finish_model(m);
            }
            let _t = aa_cry::prof::scope("unit.spawn_character");
            let root = match spawn_character(commands, cry, clips, *assets, transform) {
                Ok(r) => r,
                Err(e) => return Err(format!("cannot spawn the character: {e}")),
            };
            let mut entity = commands.entity(root);
            entity.insert(Motion::default());
            entity.insert((applied, wield));
            if !carried.is_empty() {
                entity.insert(weapons::Carried(carried));
            }
            if let Some(set) = set {
                entity.insert(Locomotion::new((*set).clone()));
            }
            if pose.is_some() {
                entity.insert(IdleClips::with_tools(pose, None, tools));
            }
            root
        }
        UnitBuild::Creature { file, cdf, models } => {
            for m in models {
                cry.finish_model(m);
            }
            let _t = aa_cry::prof::scope("unit.spawn_creature");
            creature::spawn(commands, cry, clips, &file, cdf.as_deref(), transform).ok_or_else(|| format!("{file} cannot be spawned (no skeleton model or unreadable)"))?
        }
        UnitBuild::Vehicle(pieces) => {
            let _t = aa_cry::prof::scope("unit.spawn_vehicle");
            super::vehicle::spawn(commands, cry, clips, pieces, transform)
        }
        UnitBuild::Effects(effects) => {
            let root = commands.spawn((transform, Visibility::default())).id();
            for e in effects {
                // looping (the gate's effects are continuous), primed so the gate stands at once
                let effect = aa_fx::Effect { scale: e.scale, speed_scale: e.speed_scale, count_scale: e.count_scale, prime: e.prime, no_body: true, ..aa_fx::Effect::new(e.effect.clone()) };
                commands.spawn((effect, Transform { translation: cry_to_bevy(e.pos), rotation: cry_rot_to_bevy(e.rot), ..default() }, ChildOf(root)));
            }
            root
        }
        UnitBuild::None(why) => return Err(why),
    };
    commands.entity(entity).insert((
        Name::new(format!("unit {} {}", unit.obj_id, unit.name)),
        RemoteUnit {
            obj_id: unit.obj_id,
            track: Track::new(pos, unit.yaw()),
            pos,
            facing: unit.yaw(),
            scale,
            character: unit.is_character(),
            combat: false,
            casting: true,
        },
    ));
    // different idle phases, so a crowd does not breathe in step
    let obj_id = unit.obj_id;
    commands.queue(move |world: &mut World| {
        if let Some(mut a) = world.get_mut::<Animator>(entity)
            && let Some(cur) = a.current.as_mut()
        {
            cur.time = (unit_hash(obj_id) % 3000) as f32 / 1000.0;
        }
    });
    Ok(entity)
}

fn unit_hash(v: u32) -> u32 {
    v.wrapping_mul(2654435761) >> 7
}

/// The systems that follow the movement packets and the idle poses; they run around the main chain of `sync_views`,
/// `move_units` and `cull_units` (registered by [`super::InGamePlugin`]) and [`super::anim::drive_locomotion`].
pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            receive_moves.after(super::seating::track).after(super::world_switch::clear_old_units).before(sync_views).before(move_units),
            (super::mount::fake_mount, super::mount::start_stop_riding, super::mount::pose_riders, super::mount::step_dismounts).chain().after(move_units),
            super::idle::posture_changes,
            super::idle::remote_idles.after(super::anim::drive_locomotion),
            face::select.after(cull_units),
        )
            .run_if(super::world_screens),
    );
}

/// What the movement packet says, for the track.
fn report_of(m: &Movement) -> Report {
    // standing on a deck or a vehicle, the packet's position is relative to it: the world position is the next block's
    let pos = m.support.as_ref().and_then(|s| s.world).map_or(m.pos, |(world, _)| world);
    // the air: a jump, or no ground under the feet (neither solid ground nor an object, and not swimming or flying)
    let airborne = m.move_type <= 1
        && (m.flags & move_flags::JUMPING == move_flags::JUMPING
            || m.actor_flags & actor_flags::JUMPING != 0
            || (m.actor_flags & (actor_flags::STANDING_ON_SOLID | actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT) == 0
                && !matches!(m.stance, stance::SWIM | stance::CO_SWIM | stance::FLY)));
    let combat = m.flags & move_flags::IN_COMBAT != 0 || m.alertness == alertness::COMBAT;
    Report { pos: Vec3::new(pos.x, pos.y, pos.z), yaw: m.yaw(), server_ms: m.time, airborne, combat }
}

/// Fold source lifetime and movement in receipt order. An old report cannot
/// enter a replacement with the same object ID; unrendered actors keep playback.
pub fn receive_moves(mut commands: Commands, mut events: MessageReader<NetEvent>, mut views: ResMut<Views>, mut units: Query<&mut RemoteUnit>, time: Res<Time>, mut seats: Option<ResMut<super::seating::Seats>>) {
    let now = time.elapsed_secs();
    for (NetEvent(event), message_id) in events.read_with_id() {
        match event {
            Event::UnitsRemoved(ids) => for id in ids {
                if let Some(entity) = views.units.remove(id).and_then(|view| view.entity) { commands.entity(entity).try_despawn(); }
            },
            Event::UnitSpawned(unit) => {
                let before = views.units.get(&unit.obj_id).map(|view| view.source_generation);
                views.announce_unit(&mut commands, unit);
                let view = views.units.get_mut(&unit.obj_id).expect("announced unit");
                if Some(view.source_generation) != before { view.announced_after = Some(message_id.id); }
            },
            Event::DoodadsRemoved(ids) => for id in ids {
                if let Some(entity)=views.doodads.remove(id).and_then(|view|view.entity) { commands.entity(entity).try_despawn(); }
            },
            Event::DoodadsSpawned(doodads) => for doodad in doodads { views.announce_doodad(&mut commands,doodad,time.elapsed_secs()); },
            Event::DoodadPhaseChanged { obj_id, .. }
            | Event::DoodadChanged { obj_id, .. }
            | Event::DoodadOriginator { obj_id, .. } => {
                if let Some(source)=views.doodads.get_mut(obj_id).and_then(|view|view.doodad_source.as_mut()) {
                    match event {
                        Event::DoodadPhaseChanged { func_group_id,time_left,puzzle_group,item_template_id,.. } => {
                            source.func_group_id = *func_group_id; source.time_left = *time_left;
                            source.puzzle_group = *puzzle_group; source.item_template_id = *item_template_id;
                        }
                        Event::DoodadChanged { data,.. } => source.data = *data,
                        Event::DoodadOriginator { new_owner_id,.. } => source.owner_id = *new_owner_id,
                        _ => unreachable!(),
                    }
                }
            }

            _ => {},
        }
        let obj_id = match event {
            Event::UnitMoved { obj_id, .. } | Event::UnitDeath { obj_id } | Event::Resurrected { obj_id, .. } => obj_id,
            _ => continue,
        };
        let Some(view) = views.units.get_mut(obj_id) else { continue };
        let mut rendered = view.entity.and_then(|entity| units.get_mut(entity).ok());
        let unit = if let Some(unit) = rendered.as_deref_mut() { unit } else {
            let Some(unit) = view.pending.as_mut() else { continue };
            unit
        };
        match event {
            Event::UnitMoved { movement, .. } if movement.move_type != 4 => {
                // A relative-only attachment report does not establish a world point.
                if movement.support.as_ref().is_some_and(|support| support.world.is_none()) { continue; }
                let report = report_of(movement);
                if seats.as_mut().is_some_and(|seats| seats.remote_report_after_release(*obj_id, message_id.id, view.announced_after)) {
                    unit.reset_track(report.pos, report.yaw);
                }
                unit.track.push(now, report);
            }
            Event::UnitDeath { .. } => unit.track = Track::new(unit.pos, unit.facing),
            Event::Resurrected { pos, yaw, .. } => {
                unit.pos = Vec3::new(pos.x, pos.y, pos.z);
                unit.facing = *yaw;
                unit.track = Track::new(unit.pos, unit.facing);
                // Apply the same resurrection commit to rendered roots even on
                // a zero-delta frame; pending actors already own this world pose.
                if let Some(entity) = view.entity {
                    commands.entity(entity).insert(placement(unit.pos, unit.facing, unit.scale));
                }
            }
            _ => {}
        }
    }
}

/// `AA_UNIT_SMOOTHNESS=1`: logs, every 5 s, how evenly the units that move are carried along (their speed from frame to frame
/// against the speed of the path): the mean, the spread and the extremes of `displayed speed / path speed`.
fn smoothness_wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AA_UNIT_SMOOTHNESS").is_ok_and(|v| v != "0"))
}

#[derive(Default)]
pub struct Smoothness {
    /// Characters and the others.
    ratios: [Vec<f32>; 2],
    since: f32,
    frames: u32,
}

impl Smoothness {
    fn add(&mut self, character: bool, shown: f32, path: f32) {
        if path > 0.5 {
            self.ratios[usize::from(character)].push(shown / path);
        }
    }

    fn report(&mut self, dt: f32) {
        self.since += dt;
        self.frames += 1;
        if self.since < 5.0 {
            return;
        }
        for (name, ratios) in ["npcs", "players"].iter().zip(&self.ratios) {
            if ratios.is_empty() {
                continue;
            }
            let n = ratios.len() as f32;
            let mean = ratios.iter().sum::<f32>() / n;
            let sd = (ratios.iter().map(|r| (r - mean).powi(2)).sum::<f32>() / n).sqrt();
            // the middle 90 %: a hitch or a start / stop is one or two frames
            let mut sorted = ratios.clone();
            sorted.sort_by(f32::total_cmp);
            let (p5, p95) = (sorted[sorted.len() / 20], sorted[sorted.len() * 19 / 20]);
            info!("unit smoothness {name}: {} samples in {} frames, displayed / path speed mean {mean:.3} sd {sd:.3} p5 {p5:.2} p95 {p95:.2}", ratios.len(), self.frames);
        }
        *self = Smoothness::default();
    }
}

/// How fast a unit turns to face where the server says it faces (rad/s); the actor models turn at 360 degrees a second.
const TURN_RATE: f32 = 8.0;
/// A unit within this height (m) of the terrain stands on it; one that is higher is on a bridge, a deck or in the air.
const GROUND_SNAP: f32 = 1.0;

/// Moves the displayed units along their tracks: the position the server reported, turned to face where the server says,
/// set on the terrain, and the speed and direction of travel handed to the locomotion clips ([`Motion`]). Riders sit on their
/// mounts and the mate the player drives goes with the player ([`super::mount`]).
#[allow(clippy::type_complexity)]
pub fn move_units(
    time: Res<Time>,
    terrain: Res<TerrainQuery>,
    mut views: ResMut<Views>,
    mut query: Query<(&mut RemoteUnit, &mut Transform, Option<&mut Motion>), (Without<Rider>, Without<Driven>, Without<super::seating::Seated>)>,
    timers: Option<Res<aa_world::profile::Timers>>,
    mut smoothness: Local<Smoothness>,
) {
    let _t = timers.as_deref().map(|t| t.time("units.move"));
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    for view in views.units.values_mut().filter(|view| view.entity.is_none()) {
        if let Some(actor) = view.pending.as_mut() { actor.advance(dt, &terrain); }
    }
    let measure = smoothness_wanted();
    for (mut unit, mut transform, motion) in &mut query {
        let before = unit.pos;
        let (pose, airborne) = unit.advance(dt, &terrain);
        let pos = unit.pos;
        if measure {
            smoothness.add(unit.character, flat_distance(before, pos) / dt, pose.speed);
            if unit.character && pose.speed > 0.5 {
                trace!("unit {} t {:.3} dt {:.4} shown {:.3} path {:.3} clock {:.3} delay {:.3}", unit.obj_id, time.elapsed_secs(), dt, flat_distance(before, pos) / dt, pose.speed, unit.track.clock(), unit.track.delay());
            }
        }
        // only a real move touches the transform: a changed root propagates through every bone and part of the unit and
        // re-extracts its meshes, and most of a town stands still
        let placed = placement(unit.pos, unit.facing, unit.scale);
        if !(transform.translation.abs_diff_eq(placed.translation, 1e-4) && transform.rotation.abs_diff_eq(placed.rotation, 1e-6) && transform.scale == placed.scale) {
            *transform = placed;
        }
        if let Some(mut m) = motion {
            let moving = pose.speed > remote::STANDING && !airborne;
            let local_dir = if !moving {
                Vec2::ZERO
            } else if unit.character {
                remote::local_dir(pose.heading, pose.yaw)
            } else {
                Vec2::Y
            };
            *m = Motion { speed: if moving { pose.speed } else { 0.0 }, local_dir, airborne: airborne && unit.character };
        }
    }
    if measure {
        smoothness.report(dt);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn doodad_replacement_rejects_old_workers_before_touching_current_view() {
        let source = aa_net::model::Doodad {
            obj_id:7,template_id:1236,owner_obj_id:0,parent_obj_id:0,attach_point:0,
            pos:aa_net::Pos { x:1.0,y:2.0,z:3.0 },rotation:[0.0,0.0,0.0,1.0],scale:1.0,
            has_loot_item:false,func_group_id:1951,owner_id:0,ucc_id:0,item_template_id:0,type2:0,
            time_left:5000,plant_time:0,quest_glow:0,family:0,puzzle_group:0,owner_type:0,owner_db_id:0,data:0,
        };
        let mut app = App::new();
        app.init_resource::<Time>().init_resource::<Views>().add_message::<NetEvent>().add_systems(Update,receive_moves);
        app.world_mut().write_message(NetEvent(Event::DoodadsSpawned(vec![source.clone()])));
        app.update();
        let generation=app.world().resource::<Views>().doodads[&7].source_generation;
        let entity=app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Views>().doodads.get_mut(&7).unwrap().entity=Some(entity);
        let mut refresh=source.clone(); refresh.time_left=4000;refresh.func_group_id=1952;
        app.world_mut().write_message(NetEvent(Event::DoodadsSpawned(vec![refresh.clone()])));
        app.update();
        assert_eq!(app.world().resource::<Views>().doodads[&7].source_generation,generation);
        assert!(app.world().get_entity(entity).is_ok(),"a countdown/phase refresh preserves current geometry until swap");
        let mut received=Doodads::default();
        received.apply(&Event::DoodadsSpawned(vec![refresh]));
        for event in [
            Event::DoodadPhaseChanged { obj_id:7,func_group_id:1953,time_left:3000,puzzle_group:42,item_template_id:99 },
            Event::DoodadChanged { obj_id:7,data:77 },
            Event::DoodadOriginator { obj_id:7,new_owner_id:88,new_faction:0 },
        ] {
            received.apply(&event);
            app.world_mut().write_message(NetEvent(event));
        }
        app.update();
        let view=&app.world().resource::<Views>().doodads[&7];
        assert_eq!(view.source_generation,generation);
        assert_eq!(view.doodad_source.as_ref(),received.by_id.get(&7),"receipt-local deltas agree with the actual reducer before final-state fallback");
        assert!(same_doodad_source(view.doodad_source.as_ref().unwrap(),&received.by_id[&7]));
        assert!(app.world().get_entity(entity).is_ok(),"phase metadata/owner/data changes do not evict the old geometry");
        // Removal and identical recreation in one receipt batch cannot inherit the old view.
        app.world_mut().write_message(NetEvent(Event::DoodadsRemoved(vec![7])));
        app.world_mut().write_message(NetEvent(Event::DoodadsSpawned(vec![source.clone()])));
        app.update();
        assert!(app.world().get_entity(entity).is_err());
        let next=app.world().resource::<Views>().doodads[&7].source_generation;
        assert_ne!(next,generation);
        {
            let mut views=app.world_mut().resource_mut::<Views>();
            views.doodads.get_mut(&7).unwrap().preparing=true;
            assert!(views.finish_doodad(7,generation).is_none());
            assert!(views.doodads[&7].preparing,"stale completion must not release the new worker");
        }
        let mut replacement=source;replacement.template_id=8065;
        app.world_mut().write_message(NetEvent(Event::DoodadsSpawned(vec![replacement])));
        app.update();
        let mut views=app.world_mut().resource_mut::<Views>();
        assert_ne!(views.doodads[&7].source_generation,next);
        assert!(views.finish_doodad(7,next).is_none());
        assert_eq!(views.doodads[&7].doodad_source.as_ref().unwrap().template_id,8065);
    }

    #[test]
    fn unit_replacement_rejects_old_workers_before_clearing_current_preparation() {
        let mut app = App::new();
        app.init_resource::<Time>().init_resource::<Views>().add_message::<NetEvent>().add_systems(Update, receive_moves);
        let unit = super::super::seating::fixture_unit(7, 42, None);
        app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(unit.clone()))));
        app.update();
        let original = app.world().resource::<Views>().units[&7].source_generation;
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Views>().units.get_mut(&7).unwrap().entity = Some(entity);
        // Repeated same-actor snapshots do not recreate a model solely for changing health/position.
        let mut refresh = unit.clone(); refresh.hp -= 1; refresh.pos.x += 2.0;
        app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(refresh))));
        app.update();
        assert!(app.world().get_entity(entity).is_ok());
        assert_eq!(app.world().resource::<Views>().units[&7].source_generation, original);
        let mut replacement = unit.clone(); replacement.kind = aa_net::model::UnitKind::Character { char_id: 99 };
        app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(replacement))));
        app.update();
        assert!(app.world().get_entity(entity).is_err());
        let next = app.world().resource::<Views>().units[&7].source_generation;
        assert_ne!(original, next);
        {
            let mut views = app.world_mut().resource_mut::<Views>();
            views.units.get_mut(&7).unwrap().preparing = true;
            assert!(views.finish_unit(7, original).is_none());
            assert!(views.units[&7].preparing);
        }
        app.world_mut().write_message(NetEvent(Event::UnitsRemoved(vec![7])));
        app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(unit))));
        app.update();
        assert_ne!(app.world().resource::<Views>().units[&7].source_generation, next);
        assert!(app.world_mut().resource_mut::<Views>().finish_unit(7, next).is_none());
    }

    #[test]
    fn death_stops_interpolation_and_resurrection_restarts_at_the_announced_pose() {
        let mut app = App::new();
        app.init_resource::<Time>().init_resource::<Views>().add_message::<NetEvent>().add_systems(Update, receive_moves);
        let pos = Vec3::new(3.0, 4.0, 5.0);
        let mut track = Track::new(pos, 1.0);
        track.push(0.0, Report { pos: pos + Vec3::X, yaw: 1.0, server_ms: 100, airborne: false, combat: true });
        let entity = app.world_mut().spawn(RemoteUnit { obj_id: 7, track, pos, facing: 1.0, scale: 1.0, character: true, combat: true, casting: false }).id();
        app.world_mut().resource_mut::<Views>().units.insert(7, View { entity: Some(entity), ..View::default() });
        app.world_mut().write_message(NetEvent(Event::UnitDeath { obj_id: 7 }));
        app.update();
        let pose = app.world().get::<RemoteUnit>(entity).unwrap().track.pose_at(2.0);
        assert_eq!(pose.pos, pos);
        assert_eq!(pose.speed, 0.0);
        assert!(!pose.combat);
        let resurrect = aa_net::Pos { x: 30.0, y: 40.0, z: 50.0 };
        app.world_mut().write_message(NetEvent(Event::Resurrected { obj_id: 7, pos: resurrect, yaw: 2.0 }));
        app.update();
        let unit = app.world().get::<RemoteUnit>(entity).unwrap();
        let pose = unit.track.pose_at(2.0);
        assert_eq!((pose.pos, pose.yaw, pose.speed), (Vec3::new(30.0, 40.0, 50.0), 2.0, 0.0));
        assert_eq!((unit.pos, unit.facing), (pose.pos, pose.yaw));
    }

    fn doodad_source(parent_obj_id: u32, pos: aa_net::Pos) -> aa_net::model::Doodad {
        aa_net::model::Doodad {
            obj_id: 7,
            template_id: 3446,
            owner_obj_id: 0,
            parent_obj_id,
            attach_point: 9,
            pos,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 1,
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
    fn a_parented_doodad_is_placed_in_the_parents_frame_and_an_ordinary_one_in_the_world() {
        let mut units = Units::default();
        let mut parent = super::super::seating::fixture_unit(60, 42, None);
        parent.pos = aa_net::Pos::new(100.0, 200.0, 30.0);
        parent.rotation = [0.0, 0.0, std::f32::consts::FRAC_PI_2];
        // the parent root's scale scales the parent-local offset, as `ChildOf` does
        parent.scale = 1.5;
        let yaw = parent.yaw();
        units.by_id.insert(60, parent);

        let parented = doodad_source(60, aa_net::Pos::new(1.0, 2.0, 3.0));
        let local = doodad_root_transform(&parented, 1.0);
        let parent_transform = placement(Vec3::new(100.0, 200.0, 30.0), yaw, 1.5);
        let expected = aa_world::bevy_to_cry(parent_transform.transform_point(local.translation));
        let world = doodad_world(&parented, &units).expect("the parent unit is announced");
        assert!(
            world.distance(expected) < 1e-4,
            "world {world:?} != {expected:?}"
        );

        let world_doodad = doodad_source(0, aa_net::Pos::new(1.0, 2.0, 3.0));
        assert_eq!(
            doodad_world(&world_doodad, &units),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );
        let rooted = doodad_root_transform(&world_doodad, 1.0);
        let world = placement(Vec3::new(1.0, 2.0, 3.0), 0.0, 1.0);
        assert_eq!(rooted.translation, world.translation);
        assert!(rooted.rotation.abs_diff_eq(world.rotation, 1e-6));
        assert_eq!(rooted.scale, world.scale);
    }

    #[test]
    fn a_house_parented_doodad_uses_the_drawn_house_root_not_the_unit_scale() {
        let mut units = Units::default();
        let mut house = super::super::seating::fixture_unit(80, 42, None);
        house.kind = aa_net::model::UnitKind::Housing {
            tl_id: 1,
            template_id: 2,
            build_step: 0,
        };
        house.pos = aa_net::Pos::new(100.0, 200.0, 30.0);
        house.rotation = [0.0, 0.0, std::f32::consts::FRAC_PI_2];
        // housing::render draws the house root at scale 1, not the unit's scale
        house.scale = 2.5;
        let yaw = house.yaw();
        units.by_id.insert(80, house);
        let doodad = doodad_source(80, aa_net::Pos::new(1.0, 2.0, 3.0));
        let local = doodad_root_transform(&doodad, 1.0).translation;
        let root = placement(Vec3::new(100.0, 200.0, 30.0), yaw, 1.0);
        let expected = aa_world::bevy_to_cry(root.transform_point(local));
        let world = doodad_world(&doodad, &units).expect("the house unit is announced");
        assert!(
            world.distance(expected) < 1e-4,
            "world {world:?} != {expected:?}"
        );
        // the unit row can be gone while the drawn house root still exists
        let mut houses = super::super::housing::render::DrawnHouses::default();
        houses.fixture_house(80, root);
        units.by_id.clear();
        let fallback =
            doodad_world_in(&doodad, &units, Some(&houses)).expect("the house root is drawn");
        assert!(
            fallback.distance(expected) < 1e-4,
            "fallback {fallback:?} != {expected:?}"
        );
    }

    #[test]
    fn a_parented_doodad_root_goes_away_with_its_parent() {
        let mut app = App::new();
        let parent = app.world_mut().spawn_empty().id();
        let child = app
            .world_mut()
            .spawn((RemoteDoodad { obj_id: 7 }, ChildOf(parent)))
            .id();
        app.update();
        app.world_mut().entity_mut(parent).despawn();
        app.update();
        assert!(app.world().get_entity(parent).is_err());
        assert!(
            app.world().get_entity(child).is_err(),
            "the child root is despawned with its parent"
        );
    }

    #[test]
    fn leaving_the_game_despawns_parented_doodads_with_their_unit_root() {
        let mut app = App::new();
        app.add_systems(Update, super::super::leave_game);
        let unit = app.world_mut().spawn(RemoteUnit::fixture(60, Vec3::ZERO)).id();
        let doodad = app
            .world_mut()
            .spawn((RemoteDoodad { obj_id: 7 }, ChildOf(unit)))
            .id();
        app.update();
        assert!(app.world().get_entity(unit).is_err(), "the unit root is gone");
        assert!(
            app.world().get_entity(doodad).is_err(),
            "the doodad goes with its parent"
        );
        let mut doodads = app.world_mut().query::<&RemoteDoodad>();
        assert_eq!(
            doodads.iter(app.world()).count(),
            0,
            "no orphan doodad entity is left behind"
        );
    }
}

/// Horizontal facing of a camera as a Bevy-space (x, z) vector.
fn flat_dir(camera: &GlobalTransform) -> Vec2 {
    let f = camera.forward();
    Vec2::new(f.x, f.z).normalize_or_zero()
}

/// A Bevy-space (x, z) direction as a game (x east, y north) one.
fn facing_game(v: Vec2) -> Vec2 {
    Vec2::new(v.x, -v.y)
}

/// A unit whose animation is switched off while nobody sees it (aa-anim's `animate` runs for every
/// [`Animator`], so the component is parked here).
#[derive(Component)]
pub struct Asleep(Animator);

impl Asleep {
    pub(super) fn animator(&self) -> &Animator { &self.0 }
    pub(super) fn animator_mut(&mut self) -> &mut Animator { &mut self.0 }
    pub(super) fn into_animator(self) -> Animator { self.0 }
    pub(super) fn parked(animator: Animator) -> Self { Self(animator) }
}

/// Hides the units outside the view or far away and stops animating the ones nobody looks at.
/// Skinned meshes are never frustum culled by Bevy (their bounds are unknown), so without this a
/// town of 130 NPCs is drawn in full, behind the camera too.
pub fn cull_units(
    mut commands: Commands,
    camera: Query<(&GlobalTransform, &Projection), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    mut units: Query<(Entity, &GlobalTransform, &mut Visibility, &mut RemoteUnit, Option<&mut Animator>, Has<Asleep>)>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    timers: Option<Res<aa_world::profile::Timers>>,
    // the player's mount is hidden with the player while the quest talk's close-up frames a speaker (the row's `invisible`)
    (quest, seat): (Option<Res<super::quest_camera::QuestShot>>, Option<Res<super::mount::LocalMount>>),
) {
    let _t = timers.as_deref().map(|t| t.time("units.cull"));
    let hidden_mount = seat.as_deref().and_then(|s| s.0.as_ref()).map(|s| s.mount).filter(|_| quest.as_deref().is_some_and(|q| q.pose.is_some() && q.hide));
    let Ok((cam, projection)) = camera.single() else { return };
    let (cam_pos, forward) = (cam.translation(), cam.forward().as_vec3());
    let (vfov, aspect) = match projection {
        Projection::Perspective(p) => (p.fov, p.aspect_ratio),
        _ => (0.75, 16.0 / 9.0),
    };
    let (anim_lod, shadow_radius) = tuning();
    // half angle of the view cone's diagonal
    let half_diag = ((vfov * 0.5).tan() * (1.0 + aspect * aspect).sqrt()).atan();
    for (entity, transform, mut visibility, mut unit, animator, asleep) in &mut units {
        let to = transform.translation() + Vec3::Y - cam_pos;
        let d = to.length();
        let in_view = d < 3.0 || (to / d).dot(forward).acos() < half_diag + (2.0 / d.max(2.0)).atan() + 0.1;
        let show = d < DRAW_RADIUS && in_view && hidden_mount != Some(unit.obj_id);
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != want {
            *visibility = want;
        }
        let animate = show && d < ANIMATE_RADIUS;
        let awake = animator.is_some();
        if animate && asleep {
            commands.queue(move |world: &mut World| {
                if let Some(parked) = world.entity_mut(entity).take::<Asleep>() {
                    world.entity_mut(entity).insert(parked.into_animator());
                }
            });
        } else if !animate && awake {
            commands.queue(move |world: &mut World| {
                if let Some(a) = world.entity_mut(entity).take::<Animator>() {
                    world.entity_mut(entity).insert(Asleep(a));
                }
            });
        }
        // the farther, the slower it animates
        if let Some(mut a) = animator {
            let wanted = if anim_lod { ANIMATION_LOD.iter().rev().find(|(from, _)| d > *from).map_or(0.0, |(_, interval)| *interval) } else { 0.0 };
            if a.interval != wanted {
                // stagger the updates of a crowd over the frames
                if a.interval == 0.0 {
                    a.accum = wanted * (entity.index_u32() % 7) as f32 / 7.0;
                }
                a.interval = wanted;
            }
        }
        // shadows only near the camera
        let casting = d < shadow_radius * if unit.casting { 1.1 } else { 0.9 };
        if casting != unit.casting && show {
            unit.casting = casting;
            for e in children.iter_descendants(entity) {
                if meshes.contains(e) {
                    if casting {
                        commands.entity(e).remove::<NotShadowCaster>();
                    } else {
                        commands.entity(e).insert(NotShadowCaster);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod point_tests;
