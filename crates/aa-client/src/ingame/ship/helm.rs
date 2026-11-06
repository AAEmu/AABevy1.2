//! The helm visuals of a ship (`X2::ShipUnitModel::Update`, `sub_390EBC50`): the rudder, the steering wheel and the
//! throttle animation, all driven by the steering and throttle of the ship's type-3 movement.
//!
//! * **Input.** The ship's movement controller (`sub_390B0040`) writes the interpolated sample's steering and
//!   throttle into the model every frame: `sub_390BE0B0` -> model+0x35c, `sub_390BE0D0` -> model+0x358. These two
//!   are the only writers. `sub_390B2F30` builds the sample from the type-3 bytes as `steering / 127` and
//!   `throttle / 127` (`0.0078740157`), `sub_390A16B0` lerps both between the two samples around the playback
//!   time, and after the 5 s reset both are 0 (`sub_390B2420`). So the ship this client drives shows the
//!   server's echo too, not the local request. [`super::super::vehicle_track::VehiclePose`] carries both.
//! * **Rudder** (`UpdateRudder`, `sub_390C7410`). `sub_390C7330` finds the first model element named `rudder`
//!   (`sub_39104630`, any element type) and keeps its authored local matrix. Every frame the rudder's matrix is
//!   that matrix times a rotation about its local Z by the steering value **as radians** (`cos` / `sin` of
//!   model+0x35c): full helm (127) turns the rudder 1 rad (57.3 degrees), a positive (D, right) helm swings its
//!   aft edge to starboard. A model without one logs "ship unit model(%u) has no rudder" (the cruise ship names
//!   its rudders `cruise_a_rudder_1`, so it has none).
//! * **Steering wheel** (`UpdateSteeringWheel`, `sub_390D2990`). Skipped when the ship model's path contains
//!   `smallboat`, `soloship` or `coupleduckship`. Otherwise it needs the element attached with type 35, the
//!   doodad bound at attach point 35 `$helms` (`slave_doodad_bindings`: `galleon_helms` on the galleons, the
//!   merchant ship, the cruise ship and the fishing boats, `boardship_bell` on the boarding ship;
//!   `sub_390E8E40` keeps its attach transform at model+0x364). Without it nothing below runs. The target
//!   angle is the steering in degrees times a game option (7.0 while the option object is null), truncated,
//!   and stepped to 45 degrees with C integer division: `45 * ((315 + deg + 44) / 45) - 315`. The drawn
//!   angle (model+0x360) moves toward it at `option / 2` rad/s and stops on it; the element's matrix is its
//!   attach transform times a rotation about its local Y by that angle.
//! * **Steering state** (model+0x354, `sub_390D2900`): 0 when the drawn wheel angle did not change this
//!   frame, 1 when it grew, 2 when it shrank. A change starts an animation action on the unit at attach
//!   point 1 (the helmsman); see [`SteeringState`].
//! * **Throttle animation** (`UpdateThrottleAnim`, `sub_390DB700`). When a character element of the model has
//!   `run_f`, `run_b` or `idle` (`sub_390C20E0`), every frame whose throttle differs from the last one plays
//!   `run_f` (throttle > 0), `run_b` (< 0) or `idle` (0), looped, with a 0.5 s blend, at speed `|throttle|`
//!   (1 for idle). The last throttle starts at -1, so the first frame plays. Only the duck boat's
//!   `coupleduckship_foot.chr` (`idle` / `run_f` / `run_b`) and the boarding ship's telescope (`idle`) have
//!   them among the rigged parts; the fishing boat's paddle wheel `fishboat_wheel.cga` has
//!   `fishboat_wheel_idle/run_f/run_b.anm`, but this port does not play `.cga` node animations.

use aa_anim::{Animator, BoundClip, ClipLibrary};
use aa_net::state::Doodads;
use aa_world::coords::cry_rot_to_bevy;
use bevy::prelude::*;

use super::super::data::Db;
use super::super::doodad_visual::DoodadVisual;
use super::super::units::{RemoteDoodad, RemoteUnit};
use super::super::vehicle::{VehicleIdle, VehicleMotion};

/// The prefab element `UpdateRudder` turns (`sub_390C7330` looks it up by name).
pub const RUDDER: &str = "rudder";

