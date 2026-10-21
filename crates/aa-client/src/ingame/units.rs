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
use std::time::Instant;

use aa_anim::{Animator, ClipLibrary, spawn_character};
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_cry::workers::Workers;
use aa_net::model::{Movement, Unit, actor_flags, alertness, move_flags, stance};
use aa_net::state::{Doodads, Units};
use aa_net::{Event, NetEvent};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion};
use super::creature;
use super::data::Db;
use super::doodad_visual::DoodadVisual;
use super::idle::IdleClips;
use super::mount::Rider;
use super::player::{LocalPlayer, Mover, approach_angle, placement};
use super::prepare::{DoodadBuild, UnitBuild, prepare_doodad, prepare_unit};
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
#[derive(Component)]
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

/// Which server objects have entities. `None` = known but not drawn (too far, or no model).
#[derive(Resource)]
pub struct Views {
    units: HashMap<u32, View>,
    doodads: HashMap<u32, View>,
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
        Views { units: HashMap::new(), doodads: HashMap::new(), tx, rx: Mutex::new(rx), ready: Vec::new(), jobs: 0, waiting: 0, spawned_units: 0, spawned_doodads: 0, failed: 0 }
    }
}

/// What a worker delivers.
enum Built {
    Unit(u32, UnitBuild),
    Doodad(u32, DoodadBuild),
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
}

impl Views {
    /// The entity drawing the unit `obj_id`, when it is drawn.
    pub fn unit_entity(&self, obj_id: u32) -> Option<Entity> {
        self.units.get(&obj_id).and_then(|v| v.entity)
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

fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.y - b.y).length()
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
    time: Res<Time>,
    mut last_spawn: Local<f32>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    timers: Option<Res<aa_world::profile::Timers>>,
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

    // units that are gone
    let gone: Vec<u32> = views.units.keys().filter(|id| !units.by_id.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(v) = views.units.remove(&id)
            && let Some(e) = v.entity
        {
            commands.entity(e).despawn();
        }
    }
    let gone: Vec<u32> = views.doodads.keys().filter(|id| !doodads.by_id.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(v) = views.doodads.remove(&id)
            && let Some(e) = v.entity
        {
            commands.entity(e).despawn();
        }
    }

