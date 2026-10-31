//! Cannons and siege turrets: the gunner's aim, the model bones and the rate-limited
//! `CSTurretState` sends.
//!
//! A cannon is a slave of kind 7 (siege weapon / ground cannon) or 8 (ship cannon) whose
//! `vehicle_models` row has turret limits. The gunner sits at attach point 1 of that slave
//! (`attach_anims` gives the `fist_ac_cannon_idle` / `fist_ac_ballista_idle` /
//! `fist_ba_siegeweapon_idle` pose); the server places the cannon and the gunner follows it.
//!
//! The aim frame follows the 1.2 model: `pitch` up from the model's horizontal plane and `yaw`
//! counter-clockwise about the model's up axis, radians. `39425490` clamps the camera aim to the
//! `vehicle_models` limits (`FUN_3922aed0`); `39425580` does the same for key-driven installed
//! turrets, wrapping a full-circle yaw. `FUN_39356990` sends `CSTurretState` at most once per
//! 200 ms. `FUN_390d14b0` (`UpdateGunTurret`) rate-limits the drawn aim with the
//! `turret_yaw_angvel` / `turret_pitch_angvel` columns and writes `bone_yaw` / `bone_pitch`.
//!
//! Capture-pending (see `docs/aa-client-units.md`):
//! * S12-C1: the aim point of a cannon shot (a straight terrain hit here, not the 1.2 ballistic
//!   solve) and the exact `Position` target bytes.
//! * S12-C2: which unit `CSTurretState` names (the manned slave here).
//! * S12-C3: the camera-derived aim (the 1.2 camera ray and its pitch/yaw frame).
//! * S12-C4: the key indices and key rates of installed-turret models.
//! * S12-C7: the bind-relative bone composition when a turret bone has a non-identity bind.

use aa_net::model::{Unit, UnitKind};
use aa_net::{Event, NetCommand, NetEvent, Pos, TurretAim};
use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::prelude::{TransformSystems, *};

use super::camera::OrbitCamera;
use super::cart::wrap_angle;
use super::data::Db;
use super::mount::LocalMount;
use super::units::RemoteUnit;

/// `FUN_39356990`: `DAT_3a169940 + 200 <= now` before one `CSTurretState`.
pub const SEND_INTERVAL_MS: u64 = 200;

/// The `vehicle_models` turret block, radians and radians per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretLimits {
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub yaw_min: f32,
    pub yaw_max: f32,
    pub yaw_rate: f32,
    pub pitch_rate: f32,
    /// `vehicle_models.installed_turret`: the model aims with keys at a rate (`39425580`).
    pub installed: bool,
}

impl TurretLimits {
    /// The database stores degrees and degrees per second (`vehicle_models`); every angle and
    /// rate this module uses is radians.
    pub fn from_degrees(
        pitch_min: f32,
        pitch_max: f32,
        yaw_min: f32,
        yaw_max: f32,
        yaw_rate: f32,
        pitch_rate: f32,
        installed: bool,
    ) -> Option<Self> {
        let values = [pitch_min, pitch_max, yaw_min, yaw_max, yaw_rate, pitch_rate];
        if values.iter().any(|v| !v.is_finite()) || pitch_min > pitch_max || yaw_min > yaw_max {
            return None;
        }
        let rad = std::f32::consts::PI / 180.0;
        Some(Self {
            pitch_min: pitch_min * rad,
            pitch_max: pitch_max * rad,
            yaw_min: yaw_min * rad,
            yaw_max: yaw_max * rad,
            yaw_rate: yaw_rate.max(0.0) * rad,
            pitch_rate: pitch_rate.max(0.0) * rad,
            installed,
        })
    }

    /// The clamped aim: `FUN_3922aed0` for both angles. A yaw that spans a full circle is
    /// wrapped to `[-π, π]` instead of clamped (the `39425580` branch).
    pub fn clamp(&self, aim: TurretAim) -> TurretAim {
        TurretAim {
            pitch: aim.pitch.clamp(self.pitch_min, self.pitch_max),
            yaw: if self.full_circle_yaw() {
                wrap_angle(aim.yaw)
            } else {
                aim.yaw.clamp(self.yaw_min, self.yaw_max)
            },
        }
    }

    /// `39425580`: a range of a full circle wraps; a narrower one clamps.
    pub fn full_circle_yaw(&self) -> bool {
        self.yaw_max - self.yaw_min >= std::f32::consts::TAU - 1e-4
    }
}

