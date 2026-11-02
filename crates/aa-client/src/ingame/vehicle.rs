//! Vehicles and ships (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats, siege weapons), drawn from
//! their `normal` look ([`super::data::Db::vehicle_look`]): a single model, or every part of a prefab (the hull and deck
//! brushes, and the `AnimObject` parts such as an airship's wings, propellers and planks). Wheels go on the body's wheel
//! mounts: `wheelN` bones or helper nodes take the `wheel` model, `wheel_type2N` ones `wheel2`.
//!
//! The wheels spin about their axle from the cart's forward speed (`FUN_390d6b10`) and the body's
//! `Bone_spindle` turns the front axle (`FUN_390cffd0`); see [`spin_wheels`] and [`steer_spindles`].
//!
//! Damage states ([`model_state`] / [`update_damage`]) swap the whole body prefab to the state's
//! `vehicle_models` / `ship_models` row (`damaged25` / `damaged50` / `damaged75` / `dying` / `dead`, falling
//! down to the next lower non-empty row). A death enters dying (5) first, shows that prefab for
//! `models.dying_time`, then switches to dead (6) (`FUN_390e7ad0`, `FUN_390eaac0`). The variant prefab's
//! active particle entities (the fire and smoke of the damaged and dead looks) are drawn with it, and a
//! state that worsens plays the new prefab's `_spawn_effect*` entities once ([`spawn_state_effects`]). The
//! new prefab is read on the workers ([`VehicleSwaps`]); the old parts stay until it is ready.
//!
//! The maximum is unit attribute 6 as `FUN_390b4a70` reads it: the `unit_formulas` base of the slave
//! plus its `unit_modifiers`. It is cached in [`VehicleDamage`] at spawn ([`slave_max_hp`]).
//!
//! Left out: the speed-driven exhaust and splash emitters (`FUN_390c78d0`), propeller spin, turrets.
//! Combat effects anchored to a body
//! bone (`super::attachments`) go away with the old parts on a swap: re-anchoring them by bone name is
//! not implemented (they are a skill's own one-shot effects, not the vehicle's prefab emitters).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use aa_anim::{AnimSet, Animator, ClipLibrary, pose::BoundClip};
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_cry::workers::Workers;
use aa_cry::{CryLoader, PreparedModel};
use aa_net::state::Units;
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

use super::cart::{YAW_RATE_PER_STEER, wrap_angle};
use super::combat::Dead;
use super::data::{Db, VehicleLook, model_file};
use super::housing::prefab::{self, Part};
use super::units::RemoteUnit;

/// `_DAT_399a440c` (0x399a440c = 0.52359879): the largest spindle angle, 30 degrees.
pub const SPINDLE_MAX_ANGLE: f32 = std::f32::consts::FRAC_PI_6;

/// `DAT_3999ccf0` (0x3999ccf0): the smoothing rate is `min(dt, 0.1)`.
pub const SPINDLE_LERP: f32 = 0.1;

/// The yaw rate that means full steering in the standstill fallback: the first float of the
/// model's vehicle-physics block (`*(int *)(vtable+0x1e0) + 0x34`). That block is built at
/// runtime from the vehicle data, so the value cannot be read from the unpacked dll; the captured
/// full-lock rate [`YAW_RATE_PER_STEER`] stands in for it.
pub const SPINDLE_YAW_LIMIT: f32 = YAW_RATE_PER_STEER;

/// The prefab-unit model state of `FUN_3989f8b0`: 1 normal, 2 `damaged25`, 3 `damaged50`, 4 `damaged75`,
/// 6 dead. The percentage is C integer division, so it truncates: 7599/10000 is 75 (state 2), 7600/10000
/// is 76 (state 1). `hp == 0` or `max_hp == 0` is dead before the divide. This is only the polled state:
/// 5 (dying) is entered by the death path (`FUN_390e7ad0`), never by the HP percentage.
pub fn model_state(hp: i32, max_hp: i32) -> u8 {
    if hp == 0 || max_hp == 0 {
        return 6;
    }
    let pct = hp * 100 / max_hp;
    if pct > 75 {
        1
    } else if pct > 50 {
        2
    } else if pct < 26 {
        4
    } else {
        3
    }
}

/// The damage state a vehicle is drawn in, the model ref it was built from, the cached maximum health and the
/// dying window (`models.dying_time`).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct VehicleDamage {
    pub state: u8,
    /// The variant ref, so a state change that keeps the same model does not rebuild or fire effects
    /// (`FUN_390e7910` logs "same model" and returns before the effect trigger).
    pub variant: String,
    /// The template maximum (`slave_max_hp`), constant for the vehicle's life: held here so the per-frame
    /// state check does not lock the database.
    pub max_hp: i32,
    /// The state a worker is preparing, so one state change queues one swap.
    pub requested: Option<u8>,
    /// The dying start time (`model+0x274`, in [`Time::elapsed_secs_f64`]): `Some` from entering state 5 until
    /// the dead prefab is applied. While it is set the per-frame update does not recompute the state from HP.
    pub dying_start: Option<f64>,
    /// `models.dying_time` seconds (`template+0x14`, checked by `FUN_390eaac0`); 0 means dying lasts one tick.
    pub dying_time: f32,
    /// The current death has already entered dying, so a later update does not restart it. Cleared when the unit
    /// is alive again (hp > 0 and not dead).
    pub death_seen: bool,
    /// The dead prefab's one-shot `_spawn_effect*` already played for this death: an HP-0 update shows 6 first,
    /// then the death's 6 -> 5 -> 6 route must not play them a second time. Cleared when a living state is adopted.
    pub dead_effect_played: bool,
}

/// A body part root spawned by [`spawn`]: replaced when the damage state changes.
#[derive(Component)]
pub struct VehiclePart;

/// A particle entity spawned with a damage variant: replaced when the state changes.
#[derive(Component)]
pub struct VehicleEffect;

/// A body prefab being read on the workers for [`VehicleDamage::requested`].
struct VehicleSwapBuilt {
    root: Entity,
    obj_id: u32,
    state: u8,
    variant: String,
    pieces: Vec<Piece>,
}

/// The damage variants being prepared off the main thread, and their delivery channel. A swap replaces the old
/// parts only after [`finish_damage_swaps`] receives the new ones, so a slow or failed read never leaves the
/// vehicle invisible.
#[derive(Resource)]
pub struct VehicleSwaps {
    tx: Sender<VehicleSwapBuilt>,
    rx: Mutex<Receiver<VehicleSwapBuilt>>,
}

impl Default for VehicleSwaps {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

/// The maximum health of a slave vehicle: unit attribute 6 (`FUN_390b4a70` reads it through
/// `FUN_398a0bc0`), the formula base plus the template's `unit_modifiers`. The unit's own `max_hp` is only a
/// last resort when the database has no `MaxHealth` formula; 1.2 never uses the wire maximum or
/// `SCMySlave.max_hp`.
pub fn slave_max_hp(db: &Db, unit: &aa_net::model::Unit) -> i32 {
    db.slave_max_hp(unit.kind.template_id(), unit.level)
        .unwrap_or(unit.max_hp)
        .max(0)
}

/// The damage state of a unit from its current health (`SCUnitPoints` / `SCUnitState`) and the attribute-6
/// maximum. Used when a vehicle is first announced; [`update_damage`] then uses the cached [`VehicleDamage::max_hp`].
pub fn damage_state(db: &Db, unit: &aa_net::model::Unit) -> u8 {
    model_state(unit.hp, slave_max_hp(db, unit))
}

/// The state a newly announced vehicle is built in. `InitClientUnit` (`FUN_3936b000`) and `OnModelChanged`
/// (`FUN_39364db0`) call the model's Die slot when `hp == 0`, which tries the dying state first
/// (`FUN_390eaa80` -> `FUN_390e7ad0`), so such a unit starts dying (5); every other unit takes the polled state.
pub fn initial_state(db: &Db, unit: &aa_net::model::Unit) -> u8 {
    if unit.hp == 0 {
        5
    } else {
        damage_state(db, unit)
    }
}

/// Adopt `state` without a worker swap: every state but dying ends the dying window, and a living state starts a
/// new effect life (a later death may play the dead prefab's one-shots again).
fn adopt_state(damage: &mut VehicleDamage, state: u8) {
    damage.state = state;
    if state != 5 {
        damage.dying_start = None;
    }
    if !matches!(state, 5 | 6) {
        damage.dead_effect_played = false;
    }
}

/// The wheel model slot of a mount name: `wheel0`, `wheel12` take `wheel` (0), `wheel_type2`, `wheel_type21` take
/// `wheel2` (1). Other names (`bone_steeringwheel`, `wheel_l`) are not mounts.
pub fn wheel_slot(name: &str) -> Option<usize> {
    let name = name.strip_prefix('$').unwrap_or(name).to_ascii_lowercase();
    if let Some(rest) = name.strip_prefix("wheel_type2") {
        return rest.bytes().all(|b| b.is_ascii_digit()).then_some(1);
    }
    let rest = name.strip_prefix("wheel")?;
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())).then_some(0)
}

/// The wheel mounts of a model (bones of a `.chr`, helper nodes of a `.cgf`), model space (cry).
fn mounts(model: &aa_cry::mesh::Model) -> Vec<(usize, Affine3A)> {
    let mut out: Vec<(usize, Affine3A)> = Vec::new();
    if let Some(skeleton) = &model.skeleton {
        out.extend(skeleton.bones.iter().filter_map(|b| Some((wheel_slot(&b.name)?, b.bind_model))));
    }
    if out.is_empty() {
        out.extend(model.nodes.iter().enumerate().filter_map(|(i, n)| Some((wheel_slot(&n.name)?, model.world_transform(i)))));
    }
    out
}

fn affine(part: &Part) -> Affine3A {
    Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos)
}

/// The front and rear lever arms of `Bone_spindle` (`FUN_390cffd0`): the distances from the
/// spindle bone to the furthest front / rear wheel mount along the model's forward axis (cry Y).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpindleArms {
    pub front: f32,
    pub rear: f32,
}

/// A part of the body, or a wheel placed on a body mount.
struct Placed {
    part: Part,
    /// The wheel model slot (`wheel` = 0, `wheel2` = 1); `None` for body parts.
    wheel: Option<usize>,
    /// The lever arms when this body part carries `Bone_spindle`.
    spindle: Option<SpindleArms>,
}

fn spindle_arms(model: &aa_cry::mesh::Model) -> Option<SpindleArms> {
    let skeleton = model.skeleton.as_ref()?;
    let bone = skeleton
        .bones
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case("Bone_spindle"))?;
    let y = bone.bind_model.translation.y;
    let mut front = f32::MIN;
    let mut rear = f32::MAX;
    for (_, mount) in mounts(model) {
        front = front.max(mount.translation.y);
        rear = rear.min(mount.translation.y);
    }
    let (front, rear) = (front - y, y - rear);
    (front >= 0.0 && rear >= 0.0).then_some(SpindleArms { front, rear })
}