/// The attach point of the steering wheel doodad (`model_attach_point_strings` 35 `$helms`; the element type
/// `UpdateSteeringWheel` looks for).
pub const HELMS_POINT: u8 = 35;

/// The steering-wheel game option `UpdateSteeringWheel` multiplies the angle by when its option object
/// (`dword_39AE6C90`) is null. The object is filled at run time and was not read, so this is the value used.
pub const WHEEL_OPTION: f32 = 7.0;

/// The blend of a throttle clip change (`sub_390DB700`'s parameter block starts with 0.5).
pub const THROTTLE_BLEND: f32 = 0.5;

/// Model paths `UpdateSteeringWheel` skips (`XlFindString` on the model's path).
const NO_WHEEL: [&str; 3] = ["smallboat", "soloship", "coupleduckship"];

/// The helm state (model+0x354) `sub_390D2900` stores. A change plays an animation action on the helmsman:
/// `dword_39AC38BC` for still (also the default pose of point 1 on every ship but the three without a wheel,
/// `sub_390C2150`), `dword_39AC38C0` for growing and `dword_39AC38C4` for shrinking. The ids
/// are hashes in the image's data whose names were not recovered (the likely rows are `anim_actions` 5
/// `ac_steer_idle`, 6 `ac_steer_l_01` and 7 `ac_steer_r_01`), so the helmsman clip is not played here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SteeringState {
    #[default]
    Still = 0,
    /// The wheel angle grew this frame (a positive, right, helm).
    Growing = 1,
    /// The wheel angle shrank this frame.
    Shrinking = 2,
}

/// The helm of one ship root: the inputs the movement controller writes and the drawn wheel.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipHelm {
    /// model+0x35c: the steering, -1..=1, used as radians by the rudder.
    pub steering: f32,
    /// model+0x358: the throttle, -1..=1.
    pub throttle: f32,
    /// The model has a steering wheel (its path names none of [`NO_WHEEL`]).
    pub wheel_turns: bool,
    /// model+0x360: the drawn wheel angle (rad).
    pub wheel: f32,
    /// model+0x354.
    pub state: SteeringState,
}

impl ShipHelm {
    /// A still helm for a ship model whose `ship_models.normal` is `normal`.
    pub fn new(normal: &str) -> ShipHelm {
        ShipHelm {
            wheel_turns: wheel_turns(normal),
            ..default()
        }
    }

    /// One frame of `UpdateSteeringWheel` once the wheel element is known: move the drawn angle toward the
    /// stepped target and set the state from the change.
    pub fn step_wheel(&mut self, dt: f32, option: f32) {
        let before = self.wheel;
        self.wheel = wheel_step(before, wheel_target(self.steering, option), option, dt);
        self.state = if self.wheel > before {
            SteeringState::Growing
        } else if self.wheel < before {
            SteeringState::Shrinking
        } else {
            SteeringState::Still
        };
    }
}

/// Whether `UpdateSteeringWheel` runs for a ship model of this path.
pub fn wheel_turns(normal: &str) -> bool {
    let path = normal.to_ascii_lowercase();
    !NO_WHEEL.iter().any(|name| path.contains(name))
}

/// The wheel's target angle (rad) for `steering` (rad): `deg * option` truncated, then
/// `45 * ((315 + x + 44) / 45) - 315` degrees with C (truncating) integer division, where 315 is
/// `(int)(option * 45)`. It rounds up to the next 45 degrees, except far left where the truncation of a
/// negative sum stops it at -315 (`option` 7: full left helm is -315 degrees, full right +405).
pub fn wheel_target(steering: f32, option: f32) -> f32 {
    let base = (option * 45.0) as i32;
    let x = (steering * 57.295776 * option) as i32;
    let sum = base + x + 44;
    ((45 * (sum / 45) - base) as f32) * 0.017453292
}

/// The drawn wheel angle after `dt`: toward `target` at `option * 0.5` rad/s, stopping on it.
pub fn wheel_step(current: f32, target: f32, option: f32, dt: f32) -> f32 {
    let rate = option * 0.5 * dt;
    if target > current {
        (current + rate).min(target)
    } else if target < current {
        (current - rate).max(target)
    } else {
        current
    }
}