/// The extent of a slave's turret: `Some` only for a kind-7/8 slave with a complete
/// `vehicle_models` turret block. A bad data row (a text value in an angle column) gives `None`,
/// so a cart is never mistaken for a turret.
pub fn slave_turret(unit: &Unit, db: &Db) -> Option<TurretLimits> {
    if !matches!(unit.kind, UnitKind::Slave { id: 7 | 8, .. }) {
        return None;
    }
    db.turret_limits(unit.model_id)
}

/// A direction in the model's own Cry space as an aim: `+Y` is forward, `+X` right, `+Z` up.
/// `yaw` turns counter-clockwise about `+Z`, so a direction to the model's left (`-X`) is a
/// positive yaw.
pub fn aim_from_direction(direction: Vec3) -> Option<TurretAim> {
    let horizontal = direction.truncate().length();
    if !direction.is_finite() || horizontal < 1e-6 {
        return None;
    }
    Some(TurretAim {
        pitch: direction.z.atan2(horizontal),
        yaw: (-direction.x).atan2(direction.y),
    })
}

/// The camera direction in the cannon root's frame, as a model aim.
pub fn camera_aim(root: &GlobalTransform, direction: Vec3) -> Option<TurretAim> {
    let local = root.rotation().inverse() * direction;
    aim_from_direction(bevy_to_cry(local))
}

/// The direction an aim points at, in the model's Cry space.
pub fn aim_direction(aim: TurretAim) -> Vec3 {
    let horizontal = aim.pitch.cos();
    Vec3::new(
        -aim.yaw.sin() * horizontal,
        aim.yaw.cos() * horizontal,
        aim.pitch.sin(),
    )
}

/// `UpdateGunTurret`'s rate limit and wrap: `current` moves towards `target` by at most
/// `rate * dt`, and a wrapped yaw stays in `[-π, π]`.
pub fn step_aim(current: TurretAim, target: TurretAim, limits: TurretLimits, dt: f32) -> TurretAim {
    let dt = dt.max(0.0);
    let step = |value: f32, goal: f32, rate: f32, wrap: bool| {
        let delta = if wrap {
            wrap_angle(goal - value)
        } else {
            goal - value
        };
        let max = rate * dt;
        let next = value + delta.clamp(-max, max);
        if wrap { wrap_angle(next) } else { next }
    };
    TurretAim {
        pitch: step(current.pitch, target.pitch, limits.pitch_rate, false),
        yaw: step(
            current.yaw,
            target.yaw,
            limits.yaw_rate,
            limits.full_circle_yaw(),
        ),
    }
}

/// The 1.2 send rule: a packet only when the aim changed, and no sooner than 200 ms after the
/// previous one. An aim that changed inside the window is sent by the next call after it, so a
/// slow producer never loses the final aim.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretSend {
    last_sent_ms: Option<u64>,
    last_sent: Option<TurretAim>,
}

impl TurretSend {
    /// Returns the aim to send now, or `None` when it did not change or the 200 ms window has
    /// not passed.
    pub fn step(&mut self, now_ms: u64, aim: TurretAim) -> Option<TurretAim> {
        if self.last_sent == Some(aim) {
            return None;
        }
        if self
            .last_sent_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < SEND_INTERVAL_MS)
        {
            return None;
        }
        self.last_sent_ms = Some(now_ms);
        self.last_sent = Some(aim);
        Some(aim)
    }
}

/// The aim the model should reach (the clamped camera aim, or a received type-8 posture).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct TurretTarget(pub TurretAim);

/// The drawn aim, moving towards the target at the model's rates.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretPose(pub TurretAim);

/// The per-cannon send throttle and the model extent/bones read on first aim.
#[derive(Component, Clone, Debug)]
pub struct TurretGun {
    pub limits: TurretLimits,
    pub send: TurretSend,
    pub bones: Option<BonePair>,
}

#[derive(Clone, Copy, Debug)]
pub struct BonePair {
    pub yaw: Entity,
    pub pitch: Entity,
    pub bind_yaw: Quat,
    pub bind_pitch: Quat,
    /// The bind quaternion's own yaw/pitch, subtracted the way `FUN_390d14b0` does.
    pub bind_aim: TurretAim,
}

