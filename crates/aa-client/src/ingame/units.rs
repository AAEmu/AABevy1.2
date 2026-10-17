//! Other units and doodads: spawned when the server announces them and they are near, moved and
//! turned smoothly as the server reports, despawned when removed or far away.
//!
//! The server sends units within its view range (hundreds of metres); rendering all of them would
//! cost more than the town is worth, so a unit gets a model only within [`UNIT_RADIUS`] of the
//! player (doodads: [`DOODAD_RADIUS`]) and loses it again beyond a margin. Models are loaded
//! nearest first under a per frame time budget; the cached assets make the second NPC of a kind
//! cheap.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use aa_anim::{Animator, ClipLibrary, spawn_character};
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_cry::workers::Workers;
use aa_net::model::Unit;
use aa_net::state::{Doodads, Units};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use super::anim::{Locomotion, Motion};
use super::creature;
use super::data::Db;
use super::player::{LocalPlayer, Mover, approach_angle, placement};
use super::prepare::{DoodadBuild, UnitBuild, prepare_doodad, prepare_unit};
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

/// A spawned unit, smoothed towards the server's position.
#[derive(Component)]
pub struct RemoteUnit {
    pub obj_id: u32,
    /// Displayed position / facing (game coordinates, radians).
    pos: Vec3,
    facing: f32,
    /// Last position the server reported, the velocity it came with and how long ago.
    target: Vec3,
    velocity: Vec3,
    since_update: f32,
    scale: f32,
    /// Measured ground speed, low-pass filtered (m/s).
    speed: f32,
    /// Its meshes cast shadows (see [`SHADOW_RADIUS`]).
    casting: bool,
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
    /// Counters for the log.
    pub spawned_units: u32,
    pub spawned_doodads: u32,
    pub failed: u32,
}

impl Default for Views {
    fn default() -> Self {
        let (tx, rx) = channel();
        Views { units: HashMap::new(), doodads: HashMap::new(), tx, rx: Mutex::new(rx), ready: Vec::new(), jobs: 0, spawned_units: 0, spawned_doodads: 0, failed: 0 }
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
    /// A worker is preparing it.
    preparing: bool,
}

impl Views {
    pub fn unit_entities(&self) -> usize {
        self.units.values().filter(|v| v.entity.is_some()).count()
    }

    pub fn doodad_entities(&self) -> usize {
        self.doodads.values().filter(|v| v.entity.is_some()).count()
    }