/// The `$` helper nodes of a vehicle body in unit (Cry) space: `driver`, `passenger0`, ... A vehicle root has no
/// rider Animator, so `mount::seat_world` reads the prefab helper from here.
#[derive(Component, Default)]
pub struct SeatHelpers(pub HashMap<String, Affine3A>);

/// The helper key of a body bone or node: the attach strings name `$driver`, `$passenger0`, ..., but some vehicle
/// bodies (the red ox cart's `transfers_trailer_c.chr`) name the seat bones `driver` / `passenger0` without the `$`.
/// Both spellings are the same helper; other un-prefixed body bones are not helpers (capture-pending S11-C3 checks
/// the original helper lookup).
fn helper_key(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let stripped = lower.strip_prefix('$').unwrap_or(&lower);
    (lower.starts_with('$') || stripped.starts_with("driver") || stripped.starts_with("passenger"))
        .then(|| stripped.to_string())
}

fn helper_nodes(
    model: &aa_cry::mesh::Model,
    placement: Affine3A,
    out: &mut HashMap<String, Affine3A>,
) {
    // `$`-prefixed helpers first, so a bare `driver`/`passenger` bone never displaces an authored `$driver` node
    for prefixed in [true, false] {
        let pass = |name: &str| name.starts_with('$') == prefixed;
        if let Some(skeleton) = &model.skeleton {
            for bone in skeleton.bones.iter().filter(|bone| pass(&bone.name)) {
                if let Some(name) = helper_key(&bone.name) {
                    out.entry(name).or_insert(placement * bone.bind_model);
                }
            }
        }
        for (index, node) in model.nodes.iter().enumerate().filter(|(_, node)| pass(&node.name)) {
            if let Some(name) = helper_key(&node.name) {
                out.entry(name)
                    .or_insert_with(|| placement * model.world_transform(index));
            }
        }
    }
}

fn part_helpers(part: &Part, model: &aa_cry::mesh::Model) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    helper_nodes(model, affine(part), &mut helpers);
    helpers
}

/// The body helpers of `look` (reads the body models; used by the tests, [`prepare`] reuses the models it loads).
#[cfg(test)]
fn body_helpers(look: &VehicleLook) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    for part in body_parts(look) {
        if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
            helper_nodes(&model, affine(&part), &mut helpers);
        }
    }
    helpers
}

fn body_parts(look: &VehicleLook) -> Vec<Part> {
    if look.normal.to_ascii_lowercase().starts_with("prefab://") {
        prefab::load(&look.normal)
    } else {
        model_file(&look.normal)
            .map(|model| {
                vec![Part {
                    model,
                    material: None,
                    pos: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    scale: Vec3::ONE,
                }]
            })
            .unwrap_or_default()
    }
}

/// The parts `look` is drawn with (body, then wheels) and the spindle arms of the body that has `Bone_spindle`.
fn assemble(look: &VehicleLook) -> Vec<Placed> {
    let mut out: Vec<Placed> = body_parts(look)
        .into_iter()
        .map(|part| Placed {
            part,
            wheel: None,
            spindle: None,
        })
        .collect();
    if look.wheels.iter().any(Option::is_some) {
        let mut wheels = Vec::new();
        for placed in &mut out {
            let Ok(model) = aa_cry::mesh::Model::load(&placed.part.model) else {
                continue;
            };
            placed.spindle = spindle_arms(&model);
            for (slot, mount) in mounts(&model) {
                let Some(wheel) = look.wheels[slot].as_ref().or(look.wheels[0].as_ref()) else {
                    continue;
                };
                let (scale, rot, pos) =
                    (affine(&placed.part) * mount).to_scale_rotation_translation();
                wheels.push(Placed {
                    part: Part {
                        model: wheel.clone(),
                        material: None,
                        pos,
                        rot: rot.normalize(),
                        scale,
                    },
                    wheel: Some(slot),
                    spindle: None,
                });
            }
        }
        out.extend(wheels);
    }
    out
}

/// The parts `look` is drawn with, relative to the unit's origin (cry space): the body parts, then the wheels.
pub fn parts(look: &VehicleLook) -> Vec<Part> {
    assemble(look)
        .into_iter()
        .map(|placed| placed.part)
        .collect()
}

/// The model-space bounds of every part of `look` (the slave summon space check).
pub fn bounds(look: &VehicleLook) -> Option<(Vec3, Vec3)> {
    let mut out: Option<(Vec3, Vec3)> = None;
    for part in parts(look) {
        let Ok(model) = aa_cry::mesh::Model::load(&part.model) else {
            warn!("vehicle: summon bounds: cannot load {}", part.model);
            continue;
        };
        let Some((lo, hi)) = model.bounds() else {
            warn!(
                "vehicle: summon bounds: no LOD 0 geometry in {}",
                part.model
            );
            continue;
        };
        let placement = affine(&part);
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            );
            let p = placement.transform_point3(corner);
            out = Some(match out {
                None => (p, p),
                Some((a, b)) => (a.min(p), b.max(p)),
            });
        }
    }
    out
}

/// A prepared part: a static model, or a rigged `.chr` (its models, spawned through [`super::creature::spawn`]).
pub struct Piece {
    part: Part,
    model: PieceModel,
    /// The wheel's radius (m) from its model's bounds (the axle is model-space Y: the smallest
    /// half extent), for a wheel part.
    wheel: Option<f32>,
    spindle: Option<SpindleArms>,
    helpers: HashMap<String, Affine3A>,
}

enum PieceModel {
    Static(PreparedModel),
    Rigged(Vec<PreparedModel>),
}

/// Vehicle materials are native `Illum` (the farm wagon's copper boiler uses the gloss map), so they need the Full
/// shading path rather than the world-prop Basic one.
fn model_options(material: Option<String>, cry_space: bool) -> ModelOptions {
    ModelOptions {
        material: material.filter(|m| aa_pak::Pak::global().exists(m)),
        cry_space,
        shading: Shading::Full,
        ..default()
    }
}

fn static_options(part: &Part) -> ModelOptions {
    model_options(part.material.clone(), false)
}

/// The rolling radius (m) of a wheel model: the largest half extent of its bounds. The wheel
/// models keep their axle on model-space Y (`transfers_trailer_a_wheel01`: x 1.036, y 0.332,
/// z 1.026 m), so the other two half extents are the radius.
fn wheel_radius(model: &aa_cry::mesh::Model) -> Option<f32> {
    let (lo, hi) = model.bounds()?;
    let radius = ((hi - lo) * 0.5).max_element();
    (radius > 0.01).then_some(radius)
}

/// Reads the models of `look` (callable from any thread). Parts whose model cannot be read are left out.
pub fn prepare(loader: &CryLoader, look: &VehicleLook) -> Vec<Piece> {
    assemble(look)
        .into_iter()
        .filter_map(
            |Placed {
                 part,
                 wheel,
                 spindle,
             }| {
                // a `.cdf` is a creature wrapper (several dying prefabs use one): creature::spawn reads it
                // as a rigged model, not a static mesh
                let rigged = part.model.ends_with(".cdf")
                    || (part.model.ends_with(".chr")
                        && aa_anim::pose::shared_rig(&part.model).is_ok_and(|r| !r.is_empty()));
                let (model, radius, helpers) = if rigged {
                    let models: Vec<PreparedModel> =
                        super::creature::model_files(&part.model, None)
                            .into_iter()
                            .filter_map(|(path, material)| {
                                loader.prepare_model(&path, &model_options(material, true)).ok()
                            })
                            .collect();
                    // a `.chr` part is its own main model: read the helpers off the prepared model, not a second load
                    let helpers = part_helpers(&part, &models.first()?.loaded().model);
                    (PieceModel::Rigged(models), None, helpers)
                } else {
                    match loader.prepare_model(&part.model, &static_options(&part)) {
                        Ok(m) => {
                            let radius = wheel.and_then(|_| wheel_radius(&m.loaded().model));
                            let helpers = part_helpers(&part, &m.loaded().model);
                            (PieceModel::Static(m), radius, helpers)
                        }
                        Err(e) => {
                            debug!("vehicle part {}: {e}", part.model);
                            return None;
                        }
                    }
                };
                Some(Piece {
                    part,
                    model,
                    wheel: radius,
                    spindle,
                    helpers,
                })
            },
        )
        .collect()
}

/// The model files [`prepare`] reads (the browser downloads them first).
pub fn model_files(look: &VehicleLook) -> Vec<(String, Option<String>)> {
    parts(look).into_iter().map(|p| (p.model, p.material)).collect()
}

/// A wheel's spin state (`FUN_390d6b10`): the holder rotates about the wheel model's axle
/// (model-space Y, which is holder-local -Z after [`cry_rot_to_bevy`]).
#[derive(Component, Clone, Copy, Debug)]
pub struct WheelSpin {
    /// The vehicle root the wheel belongs to.
    pub root: Entity,
    /// Rolling radius (m) from the wheel model's bounds.
    pub radius: f32,
    /// Angle (rad), advanced by `-forward_speed / radius * dt` and wrapped to 2π.
    pub angle: f32,
    /// The mount rotation the spin is added to.
    pub base: Quat,
}

/// The body's `Bone_spindle` (`FUN_390cffd0`) and the lever arms to its front and rear wheels.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Spindle {
    pub front: f32,
    pub rear: f32,
    /// The smoothed angle stored at model+0x3f8 in the original.
    pub angle: f32,
    /// The bone entity, the body's [`Animator`] and the bone index, found on first use.
    pub joint: Option<(Entity, Entity, usize)>,
}

/// The looping idle of a prefab body (`X2::PrefabUnitModel::UpdateIdleAnim`, `FUN_390d6690`): the
/// body's `fist_ba_relaxed_idle` runs while the model exists, whatever its driver or speed. The 1.2
/// callers skip it in prefab damage state 5 (dying) or 6 (dead): `FUN_390e69d0` (DEC 184925),
/// `FUN_390eb7d0` (DEC 187546), `FUN_390ebaa0` (DEC 187601) and the interaction "opened" path
/// `FUN_390d64a0` (DEC 173191). This branch has no dying state yet (PR #69), so it gates on [`Dead`]
/// (state 6). On the body's Animator; [`start_vehicle_idle`] starts it whenever the model has no clip.
/// A destroyed vehicle stops it and plays the `.cal`'s `dead_01`/`dead_02` ([`stop_dead_vehicles`]).
#[derive(Component, Clone, Debug)]
pub struct VehicleIdle {
    pub clip: BoundClip,
    /// The body's death clip (`dead_01`, else `dead_02`), played once when the unit dies.
    pub dead: Option<BoundClip>,
    /// The unit is dead (native: damage state 5 or 6, but only state 6 exists here until PR #69): the
    /// idle stays stopped until a resurrection clears [`Dead`].
    pub stopped: bool,
}