/// Where the manned cannon is aimed, for the HUD's fire skill (`Command::UseMateSkillAt`).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Sight {
    pub slave: u32,
    pub aim: TurretAim,
    /// The aimed ground point, game coordinates.
    pub pos: Pos,
    /// The spot's facing, radians.
    pub rot: f32,
}

/// The longest aim ray, from the server's cannon skill 10473 (`min_range` 4, `max_range` 120 m;
/// `systems/vehicles/flow_tests.rs`). The data-driven range is capture-pending S12-C1.
const MAX_AIM_RANGE: f32 = 120.0;

/// The cannon's aim point: a straight ray from the model origin (0.7 m up, the prefab's
/// `aimPoint` height) to the terrain (S12-C1).
fn sight_point(
    root: &GlobalTransform,
    aim: TurretAim,
    terrain: Option<&TerrainQuery>,
) -> (Pos, f32) {
    let start = root.translation() + Vec3::Y * 0.7;
    let direction = (root.rotation() * cry_to_bevy(aim_direction(aim))).normalize_or_zero();
    let yaw = (-direction.x).atan2(-direction.z);
    let mut end = start + direction * MAX_AIM_RANGE;
    if let Some(terrain) = terrain {
        let mut distance = 2.0;
        while distance <= MAX_AIM_RANGE {
            let point = start + direction * distance;
            let cry = bevy_to_cry(point);
            if terrain
                .ground_at(cry.x, cry.y)
                .is_some_and(|ground| point.y <= ground.height)
            {
                end = point;
                break;
            }
            distance += 2.0;
        }
    }
    let cry = bevy_to_cry(end);
    (Pos::new(cry.x, cry.y, cry.z), wrap_angle(yaw))
}

/// The turret aim of the local gunner: every frame from the orbit camera, clamped to the
/// cannon's `vehicle_models` limits, sent through the 200 ms change-only rule, and published as
/// [`Sight`] for the HUD.
#[allow(clippy::too_many_arguments)]
pub fn local_aim(
    mut commands: Commands,
    seat: Res<LocalMount>,
    units: Res<aa_net::state::Units>,
    db: Option<Res<Db>>,
    terrain: Option<Res<TerrainQuery>>,
    time: Res<Time>,
    cameras: Query<&OrbitCamera>,
    roots: Query<(Entity, &RemoteUnit, &GlobalTransform)>,
    mut targets: Query<&mut TurretTarget>,
    mut guns: Query<&mut TurretGun>,
    mut out: MessageWriter<NetCommand>,
) {
    let Some((slave, db)) = seat.turret_seat().zip(db.as_deref()) else {
        commands.remove_resource::<Sight>();
        return;
    };
    let Some(unit) = units.by_id.get(&slave) else {
        commands.remove_resource::<Sight>();
        return;
    };
    let Some(limits) = slave_turret(unit, db) else {
        commands.remove_resource::<Sight>();
        return;
    };
    let Ok(orbit) = cameras.single() else { return };
    let Some((entity, _, root)) = roots.iter().find(|(_, unit, _)| unit.obj_id == slave) else {
        return;
    };
    // camera_pose total pitch for the ordinary (< 50°) arm; a steeper view is capture-pending
    let direction =
        Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch) * Vec3::NEG_Z;
    let Some(aim) = camera_aim(root, direction).map(|aim| limits.clamp(aim)) else {
        return;
    };
    match targets.get_mut(entity) {
        Ok(mut target) => target.0 = aim,
        Err(_) => {
            commands.entity(entity).insert(TurretTarget(aim));
        }
    }
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u64;
    let sent = match guns.get_mut(entity) {
        Ok(mut gun) => gun.send.step(now_ms, aim),
        Err(_) => {
            let mut gun = TurretGun {
                limits,
                send: TurretSend::default(),
                bones: None,
            };
            let sent = gun.send.step(now_ms, aim);
            commands.entity(entity).insert(gun);
            sent
        }
    };
    if sent.is_some() {
        out.write(NetCommand(aa_net::Command::TurretState {
            unit_id: slave,
            pitch: aim.pitch,
            yaw: aim.yaw,
        }));
    }
    let (pos, rot) = sight_point(root, aim, terrain.as_deref());
    commands.insert_resource(Sight {
        slave,
        aim,
        pos,
        rot,
    });
}

