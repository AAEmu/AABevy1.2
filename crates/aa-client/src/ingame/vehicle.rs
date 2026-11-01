//! Vehicles and ships (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats, siege weapons), drawn from
//! their `normal` look ([`super::data::Db::vehicle_look`]): a single model, or every part of a prefab (the hull and deck
//! brushes, and the `AnimObject` parts such as an airship's wings, propellers and planks). Wheels go on the body's wheel
//! mounts: `wheelN` bones or helper nodes take the `wheel` model, `wheel_type2N` ones `wheel2`.
//!
//! The wheels spin about their axle from the cart's forward speed (`FUN_390d6b10`) and the body's
//! `Bone_spindle` turns the front axle (`FUN_390cffd0`); see [`spin_wheels`] and [`steer_spindles`].
//!
//! Left out: the damaged and dead looks, the prefabs' particle effects, propeller spin, turrets.

use std::collections::HashMap;

use aa_anim::{AnimSet, Animator, ClipLibrary, pose::BoundClip};
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_cry::{CryLoader, PreparedModel};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

use super::cart::{YAW_RATE_PER_STEER, wrap_angle};
use super::combat::Dead;
use super::data::{VehicleLook, model_file};
use super::housing::prefab::{self, Part};

/// `_DAT_399a440c` (0x399a440c = 0.52359879): the largest spindle angle, 30 degrees.
pub const SPINDLE_MAX_ANGLE: f32 = std::f32::consts::FRAC_PI_6;

/// `DAT_3999ccf0` (0x3999ccf0): the smoothing rate is `min(dt, 0.1)`.
pub const SPINDLE_LERP: f32 = 0.1;

/// The yaw rate that means full steering in the standstill fallback: the first float of the
/// model's vehicle-physics block (`*(int *)(vtable+0x1e0) + 0x34`). That block is built at
/// runtime from the vehicle data, so the value cannot be read from the unpacked dll; the captured
/// full-lock rate [`YAW_RATE_PER_STEER`] stands in for it.
pub const SPINDLE_YAW_LIMIT: f32 = YAW_RATE_PER_STEER;

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
                let rigged = part.model.ends_with(".chr")
                    && aa_anim::pose::shared_rig(&part.model).is_ok_and(|r| !r.is_empty());
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
            commands.entity(e).insert(ChildOf(root));
        }
    }
    commands.entity(root).insert(SeatHelpers(helpers));
    root
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
}