    // units: follow the server, drop the far ones, collect the ones to prepare
    let mut wanted: Vec<(f32, u32)> = Vec::new();
    for (id, unit) in &units.by_id {
        let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
        let d = flat_distance(pos, me);
        let view = views.units.entry(*id).or_default();
        match view.entity {
            Some(e) => {
                if d > UNIT_RADIUS * 1.25 {
                    commands.entity(e).despawn();
                    view.entity = None;
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
        views.units.entry(id).or_default().preparing = true;
        views.jobs += 1;
        let (unit, db, loader, clips, tx) = (unit.clone(), db.clone(), loader.clone(), clips.clone(), views.tx.clone());
        Workers::global().spawn(priority, move || {
            // a panic must not leave the unit "preparing" for ever
            let build = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_unit(&db, &loader, &clips, &unit)))
                .unwrap_or_else(|_| UnitBuild::None("the preparation panicked".into()));
            let _ = tx.send(Built::Unit(unit.obj_id, build));
        });
    }

    // doodads: drop the far ones, collect the ones to prepare
    let mut wanted: Vec<(f32, u32)> = Vec::new();
    for (id, doodad) in &doodads.by_id {
        let d = flat_distance(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z), me);
        let view = views.doodads.entry(*id).or_default();
        match view.entity {
            Some(e) if d > DOODAD_RADIUS * 1.25 => {
                commands.entity(e).despawn();
                view.entity = None;
            }
            Some(_) => {}
            None if !view.no_model && !view.preparing && d <= DOODAD_RADIUS => wanted.push((d, *id)),
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
        views.doodads.entry(id).or_default().preparing = true;
        views.jobs += 1;
        let (template, group, db, loader, tx) = (doodad.template_id, doodad.func_group_id, db.clone(), loader.clone(), views.tx.clone());
        Workers::global().spawn(priority, move || {
            let build = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_doodad(&db, &loader, template, group)))
                .unwrap_or(DoodadBuild { path: String::new(), model: None, func_group_id: group, growth: None });
            let _ = tx.send(Built::Doodad(id, build));
        });
    }

    views.waiting = waiting;

    // spawn what is ready, nearest first, while the frame can take it
    let mut ready = std::mem::take(&mut views.ready);
    let distance = |b: &Built| match b {
        Built::Unit(id, _) => units.by_id.get(id).map_or(f32::MAX, |u| flat_distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z), me)),
        Built::Doodad(id, _) => doodads.by_id.get(id).map_or(f32::MAX, |d| flat_distance(Vec3::new(d.pos.x, d.pos.y, d.pos.z), me)),
    };
    ready.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    let mut later = Vec::new();
    for built in ready {
        if !may_build || started.elapsed().as_secs_f32() * 1000.0 > SPAWN_BUDGET_MS {
            later.push(built);
            continue;
        }
        match built {
            Built::Unit(id, build) => {
                let view = views.units.entry(id).or_default();
                view.preparing = false;
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
                        view.entity = Some(e);
                        views.spawned_units += 1;
                    }
                    Err(why) => {
                        view.no_model = true;
                        view.why = why;
                        views.failed += 1;
                    }
                }
            }
            Built::Doodad(id, build) => {
                let view = views.doodads.entry(id).or_default();
                view.preparing = false;
                let Some(doodad) = doodads.by_id.get(&id) else { continue };
                // a phase change while the worker ran: prepare it again with the new phase
                if build.func_group_id != doodad.func_group_id {
                    continue;
                }
                if view.entity.is_some() || flat_distance(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z), me) > DOODAD_RADIUS * 1.25 {
                    continue;
                }
                let t_doodad = timers.as_deref().map(|t| t.time("units.spawn_doodad"));
                let DoodadBuild { path, model: prepared, func_group_id, growth } = build;
                let entity = prepared.map(|prepared| {
                    let loaded = cry.finish_model(prepared);
                    let options = ModelOptions::default();
                    let model = spawn_loaded(&mut commands, &path, &loaded, &options);
                    let [x, y, z, w] = doodad.rotation;
                    let base_scale = if doodad.scale > 0.0 { doodad.scale } else { 1.0 };
                    let remaining_ms = doodad.time_left as f32;
                    let total_ms = growth.map_or(0.0, |g| g.total_ms(remaining_ms, true));
                    let scale = base_scale * growth.map_or(1.0, |g| g.scale_at(remaining_ms, total_ms));
                    let transform = Transform {
                        translation: cry_to_bevy(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z)),
                        rotation: cry_rot_to_bevy(Quat::from_xyzw(x, y, z, w).normalize()),
                        scale: Vec3::splat(scale),
                    };
                    let visual = DoodadVisual { path: path.clone(), model: Some(model), func_group_id, growth, remaining_ms, total_ms, base_scale };
                    let root = commands
                        .spawn((Name::new(format!("doodad {}", doodad.template_id)), RemoteDoodad { obj_id: id }, visual, transform, Visibility::default()))
                        .id();
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
fn spawn_unit(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, unit: &Unit, build: UnitBuild) -> Result<Entity, String> {
    let scale = if unit.scale > 0.0 { unit.scale } else { 1.0 };
    let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let transform = placement(pos, unit.yaw(), scale);
    debug!("unit {} {:?} {:?} model {} at ({:.1}, {:.1}, {:.1}) scale {:.2}", unit.obj_id, unit.name, unit.kind, unit.model_id, pos.x, pos.y, pos.z, scale);
    let entity = match build {
        UnitBuild::Character { assets, models, carried, clips: set, pose } => {
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
            if !carried.is_empty() {
                entity.insert(weapons::Carried(carried));
            }
            if let Some(set) = set {
                entity.insert(Locomotion::new((*set).clone()));
            }
            if pose.is_some() {
                entity.insert(IdleClips::new(pose, None));
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
            receive_moves.before(move_units),
            (super::mount::fake_mount, super::mount::start_stop_riding, super::mount::pose_riders).chain().after(move_units),
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

/// Feeds the movement packets to the tracks of the units that are drawn (the others take their position from the server's
/// state when they are built).
pub fn receive_moves(mut events: MessageReader<NetEvent>, views: Res<Views>, mut units: Query<&mut RemoteUnit>, time: Res<Time>) {
    let now = time.elapsed_secs();
    for NetEvent(event) in events.read() {
        let obj_id = match event {
            Event::UnitMoved { obj_id, .. } | Event::UnitDeath { obj_id } | Event::Resurrected { obj_id, .. } => obj_id,
            _ => continue,
        };
        let Some(entity) = views.units.get(obj_id).and_then(|v| v.entity) else { continue };
        if let Ok(mut unit) = units.get_mut(entity) {
            match event {
                // A ship request has no position.
                Event::UnitMoved { movement, .. } if movement.move_type != 4 => unit.track.push(now, report_of(movement)),
                Event::UnitDeath { .. } => unit.track = Track::new(unit.pos, unit.facing),
                Event::Resurrected { pos, yaw, .. } => {
                    unit.pos = Vec3::new(pos.x, pos.y, pos.z);
                    unit.facing = *yaw;
                    unit.track = Track::new(unit.pos, unit.facing);
                }
                _ => {}
            }
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
/// set on the terrain, and the speed and direction of travel handed to the locomotion clips ([`Motion`]).
pub fn move_units(
    time: Res<Time>,
    terrain: Res<TerrainQuery>,
    mut query: Query<(&mut RemoteUnit, &mut Transform, Option<&mut Motion>), Without<Rider>>,
    timers: Option<Res<aa_world::profile::Timers>>,
    mut smoothness: Local<Smoothness>,
) {
    let _t = timers.as_deref().map(|t| t.time("units.move"));
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let measure = smoothness_wanted();
    for (mut unit, mut transform, motion) in &mut query {
        let before = unit.pos;
        let pose = unit.track.sample(dt);
        let mut pos = pose.pos;
        // on the ground: the terrain under it (the server's height differs by a few centimetres from our mesh)
        let ground = terrain.ground_at(pos.x, pos.y).map(|ground| ground.height);
        let airborne = pose.airborne && ground.is_none_or(|g| pos.z - g > 0.25);
        if !airborne && let Some(g) = ground.filter(|g| (pos.z - g).abs() < GROUND_SNAP) {
            pos.z = g;
        }
        unit.pos = pos;
        if measure {
            smoothness.add(unit.character, flat_distance(before, pos) / dt, pose.speed);
            if unit.character && pose.speed > 0.5 {
                trace!("unit {} t {:.3} dt {:.4} shown {:.3} path {:.3} clock {:.3} delay {:.3}", unit.obj_id, time.elapsed_secs(), dt, flat_distance(before, pos) / dt, pose.speed, unit.track.clock(), unit.track.delay());
            }
        }
        unit.combat = pose.combat;
        // a unit that walks faces where it goes; one that turns on the spot follows the server's heading
        unit.facing = approach_angle(unit.facing, pose.yaw, TURN_RATE * dt);
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
}

/// Hides the units outside the view or far away and stops animating the ones nobody looks at.
/// Skinned meshes are never frustum culled by Bevy (their bounds are unknown), so without this a
/// town of 130 NPCs is drawn in full, behind the camera too.
pub fn cull_units(
    mut commands: Commands,
    camera: Query<(&GlobalTransform, &Projection), With<Camera3d>>,
    mut units: Query<(Entity, &GlobalTransform, &mut Visibility, &mut RemoteUnit, Option<&mut Animator>, Has<Asleep>)>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    timers: Option<Res<aa_world::profile::Timers>>,
) {
    let _t = timers.as_deref().map(|t| t.time("units.cull"));
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
        let show = d < DRAW_RADIUS && in_view;
        let want = if show { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != want {
            *visibility = want;
        }
        let animate = show && d < ANIMATE_RADIUS;
        let awake = animator.is_some();
        if animate && asleep {
            commands.queue(move |world: &mut World| {
                if let Some(Asleep(a)) = world.entity_mut(entity).take::<Asleep>() {
                    world.entity_mut(entity).insert(a);
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