/// Applies received posture type 8 to remote turrets (and to a local cannon nobody mans).
/// `Unit::turret` also carries the `SCUnitState` snapshot of a late viewer.
pub fn remote_targets(
    mut commands: Commands,
    seat: Res<LocalMount>,
    mut events: MessageReader<NetEvent>,
    roots: Query<(Entity, &RemoteUnit)>,
    mut targets: Query<&mut TurretTarget>,
) {
    let local = seat.turret_seat();
    let entity_of = |obj_id: u32| {
        roots
            .iter()
            .find(|(_, u)| u.obj_id == obj_id)
            .map(|(e, _)| e)
    };
    for NetEvent(event) in events.read() {
        match event {
            Event::UnitPosture {
                obj_id,
                kind,
                turret,
                ..
            } => {
                if local == Some(*obj_id) {
                    continue;
                }
                let Some(entity) = entity_of(*obj_id) else {
                    continue;
                };
                match (*kind, *turret) {
                    (8, Some(aim)) => match targets.get_mut(entity) {
                        Ok(mut target) => target.0 = aim,
                        Err(_) => {
                            commands.entity(entity).insert(TurretTarget(aim));
                        }
                    },
                    (8, None) => {}
                    _ => {
                        commands.entity(entity).remove::<TurretTarget>();
                    }
                }
            }
            Event::UnitSpawned(unit) => {
                let Some(aim) = unit.turret else { continue };
                if local == Some(unit.obj_id) {
                    continue;
                }
                let Some(entity) = entity_of(unit.obj_id) else {
                    continue;
                };
                match targets.get_mut(entity) {
                    Ok(mut target) => target.0 = aim,
                    Err(_) => {
                        commands.entity(entity).insert(TurretTarget(aim));
                    }
                }
            }
            _ => {}
        }
    }
}

/// `UpdateGunTurret` (`FUN_390d14b0`): the drawn aim follows the target at the model's rates and
/// the `bone_yaw` / `bone_pitch` rotations are written. Runs after the animation pass so a clip
/// that touches the turret bones does not overwrite them.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn aim_turrets(
    mut commands: Commands,
    time: Res<Time>,
    units: Res<aa_net::state::Units>,
    db: Option<Res<Db>>,
    mut roots: Query<(
        Entity,
        &RemoteUnit,
        Option<&TurretTarget>,
        Option<&mut TurretGun>,
        Option<&mut TurretPose>,
    )>,
    children: Query<&Children>,
    animators: Query<&aa_anim::Animator>,
    mut bones: Query<&mut Transform, Without<RemoteUnit>>,
) {
    let Some(db) = db.as_deref() else { return };
    let dt = time.delta_secs().min(0.1);
    for (entity, remote, target, gun, pose) in &mut roots {
        let Some(unit) = units.by_id.get(&remote.obj_id) else {
            continue;
        };
        // A late viewer's SCUnitState carries the stored aim (`Unit::turret`); a live
        // SCUnitModelPostureChanged puts the same aim in `TurretTarget` (which also wins for the
        // local gunner, whose camera aim is newer than any server echo).
        let Some(target) = target.map(|target| target.0).or(unit.turret) else {
            continue;
        };
        let stored = gun.as_ref().map(|gun| gun.limits);
        let Some(limits) = stored.or_else(|| slave_turret(unit, db)) else {
            continue;
        };
        let pair = gun
            .as_deref()
            .and_then(|gun| gun.bones)
            .or_else(|| find_bones(entity, &children, &animators));
        let mut gun = match gun {
            Some(gun) => gun,
            None => {
                let gun = TurretGun {
                    limits,
                    send: TurretSend::default(),
                    bones: pair,
                };
                commands.entity(entity).insert(gun);
                continue;
            }
        };
        if gun.bones.is_none() {
            gun.bones = pair;
        }
        let current = pose.as_deref().copied().unwrap_or_default().0;
        let next = step_aim(current, target, limits, dt);
        match pose {
            Some(mut pose) => pose.0 = next,
            None => {
                commands.entity(entity).insert(TurretPose(next));
            }
        }
        let Some(pair) = gun.bones else { continue };
        let yaw = pair.bind_yaw * Quat::from_rotation_z(wrap_angle(next.yaw - pair.bind_aim.yaw));
        let pitch = pair.bind_pitch * Quat::from_rotation_x(next.pitch - pair.bind_aim.pitch);
        if let Ok(mut transform) = bones.get_mut(pair.yaw) {
            transform.rotation = yaw;
        }
        if let Ok(mut transform) = bones.get_mut(pair.pitch) {
            transform.rotation = pitch;
        }
    }
}