    /// Objects being prepared or waiting to be spawned.
    pub fn pending(&self) -> usize {
        self.jobs + self.ready.len()
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
    mut remote: Query<&mut RemoteUnit>,
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
    let facing = camera.single().map(|c| flat_dir(c)).unwrap_or(Vec2::ZERO);

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
        if let Some(v) = views.units.remove(&id) {
            if let Some(e) = v.entity {
                commands.entity(e).despawn();
            }
        }
    }
    let gone: Vec<u32> = views.doodads.keys().filter(|id| !doodads.by_id.contains_key(id)).copied().collect();
    for id in gone {
        if let Some(v) = views.doodads.remove(&id) {
            if let Some(e) = v.entity {
                commands.entity(e).despawn();
            }
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
                } else if let Ok(mut r) = remote.get_mut(e) {
                    if r.target != pos {
                        r.target = pos;
                        r.since_update = 0.0;
                    }
                    r.velocity = unit.movement.as_ref().map_or(Vec3::ZERO, |m| {
                        if m.flags & aa_net::model::move_flags::MOVING != 0 {
                            Vec3::new(f32::from(m.velocity[0]), f32::from(m.velocity[1]), 0.0) / 1024.0
                        } else {
                            Vec3::ZERO
                        }
                    });
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
    for (priority, id) in wanted {
        if views.jobs >= MAX_JOBS {
            break;
        }
        let Some(unit) = units.by_id.get(&id) else { continue };
        views.units.entry(id).or_default().preparing = true;
        views.jobs += 1;
        let (unit, db, loader, clips, tx) = (unit.clone(), db.clone(), loader.clone(), clips.clone(), views.tx.clone());
        Workers::global().spawn(priority, move || {
            let build = prepare_unit(&db, &loader, &clips, &unit);
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
    for (priority, id) in wanted {
        if views.jobs >= MAX_JOBS {
            break;
        }
        let Some(doodad) = doodads.by_id.get(&id) else { continue };
        views.doodads.entry(id).or_default().preparing = true;
        views.jobs += 1;
        let (template, db, loader, tx) = (doodad.template_id, db.clone(), loader.clone(), views.tx.clone());
        Workers::global().spawn(priority, move || {
            let build = prepare_doodad(&db, &loader, template);
            let _ = tx.send(Built::Doodad(id, build));
        });
    }

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
                let ms = t0.elapsed().as_secs_f32() * 1000.0;
                if ms > 30.0 {
                    debug!("spawning unit {} ({:?}) took {ms:.0} ms", unit.obj_id, unit.kind);
                }
                let view = views.units.entry(id).or_default();
                match entity {
                    Some(e) => {
                        view.entity = Some(e);
                        views.spawned_units += 1;
                    }
                    None => {
                        view.no_model = true;
                        views.failed += 1;
                    }
                }
            }
            Built::Doodad(id, build) => {
                let view = views.doodads.entry(id).or_default();
                view.preparing = false;
                let Some(doodad) = doodads.by_id.get(&id) else { continue };
                if view.entity.is_some() || flat_distance(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z), me) > DOODAD_RADIUS * 1.25 {
                    continue;
                }
                let t_doodad = timers.as_deref().map(|t| t.time("units.spawn_doodad"));
                let entity = build.model.and_then(|prepared| {
                    let loaded = cry.finish_model(prepared);
                    let options = ModelOptions::default();
                    let model = spawn_loaded(&mut commands, &build.path, &loaded, &options);
                    let [x, y, z, w] = doodad.rotation;
                    let transform = Transform {
                        translation: cry_to_bevy(Vec3::new(doodad.pos.x, doodad.pos.y, doodad.pos.z)),
                        rotation: cry_rot_to_bevy(Quat::from_xyzw(x, y, z, w).normalize()),
                        scale: Vec3::splat(if doodad.scale > 0.0 { doodad.scale } else { 1.0 }),
                    };
                    let root = commands
                        .spawn((Name::new(format!("doodad {}", doodad.template_id)), RemoteDoodad { obj_id: id }, transform, Visibility::default()))
                        .id();
                    commands.entity(model).insert(ChildOf(root));
                    Some(root)
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
fn spawn_unit(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, unit: &Unit, build: UnitBuild) -> Option<Entity> {
    let scale = if unit.scale > 0.0 { unit.scale } else { 1.0 };
    let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let transform = placement(pos, unit.yaw(), scale);
    debug!("unit {} {:?} {:?} model {} at ({:.1}, {:.1}, {:.1}) scale {:.2}", unit.obj_id, unit.name, unit.kind, unit.model_id, pos.x, pos.y, pos.z, scale);
    let entity = match build {
        UnitBuild::Character { assets, models, carried, clips: set } => {
            for m in models {
                cry.finish_model(m);
            }
            let _t = aa_cry::prof::scope("unit.spawn_character");
            let root = match spawn_character(commands, cry, clips, *assets, transform) {
                Ok(r) => r,
                Err(e) => {
                    error!("cannot spawn a character: {e}");
                    return None;
                }
            };
            let mut entity = commands.entity(root);
            entity.insert(Motion::default());
            if !carried.is_empty() {
                entity.insert(weapons::Carried(carried));
            }
            if let Some(set) = set {
                entity.insert(Locomotion::new((*set).clone()));
            }
            root
        }
        UnitBuild::Creature { file, models } => {
            for m in models {
                cry.finish_model(m);
            }
            let _t = aa_cry::prof::scope("unit.spawn_creature");
            creature::spawn(commands, cry, clips, &file, transform)?
        }
        UnitBuild::None => return None,
    };
    commands.entity(entity).insert((
        Name::new(format!("unit {} {}", unit.obj_id, unit.name)),
        RemoteUnit {
            obj_id: unit.obj_id,
            pos,
            facing: unit.yaw(),
            target: pos,
            velocity: Vec3::ZERO,
            since_update: 1.0,
            scale,
            speed: 0.0,
            casting: true,
        },
    ));
    // different idle phases, so a crowd does not breathe in step
    let obj_id = unit.obj_id;
    commands.queue(move |world: &mut World| {
        if let Some(mut a) = world.get_mut::<Animator>(entity) {
            if let Some(cur) = a.current.as_mut() {
                cur.time = (unit_hash(obj_id) % 3000) as f32 / 1000.0;
            }
        }
    });
    Some(entity)
}

fn unit_hash(v: u32) -> u32 {
    v.wrapping_mul(2654435761) >> 7
}

/// Moves the displayed units towards what the server last said, with dead reckoning between reports.
pub fn move_units(
    time: Res<Time>,
    units: Res<Units>,
    mut query: Query<(&mut RemoteUnit, &mut Transform, Option<&mut Motion>)>,
    timers: Option<Res<aa_world::profile::Timers>>,
) {
    let _t = timers.as_deref().map(|t| t.time("units.move"));
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let blend = 1.0 - (-14.0 * dt).exp();
    for (mut unit, mut transform, motion) in &mut query {
        unit.since_update += dt;
        // where the unit should be by now
        let ahead = unit.velocity * unit.since_update.min(0.6);
        let predicted = unit.target + ahead;
        let from = unit.pos;
        if (predicted - from).length() > 12.0 {
            unit.pos = predicted; // a jump (teleport / respawn): do not glide across the town
        } else {
            unit.pos = from.lerp(predicted, blend);
        }
        let moved = flat_distance(unit.pos, from);
        let speed_now = moved / dt;
        unit.speed += (speed_now - unit.speed) * (1.0 - (-8.0 * dt).exp());
        // facing: the server's heading, or the direction of travel when it is walking
        let want = units.by_id.get(&unit.obj_id).map(|u| u.yaw()).unwrap_or(unit.facing);
        let heading = if unit.speed > 0.5 && moved > 1e-4 {
            let d = unit.pos - from;
            (-d.x).atan2(d.y)
        } else {
            want
        };
        unit.facing = approach_angle(unit.facing, heading, 10.0 * dt);
        *transform = placement(unit.pos, unit.facing, unit.scale);
        if let Some(mut m) = motion {
            let moving = unit.speed > 0.4;
            *m = Motion { speed: if moving { unit.speed } else { 0.0 }, local_dir: if moving { Vec2::Y } else { Vec2::ZERO }, airborne: false };
        }
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
