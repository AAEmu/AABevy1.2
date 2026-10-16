//! Other units and doodads: spawned when the server announces them and they are near, moved and
//! turned smoothly as the server reports, despawned when removed or far away.
//!
//! The server sends units within its view range (hundreds of metres); rendering all of them would
//! cost more than the town is worth, so a unit gets a model only within [`UNIT_RADIUS`] of the
//! player (doodads: [`DOODAD_RADIUS`]) and loses it again beyond a margin. Models are loaded
//! nearest first under a per frame time budget; the cached assets make the second NPC of a kind
//! cheap.

use std::collections::HashMap;
use std::time::Instant;

use aa_anim::{Animator, ClipLibrary, Costume, Gender, Race};
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_net::model::{Unit, UnitKind};
use aa_net::state::{Doodads, Units};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::prelude::*;

use super::anim::Motion;
use super::appearance;
use super::creature;
use super::data::Db;
use super::player::{LocalPlayer, Mover, approach_angle, placement, spawn_with_locomotion};

/// Units are drawn within this many metres of the player, and dropped again beyond `* 1.25`.
pub const UNIT_RADIUS: f32 = 140.0;
pub const DOODAD_RADIUS: f32 = 160.0;
/// Main thread time per frame for building models (ms); one model is always built.
const SPAWN_BUDGET_MS: f32 = 10.0;

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
}

#[derive(Component)]
pub struct RemoteDoodad {
    pub obj_id: u32,
}

/// Which server objects have entities. `None` = known but not drawn (too far, or no model).
#[derive(Resource, Default)]
pub struct Views {
    units: HashMap<u32, View>,
    doodads: HashMap<u32, View>,
    /// Counters for the log.
    pub spawned_units: u32,
    pub spawned_doodads: u32,
    pub failed: u32,
}

#[derive(Default)]
struct View {
    entity: Option<Entity>,
    /// The model failed to build or the object has none: do not try again.
    no_model: bool,
}

impl Views {
    pub fn unit_entities(&self) -> usize {
        self.units.values().filter(|v| v.entity.is_some()).count()
    }

    pub fn doodad_entities(&self) -> usize {
        self.doodads.values().filter(|v| v.entity.is_some()).count()
    }
}

fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.y - b.y).length()
}

/// Race and gender of a humanoid model file (`objects/characters/<race>/<gender>/nude/...`).
fn humanoid_of_file(file: &str) -> Option<(Race, Gender)> {
    let rest = file.split("characters/").nth(1)?;
    let mut it = rest.split('/');
    let folder = it.next()?;
    let race = Race::ALL.into_iter().find(|r| r.folder() == folder)?;
    let gender = match it.next()? {
        "male" => Gender::Male,
        "female" => Gender::Female,
        _ => return None,
    };
    (it.next()? == "nude").then_some((race, gender))
}

/// Creates and removes entities for the units and doodads the server reports.
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
) {
    let (Some(db), Ok(me)) = (db, player.single()) else { return };
    let me = me.pos;
    let started = Instant::now();

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

    // units: follow the server, drop the far ones, collect the ones to build
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
            None if !view.no_model && d <= UNIT_RADIUS => wanted.push((d, *id)),
            None => {}
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, id) in wanted {
        if started.elapsed().as_secs_f32() * 1000.0 > SPAWN_BUDGET_MS {
            break;
        }
        let Some(unit) = units.by_id.get(&id) else { continue };
        let entity = spawn_unit(&mut commands, &mut cry, &clips, &db, unit);
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

    // doodads
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
            None if !view.no_model && d <= DOODAD_RADIUS => wanted.push((d, *id)),
            None => {}
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, id) in wanted {
        if started.elapsed().as_secs_f32() * 1000.0 > SPAWN_BUDGET_MS {
            break;
        }
        let Some(doodad) = doodads.by_id.get(&id) else { continue };
        let entity = db.doodad_model(doodad.template_id).and_then(|path| {
            let options = ModelOptions::default();
            let loaded = match cry.load_model(&path, &options) {
                Ok(l) => l,
                Err(e) => {
                    debug!("doodad {} model {path}: {e}", doodad.template_id);
                    return None;
                }
            };
            let model = spawn_loaded(&mut commands, &path, &loaded, &options);
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

/// Builds the model of one unit. `None` when it has none we can draw.
fn spawn_unit(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, db: &Db, unit: &Unit) -> Option<Entity> {
    let scale = if unit.scale > 0.0 { unit.scale } else { 1.0 };
    let pos = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let transform = placement(pos, unit.yaw(), scale);
    let file = db.actor_model_file(unit.model_id);
    let humanoid = match unit.kind {
        UnitKind::Character { .. } => appearance::race_gender(unit.model_id, unit.race(), unit.gender()),
        _ => file.as_deref().and_then(humanoid_of_file),
    };
    info!(
        "unit {} {:?} {:?} model {} {:?} humanoid {:?} at ({:.1}, {:.1}, {:.1}) scale {:.2}",
        unit.obj_id,
        unit.name,
        unit.kind,
        unit.model_id,
        file,
        humanoid,
        pos.x,
        pos.y,
        pos.z,
        scale
    );
    let entity = if let Some((race, gender)) = humanoid {
        let game = db.game.lock().unwrap();
        let mut desc = appearance::describe(&game, race, gender, &unit.equipment, &unit.model_params)?;
        if matches!(desc.costume, Costume::Items(ref i) if i.is_empty()) && !unit.is_character() {
            // an NPC without armour data: the default outfit beats a naked body
            desc.costume = Costume::Login;
        }
        let assets = desc.resolve(&game).map_err(|e| debug!("unit {} {}: {e}", unit.obj_id, unit.name)).ok()?;
        drop(game);
        spawn_with_locomotion(commands, cry, clips, assets, transform)?
    } else {
        creature::spawn(commands, cry, clips, file.as_deref()?, transform)?
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
) {
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