/// Finds `bone_yaw` / `bone_pitch` under a vehicle root and keeps their bind rotations. The
/// cannon prefab's `.chr` has both at the identity bind; another model's bind is composed
/// bind-relative the way `FUN_390d14b0` reads it (S12-C7).
fn find_bones(
    root: Entity,
    children: &Query<&Children>,
    animators: &Query<&aa_anim::Animator>,
) -> Option<BonePair> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        let Ok(names) = children.get(entity) else {
            continue;
        };
        stack.extend(names.iter());
        let Ok(animator) = animators.get(entity) else {
            continue;
        };
        let bone = |name: &str| {
            let index = animator.rig.skeleton.find_ignore_case(name)?;
            let entity = animator.bones.get(index).copied()?;
            let bind = animator.rig.bind.rot.get(index).copied()?;
            Some((entity, bind))
        };
        if let (Some((yaw, yaw_bind)), Some((pitch, pitch_bind))) =
            (bone("bone_yaw"), bone("bone_pitch"))
        {
            let (yaw_z, _, _) = yaw_bind.to_euler(EulerRot::ZYX);
            let (_, _, pitch_x) = pitch_bind.to_euler(EulerRot::ZYX);
            return Some(BonePair {
                yaw,
                pitch,
                bind_yaw: yaw_bind,
                bind_pitch: pitch_bind,
                bind_aim: TurretAim {
                    pitch: pitch_x,
                    yaw: yaw_z,
                },
            });
        }
    }
    None
}