/// Starts the prefab body's idle when the model has no clip, as 1.2 does on model init and refresh.
/// `UpdateIdleAnim` itself has no driver or speed condition; its 1.2 callers skip it in prefab damage
/// state 5 (dying) or 6 (dead) - not for the Mate and Shipyard model kinds. This branch gates on
/// [`Dead`] (state 6) until PR #69 adds dying; ships and land vehicles get the loop.
pub fn start_vehicle_idle(mut bodies: Query<(&VehicleIdle, &mut Animator)>) {
    for (idle, mut animator) in &mut bodies {
        if !idle.stopped && animator.starts_enabled() && animator.current.is_none() {
            animator.play(idle.clip.clone(), true, 0.0);
        }
    }
}

/// Stops one prefab body's idle and plays its `.cal` death clip.
fn stop_vehicle_idle(animator: &mut Animator, idle: &mut VehicleIdle) {
    if idle.stopped {
        return;
    }
    idle.stopped = true;
    animator.stop_layer(0.0);
    animator.random_idle = None;
    match idle.dead.clone() {
        Some(dead) => {
            animator.speed = 1.0;
            animator.play(dead, false, 0.0);
        }
        // no death clip: leave the body on its last pose rather than restarting the idle
        None => animator.current = None,
    }
}

/// Stops a destroyed vehicle's idle (the prefab body's Animator is a child of the root that carries [`Dead`],
/// which `combat::ensure_corpses` does not visit).
fn stop_dead_vehicles(
    dead: Query<Entity, Added<Dead>>,
    children: Query<&Children>,
    mut bodies: Query<(&mut Animator, &mut VehicleIdle)>,
) {
    for root in &dead {
        let Ok(kids) = children.get(root) else { continue };
        for child in kids.iter() {
            let Ok((mut animator, mut idle)) = bodies.get_mut(child) else { continue };
            stop_vehicle_idle(&mut animator, &mut idle);
        }
    }
}

/// A body that appears after its unit was announced dead (a corpse in view) is stopped as soon as it exists.
fn stop_dead_new_bodies(
    mut bodies: Query<(&mut Animator, &mut VehicleIdle, &ChildOf), Added<VehicleIdle>>,
    parents: Query<&Dead>,
) {
    for (mut animator, mut idle, parent) in &mut bodies {
        if parents.contains(parent.parent()) {
            stop_vehicle_idle(&mut animator, &mut idle);
        }
    }
}

/// A resurrected vehicle starts its idle again (the server removed [`Dead`]).
fn resume_vehicle_idle(
    mut removed: RemovedComponents<Dead>,
    children: Query<&Children>,
    mut bodies: Query<(&mut Animator, &mut VehicleIdle)>,
) {
    for root in removed.read() {
        let Ok(kids) = children.get(root) else { continue };
        for child in kids.iter() {
            let Ok((mut animator, mut idle)) = bodies.get_mut(child) else { continue };
            if !idle.stopped {
                continue;
            }
            idle.stopped = false;
            animator.current = None;
        }
    }
}

/// Registers the vehicle systems. Add once.
pub(crate) struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (stop_dead_vehicles, stop_dead_new_bodies, resume_vehicle_idle, start_vehicle_idle).chain(),
        )
        // the spindle turns the animated pose, not the bind pose: run after the clips write the bones
        .add_systems(
            PostUpdate,
            steer_spindles.after(AnimSet).before(TransformSystems::Propagate),
        );
    }
}

/// The motion of a vehicle root, measured from its transform every frame: the wheel and spindle
/// inputs for a driven cart and for remote carts whose position is played back.
#[derive(Component, Clone, Copy, Debug)]
pub struct VehicleMotion {
    last_pos: Vec3,
    last_yaw: f32,
    /// World horizontal velocity (Bevy space, m/s).
    pub velocity: Vec3,
    /// Yaw rate about +Y (rad/s), positive to the left.
    pub yaw_rate: f32,
    /// Speed along the root's forward (-Z), signed (negative in reverse).
    pub forward_speed: f32,
}

impl VehicleMotion {
    pub(crate) fn at(transform: &Transform) -> Self {
        Self {
            last_pos: transform.translation,
            last_yaw: transform.rotation.to_euler(EulerRot::YXZ).0,
            velocity: Vec3::ZERO,
            yaw_rate: 0.0,
            forward_speed: 0.0,
        }
    }
}

/// The wheel angle after `dt` (`FUN_390d6b10`, non-physics branch): `angle -= forward_speed /
/// radius * dt`, wrapped to 2π.
pub fn wheel_angle(angle: f32, forward_speed: f32, radius: f32, dt: f32) -> f32 {
    if radius <= 1e-4 || dt <= 0.0 {
        return angle;
    }
    wrap_angle(angle - forward_speed / radius * dt)
}

/// The spindle angle `FUN_390cffd0` asks for: with the turn radius `R = |v_forward| / |yaw rate|`,
/// the mean of `atan(front / R)` and `atan(rear / R)`, clamped to [`SPINDLE_MAX_ANGLE`] and signed
/// by the direction of the turn (so it flips in reverse). Without yaw rate it is 0; at a standstill
/// the yaw rate over [`SPINDLE_YAW_LIMIT`] stands in.
pub fn spindle_angle(forward_speed: f32, yaw_rate: f32, front: f32, rear: f32) -> f32 {
    if yaw_rate.abs() < 1e-6 {
        return 0.0;
    }
    // reverse flips the sign; at a standstill the yaw rate alone gives it
    let sign = if forward_speed * yaw_rate < 0.0 || (forward_speed == 0.0 && yaw_rate < 0.0) {
        -1.0
    } else {
        1.0
    };
    let radius = forward_speed.abs() / yaw_rate.abs();
    let magnitude = if radius <= 1e-4 {
        (yaw_rate.abs() / SPINDLE_YAW_LIMIT).min(1.0) * SPINDLE_MAX_ANGLE
    } else {
        (((front.abs() / radius).atan() + (rear.abs() / radius).atan()) * 0.5)
            .min(SPINDLE_MAX_ANGLE)
    };
    sign * magnitude
}

/// One frame of the stored spindle angle: `current + (target - current) * min(dt, 0.1)`.
pub fn spindle_step(current: f32, target: f32, dt: f32) -> f32 {
    current + (target - current) * dt.clamp(0.0, SPINDLE_LERP)
}

/// Adds a piece's `$` helpers: the first piece to name one keeps it, so the body's helpers win over a wheel part's.
fn merge_helpers(helpers: &mut HashMap<String, Affine3A>, piece: HashMap<String, Affine3A>) {
    for (name, helper) in piece {
        helpers.entry(name).or_insert(helper);
    }
}

/// Spawns the prepared parts under a new root at `transform`.
pub fn spawn(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    pieces: Vec<Piece>,
    transform: Transform,
) -> Entity {
    let root = commands
        .spawn((
            transform,
            Visibility::default(),
            VehicleMotion::at(&transform),
        ))
        .id();
    add_parts(commands, cry, clips, pieces, root);
    root
}

/// Spawns the prepared parts as children of `root` and installs the seat helpers and spindle they define.
/// Used for the initial look and for every damage-state body swap; the caller removes the old `Spindle`
/// and the old [`VehiclePart`] children first.
pub fn add_parts(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, pieces: Vec<Piece>, root: Entity) {
    let mut helpers = HashMap::new();
    for Piece {
        part,
        model,
        wheel,
        spindle,
        helpers: piece_helpers,
    } in pieces
    {
        // the body parts come first: a same-named helper on a wheel part never takes the driver's seat
        merge_helpers(&mut helpers, piece_helpers);
        if let Some(arms) = spindle {
            commands.entity(root).insert(Spindle {
                front: arms.front,
                rear: arms.rear,
                ..default()
            });
        }
        let placed = Transform {
            translation: cry_to_bevy(part.pos),
            rotation: cry_rot_to_bevy(part.rot),
            scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
        };
        let entity = match model {
            PieceModel::Static(prepared) => {
                let options = static_options(&part);
                let loaded = cry.finish_model(prepared);
                let model = spawn_loaded(commands, &part.model, &loaded, &options);
                let holder = commands.spawn((placed, Visibility::default())).id();
                commands.entity(model).insert(ChildOf(holder));
                if let Some(radius) = wheel {
                    commands.entity(holder).insert(WheelSpin {
                        root,
                        radius,
                        angle: 0.0,
                        base: placed.rotation,
                    });
                }
                Some(holder)
            }
            PieceModel::Rigged(models) => {
                for m in models {
                    cry.finish_model(m);
                }
                // the vehicle path owns the idle (the 1.2 `UpdateIdleAnim` rule), not the creature spawn
                let spawned = super::creature::spawn_with_idle(commands, cry, clips, &part.model, None, placed, false);
                if let Some(bodies) = &spawned
                    && let Some(idle) = &bodies.idle
                {
                    commands.entity(bodies.root).insert(VehicleIdle {
                        clip: idle.clone(),
                        dead: bodies.dead.clone(),
                        stopped: false,
                    });
                }
                spawned.map(|spawned| spawned.root)
            }
        };
        if let Some(e) = entity {
            commands.entity(e).insert((ChildOf(root), VehiclePart));
        }
    }
    commands.entity(root).insert(SeatHelpers(helpers));
}

/// The prefab's speed-driven emitters. `FUN_390eaf40` collects the type-5 particles named exactly `splash_l`,
/// `splash_r` (the bubble list at +0x2f0) and `exhaust` (the list at +0x2c0); `FUN_390c78d0` scales them from
/// the ship's speed and angular velocity. This slice does not draw them, so a swap leaves them out instead of
/// showing a wake at a standstill. Other active prefab particles are ordinary authored emitters.
fn is_speed_emitter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "exhaust" | "splash_l" | "splash_r" | "bubble"
    )
}

fn spawn_effect(commands: &mut Commands, root: Entity, effect: &aa_world::placement::PlacedEffect, one_shot: bool) -> Entity {
    let mut command = commands.spawn((
        aa_fx::Effect {
            scale: effect.scale,
            speed_scale: effect.speed_scale,
            count_scale: effect.count_scale,
            prime: if one_shot { false } else { effect.prime },
            despawn_when_finished: true,
            no_body: true,
            ..aa_fx::Effect::new(effect.effect.clone())
        },
        Transform {
            translation: cry_to_bevy(effect.pos),
            rotation: cry_rot_to_bevy(effect.rot),
            ..default()
        },
        VehicleEffect,
    ));
    let entity = command.id();
    command.insert(ChildOf(root));
    entity
}

/// Whether a swap plays the new prefab's one-shot `_spawn_effect*` elements (`FUN_39109120`). The Die path
/// always passes 1 to `FUN_390e7910`, so entering dying (5) does too, even on the 6 -> 5 route of an HP-0 update
/// followed by the death packet. A worsening state plays them, except that the dead prefab's one-shots are not
/// replayed when that death already showed 6 through the poller.
fn plays_state_effects(old: u8, state: u8, dead_effect_played: bool) -> bool {
    state == 5 || (state > old && !(state == 6 && dead_effect_played))
}