/// The rudder rotation (Bevy space): its authored rotation `base` times a turn of `steering` rad about the
/// rudder's own (cry) Z.
pub fn rudder_rotation(base: Quat, steering: f32) -> Quat {
    base * cry_rot_to_bevy(Quat::from_rotation_z(steering))
}

/// The wheel's local transform: a turn of `angle` rad about the doodad's own (cry) Y, applied to the model's
/// transform under the doodad root.
pub fn wheel_transform(base: Transform, angle: f32) -> Transform {
    Transform::from_rotation(cry_rot_to_bevy(Quat::from_rotation_y(angle))) * base
}

/// The throttle clip `UpdateThrottleAnim` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrottleClip {
    Idle,
    RunF,
    RunB,
}

/// The clip and playback speed for `throttle`: `run_f` at `throttle`, `run_b` at `-throttle`, `idle` at 1.
pub fn throttle_clip(throttle: f32) -> (ThrottleClip, f32) {
    if throttle > 0.0 {
        (ThrottleClip::RunF, throttle)
    } else if throttle < 0.0 {
        (ThrottleClip::RunB, -throttle)
    } else {
        (ThrottleClip::Idle, 1.0)
    }
}

/// The rudder part of a ship body: its authored rotation (Bevy space), turned by [`turn_rudders`].
#[derive(Component, Clone, Copy, Debug)]
pub struct Rudder {
    pub base: Quat,
}

/// The model entity of a ship's `$helms` doodad: the ship root and the model's transform before any turn.
#[derive(Component, Clone, Copy, Debug)]
pub struct HelmWheel {
    pub ship: Entity,
    pub base: Transform,
}

/// A rigged ship part with throttle clips (`sub_390C20E0`), and the throttle its last clip was chosen for
/// (model+0x3a0, -1 at load).
#[derive(Component, Clone)]
pub struct ThrottleClips {
    pub idle: Option<BoundClip>,
    pub run_f: Option<BoundClip>,
    pub run_b: Option<BoundClip>,
    pub last: f32,
    /// The clip this part plays now, so a same-direction change only retimes it.
    playing: Option<ThrottleClip>,
}

impl ThrottleClips {
    /// Binds `idle`, `run_f` and `run_b` from a rigged `.chr` part's `.cal`; `None` when it has none of them.
    pub fn bind(model: &str, clips: &ClipLibrary) -> Option<ThrottleClips> {
        let skeleton = model.strip_suffix(".chr")?;
        let cal = aa_anim::Cal::shared(&format!("{skeleton}.cal")).ok()?;
        let rig = aa_anim::pose::shared_rig(model).ok()?;
        let bind = |name| super::super::anim::bind_clip(&rig, &cal, clips, name);
        let clips = ThrottleClips {
            idle: bind("idle"),
            run_f: bind("run_f"),
            run_b: bind("run_b"),
            last: -1.0,
            playing: None,
        };
        (clips.idle.is_some() || clips.run_f.is_some() || clips.run_b.is_some()).then_some(clips)
    }

    fn clip(&self, kind: ThrottleClip) -> Option<&BoundClip> {
        match kind {
            ThrottleClip::Idle => self.idle.as_ref(),
            ThrottleClip::RunF => self.run_f.as_ref(),
            ThrottleClip::RunB => self.run_b.as_ref(),
        }
    }
}

/// Gives every drawn ship root its [`ShipHelm`] (the ship model's `normal` path decides the wheel).
pub fn attach_ship_helms(
    mut commands: Commands,
    db: Option<Res<Db>>,
    roots: Query<(Entity, &RemoteUnit), (With<VehicleMotion>, Without<ShipHelm>)>,
    mut ships: Local<std::collections::HashMap<u32, Option<String>>>,
) {
    let Some(db) = db else { return };
    for (entity, remote) in &roots {
        let normal = ships.entry(remote.model_id).or_insert_with(|| {
            db.is_ship(remote.model_id).then(|| {
                db.vehicle_look(remote.model_id)
                    .map(|look| look.normal)
                    .unwrap_or_default()
            })
        });
        if let Some(normal) = normal {
            commands.entity(entity).insert(ShipHelm::new(normal));
        }
    }
}