/// The turret systems. `local_aim` is placed in the game's main chain (after the seat and the
/// camera); the rest run here.
pub fn plugin(app: &mut App) {
    app.add_systems(Update, remote_targets).add_systems(
        PostUpdate,
        aim_turrets
            .after(aa_anim::AnimSet)
            .before(TransformSystems::Propagate),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vm10() -> TurretLimits {
        TurretLimits::from_degrees(-20.0, 35.0, -22.0, 22.0, 35.0, 20.0, false).unwrap()
    }

    #[test]
    fn the_galleon_cannon_limits_are_the_vehicle_models_row() {
        // vehicle_models 10, the model of slave 10/25/31/... (`models.id` 117): pitch -20..35,
        // yaw -22..22, yaw rate 35 deg/s, pitch rate 20 deg/s, installed_turret false.
        let limits = vm10();
        assert!((limits.pitch_min.to_degrees() + 20.0).abs() < 1e-4);
        assert!((limits.pitch_max.to_degrees() - 35.0).abs() < 1e-4);
        assert!((limits.yaw_min.to_degrees() + 22.0).abs() < 1e-4);
        assert!((limits.yaw_max.to_degrees() - 22.0).abs() < 1e-4);
        assert!((limits.yaw_rate.to_degrees() - 35.0).abs() < 1e-4);
        assert!((limits.pitch_rate.to_degrees() - 20.0).abs() < 1e-4);
        assert!(!limits.full_circle_yaw());
    }

    #[test]
    fn the_camera_aim_clamps_and_a_full_circle_wraps() {
        let limits = vm10();
        let aim = limits.clamp(TurretAim::new(1.2, 1.2));
        assert!((aim.pitch.to_degrees() - 35.0).abs() < 1e-4);
        assert!((aim.yaw.to_degrees() - 22.0).abs() < 1e-4);
        let aim = limits.clamp(TurretAim::new(-1.2, -1.2));
        assert!((aim.pitch.to_degrees() + 20.0).abs() < 1e-4);
        assert!((aim.yaw.to_degrees() + 22.0).abs() < 1e-4);
        // a trebuchet's yaw is a full circle (vehicle_models 2: -180..180)
        let full = TurretLimits::from_degrees(-10.0, 70.0, -180.0, 180.0, 0.1, 10.0, true).unwrap();
        assert!(full.full_circle_yaw());
        let wrapped = full.clamp(TurretAim::new(0.0, 3.0 * std::f32::consts::PI));
        assert!((wrapped.yaw.abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn the_send_rule_is_200_ms_and_change_only() {
        let mut send = TurretSend::default();
        let a = TurretAim::new(0.1, 0.1);
        let b = TurretAim::new(0.2, 0.2);
        assert_eq!(send.step(0, a), Some(a), "the first aim is sent");
        assert_eq!(send.step(0, a), None, "an unchanged aim never is");
        assert_eq!(send.step(100, a), None, "nor inside the window");
        assert_eq!(send.step(50, b), None, "a change inside the window waits");
        assert_eq!(send.step(199, b), None, "the window is 200 ms");
        assert_eq!(
            send.step(200, b),
            Some(b),
            "the newest aim goes at the window"
        );
        assert_eq!(send.step(250, b), None, "and not twice");
        assert_eq!(send.step(400, a), Some(a));
        assert_eq!(
            send.step(600, a),
            None,
            "an unchanged aim stays quiet after the window too"
        );
    }

    #[test]
    fn the_drawn_aim_rate_limits_and_wraps() {
        let limits = vm10();
        let target = TurretAim::new(1.0, 1.0);
        let next = step_aim(TurretAim::default(), target, limits, 0.05);
        assert!((next.pitch - limits.pitch_rate * 0.05).abs() < 1e-6);
        assert!((next.yaw - limits.yaw_rate * 0.05).abs() < 1e-6);
        // never past the target
        let next = step_aim(
            TurretAim::new(0.09, 0.1),
            TurretAim::new(0.1, 0.1),
            limits,
            1.0,
        );
        assert_eq!(next, TurretAim::new(0.1, 0.1));
        // a full-circle yaw wraps by the shortest way
        let full =
            TurretLimits::from_degrees(-10.0, 70.0, -180.0, 180.0, 360.0, 10.0, true).unwrap();
        let next = step_aim(
            TurretAim::new(0.0, 3.0),
            TurretAim::new(0.0, -3.0),
            full,
            1.0,
        );
        // 3.0 -> -3.0 the short way is +0.283 rad, past π, which the model aim wraps back to -3.0
        assert!((next.yaw + 3.0).abs() < 1e-5, "{}", next.yaw);
    }

    #[test]
    fn a_direction_maps_to_the_model_aim() {
        // forward is +Y in the model's Cry space; a left turn is +yaw about +Z
        let aim = aim_from_direction(Vec3::new(0.0, 1.0, 0.0)).unwrap();
        assert!(aim.pitch.abs() < 1e-6 && aim.yaw.abs() < 1e-6);
        let aim = aim_from_direction(Vec3::new(-1.0, 0.0, 0.0)).unwrap();
        assert!((aim.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let aim = aim_from_direction(Vec3::new(0.0, 1.0, 1.0)).unwrap();
        assert!((aim.pitch - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
        assert!(aim_from_direction(Vec3::ZERO).is_none());
    }

    #[test]
    fn a_bad_turret_row_is_not_a_turret() {
        assert!(TurretLimits::from_degrees(f32::NAN, 0.0, 0.0, 0.0, 1.0, 1.0, false).is_none());
        assert!(TurretLimits::from_degrees(1.0, 0.0, 0.0, 0.0, 1.0, 1.0, false).is_none());
    }

    /// The real `models` -> `vehicle_models` rows: model 117 is the galleon cannon (slave 10/25/
    /// 31/...), model 2 the trebuchet. Model 5 (a siege ladder) has a text value in
    /// `turret_yaw_angle_max`, so the row is skipped rather than read as a zero yaw.
    #[test]
    #[ignore = "requires AA_DB"]
    fn the_real_vehicle_models_turret_block_is_read_and_bad_rows_are_skipped() {
        let Some(db) = Db::open() else { return };
        let gun = db
            .turret_limits(117)
            .expect("model 117 is the galleon cannon");
        assert!((gun.pitch_min.to_degrees() + 20.0).abs() < 1e-4);
        assert!((gun.pitch_max.to_degrees() - 35.0).abs() < 1e-4);
        assert!((gun.yaw_min.to_degrees() + 22.0).abs() < 1e-4);
        assert!((gun.yaw_max.to_degrees() - 22.0).abs() < 1e-4);
        assert!((gun.yaw_rate.to_degrees() - 35.0).abs() < 1e-4);
        assert!((gun.pitch_rate.to_degrees() - 20.0).abs() < 1e-4);
        assert!(!gun.installed);
        let trebuchet = db.turret_limits(2).expect("model 2 is the trebuchet");
        assert!(trebuchet.installed);
        assert!(trebuchet.full_circle_yaw());
        assert!(
            db.turret_limits(5).is_none(),
            "the siege ladder's yaw max is not a number"
        );
    }
}