/// Spawns the particle entities of a damage variant: its active standing effects (damaged fire, dead
/// smoke) and, when [`plays_state_effects`] says so, its `_spawn_effect*` and `_dead_effect` bursts
/// (`FUN_39109120`). A unit that arrives already dead only gets the standing smoke, not the explosion. The
/// `_despawn_effect` trigger has no traced call site and is left out.
pub fn spawn_state_effects(commands: &mut Commands, root: Entity, variant: &str, worsening: bool) {
    if !variant.to_ascii_lowercase().starts_with("prefab://") {
        return;
    }
    for named in super::data::prefab_effects_named(variant) {
        let name = named.name.to_ascii_lowercase();
        if name.starts_with("_spawn_effect") {
            if worsening {
                spawn_effect(commands, root, &named.effect, true);
            }
            continue;
        }
        if name.starts_with("_dead_effect") && !worsening {
            continue;
        }
        if name.starts_with("_despawn_effect") || !named.active || is_speed_emitter(&name) {
            continue;
        }
        spawn_effect(commands, root, &named.effect, false);
    }
}

/// The state check of `FUN_390eb610`: while the unit is dying (5) it only checks the dying window
/// (`FUN_390eaac0`); otherwise the unit's health picks the state. A death (`SCUnitDeath`, i.e. `unit.dead`, or
/// an announcement at 0 hp) enters dying first (`FUN_390e7ad0`), even when an earlier HP-0 update already showed
/// the dead prefab. The new variant's prefab is read on the workers (`FUN_390e7b20` -> `FUN_390e7910`); nothing
/// is removed yet, [`finish_damage_swaps`] applies the swap once the models are ready. A state whose row names
/// the same prefab only updates the state, as `FUN_390e7910` returns before the brush swap ("same model").
pub fn update_damage(
    db: Option<Res<Db>>,
    units: Option<Res<Units>>,
    mut cry: CryAssets,
    swaps: ResMut<VehicleSwaps>,
    time: Res<Time>,
    mut roots: Query<(Entity, &RemoteUnit, &mut VehicleDamage)>,
) {
    let (Some(db), Some(units)) = (db, units) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for (root, remote, mut damage) in &mut roots {
        let Some(unit) = units.by_id.get(&remote.obj_id) else {
            continue;
        };
        if unit.hp > 0 && !unit.dead {
            damage.death_seen = false;
        }
        let mut started = false;
        if unit.dead && !damage.death_seen {
            damage.death_seen = true;
            damage.dying_start = Some(now);
            started = true;
            debug!("vehicle {}: entering dying (5)", remote.obj_id);
        } else if damage.state == 5 && damage.dying_start.is_none() {
            // built already dead at 0 hp (`InitClientUnit` calls Die), so its dying window starts now
            damage.death_seen = true;
            damage.dying_start = Some(now);
            started = true;
        }
        let state = match damage.dying_start {
            // `FUN_390eaac0`: `dying_time <= elapsed` switches to dead; 0 lasts the entry tick only
            Some(start) if !started && now - start >= f64::from(damage.dying_time) => {
                debug!(
                    "vehicle {}: dying expired after {:.3} s",
                    remote.obj_id, damage.dying_time
                );
                6
            }
            Some(_) => 5,
            None => model_state(unit.hp, damage.max_hp),
        };
        if state == damage.state || damage.requested == Some(state) {
            continue;
        }
        let Some(look) = db.vehicle_look(unit.model_id) else {
            adopt_state(&mut damage, state);
            continue;
        };
        let Some(variants) = db.vehicle_variants(unit.model_id) else {
            adopt_state(&mut damage, state);
            continue;
        };
        let variant = variants.for_state(state).to_string();
        if variant == damage.variant {
            adopt_state(&mut damage, state);
            continue;
        }
        debug!(
            "vehicle {}: state {} -> {state} ({variant})",
            remote.obj_id, damage.state
        );
        damage.requested = Some(state);
        let look = VehicleLook {
            normal: variant.clone(),
            wheels: look.wheels,
        };
        let loader = cry.loader();
        let tx = swaps.tx.clone();
        let priority = 0.0;
        let obj_id = remote.obj_id;
        Workers::global().spawn(priority, move || {
            // the browser downloads the models, then their textures, before reading them; natively this runs straight on
            let files = if cfg!(target_arch = "wasm32") {
                let mut files: Vec<String> = model_files(&look)
                    .iter()
                    .flat_map(|(path, material)| {
                        aa_cry::deps::model_files(path, material.as_deref())
                    })
                    .collect();
                files.sort();
                files.dedup();
                files
            } else {
                Vec::new()
            };
            Workers::global().then_after(priority, files.clone(), move || {
                let textures = aa_cry::deps::texture_files(&files);
                Workers::global().then_after(priority, textures, move || {
                    let pieces = prepare(&loader, &look);
                    let _ = tx.send(VehicleSwapBuilt {
                        root,
                        obj_id,
                        state,
                        variant,
                        pieces,
                    });
                });
            });
        });
    }
}

/// Applies the swaps the workers finished: only now are the old parts and effects despawned (the state may have
/// moved on meanwhile, in which case the newer request wins and this build is dropped). Installs the new parts,
/// resets the spindle and the turret bone cache (`TurretGun`; the new skeleton's `bone_yaw` / `bone_pitch` are
/// found again by `turret::aim_turrets`), and plays the new prefab's `_spawn_effect*` when it worsened.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn finish_damage_swaps(
    mut commands: Commands,
    units: Option<Res<Units>>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    swaps: ResMut<VehicleSwaps>,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut VehicleDamage)>,
    stale: Query<(Entity, &ChildOf), Or<(With<VehiclePart>, With<VehicleEffect>)>>,
) {
    let mut done = Vec::new();
    {
        let rx = swaps.rx.lock().unwrap_or_else(|e| e.into_inner());
        while let Ok(built) = rx.try_recv() {
            done.push(built);
        }
    }
    let now = time.elapsed_secs_f64();
    for built in done {
        let Ok((root, mut damage)) = roots.get_mut(built.root) else {
            continue;
        };
        if damage.requested != Some(built.state) {
            continue;
        }
        // the health may have moved on (damage then a heal) while the models were read: keep the old,
        // correct look and let the next frame request the current state. While dying, HP does not drive
        // the state; the dying window decides between 5 and 6.
        let desired = match damage.dying_start {
            Some(start) if now - start >= f64::from(damage.dying_time) => 6,
            Some(_) => 5,
            None => units
                .as_deref()
                .and_then(|units| units.by_id.get(&built.obj_id))
                .map_or(built.state, |unit| model_state(unit.hp, damage.max_hp)),
        };
        if desired != built.state {
            damage.requested = None;
            continue;
        }
        if built.pieces.is_empty() {
            warn!(
                "vehicle {}: the state {} prefab {} has no parts; keeping the old body",
                built.root, built.state, built.variant
            );
            adopt_state(&mut damage, built.state);
            damage.requested = None;
            continue;
        }
        let old = damage.state;
        let effects = plays_state_effects(old, built.state, damage.dead_effect_played);
        for (entity, parent) in &stale {
            if parent.parent() == root {
                commands.entity(entity).try_despawn();
            }
        }
        commands.entity(root).remove::<Spindle>();
        commands.entity(root).remove::<super::turret::TurretGun>();
        add_parts(&mut commands, &mut cry, &clips, built.pieces, root);
        spawn_state_effects(&mut commands, root, &built.variant, effects);
        if built.state == 6 {
            damage.dead_effect_played = true;
        }
        adopt_state(&mut damage, built.state);
        damage.variant = built.variant;
        damage.requested = None;
    }
}

/// Measures each vehicle's motion from its root transform: the position and yaw change since the
/// last frame, in the root's forward frame. A jump over [`TELEPORT_STEP`] is a spawn or teleport,
/// not motion.
pub fn track_motion(time: Res<Time>, mut roots: Query<(&mut VehicleMotion, &Transform)>) {
    let dt = time.delta_secs().min(0.1);
    for (mut motion, transform) in &mut roots {
        let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        let delta = transform.translation - motion.last_pos;
        let teleport = Vec2::new(delta.x, delta.z).length() > TELEPORT_STEP;
        let velocity = if dt > 0.0 && !teleport {
            Vec3::new(delta.x, 0.0, delta.z) / dt
        } else {
            Vec3::ZERO
        };
        let yaw_rate = if dt > 0.0 && !teleport {
            wrap_angle(yaw - motion.last_yaw) / dt
        } else {
            0.0
        };
        let mut forward = transform.rotation * Vec3::NEG_Z;
        forward.y = 0.0;
        let forward = forward.normalize_or_zero();
        motion.velocity = velocity;
        motion.yaw_rate = yaw_rate;
        motion.forward_speed = velocity.dot(forward);
        motion.last_pos = transform.translation;
        motion.last_yaw = yaw;
    }
}

/// A root step longer than this (m) is a teleport or a fresh spawn; its wheels do not spin.
const TELEPORT_STEP: f32 = 2.0;