/// Copies the steering and throttle the ship's movement controller draws into its [`ShipHelm`].
pub fn follow_ship_helms(mut ships: Query<(&RemoteUnit, &mut ShipHelm)>) {
    for (remote, mut helm) in &mut ships {
        if let Some((steering, throttle)) = remote.helm()
            && (helm.steering, helm.throttle) != (steering, throttle)
        {
            helm.steering = steering;
            helm.throttle = throttle;
        }
    }
}

/// `UpdateRudder`: turns each rudder part about its own Z by its ship's steering.
pub fn turn_rudders(
    ships: Query<&ShipHelm>,
    mut rudders: Query<(&Rudder, &ChildOf, &mut Transform)>,
) {
    for (rudder, parent, mut transform) in &mut rudders {
        let Ok(helm) = ships.get(parent.parent()) else {
            continue;
        };
        let rotation = rudder_rotation(rudder.base, helm.steering);
        if transform.rotation != rotation {
            transform.rotation = rotation;
        }
    }
}

/// Finds the model of each doodad attached at [`HELMS_POINT`] to a drawn ship and marks it a [`HelmWheel`].
pub fn mark_helm_wheels(
    mut commands: Commands,
    doodads: Option<Res<Doodads>>,
    roots: Query<(&RemoteDoodad, &DoodadVisual, &ChildOf)>,
    ships: Query<(), With<ShipHelm>>,
    models: Query<&Transform, Without<HelmWheel>>,
) {
    let Some(doodads) = doodads else { return };
    for (doodad, visual, parent) in &roots {
        let Some(model) = visual.model else { continue };
        let Ok(base) = models.get(model) else {
            continue;
        };
        if ships.contains(parent.parent())
            && doodads
                .by_id
                .get(&doodad.obj_id)
                .is_some_and(|d| d.attach_point == HELMS_POINT)
        {
            commands.entity(model).insert(HelmWheel {
                ship: parent.parent(),
                base: *base,
            });
        }
    }
}

/// `UpdateSteeringWheel`: steps each wheel-carrying ship's drawn angle once a frame and turns its wheel.
pub fn turn_helm_wheels(
    time: Res<Time>,
    mut ships: Query<&mut ShipHelm>,
    mut wheels: Query<(&HelmWheel, &mut Transform)>,
    mut stepped: Local<Vec<Entity>>,
) {
    stepped.clear();
    for (wheel, mut transform) in &mut wheels {
        let Ok(mut helm) = ships.get_mut(wheel.ship) else {
            continue;
        };
        if !helm.wheel_turns {
            continue;
        }
        // the model has one type-35 element: a second wheel only follows the first one's angle
        if !stepped.contains(&wheel.ship) {
            helm.step_wheel(time.delta_secs(), WHEEL_OPTION);
            stepped.push(wheel.ship);
        }
        let turned = wheel_transform(wheel.base, helm.wheel);
        if *transform != turned {
            *transform = turned;
        }
    }
}

/// `UpdateThrottleAnim`: a throttle change plays its clip on the ship's rigged parts that have it. A change in the
/// same direction (`last * throttle > 0`, the native "keep going" flag) only retimes the playing clip.
pub fn play_throttle_anims(
    ships: Query<&ShipHelm>,
    mut parts: Query<(
        &mut ThrottleClips,
        &ChildOf,
        &mut Animator,
        Option<&VehicleIdle>,
    )>,
) {
    for (mut clips, parent, mut animator, idle) in &mut parts {
        let Ok(helm) = ships.get(parent.parent()) else {
            continue;
        };
        // a dead body plays its death clip instead
        if idle.is_some_and(|idle| idle.stopped) || helm.throttle == clips.last {
            continue;
        }
        let same_way = clips.last * helm.throttle > 0.0;
        clips.last = helm.throttle;
        let (kind, speed) = throttle_clip(helm.throttle);
        let Some(clip) = clips.clip(kind).cloned() else {
            continue;
        };
        if !(same_way && clips.playing == Some(kind)) {
            animator.play(clip, true, THROTTLE_BLEND);
            clips.playing = Some(kind);
        }
        animator.speed = speed;
    }
}

#[cfg(test)]
mod tests;