/// Spins every wheel by its root's forward speed (`FUN_390d6b10`).
pub fn spin_wheels(
    time: Res<Time>,
    motion: Query<&VehicleMotion>,
    mut wheels: Query<(&mut WheelSpin, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    for (mut wheel, mut transform) in &mut wheels {
        let Ok(motion) = motion.get(wheel.root) else {
            continue;
        };
        wheel.angle = wheel_angle(wheel.angle, motion.forward_speed, wheel.radius, dt);
        transform.rotation = wheel.base * Quat::from_axis_angle(Vec3::NEG_Z, wheel.angle);
    }
}

/// Turns the body's `Bone_spindle` by [`spindle_angle`] and [`spindle_step`] (`FUN_390cffd0`), over the rotation
/// the idle has just sampled (PostUpdate, after [`AnimSet`]): the same local Z turn on the bind pose would be
/// overwritten every frame by the animation.
pub fn steer_spindles(
    time: Res<Time>,
    motion: Query<&VehicleMotion>,
    mut spindles: Query<(Entity, &mut Spindle, &Children)>,
    animators: Query<&Animator>,
    mut joints: Query<&mut Transform, Without<Spindle>>,
) {
    let dt = time.delta_secs().min(0.1);
    for (entity, mut spindle, children) in &mut spindles {
        if spindle.joint.is_none() {
            for child in children.iter() {
                let Ok(animator) = animators.get(child) else {
                    continue;
                };
                let Some(index) = animator
                    .rig
                    .skeleton
                    .find("Bone_spindle")
                    .or_else(|| animator.rig.skeleton.find_ignore_case("Bone_spindle"))
                else {
                    continue;
                };
                let Some(&joint) = animator.bones.get(index) else {
                    continue;
                };
                spindle.joint = Some((joint, child, index));
                break;
            }
        }
        let (Some((joint, animator_entity, index)), Ok(motion)) = (spindle.joint, motion.get(entity)) else {
            continue;
        };
        let target = spindle_angle(
            motion.forward_speed,
            motion.yaw_rate,
            spindle.front,
            spindle.rear,
        );
        spindle.angle = spindle_step(spindle.angle, target, dt);
        let Ok(animator) = animators.get(animator_entity) else {
            continue;
        };
        let Some(animated) = animator.pose().rot.get(index).copied() else {
            continue;
        };
        if let Ok(mut transform) = joints.get_mut(joint) {
            transform.rotation = animated * Quat::from_rotation_z(spindle.angle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::SystemState;

    #[test]
    fn vehicle_parts_request_full_illum_shading() {
        let part = Part {
            model: "objects/env/06_unit/05_carriage/transfers_trailer_a_body.chr".into(),
            material: None,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        assert_eq!(static_options(&part).shading, Shading::Full);
        let rigged = model_options(None, true);
        assert_eq!(rigged.shading, Shading::Full, "the body .chr keeps the same Full Illum path");
        assert!(rigged.cry_space);
    }

    #[test]
    fn the_body_helper_wins_over_a_same_named_wheel_helper() {
        let body = Affine3A::from_translation(Vec3::new(-0.02, -0.50, 1.35));
        let wheel = Affine3A::from_translation(Vec3::new(9.0, 9.0, 9.0));
        let mut helpers = HashMap::new();
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), body)]));
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), wheel), ("wheel0".to_string(), wheel)]));
        assert_eq!(helpers["driver"], body, "the body's $driver helper wins");
        assert_eq!(helpers["wheel0"], wheel, "the wheel's own helper is kept");
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_wagon_body_exposes_its_driver_helper() {
        let look = VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf".into()),
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf".into()),
            ],
        };
        let helpers = body_helpers(&look);
        let driver = helpers.get("driver").expect("the body's $driver helper");
        let expected = Vec3::new(-0.02, -0.50, 1.35);
        assert!(
            driver.translation.distance(expected.into()) < 0.01,
            "driver at {:?}",
            driver.translation
        );
    }

    #[test]
    fn helper_names_match_with_or_without_the_dollar_prefix() {
        assert_eq!(helper_key("$driver").as_deref(), Some("driver"));
        assert_eq!(helper_key("$Passenger3").as_deref(), Some("passenger3"));
        assert_eq!(
            helper_key("driver").as_deref(),
            Some("driver"),
            "the red ox cart names it without $"
        );
        assert_eq!(helper_key("passenger0").as_deref(), Some("passenger0"));
        assert_eq!(helper_key("Bone_spindle"), None);
        assert_eq!(helper_key("dummy_passenger"), None);
    }

    /// The red ox cart's body names its two seats without the `$` the attach strings use; with only a `$` prefix scan
    /// its passenger seat fell back to the cart origin.
    #[test]
    #[ignore = "requires the client pak and the game database"]
    fn the_red_ox_cart_exposes_both_seat_helpers() {
        let Some(db) = super::super::data::Db::open() else { return };
        let look = db.vehicle_look(1553).expect("the red ox cart's look");
        let helpers = body_helpers(&look);
        let driver = helpers.get("driver").expect("the cart's driver helper");
        let passenger = helpers.get("passenger0").expect("the cart's passenger helper");
        assert!(driver.translation.length() > 0.1);
        assert!(
            passenger.translation.distance(driver.translation.into()) > 0.1,
            "the passenger and driver helpers are different places"
        );
    }

    #[test]
    fn wheel_mounts_are_named_wheel_n_and_wheel_type2_n() {
        assert_eq!(wheel_slot("wheel0"), Some(0));
        assert_eq!(wheel_slot("Wheel12"), Some(0));
        assert_eq!(wheel_slot("$wheel1"), Some(0));
        assert_eq!(wheel_slot("wheel_type2"), Some(1));
        assert_eq!(wheel_slot("wheel_type21"), Some(1));
        assert_eq!(wheel_slot("wheel"), None);
        assert_eq!(wheel_slot("bone_steeringwheel"), None);
        assert_eq!(wheel_slot("wheel_l"), None);
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_airship_has_its_moving_parts_and_a_cart_its_wheels() {
        let airship = parts(&VehicleLook { normal: "prefab://Prefabs/airship.xml/airship.normal".into(), wheels: [None, None] });
        let models: Vec<&str> = airship.iter().map(|p| p.model.as_str()).collect();
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_base.cgf"), "{models:?}");
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_propeller_l.chr"), "{models:?}");
        assert_eq!(models.len(), 7, "{models:?}");

        let wheel = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf";
        let wheel2 = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf";
        let tractor_look = VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [Some(wheel.into()), Some(wheel2.into())],
        };
        let tractor: Vec<Part> = parts(&tractor_look);
        let wheels: Vec<&Part> = tractor.iter().filter(|p| p.model == wheel || p.model == wheel2).collect();
        assert_eq!(wheels.len(), 4, "{tractor:?}");
        assert_eq!(wheels.iter().filter(|p| p.model == wheel2).count(), 2);
        // left and right of the body
        assert!(wheels.iter().any(|p| p.pos.x < -0.3) && wheels.iter().any(|p| p.pos.x > 0.3), "{wheels:?}");

        // the body's Bone_spindle sits between the front (y 1.61) and rear (y -2.545) mounts
        let arms = assemble(&tractor_look)
            .iter()
            .find_map(|p| p.spindle)
            .expect("Bone_spindle");
        assert!((arms.front - 2.389).abs() < 0.01, "{arms:?}");
        assert!((arms.rear - 1.766).abs() < 0.01, "{arms:?}");

        let front = aa_cry::mesh::Model::load(wheel).unwrap();
        let rear = aa_cry::mesh::Model::load(wheel2).unwrap();
        assert!((wheel_radius(&front).unwrap() - 0.518).abs() < 0.01);
        assert!((wheel_radius(&rear).unwrap() - 0.885).abs() < 0.01);
    }

    /// A spawned Farm Wagon (model 1008) and its root, with the vehicle systems ticking.
    fn wagon() -> Option<(App, Entity)> {
        use bevy::time::TimeUpdateStrategy;
        if !aa_cry::pak_available() {
            return None; // no client pak
        }
        let db = super::super::data::Db::open()?;
        let look = db.vehicle_look(1008).expect("the Farm Wagon's look");
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            VehiclePlugin,
        ));
        app.init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(1.0 / 30.0)));
        let mut state = SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(app.world_mut());
        let pieces = {
            let (_, mut cry, _) = state.get_mut(app.world_mut()).unwrap();
            prepare(&cry.loader(), &look)
        };
        let root = {
            let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
            spawn(&mut commands, &mut cry, &clips, pieces, Transform::default())
        };
        state.apply(app.world_mut());
        app.update();
        Some((app, root))
    }

    /// `UpdateIdleAnim` (`FUN_390d6690`): the Farm Wagon's body loops `fist_ba_relaxed_idle`, so its engine
    /// bones move while the cart stands still. The vehicle idle owns the loop (no generic creature idle).
    #[test]
    fn the_wagon_body_idles_its_engine() {
        let Some((mut app, _)) = wagon() else { return };
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<VehicleIdle>>()
            .iter(app.world())
            .next()
            .expect("the wagon body carries its idle");
        let animator = app.world().get::<aa_anim::Animator>(body).expect("the body's animator");
        let playing = animator.current.as_ref().expect("the idle is playing");
        assert!(
            playing.clip.source.as_deref().is_some_and(|s| s.contains("fist_ba_relaxed_idle")),
            "the body plays its .cal idle: {:?}",
            playing.clip.source
        );
        let bone = find_named(app.world_mut(), "Bone_engine01").expect("the body's Bone_engine01");
        let first = app.world().get::<Transform>(bone).expect("the engine bone transform").rotation;
        for _ in 0..20 {
            app.update();
        }
        let second = app.world().get::<Transform>(bone).expect("the engine bone transform").rotation;
        assert!(first.angle_between(second) > 0.02, "the engine bone moved from {first:?} to {second:?}");
    }

    /// The idle writes `Bone_spindle` every frame; the steering has to turn that sampled rotation, not the bind
    /// pose it was found with. Driving with a yaw rate must leave the bone turned away from the idle pose.
    #[test]
    fn the_idle_does_not_overwrite_the_steered_front_axle() {
        let Some((mut app, root)) = wagon() else { return };
        for _ in 0..10 {
            app.update();
        }
        {
            let mut motion = app.world_mut().get_mut::<VehicleMotion>(root).expect("the vehicle root's motion");
            motion.forward_speed = 4.0;
            motion.yaw_rate = 0.4;
            motion.velocity = Vec3::new(4.0, 0.0, 0.0);
        }
        for _ in 0..30 {
            app.update();
        }
        let spindle = *app.world().get::<Spindle>(root).expect("the wagon's spindle");
        assert!(spindle.angle.abs() > 0.05, "the front axle should steer, angle {}", spindle.angle);
        let (joint, animator_entity, index) = spindle.joint.expect("the spindle bone");
        let animated = app.world().get::<Animator>(animator_entity).expect("the body's animator").pose().rot[index];
        let bone = app.world().get::<Transform>(joint).expect("the spindle bone transform").rotation;
        let turned = animated.angle_between(bone);
        assert!(turned > 0.05, "the idle overwrote the steered spindle (the bone is {turned} from its pose)");
    }

    /// A destroyed vehicle stops its idle and plays the body's `dead_01` (its Animator is a child of the root that
    /// carries `Dead`, so `combat::ensure_corpses` alone never reaches it); a resurrection starts the idle again.
    #[test]
    fn a_destroyed_wagon_stops_its_idle_and_plays_dead_01() {
        let Some((mut app, root)) = wagon() else { return };
        for _ in 0..5 {
            app.update();
        }
        let body = app
            .world_mut()
            .query_filtered::<Entity, With<VehicleIdle>>()
            .iter(app.world())
            .next()
            .expect("the wagon body carries its idle");
        let clip = |app: &App| {
            app.world()
                .get::<Animator>(body)
                .and_then(|a| a.current.as_ref())
                .and_then(|p| p.clip.source.as_deref())
                .map(str::to_string)
        };
        assert!(clip(&app).is_some_and(|s| s.contains("fist_ba_relaxed_idle")), "the idle starts: {:?}", clip(&app));
        app.world_mut().entity_mut(root).insert(Dead { at: 0.0 });
        app.update();
        assert!(app.world().get::<VehicleIdle>(body).unwrap().stopped, "the idle stays stopped on death");
        assert!(clip(&app).is_some_and(|s| s.contains("dead_01")), "the body plays dead_01: {:?}", clip(&app));
        app.world_mut().entity_mut(root).remove::<Dead>();
        for _ in 0..2 {
            app.update();
        }
        assert!(!app.world().get::<VehicleIdle>(body).unwrap().stopped, "resurrection re-enables the idle");
        assert!(clip(&app).is_some_and(|s| s.contains("fist_ba_relaxed_idle")), "the idle resumes: {:?}", clip(&app));
    }

    fn find_named(world: &mut World, name: &str) -> Option<Entity> {
        let mut query = world.query::<(&Name, Entity)>();
        query.iter(world).find(|(n, _)| n.as_str() == name).map(|(_, e)| e)
    }

    #[test]
    fn a_wheel_advances_against_the_forward_speed() {
        // 4 m/s on a 0.5 m wheel for 0.1 s: 0.8 rad backwards (FUN_390d6b10)
        assert!((wheel_angle(0.0, 4.0, 0.5, 0.1) + 0.8).abs() < 1e-5);
        assert!((wheel_angle(0.0, 2.0, 0.5, 0.05) + 0.2).abs() < 1e-5);
        let mut angle = 0.0;
        for _ in 0..100 {
            angle = wheel_angle(angle, 4.0, 0.5, 0.1);
        }
        assert!(
            angle.abs() <= std::f32::consts::PI + 1e-5,
            "wrapped to 2π: {angle}"
        );
    }

    #[test]
    fn a_reversing_wheel_turns_the_other_way() {
        let forward = wheel_angle(0.0, 4.0, 0.5, 0.1);
        let reverse = wheel_angle(0.0, -4.0, 0.5, 0.1);
        assert!(reverse > 0.0 && forward < 0.0);
        assert!((forward + reverse).abs() < 1e-6);
    }

    #[test]
    fn the_spindle_follows_the_turn_radius() {
        // 4 m/s at 0.4 rad/s: R = 10 m; atan(2.389 / 10) and atan(1.766 / 10), averaged
        let expected = ((2.389f32 / 10.0).atan() + (1.766f32 / 10.0).atan()) * 0.5;
        let angle = spindle_angle(4.0, 0.4, 2.389, 1.766);
        assert!((angle - expected).abs() < 1e-5, "{angle} vs {expected}");
        assert!(angle > 0.0 && angle < SPINDLE_MAX_ANGLE);
    }

    #[test]
    fn the_spindle_flips_in_reverse() {
        let forward = spindle_angle(4.0, 0.4, 2.389, 1.766);
        let reverse = spindle_angle(-4.0, 0.4, 2.389, 1.766);
        assert!((forward + reverse).abs() < 1e-5, "{forward} {reverse}");
        // a right turn (negative yaw rate) is negative going forward
        assert!(spindle_angle(4.0, -0.4, 2.389, 1.766) < 0.0);
    }

    #[test]
    fn a_hard_turn_is_clamped_to_thirty_degrees() {
        // R = 0.04 m: atan(2.389 / R) is far past the limit
        assert!((spindle_angle(0.2, 5.0, 2.389, 1.766) - SPINDLE_MAX_ANGLE).abs() < 1e-6);
        assert!((spindle_angle(-0.2, 5.0, 2.389, 1.766) + SPINDLE_MAX_ANGLE).abs() < 1e-6);
        // standstill fallback: yaw rate over the limit, here the captured full-lock rate
        assert!((spindle_angle(0.0, 0.4, 2.389, 1.766) - SPINDLE_MAX_ANGLE).abs() < 1e-6);
        assert_eq!(spindle_angle(0.0, 0.0, 2.389, 1.766), 0.0);
        assert!((spindle_angle(0.0, -0.4, 2.389, 1.766) + SPINDLE_MAX_ANGLE).abs() < 1e-6, "right at a standstill");
    }

    #[test]
    fn the_spindle_is_smoothed_by_min_dt_100ms() {
        assert!((spindle_step(0.0, 1.0, 1.0) - 0.1).abs() < 1e-6);
        assert!((spindle_step(0.0, 1.0, 0.05) - 0.05).abs() < 1e-6);
        let mut angle = 0.0;
        for _ in 0..100 {
            angle = spindle_step(angle, 1.0, 0.1);
        }
        assert!((angle - 1.0).abs() < 1e-4, "converges: {angle}");
    }

    /// `FUN_3989f8b0`: 100/76 -> 1, 75/51 -> 2, 50/26 -> 3, 25 and the truncated 0 -> 4, hp 0 or max 0 -> 6.
    #[test]
    fn the_damage_state_follows_the_decompiled_thresholds() {
        for (hp, max_hp, state) in [
            (100, 100, 1),
            (76, 100, 1),
            (75, 100, 2),
            (51, 100, 2),
            (50, 100, 3),
            (26, 100, 3),
            (25, 100, 4),
            (9, 1000, 4),
            (0, 100, 6),
            (0, 0, 6),
            (100, 0, 6),
        ] {
            assert_eq!(model_state(hp, max_hp), state, "{hp}/{max_hp}");
        }
        // C integer division truncates: 7599 * 100 / 10000 is 75, not 76
        assert_eq!(model_state(7599, 10000), 2);
        assert_eq!(model_state(7600, 10000), 1);
        assert_eq!(model_state(5099, 10000), 3);
        assert_eq!(model_state(5100, 10000), 2);
        assert_eq!(model_state(2599, 10000), 4);
        assert_eq!(model_state(2600, 10000), 3);
    }

    /// `FUN_390e7910(5,1)` plays the prefab effects on every death, including 6 -> 5, but the dead prefab's
    /// one-shots are not replayed when the HP-0 poller already showed 6 for this death.
    #[test]
    fn the_die_path_plays_effects_and_the_dead_ones_only_once() {
        assert!(plays_state_effects(6, 5, false), "6 -> 5 plays dying");
        assert!(plays_state_effects(6, 5, true), "6 -> 5 still plays dying");
        assert!(plays_state_effects(1, 6, false), "a first dead state plays");
        assert!(
            !plays_state_effects(5, 6, true),
            "the dead one-shots do not replay after the poller showed them"
        );
        assert!(!plays_state_effects(4, 2, false), "a heal plays none");
        assert!(plays_state_effects(1, 4, false), "a worse state plays");
    }

    fn slave_unit(obj_id: u32, template_id: u32, model_id: u32, hp: i32) -> aa_net::model::Unit {
        aa_net::model::Unit {
            obj_id,
            name: format!("slave {obj_id}"),
            kind: aa_net::model::UnitKind::Slave {
                id: template_id,
                tl_id: 1,
                template_id,
                summoner_id: 1,
            },
            owner_name: String::new(),
            pos: aa_net::Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 50,
            model_id,
            equipment: aa_net::model::Equipment::default(),
            model_params: aa_net::model::ModelParams::default(),
            hp,
            mp: 0,
            max_hp: hp,
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
            turret: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        }
    }

    fn slave_unit_kind(
        obj_id: u32,
        kind: u32,
        template_id: u32,
        model_id: u32,
        hp: i32,
        max_hp: i32,
    ) -> aa_net::model::Unit {
        let mut unit = slave_unit(obj_id, template_id, model_id, hp);
        unit.kind = aa_net::model::UnitKind::Slave {
            id: kind,
            tl_id: 1,
            template_id,
            summoner_id: 1,
        };
        unit.max_hp = max_hp;
        unit
    }

    /// The state of a real slave template on the `SCUnitPoints` path (`Units::apply`), max from attribute 6.
    #[test]
    fn an_hp_update_changes_the_state_of_a_real_slave_template() {
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let max_hp = db
            .slave_max_hp(21, 50)
            .expect("Eznan Cutter template 21's MaxHealth");
        assert_eq!(max_hp, 52_216);
        let unit = slave_unit(7, 21, 393, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit)));
        let state = |units: &Units| units.by_id.get(&7).map(|unit| damage_state(&db, unit));
        assert_eq!(state(&units), Some(1));

        let update = |units: &mut Units, hp: i32| {
            units.apply(&aa_net::Event::UnitPoints {
                obj_id: 7,
                hp,
                mp: 0,
            });
        };
        update(&mut units, max_hp * 3 / 4); // 75 %
        assert_eq!(state(&units), Some(2));
        update(&mut units, max_hp / 2); // 50 %
        assert_eq!(state(&units), Some(3));
        update(&mut units, max_hp / 4); // 25 %
        assert_eq!(state(&units), Some(4));
        update(&mut units, 0);
        assert_eq!(state(&units), Some(6), "dead");
        assert_eq!(units.by_id.get(&99), None, "not in view");
    }

    /// The unit's own `max_hp` (wire / `SCMySlave`) does not override unit attribute 6: a 500-HP slave of
    /// template 21 is at 0 % of 52216, state 4, even when the wire says its maximum is 1000.
    #[test]
    fn the_owned_slave_report_does_not_override_the_attribute() {
        let Some(db) = Db::open() else { return };
        let unit = slave_unit(7, 21, 393, 500);
        assert_eq!(unit.max_hp, 500, "the wire maximum");
        assert_eq!(slave_max_hp(&db, &unit), 52_216);
        assert_eq!(damage_state(&db, &unit), 4);
    }

    /// The damaged and dead tractor prefabs carry the `_spawn_effect*` trigger and the standing fire / smoke.
    #[test]
    fn the_damage_prefabs_name_their_effects() {
        if !aa_cry::pak_available() {
            return;
        }
        let damaged = super::super::data::prefab_effects_named(
            "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body_damage75",
        );
        assert!(
            damaged
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case("_spawn_effect_dmg") && !e.active),
            "the damage trigger"
        );
        assert!(
            damaged.iter().any(|e| e.name == "_spawn" && e.active),
            "the standing fire"
        );
        let dead = super::super::data::prefab_effects_named(
            "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body_dead",
        );
        assert!(
            dead.iter().any(|e| e.name == "_spawn" && e.active),
            "the burning smoke"
        );
        assert!(dead.iter().any(|e| e.name == "_dead_effect" && e.active));
        // the exact names `FUN_390eaf40` collects; a prefab's other active emitters are not speed-driven
        assert!(is_speed_emitter("exhaust"));
        assert!(is_speed_emitter("splash_l"));
        assert!(is_speed_emitter("splash_r"));
        assert!(!is_speed_emitter("_spawn"));
        assert!(!is_speed_emitter("caravel_cruise_pool"));
        assert!(!is_speed_emitter("airship_smoke"));
    }

    /// The Red Farm Hauler (slave 146, model 1553, `vehicle_models` 65) is the owner's destroyed vehicle:
    /// dying carries the fire particle and dead carries the burning smoke and the explosion. Both must be
    /// spawned as `aa_fx::Effect` entities, not just the mesh swap.
    #[test]
    fn the_red_farm_haulers_dying_and_dead_states_spawn_their_effects() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1553) else {
            return;
        };
        let max_hp = db
            .slave_max_hp(146, 50)
            .expect("Red Farm Hauler template 146's MaxHealth");
        assert_eq!(max_hp, 7_296);
        let unit = slave_unit_kind(7, 146, 146, 1553, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, true);
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;

        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(dying.variant.ends_with("tractor_body_dying"));
        let names = effect_names(&mut app, root);
        assert!(
            names
                .iter()
                .any(|name| name == "x2_effects.housing_test.housing_fire_b"),
            "the dying fire: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.starts_with("x2_sounds.")),
            "the sound particle is filtered: {names:?}"
        );

        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(dead.variant.ends_with("tractor_body_dead"));
        let effects = effect_flags(&mut app, root);
        assert!(
            effects
                .iter()
                .any(|(name, _)| name == "x2_effects.fire_smoke.burning_smoke_01"),
            "the dead smoke: {effects:?}"
        );
        assert!(
            effects.iter().any(|(name, despawn)| {
                name == "x2_effects.housing_test.small_ship_exp" && *despawn
            }),
            "the dead explosion is a burst that retires: {effects:?}"
        );
    }

    /// A slave that arrives already dead only gets the dead prefab's standing smoke; the `_dead_effect`
    /// explosion waits for a worsening transition into 6.
    #[test]
    fn a_slave_announced_dead_does_not_play_the_explosion() {
        if !aa_cry::pak_available() {
            return;
        }
        let dead = "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor_c.tractor_body_dead";
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        let root = app.world_mut().spawn(()).id();
        app.add_systems(Startup, move |mut commands: Commands| {
            spawn_state_effects(&mut commands, root, dead, false);
        });
        app.update();
        let names = effect_names(&mut app, root);
        assert!(
            names
                .iter()
                .any(|name| name == "x2_effects.fire_smoke.burning_smoke_01"),
            "the standing smoke: {names:?}"
        );
        assert!(
            !names
                .iter()
                .any(|name| name == "x2_effects.housing_test.small_ship_exp"),
            "no explosion on arrival: {names:?}"
        );
    }

    /// The dead `_dead_effect` burst is spawned unprimed and actually retires when its simulation ends; the
    /// continuous smoke stays.
    #[test]
    fn the_dead_burst_despawns_when_it_finishes() {
        if !aa_cry::pak_available() {
            return;
        }
        use bevy::asset::AssetPlugin;
        use bevy::transform::TransformPlugin;
        let dead = "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor_c.tractor_body_dead";
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            aa_fx::FxPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        app.world_mut().spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        let root = app.world_mut().spawn(Transform::default()).id();
        app.add_systems(Startup, move |mut commands: Commands| {
            spawn_state_effects(&mut commands, root, dead, true);
        });
        app.update();
        let explosion = effect_entity(
            &mut app,
            root,
            "X2_EFFECTS.housing_test.small_ship_exp",
        )
        .expect("the spawned explosion");
        // run the 6 s burst at 30x so the test does not take its real time
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_relative_speed(30.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.world().get_entity(explosion).is_ok() {
            assert!(
                std::time::Instant::now() < deadline,
                "the finished burst never retired"
            );
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(
            effect_entity(
                &mut app,
                root,
                "X2_EFFECTS.fire_smoke.burning_smoke_01"
            )
            .is_some(),
            "the continuous smoke stays"
        );
    }

    /// A slave first announced already damaged is built in its state's prefab with the same attribute-6
    /// maximum the per-frame check will use, including a template with no `unit_modifiers` row.
    #[test]
    fn a_slave_first_seen_damaged_plans_its_state() {
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let damaged = slave_unit(7, 60, 1008, 7_296 / 4);
        let plan = super::super::prepare::plan_unit(&db, &damaged);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (4, 7_296));
        assert!(
            plan.look.normal.ends_with("tractor_body_damage75"),
            "{}",
            plan.look.normal
        );

        // template 105 has no unit_modifiers attribute-6 row: the formula's 4796 at level 50 alone
        let no_row = slave_unit(8, 105, 1293, 4_796 / 4);
        let plan = super::super::prepare::plan_unit(&db, &no_row);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("template 105 is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (4, 4_796));

        // a unit announced at 0 hp is dead already: `InitClientUnit` calls Die, so it is built dying
        let dead = slave_unit(9, 60, 1008, 0);
        let plan = super::super::prepare::plan_unit(&db, &dead);
        let super::super::prepare::UnitPlan::Vehicle(plan) = plan else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!((plan.state, plan.max_hp), (5, 7_296));
        assert_eq!(plan.dying_time, 3.3);
        assert!(
            plan.look.normal.ends_with("tractor_body_dying"),
            "{}",
            plan.look.normal
        );
    }

    #[derive(Resource)]
    struct DamageRoot(Entity);

    fn damage_app(
        db: &Db,
        look: VehicleLook,
        unit: aa_net::model::Unit,
        state: u8,
        variant: String,
        max_hp: i32,
        finish: bool,
    ) -> App {
        use bevy::asset::AssetPlugin;
        use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
        let dying_time = db.vehicle_dying_time(unit.model_id);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
        ))
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        .init_asset::<SkinnedMeshInverseBindposes>()
        .insert_resource(db.clone())
        .init_resource::<Units>()
        .init_resource::<VehicleSwaps>();
        app.add_systems(
            Startup,
            move |mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>| {
                let loader = cry.loader();
                let pieces = prepare(&loader, &look);
                assert!(!pieces.is_empty(), "the vehicle's prefab has parts");
                let root = spawn(
                    &mut commands,
                    &mut cry,
                    &clips,
                    pieces,
                    Transform::default(),
                );
                commands.entity(root).insert((
                    RemoteUnit::announced(&unit),
                    VehicleDamage {
                        state,
                        variant: variant.clone(),
                        max_hp,
                        requested: None,
                        dying_start: None,
                        dying_time,
                        death_seen: state == 5,
                        dead_effect_played: false,
                    },
                ));
                commands.insert_resource(DamageRoot(root));
            },
        );
        if finish {
            app.add_systems(Update, (update_damage, finish_damage_swaps).chain());
        } else {
            app.add_systems(Update, update_damage);
        }
        app
    }

    fn set_dying_time(app: &mut App, seconds: f32) {
        let root = app.world().resource::<DamageRoot>().0;
        app.world_mut()
            .get_mut::<VehicleDamage>(root)
            .expect("the vehicle root")
            .dying_time = seconds;
    }

    /// Shorten the running dying window so it ends `seconds` from now, keeping its original start time.
    fn end_dying_window_in(app: &mut App, seconds: f32) {
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        let root = app.world().resource::<DamageRoot>().0;
        let mut damage = app
            .world_mut()
            .get_mut::<VehicleDamage>(root)
            .expect("the vehicle root");
        let start = damage.dying_start.expect("a running dying window");
        damage.dying_time = (now - start) as f32 + seconds.max(0.0);
    }

    fn die(app: &mut App) {
        app.world_mut()
            .resource_mut::<Units>()
            .apply(&aa_net::Event::UnitDeath { obj_id: 7 });
    }

    fn wait_for_state(app: &mut App, wanted: u8) -> VehicleDamage {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            app.update();
            let root = app.world().resource::<DamageRoot>().0;
            let damage = app
                .world()
                .get::<VehicleDamage>(root)
                .expect("the vehicle root")
                .clone();
            if damage.state == wanted {
                return damage;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the swap to state {wanted} never finished (state {})",
                damage.state
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn set_hp(app: &mut App, hp: i32) {
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .get_mut(&7)
            .expect("the vehicle")
            .hp = hp;
    }

    fn parts_under(app: &mut App, root: Entity) -> Vec<Entity> {
        app.world_mut()
            .query::<(Entity, &ChildOf, &VehiclePart)>()
            .iter(app.world())
            .filter(|(_, parent, _)| parent.parent() == root)
            .map(|(entity, _, _)| entity)
            .collect()
    }

    fn effect_names(app: &mut App, root: Entity) -> Vec<String> {
        effect_flags(app, root)
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    fn effect_flags(app: &mut App, root: Entity) -> Vec<(String, bool)> {
        app.world_mut()
            .query::<(&ChildOf, &aa_fx::Effect, &VehicleEffect)>()
            .iter(app.world())
            .filter(|(parent, _, _)| parent.parent() == root)
            .map(|(_, effect, _)| {
                (
                    effect.name.to_ascii_lowercase(),
                    effect.despawn_when_finished,
                )
            })
            .collect()
    }

    fn effect_entity(app: &mut App, root: Entity, name: &str) -> Option<Entity> {
        app.world_mut()
            .query::<(Entity, &ChildOf, &aa_fx::Effect, &VehicleEffect)>()
            .iter(app.world())
            .find(|(_, parent, effect, _)| {
                parent.parent() == root && effect.name.eq_ignore_ascii_case(name)
            })
            .map(|(entity, _, _, _)| entity)
    }

    fn hp_update(app: &mut App, hp: i32) {
        app.world_mut()
            .resource_mut::<Units>()
            .apply(&aa_net::Event::UnitPoints { obj_id: 7, hp, mp: 0 });
    }

    /// A damage state change swaps the body off the main thread: the old parts and effects go only once the
    /// new prefab is ready, the new prefab's `_spawn_effect*` plays on a worsening, and a heal swaps back
    /// without playing one.
    #[test]
    fn update_damage_swaps_the_body_and_effects_and_a_heal_swaps_back() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        assert_eq!(max_hp, 7_296);
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let old_parts = parts_under(&mut app, root);
        assert!(!old_parts.is_empty());
        let fake_effect = app.world_mut().spawn((VehicleEffect, ChildOf(root))).id();

        set_hp(&mut app, max_hp / 4);
        let damage = wait_for_state(&mut app, 4);
        assert!(
            damage.variant.ends_with("tractor_body_damage75"),
            "{}",
            damage.variant
        );
        for part in old_parts {
            assert!(
                app.world().get_entity(part).is_err(),
                "an old part outlives the swap"
            );
        }
        assert!(
            app.world().get_entity(fake_effect).is_err(),
            "an old effect outlives the swap"
        );
        assert!(
            !parts_under(&mut app, root).is_empty(),
            "the new body has parts"
        );
        let one_shots = app
            .world_mut()
            .query::<(&VehicleEffect, &aa_fx::Effect)>()
            .iter(app.world())
            .filter(|(_, effect)| effect.despawn_when_finished)
            .count();
        assert!(
            one_shots > 0,
            "the damage prefab's _spawn_effect_dmg plays once"
        );

        let limits = db.turret_limits(1008).expect("the tractor's turret block");
        app.world_mut()
            .entity_mut(root)
            .insert(super::super::turret::TurretGun {
                limits,
                bones: None,
                bones_checked: true,
            });
        set_hp(&mut app, max_hp * 3 / 4);
        let damage = wait_for_state(&mut app, 2);
        assert!(
            damage.variant.ends_with("tractor_body"),
            "{}",
            damage.variant
        );
        assert!(
            app.world()
                .get::<super::super::turret::TurretGun>(root)
                .is_none(),
            "a swap drops the stale turret bones so the next aim resolves them again"
        );
        assert_eq!(
            app.world_mut()
                .query::<(&VehicleEffect, &aa_fx::Effect)>()
                .iter(app.world())
                .filter(|(_, effect)| effect.despawn_when_finished)
                .count(),
            0,
            "healing back plays no _spawn_effect"
        );
    }

    /// While the worker reads the new prefab the old body is untouched: `update_damage` only records the request,
    /// so a slow read never shows an empty vehicle.
    #[test]
    fn update_damage_keeps_the_old_body_until_the_swap_finishes() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            false,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let old_parts = parts_under(&mut app, root);
        assert!(!old_parts.is_empty());

        set_hp(&mut app, max_hp / 4);
        app.update();
        for part in &old_parts {
            assert!(
                app.world().get_entity(*part).is_ok(),
                "the old body is despawned before the new prefab is ready"
            );
        }
        let damage = app
            .world()
            .get::<VehicleDamage>(root)
            .expect("the vehicle root");
        assert_eq!((damage.state, damage.requested), (1, Some(4)));
        assert_eq!(
            parts_under(&mut app, root),
            old_parts,
            "no new parts before they are ready"
        );
    }

    fn synthetic_turret_bones(app: &mut App, root: Entity) -> (Entity, Entity, Entity) {
        let bone = |name: &str| aa_cry::Bone {
            name: name.into(),
            controller_id: name
                .bytes()
                .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b))),
            parent: None,
            limb_id: 0,
            bind_model: default(),
            bind_local: default(),
            physics: [default(); 2],
        };
        let rig = std::sync::Arc::new(aa_anim::Rig::new(aa_cry::Skeleton {
            bones: vec![bone("bone_yaw"), bone("bone_pitch")],
        }));
        let holder = app
            .world_mut()
            .spawn((ChildOf(root), Transform::default()))
            .id();
        let yaw = app
            .world_mut()
            .spawn((ChildOf(holder), Transform::default()))
            .id();
        let pitch = app
            .world_mut()
            .spawn((ChildOf(holder), Transform::default()))
            .id();
        app.world_mut()
            .entity_mut(holder)
            .insert(Animator::new(rig, vec![yaw, pitch]));
        (holder, yaw, pitch)
    }

    /// After a swap the turret's cached `bone_yaw` / `bone_pitch` entities are gone; with the cache reset,
    /// `aim_turrets` resolves the new prefab's bones on the next frame.
    #[test]
    fn a_swap_reresolves_the_turret_bones() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1008) else {
            return;
        };
        // The turret gate reads the slave kind from the template id: template 73 is a real
        // kind-7 slave on the same tractor model (1008), template 60 is a kind-6 farm wagon.
        let max_hp = db.slave_max_hp(73, 50).expect("the cotton-field cart's MaxHealth");
        let unit = slave_unit_kind(7, 7, 73, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let (old_holder, old_yaw, old_pitch) = synthetic_turret_bones(&mut app, root);
        let limits = db.turret_limits(1008).expect("the tractor's turret block");
        app.world_mut().entity_mut(root).insert((
            super::super::turret::TurretGun {
                limits,
                bones: Some(super::super::turret::BonePair {
                    yaw: old_yaw,
                    pitch: old_pitch,
                    bind_yaw: Quat::IDENTITY,
                    bind_pitch: Quat::IDENTITY,
                    bind_aim: Default::default(),
                }),
                bones_checked: true,
            },
            super::super::turret::TurretTarget(aa_net::TurretAim {
                pitch: 0.2,
                yaw: 0.3,
            }),
        ));

        set_hp(&mut app, max_hp / 4);
        wait_for_state(&mut app, 4);
        assert!(
            app.world()
                .get::<super::super::turret::TurretGun>(root)
                .is_none()
        );
        app.world_mut().entity_mut(old_holder).despawn();
        let (_new_holder, new_yaw, new_pitch) = synthetic_turret_bones(&mut app, root);
        app.add_systems(Update, super::super::turret::aim_turrets);
        app.update();
        let gun = app
            .world()
            .get::<super::super::turret::TurretGun>(root)
            .expect("the new turret cache");
        let bones = gun.bones.expect("the new prefab's bone_yaw / bone_pitch");
        assert_eq!((bones.yaw, bones.pitch), (new_yaw, new_pitch));
        assert_ne!(bones.yaw, old_yaw, "the new bones, not the removed ones");
    }

    fn tractor_app(finish: bool) -> Option<(App, Db, i32)> {
        if !aa_cry::pak_available() {
            return None;
        }
        let db = Db::open()?;
        let look = db.vehicle_look(1008)?;
        let max_hp = db.slave_max_hp(60, 50).expect("the Farm Wagon's MaxHealth");
        let unit = slave_unit_kind(7, 7, 60, 1008, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, finish);
        app.insert_resource(units);
        app.update();
        Some((app, db, max_hp))
    }

    /// `SCUnitDeath` enters dying (5) with the `dying` prefab, keeps it for `models.dying_time`, then switches
    /// to dead (6) (`FUN_390e7ad0`, `FUN_390eaac0`).
    #[test]
    fn a_death_enters_dying_for_its_time_then_switches_to_dead() {
        let Some((mut app, _db, _max_hp)) = tractor_app(true) else {
            return;
        };
        // long enough that a slow prefab build cannot expire the window before state 5 is observed
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("tractor_body_dying"),
            "{}",
            dying.variant
        );
        // a 5 s window still shows dying a fifth of the way in, then the shortened window expires to dead
        end_dying_window_in(&mut app, 5.0);
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        assert_eq!(
            app.world().get::<VehicleDamage>(root).expect("the root").state,
            5,
            "dying does not end early"
        );
        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(
            dead.variant.ends_with("tractor_body_dead"),
            "{}",
            dead.variant
        );
    }

    /// A model without a `dying` row (the real sigetank, model 1465) dies in the next lower prefab
    /// (`FUN_397a7980` walks 5 -> 4 -> 3 -> 2 -> 1).
    #[test]
    fn a_death_without_a_dying_prefab_uses_the_damaged_one() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(1465) else {
            return;
        };
        let variants = db.vehicle_variants(1465).expect("the sigetank's state rows");
        assert!(variants.dying.is_none());
        let max_hp = db.slave_max_hp(60, 50).expect("a maximum");
        let unit = slave_unit_kind(7, 7, 60, 1465, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(&db, look.clone(), unit, 1, look.normal, max_hp, true);
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("sigetank_rus_a_damage75"),
            "{}",
            dying.variant
        );
    }

    /// A model with neither a `dying` nor a `dead` row keeps its `normal` prefab through 5 and 6. The early
    /// `variant == damage.variant` path must still end the dying window, or a revive can never leave 6.
    #[test]
    fn a_dead_model_without_dead_or_dying_rows_heals_again() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let Some(look) = db.vehicle_look(657) else {
            return;
        };
        let variants = db.vehicle_variants(657).expect("the airship's state rows");
        assert!(variants.dying.is_none() && variants.dead.is_none());
        let max_hp = db.slave_max_hp(60, 50).expect("a maximum");
        let unit = slave_unit_kind(7, 7, 60, 657, max_hp, max_hp);
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            look.clone(),
            unit,
            1,
            look.normal.clone(),
            max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert_eq!(dying.variant, look.normal, "no dying prefab: normal");
        end_dying_window_in(&mut app, 0.2);
        wait_for_state(&mut app, 6);
        hp_update(&mut app, max_hp);
        let healed = wait_for_state(&mut app, 1);
        assert_eq!(healed.variant, look.normal);
        assert!(
            app.world()
                .get::<VehicleDamage>(app.world().resource::<DamageRoot>().0)
                .expect("the root")
                .dying_start
                .is_none(),
            "the heal cleared the stale dying window"
        );
    }

    /// The polled HP-0 update may show dead first (`FUN_390eb610` recomputes from HP); the later death still
    /// forces dying, then dead (`0x390e7ad0` has no state guard).
    #[test]
    fn an_hp_zero_update_before_the_death_still_enters_dying() {
        let Some((mut app, _db, _max_hp)) = tractor_app(true) else {
            return;
        };
        set_dying_time(&mut app, 60.0);
        hp_update(&mut app, 0);
        let polled = wait_for_state(&mut app, 6);
        assert!(
            polled.variant.ends_with("tractor_body_dead"),
            "{}",
            polled.variant
        );
        die(&mut app);
        let dying = wait_for_state(&mut app, 5);
        assert!(
            dying.variant.ends_with("tractor_body_dying"),
            "{}",
            dying.variant
        );
        end_dying_window_in(&mut app, 0.2);
        wait_for_state(&mut app, 6);
    }

    /// `dying_time == 0` switches to dead on the update after the death, not in the entry tick
    /// (`FUN_390eaac0` checks `limit <= elapsed`).
    #[test]
    fn a_zero_dying_time_lasts_one_tick() {
        let Some((mut app, _db, _max_hp)) = tractor_app(false) else {
            return;
        };
        set_dying_time(&mut app, 0.0);
        die(&mut app);
        app.update();
        let root = app.world().resource::<DamageRoot>().0;
        let damage = app.world().get::<VehicleDamage>(root).expect("the root");
        assert_eq!(damage.requested, Some(5), "the death tick enters dying");
        app.update();
        let damage = app.world().get::<VehicleDamage>(root).expect("the root");
        assert_eq!(damage.requested, Some(6), "the next update expires dying");
    }

    /// A vehicle announced at 0 hp is built dying (`InitClientUnit` / `OnModelChanged` call Die), and its
    /// dying window then expires to dead like any other death.
    #[test]
    fn a_vehicle_announced_at_zero_hp_starts_dying() {
        if !aa_cry::pak_available() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let unit = slave_unit(7, 60, 1008, 0);
        let super::super::prepare::UnitPlan::Vehicle(plan) =
            super::super::prepare::plan_unit(&db, &unit)
        else {
            panic!("the Farm Wagon is a vehicle");
        };
        assert_eq!(plan.state, 5);
        assert!(plan.look.normal.ends_with("tractor_body_dying"));
        let mut units = Units::default();
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
        let mut app = damage_app(
            &db,
            plan.look.clone(),
            unit,
            plan.state,
            plan.look.normal.clone(),
            plan.max_hp,
            true,
        );
        app.insert_resource(units);
        app.update();
        set_dying_time(&mut app, 60.0);
        let dying = wait_for_state(&mut app, 5);
        assert!(dying.variant.ends_with("tractor_body_dying"));
        end_dying_window_in(&mut app, 0.2);
        let dead = wait_for_state(&mut app, 6);
        assert!(dead.variant.ends_with("tractor_body_dead"));
    }
}
