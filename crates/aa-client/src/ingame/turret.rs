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
//! The fire packet follows path A of `FUN_390f3b20`: `CSStartSkill` with the mount skill's own
//! skill id, a `Mount` caster (the cannon slave and `mount_skills.id`), a `Position` target at the
//! impact point of the ballistic solve (`FUN_394322f0`) and `rot` 0. The solve launches from the
//! `item_hand_r` muzzle at the skill's `effect_speed`, over `min(300, max_range)`, and traces the
//! arc in 0.1 s steps; the point it sends is the first hit, or the camera hit when the arc hits
//! nothing (the decompile leaves `local_24` unchanged).
//!
//! Capture-pending (see `docs/aa-client-units.md`):
//! * S12-C1: the exact `Position` bytes of a live shot (the solve is from the decompile; the
//!   `item_hand_r` muzzle and the `0x80` option flag are inferred).
//! * S12-C2: which unit `CSTurretState` names (the manned slave here).
//! * S12-C3: the camera ray (the 1.2 screen-centre ray; this client uses the orbit camera).
//! * S12-C4: the `FUN_39094500` / `+0x74` gate before key aim; the actions are 0/1 pitch and
//!   4/5 yaw.
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

/// `FUN_394322f0`: the aim range is `min(300, skills.max_range)` (`0x3999cfe0` = 300).
pub const MAX_AIM_RANGE: f32 = 300.0;

/// `0x399d8d40` = g*dt and `0x399d8d3c` = g*dt/2 with dt 0.1: g = 9.81 m/s².
pub const GRAVITY: f32 = 9.81;

/// The arc trace step (`0x3999ccf0` = 0.1 s).
pub const TRACE_STEP: f32 = 0.1;

/// The `mount_skills` -> `skills` row a gunner's slot fires: the mount skill's own skill id, its
/// projectile speed and its maximum range (`FUN_396f6390(DAT_3a5a4eb0)` + `+0xbc` / `+0x1b0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CannonSkill {
    pub id: u32,
    /// `skills.effect_speed` (`+0xbc`), m/s.
    pub speed: f32,
    pub max_range: f32,
}

/// The `vehicle_models` turret block, radians and radians per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretLimits {
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub yaw_min: f32,
    pub yaw_max: f32,
    pub yaw_rate: f32,
    pub pitch_rate: f32,
    /// `vehicle_models.velocity`: a substitute for the model controller type `FUN_390b3750`
    /// returns (3 = turret, 4 = ship). That type is not in the database copy; a model that can
    /// drive (`velocity > 0`) is not a turret seat (S12-C6).
    pub velocity: f32,
    /// `vehicle_models.installed_turret`: the model aims with keys at a rate (`39425580`).
    pub installed: bool,
}

impl TurretLimits {
    /// The database stores degrees and degrees per second (`vehicle_models`); every angle and
    /// rate this module uses is radians.
    #[allow(clippy::too_many_arguments)]
    pub fn from_degrees(
        pitch_min: f32,
        pitch_max: f32,
        yaw_min: f32,
        yaw_max: f32,
        yaw_rate: f32,
        pitch_rate: f32,
        velocity: f32,
        installed: bool,
    ) -> Option<Self> {
        let values = [
            pitch_min, pitch_max, yaw_min, yaw_max, yaw_rate, pitch_rate, velocity,
        ];
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
            velocity,
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

    /// A real turret block: the row parses and has some aim range. The all-zero rows
    /// (`vehicle_models` 37/40: slaves 59, 64-68, 71, 99) are not turrets.
    pub fn is_aimable(&self) -> bool {
        (self.pitch_max - self.pitch_min).abs() > 1e-4 || (self.yaw_max - self.yaw_min).abs() > 1e-4
    }
}

/// The extent of a slave's turret: `Some` only for a kind-7/8 slave with a complete, non-zero
/// `vehicle_models` turret block. A bad data row (a text value in an angle column) gives `None`,
/// so a cart is never mistaken for a turret.
pub fn slave_turret(unit: &Unit, db: &Db) -> Option<TurretLimits> {
    if !matches!(unit.kind, UnitKind::Slave { id: 7 | 8, .. }) {
        return None;
    }
    db.turret_limits(unit.model_id)
        .filter(TurretLimits::is_aimable)
}

/// Whether attaching at the driver point makes this slave a turret the gunner aims. The 1.2
/// gate is the model controller type 3 (`FUN_390b3750`), which the database copy does not
/// carry; `vehicle_models.velocity == 0` is the substitute (S12-C6). A trebuchet or tank
/// (velocity > 0) stays a vehicle the player drives; an all-zero turret block is not a turret.
pub fn turret_seat(unit: &Unit, db: &Db) -> bool {
    let UnitKind::Slave { id, .. } = unit.kind else {
        return false;
    };
    turret_seat_model(id, unit.model_id, db)
}

/// [`turret_seat`] from the two ids the slave unit carries. The real-data test iterates the
/// `slaves` table through this same gate.
pub fn turret_seat_model(kind: u32, model_id: u32, db: &Db) -> bool {
    matches!(kind, 7 | 8)
        && db
            .turret_limits(model_id)
            .is_some_and(|limits| limits.velocity == 0.0 && limits.is_aimable())
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

/// The 1.2 send rule: one global timer (`DAT_3a169940`) and a packet only when the model aim
/// changed. A change inside the 200 ms window is **dropped**, not queued (`FUN_39356990` returns
/// without sending); the server's trailing relay covers the viewers. The state is reset when the
/// local player mounts a gunner seat, so the same clamped aim is sent again.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretSend {
    last_seen: Option<(u32, TurretAim)>,
    last_sent_ms: Option<u64>,
}

impl TurretSend {
    /// Records an observation. Returns `true` when this aim should be written now: it differs
    /// from the last observed aim and the global 200 ms window has passed. A change inside the
    /// window is recorded and dropped.
    pub fn observe(&mut self, now_ms: u64, unit_id: u32, aim: TurretAim) -> bool {
        if self.last_seen == Some((unit_id, aim)) {
            return false;
        }
        self.last_seen = Some((unit_id, aim));
        if self
            .last_sent_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < SEND_INTERVAL_MS)
        {
            return false;
        }
        self.last_sent_ms = Some(now_ms);
        true
    }

    /// A new gunner seat starts its own change detection; the 1.2 timer is global.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The aim the model should reach (the clamped camera aim, or a received type-8 posture).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct TurretTarget(pub TurretAim);

/// The drawn aim, moving towards the target at the model's rates.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretPose(pub TurretAim);

/// The model extent and bones, read once per cannon (a miss is cached, so a kind-7/8 model
/// without `bone_yaw`/`bone_pitch` is not walked every frame).
#[derive(Component, Clone, Debug)]
pub struct TurretGun {
    pub limits: TurretLimits,
    pub bones: Option<BonePair>,
    pub bones_checked: bool,
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
    /// The turret aim sent in `CSTurretState` (model frame).
    pub aim: TurretAim,
    /// The impact point of the 1.2 ballistic solve, game coordinates.
    pub pos: Pos,
    /// The fire skill the solve used (`mount_skills.skill_id`); 0 when the seat has no
    /// `BallisticPos` gunner skill.
    pub skill: u32,
    /// The aim preview's refusal (`FUN_390f3b20` checks `DAT_3a5a4ea4`): the click sends
    /// nothing when it is set.
    pub refusal: Option<u8>,
}

/// The launch and flight of `FUN_394322f0`: the horizontal velocity is `effect_speed` towards
/// the camera hit, the climb is `dz / t + g/2 * t` with `t` the 3-D launch distance / speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ballistic {
    pub start: Vec3,
    /// Bevy axes: `x` / `z` horizontal, `y` up.
    pub velocity: Vec3,
    /// `len / speed`, seconds.
    pub flight: f32,
    /// The 3-D launch distance (`local_58[0]`).
    pub len: f32,
}

/// The 1.2 launch for a muzzle `start` and the camera hit `hit` (`FUN_394322f0`). `None` for a
/// zero horizontal aim or a non-positive speed (the decompile has a fallback direction there).
pub fn launch(start: Vec3, hit: Vec3, speed: f32) -> Option<Ballistic> {
    let delta = hit - start;
    let len = delta.length();
    if !len.is_finite() || len <= 1e-3 || !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    let horizontal = Vec3::new(delta.x, 0.0, delta.z);
    let direction = horizontal.normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    let flight = len / speed;
    let climb = delta.y / flight + 0.5 * GRAVITY * flight;
    Some(Ballistic {
        start,
        velocity: Vec3::new(direction.x * speed, climb, direction.z * speed),
        flight,
        len,
    })
}

impl Ballistic {
    /// The projectile's point at `t` seconds (the same curve the 0.1 s trace samples).
    #[allow(dead_code)]
    pub fn at(&self, t: f32) -> Vec3 {
        self.start + self.velocity * t - Vec3::Y * (0.5 * GRAVITY * t * t)
    }
}

/// The 1.2 trace (`FUN_394322f0` loop): 0.1 s segments until `flight`, the first ground hit, or
/// the camera hit when nothing is hit (`local_24` is left unchanged then).
pub fn trace_impact(
    launch: &Ballistic,
    fallback: Vec3,
    ground: impl Fn(Vec3) -> Option<f32>,
) -> Vec3 {
    let horizontal = Vec3::new(launch.velocity.x, 0.0, launch.velocity.z);
    let mut point = launch.start;
    let mut climb = launch.velocity.y;
    let mut elapsed = 0.0;
    while elapsed <= launch.flight {
        let next = point
            + horizontal * TRACE_STEP
            + Vec3::Y * (climb * TRACE_STEP - 0.5 * GRAVITY * TRACE_STEP * TRACE_STEP);
        if ground(next).is_some_and(|height| next.y <= height) {
            // The segment crosses the surface: find the crossing, as the ray test does.
            let (mut low, mut high) = (point, next);
            for _ in 0..8 {
                let middle = low.lerp(high, 0.5);
                if ground(middle).is_some_and(|height| middle.y > height) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return high;
        }
        point = next;
        climb -= GRAVITY * TRACE_STEP;
        elapsed += TRACE_STEP;
    }
    fallback
}

/// The model-frame aim of a ballistic launch (`FUN_394322f0`): `pitch = atan2(vz, len)` and the
/// horizontal direction turned into the cannon's frame like [`camera_aim`].
pub fn ballistic_aim(
    root: &GlobalTransform,
    start: Vec3,
    hit: Vec3,
    speed: f32,
) -> Option<(Ballistic, TurretAim)> {
    let ballistic = launch(start, hit, speed)?;
    let horizontal = Vec3::new(ballistic.velocity.x, 0.0, ballistic.velocity.z).normalize_or_zero();
    let local = root.rotation().inverse() * horizontal;
    let yaw = aim_from_direction(bevy_to_cry(local))?.yaw;
    Some((
        ballistic,
        TurretAim::new(ballistic.velocity.y.atan2(ballistic.len), yaw),
    ))
}

/// The aim ray of `FUN_394281b0` / `DAT_3a5a4ed8`: from the muzzle along the camera direction,
/// up to `range`. `None` only for a zero direction or range; when the terrain copy holds no
/// ground on the way (a shot over water), the ray's end is the aim point (S12-C1).
pub fn aim_hit(
    start: Vec3,
    direction: Vec3,
    range: f32,
    terrain: Option<&TerrainQuery>,
) -> Option<Vec3> {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO || range <= 0.0 {
        return None;
    }
    let end = start + direction * range;
    let Some(terrain) = terrain else {
        return Some(end);
    };
    let step = 0.5;
    let mut distance = step;
    while distance <= range {
        let point = start + direction * distance;
        let cry = bevy_to_cry(point);
        if terrain
            .ground_at(cry.x, cry.y)
            .is_some_and(|ground| point.y <= ground.height)
        {
            return Some(point);
        }
        distance += step;
    }
    Some(end)
}

/// `FUN_394322f0` stores `pitch`/`yaw` in radians; the model limits are the data row.
pub fn aim_range(max_range: f32) -> f32 {
    if max_range.is_finite() {
        max_range.clamp(0.0, MAX_AIM_RANGE)
    } else {
        0.0
    }
}

/// The key aim (`FUN_39425580`) is only for installed turrets, and only while a key is held.
pub fn keys_apply(limits: TurretLimits, pitch: f32, yaw: f32) -> bool {
    limits.installed && (pitch != 0.0 || yaw != 0.0)
}

/// `FUN_39425580`: one frame of key aim at the model's `turret_*_angvel`, then the model clamp.
pub fn key_step(
    current: TurretAim,
    limits: TurretLimits,
    pitch: f32,
    yaw: f32,
    dt: f32,
) -> TurretAim {
    let dt = dt.clamp(0.0, 0.1);
    limits.clamp(TurretAim {
        pitch: current.pitch + pitch.signum() * limits.pitch_rate * dt,
        yaw: current.yaw + yaw.signum() * limits.yaw_rate * dt,
    })
}

/// The persistent key aim of the manned installed turret (`DAT_3a5a4e94` / `_e98`).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyAim {
    seat: Option<u32>,
    aim: TurretAim,
}

impl KeyAim {
    fn current(&self, seat: u32, initial: TurretAim) -> TurretAim {
        if self.seat == Some(seat) {
            self.aim
        } else {
            initial
        }
    }

    fn set(&mut self, seat: u32, aim: TurretAim) {
        self.seat = Some(seat);
        self.aim = aim;
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The queries of [`local_aim`], grouped so the system stays inside Bevy's parameter tuple limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TurretQueries<'w, 's> {
    cameras: Query<'w, 's, &'static OrbitCamera>,
    roots: Query<'w, 's, (Entity, &'static RemoteUnit, &'static GlobalTransform)>,
    children: Query<'w, 's, &'static Children>,
    animators: Query<'w, 's, &'static aa_anim::Animator>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    targets: Query<'w, 's, &'static mut TurretTarget>,
    guns: Query<'w, 's, &'static mut TurretGun>,
}

/// The turret aim of the local gunner: every frame from the orbit camera, solved ballistically
/// (`FUN_394322f0`), clamped to the cannon's `vehicle_models` limits, sent through the 200 ms
/// change-only rule, and published as [`Sight`] for the HUD.
#[allow(clippy::too_many_arguments)]
pub fn local_aim(
    mut commands: Commands,
    seat: Res<LocalMount>,
    units: Res<aa_net::state::Units>,
    db: Option<Res<Db>>,
    terrain: Option<Res<TerrainQuery>>,
    time: Res<Time>,
    actions: Option<Res<crate::settings::Actions>>,
    mut queries: TurretQueries,
    mut send: ResMut<TurretSend>,
    mut key_aim: ResMut<KeyAim>,
    mut last_seat: Local<Option<u32>>,
    mut muzzle: Local<Option<(u32, Option<Entity>)>>,
    mut out: MessageWriter<NetCommand>,
) {
    // A sight that is not published this frame must not survive: a gunner who just left (or a
    // camera that is not ready) must not keep firing at the old aim.
    commands.remove_resource::<Sight>();
    let seated = seat.turret_seat();
    if *last_seat != seated {
        // A new mount starts its own change detection, so the same clamped aim sends again.
        send.reset();
        key_aim.reset();
        *muzzle = None;
        *last_seat = seated;
    }
    let Some((slave, db)) = seated.zip(db.as_deref()) else {
        return;
    };
    let Some(unit) = units.by_id.get(&slave) else {
        return;
    };
    let Some(limits) = slave_turret(unit, db) else {
        return;
    };
    let Ok(orbit) = queries.cameras.single() else {
        return;
    };
    let Some((entity, _, root)) = queries
        .roots
        .iter()
        .find(|(_, unit, _)| unit.obj_id == slave)
    else {
        return;
    };
    let UnitKind::Slave { template_id, .. } = unit.kind else {
        return;
    };
    // The muzzle is the slave bone `item_hand_r` (`FUN_390b8210`); a model without it falls
    // back to the prefab's 0.7 m aim point (S12-C1).
    let bone = match *muzzle {
        Some((id, bone)) if id == slave => bone,
        _ => {
            let bone = find_muzzle(entity, &queries.children, &queries.animators);
            *muzzle = Some((slave, bone));
            bone
        }
    };
    let start = bone
        .and_then(|bone| queries.globals.get(bone).ok())
        .map_or_else(
            || root.translation() + Vec3::Y * 0.7,
            |bone| bone.translation(),
        );
    // camera_pose total pitch for the ordinary (< 50°) arm; a steeper view is capture-pending
    let camera =
        Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch) * Vec3::NEG_Z;
    let Some(camera) = camera_aim(root, camera) else {
        return;
    };
    let camera = limits.clamp(camera);
    let pitch = actions.as_ref().map_or(0.0, |a| {
        f32::from(i8::from(a.pressed("moveforward", 0)) - i8::from(a.pressed("moveback", 0)))
    });
    let yaw = actions.as_ref().map_or(0.0, |a| {
        f32::from(i8::from(a.pressed("turnleft", 0)) - i8::from(a.pressed("turnright", 0)))
    });
    let keyed = keys_apply(limits, pitch, yaw);
    let key = if keyed {
        let next = key_step(
            key_aim.current(slave, camera),
            limits,
            pitch,
            yaw,
            time.delta_secs(),
        );
        key_aim.set(slave, next);
        Some(next)
    } else {
        None
    };
    let direction = key.map_or_else(
        || Quat::from_rotation_y(orbit.yaw) * Quat::from_rotation_x(orbit.pitch) * Vec3::NEG_Z,
        |aim| root.rotation() * cry_to_bevy(aim_direction(aim)),
    );
    let cannon = db.cannon_skill(template_id);
    let range = cannon.map_or(MAX_AIM_RANGE, |cannon| aim_range(cannon.max_range));
    let hit = aim_hit(start, direction, range, terrain.as_deref());
    let (impact, solve) = match (hit, cannon) {
        (Some(hit), Some(cannon)) => match ballistic_aim(root, start, hit, cannon.speed) {
            Some((ballistic, aim)) => (
                trace_impact(&ballistic, hit, |point| {
                    let cry = bevy_to_cry(point);
                    terrain
                        .as_deref()
                        .and_then(|terrain| terrain.ground_at(cry.x, cry.y))
                        .map(|ground| ground.height)
                }),
                Some(aim),
            ),
            None => (hit, None),
        },
        (Some(hit), None) => (hit, None),
        (None, _) => (start + direction.normalize_or_zero() * range, None),
    };
    let aim = key.unwrap_or_else(|| solve.map_or(camera, |aim| limits.clamp(aim)));
    // `FUN_394322f0` sets result 9 when its aim ray fails (`FUN_394281b0`). The aim itself is
    // not refused here: the decompile's `CanUse` runs on the current unit target, not on the
    // Position (see `local_170` / `DAT_3a5a4d70`), and the turret only clamps.
    let refusal = hit.is_none().then_some(aa_net::skill::result::NO_TARGET);
    match queries.targets.get_mut(entity) {
        Ok(mut target) => target.0 = aim,
        Err(_) => {
            commands.entity(entity).insert(TurretTarget(aim));
        }
    }
    if queries.guns.get(entity).is_err() {
        commands.entity(entity).insert(TurretGun {
            limits,
            bones: None,
            bones_checked: false,
        });
    }
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u64;
    if send.observe(now_ms, slave, aim) {
        out.write(NetCommand(aa_net::Command::TurretState {
            unit_id: slave,
            pitch: aim.pitch,
            yaw: aim.yaw,
        }));
    }
    let cry = bevy_to_cry(impact);
    commands.insert_resource(Sight {
        slave,
        aim,
        pos: Pos::new(cry.x, cry.y, cry.z),
        skill: cannon.map_or(0, |cannon| cannon.id),
        refusal,
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
        let cached = gun.as_deref().and_then(|gun| gun.bones);
        let pair = cached.or_else(|| {
            if gun.as_deref().is_some_and(|gun| gun.bones_checked) {
                None
            } else {
                find_bones(entity, &children, &animators)
            }
        });
        let mut gun = match gun {
            Some(gun) => gun,
            None => {
                commands.entity(entity).insert(TurretGun {
                    limits,
                    bones: pair,
                    bones_checked: true,
                });
                continue;
            }
        };
        if gun.bones.is_none() && !gun.bones_checked {
            gun.bones = pair;
            gun.bones_checked = true;
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

/// The `item_hand_r` bone a cannon fires from (`FUN_390b8210(0, slave, "item_hand_r", ...)`).
fn find_muzzle(
    root: Entity,
    children: &Query<&Children>,
    animators: &Query<&aa_anim::Animator>,
) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        let Ok(names) = children.get(entity) else {
            continue;
        };
        stack.extend(names.iter());
        let Ok(animator) = animators.get(entity) else {
            continue;
        };
        if let Some(index) = animator.rig.skeleton.find_ignore_case("item_hand_r")
            && let Some(bone) = animator.bones.get(index)
        {
            return Some(*bone);
        }
    }
    None
}

/// The turret systems. `local_aim` is placed in the game's main chain (after the seat and the
/// camera); the rest run here.
pub fn plugin(app: &mut App) {
    app.init_resource::<TurretSend>()
        .init_resource::<KeyAim>()
        .add_systems(Update, remote_targets)
        .add_systems(
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
        TurretLimits::from_degrees(-20.0, 35.0, -22.0, 22.0, 35.0, 20.0, 0.0, false).unwrap()
    }

    /// A gunner who left (or has no camera yet) must not keep a stale sight: every early return
    /// in `local_aim` removes it, here the no-seat path.
    #[test]
    fn leaving_the_seat_clears_the_sight() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<aa_net::state::Units>()
            .init_resource::<LocalMount>()
            .init_resource::<TurretSend>()
            .init_resource::<KeyAim>()
            .add_message::<NetCommand>()
            .insert_resource(Sight {
                slave: 7,
                aim: TurretAim::default(),
                pos: Pos::default(),
                skill: 0,
                refusal: None,
            })
            .add_systems(Update, local_aim);
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_none(),
            "a seat that is not manned has no sight"
        );
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
        let full =
            TurretLimits::from_degrees(-10.0, 70.0, -180.0, 180.0, 0.1, 10.0, 2.5, true).unwrap();
        assert!(full.full_circle_yaw());
        let wrapped = full.clamp(TurretAim::new(0.0, 3.0 * std::f32::consts::PI));
        assert!((wrapped.yaw.abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn the_send_rule_is_one_global_200_ms_timer_and_drops_in_window_changes() {
        let mut send = TurretSend::default();
        let a = TurretAim::new(0.1, 0.1);
        let b = TurretAim::new(0.2, 0.2);
        let c = TurretAim::new(0.3, 0.3);
        assert!(send.observe(0, 1, a), "the first aim is sent");
        assert!(!send.observe(0, 1, a), "an unchanged aim never is");
        assert!(
            !send.observe(50, 1, b),
            "a change inside the window is dropped"
        );
        assert!(
            !send.observe(199, 1, b),
            "and stays dropped after the window if it did not change again"
        );
        assert!(
            !send.observe(200, 1, b),
            "the same dropped aim is not retried"
        );
        assert!(send.observe(200, 1, c), "a new aim after the window goes");
        assert!(!send.observe(250, 1, c), "and not twice");
        assert!(send.observe(400, 1, a), "an older aim is a change");
        assert!(
            !send.observe(600, 1, a),
            "unchanged after the window is quiet"
        );

        // The timer is the client's one global DAT_3a169940, not per cannon.
        let mut shared = TurretSend::default();
        assert!(shared.observe(0, 1, a), "cannon 1 sends");
        assert!(
            !shared.observe(100, 2, c),
            "cannon 2 waits for the same 200 ms timer and its aim is dropped"
        );
        assert!(!shared.observe(200, 2, c), "the dropped aim is not retried");
        let d = TurretAim::new(0.4, 0.4);
        assert!(shared.observe(200, 2, d), "a new aim goes after the window");

        // A new mount starts over: the same clamped aim must go again.
        let mut remount = TurretSend::default();
        assert!(remount.observe(0, 1, a));
        remount.reset();
        assert!(
            remount.observe(100, 1, a),
            "a re-mount sends the same aim again"
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
            TurretLimits::from_degrees(-10.0, 70.0, -180.0, 180.0, 360.0, 10.0, 2.5, true).unwrap();
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
        assert!(
            TurretLimits::from_degrees(f32::NAN, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, false).is_none()
        );
        assert!(TurretLimits::from_degrees(1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, false).is_none());
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
        assert_eq!(gun.velocity, 0.0);
        assert!(gun.is_aimable());
        let trebuchet = db.turret_limits(2).expect("model 2 is the trebuchet");
        assert!(trebuchet.installed);
        assert!(trebuchet.full_circle_yaw());
        assert!(trebuchet.velocity > 0.0);
        assert!(
            db.turret_limits(5).is_none(),
            "the siege ladder's yaw max is not a number"
        );
    }

    /// Every kind-7/8 slave row through the same gate `track_local_mount` uses: a model that can
    /// drive (`velocity > 0`) stays a vehicle, a zero-velocity model with a non-zero turret range
    /// is aimable, and an all-zero or unreadable row is neither.
    #[test]
    #[ignore = "requires AA_DB"]
    fn the_real_kind_7_8_rows_split_into_drivable_vehicles_and_aimable_turrets() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let Some(db) = Db::open() else { return };
        let conn = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("read the game database");
        let mut query = conn
            .prepare(
                "SELECT s.id, s.name, s.slave_kind_id, s.model_id, v.velocity FROM slaves s                  JOIN models m ON m.id = s.model_id                  LEFT JOIN vehicle_models v ON v.id = m.sub_id AND m.sub_type = 'VehicleModel'                  WHERE s.slave_kind_id IN (7, 8)",
            )
            .expect("slave query");
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                ))
            })
            .expect("query slaves")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("read slaves");
        let mut aimable = 0;
        let mut drivable = 0;
        for (id, name, kind, model, velocity) in rows {
            let seat = turret_seat_model(kind, model, &db);
            if velocity.is_some_and(|v| v > 0.0) {
                drivable += 1;
                assert!(
                    !seat,
                    "slave {id} {name} (kind {kind}, model {model}) can drive"
                );
                continue;
            }
            if db
                .turret_limits(model)
                .is_some_and(|limits| limits.is_aimable())
            {
                aimable += 1;
                assert!(
                    seat,
                    "slave {id} {name} (kind {kind}, model {model}) is a gun turret"
                );
            } else {
                assert!(
                    !seat,
                    "slave {id} {name} (kind {kind}, model {model}) has no turret range"
                );
            }
        }
        assert!(
            aimable >= 20 && drivable >= 15,
            "aimable {aimable}, drivable {drivable}"
        );
        // The galleon cannons (kind 8 and ground kind 7) aim; the trebuchet, tank, cotton cart and
        // an all-zero human cannon do not.
        assert!(turret_seat_model(8, 117, &db), "ship cannon");
        assert!(turret_seat_model(7, 117, &db), "ground cannon");
        assert!(turret_seat_model(7, 1535, &db), "galleon cannon model 61");
        assert!(
            !turret_seat_model(7, 2, &db),
            "trebuchet drives (velocity 2.5)"
        );
        assert!(
            !turret_seat_model(7, 892, &db),
            "tank drives (velocity 3.5)"
        );
        assert!(
            !turret_seat_model(7, 1008, &db),
            "cotton cart drives (velocity 4)"
        );
        assert!(!turret_seat_model(7, 1005, &db), "all-zero turret row");
    }

    /// `FUN_394322f0`: the arc launches at the skill's `effect_speed` with
    /// `vz = dz/t + g/2*t` and `t = 3-D distance / speed`, then traces its own path. On a
    /// slope the impact is therefore not the camera hit: the 3-D length makes the shot
    /// overshoot horizontally. (A code that sent the camera hit as the target fails here.)
    #[test]
    fn the_ballistic_impact_is_the_arc_not_the_camera_hit() {
        let start = Vec3::new(0.0, 2.0, 0.0);
        let hit = Vec3::new(100.0, 50.0, 0.0);
        let ballistic = launch(start, hit, 25.0).expect("a 110.9 m launch");
        assert!((ballistic.len - 12304f32.sqrt()).abs() < 1e-3);
        assert!((ballistic.flight - 12304f32.sqrt() / 25.0).abs() < 1e-5);
        // The arc reaches the hit's height at the flight time, but past it horizontally.
        let end = ballistic.at(ballistic.flight);
        assert!((end.y - 50.0).abs() < 1e-2, "end {end:?}");
        assert!(end.x > 110.0, "the arc overshoots to {end:?}");
        let slope = |point: Vec3| Some(0.5 * point.x);
        let impact = trace_impact(&ballistic, hit, slope);
        assert!(
            impact.x > 101.0,
            "the impact must be the arc, not the 100 m camera hit: {impact:?}"
        );
        assert!(
            (impact.y - 0.5 * impact.x).abs() < 0.01,
            "the impact is on the slope: {impact:?}"
        );
    }

    /// The apex of the launch: `vz/g` seconds up, `vz² / 2g` above the muzzle.
    #[test]
    fn the_arc_apex_matches_the_launch() {
        let ballistic = launch(Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), 25.0).unwrap();
        let apex_at = ballistic.velocity.y / GRAVITY;
        let apex = ballistic.at(apex_at);
        let climb = ballistic.velocity.y * ballistic.velocity.y / (2.0 * GRAVITY);
        assert!((apex.y - climb).abs() < 1e-3, "{apex:?} vs {climb}");
        assert!((apex.x - ballistic.velocity.x * apex_at).abs() < 1e-3);
        // Range: back at the muzzle height at the flight time, at the camera hit's distance.
        let end = ballistic.at(ballistic.flight);
        assert!(end.y.abs() < 1e-3);
        assert!((end.x - 100.0).abs() < 1e-2, "{end:?}");
    }

    /// `FUN_394322f0`: `min(300.0, FUN_39354310(skill, 2))`.
    #[test]
    fn the_aim_range_is_the_skill_range_capped_at_300() {
        assert_eq!(aim_range(100.0), 100.0);
        assert_eq!(aim_range(1000.0), 300.0);
        assert_eq!(aim_range(0.0), 0.0);
        assert_eq!(aim_range(f32::NAN), 0.0);
    }

    /// `FUN_39425580`: only `vehicle_models.installed_turret` models aim with keys (actions 0/1
    /// pitch, 4/5 yaw) and the step is the row's angular speed.
    #[test]
    fn only_installed_turrets_take_the_key_aim_at_the_model_rate() {
        let cannon = vm10();
        assert!(!keys_apply(cannon, 1.0, 0.0), "a cannon 77 aims by camera");
        let tower =
            TurretLimits::from_degrees(-20.0, 30.0, -180.0, 180.0, 35.0, 20.0, 0.0, true).unwrap();
        assert!(tower.installed && tower.full_circle_yaw());
        assert!(keys_apply(tower, 1.0, 0.0));
        assert!(keys_apply(tower, 0.0, -1.0));
        assert!(!keys_apply(tower, 0.0, 0.0), "no key, no key aim");
        let pitch = key_step(TurretAim::default(), tower, 1.0, 0.0, 0.1);
        assert!((pitch.pitch - tower.pitch_rate * 0.1).abs() < 1e-6);
        let down = key_step(TurretAim::default(), tower, -1.0, 0.0, 0.1);
        assert!((down.pitch + tower.pitch_rate * 0.1).abs() < 1e-6);
        let left = key_step(TurretAim::default(), tower, 0.0, 1.0, 0.1);
        assert!((left.yaw - tower.yaw_rate * 0.1).abs() < 1e-6);
        let right = key_step(TurretAim::default(), tower, 0.0, -1.0, 0.1);
        assert!((right.yaw + tower.yaw_rate * 0.1).abs() < 1e-6);
        // The step clamps to the model window.
        let high = key_step(TurretAim::new(1.0, 0.0), tower, 1.0, 0.0, 10.0);
        assert!((high.pitch - tower.pitch_max).abs() < 1e-6);
    }

    /// `FUN_394322f0` clamps the solved aim for the turret; the shot keeps the unclamped
    /// impact point of the arc.
    #[test]
    fn the_solved_aim_is_clamped_for_the_turret() {
        let limits = vm10();
        let inside = TurretAim::new(0.1, -0.2);
        assert_eq!(limits.clamp(inside), inside);
        let clamped = limits.clamp(TurretAim::new(1.0, 1.0));
        assert!((clamped.pitch - limits.pitch_max).abs() < 1e-6);
        assert!((clamped.yaw - limits.yaw_max).abs() < 1e-6);
    }

    /// The real gunner rows: slave 77's cannon fires `mount_skills` 116 -> skill 17742
    /// (`effect_speed` 25, range 10..100), slave 10's 16 -> 10473 (35, 4..120), and the tower
    /// deployment 20160 (target type 0) is not a ballistic shot.
    #[test]
    #[ignore = "requires AA_DB"]
    fn the_real_cannon_fire_skill_is_the_mount_skill_row() {
        let Some(db) = Db::open() else { return };
        let sample = db.cannon_skill(77).expect("slave 77 fires 17742");
        assert_eq!(sample.id, 17742);
        assert!((sample.speed - 25.0).abs() < 1e-4);
        assert!((sample.max_range - 100.0).abs() < 1e-4);
        let galleon = db.cannon_skill(10).expect("slave 10 fires 10473");
        assert_eq!(galleon.id, 10473);
        assert!((galleon.speed - 35.0).abs() < 1e-4);
        assert!((galleon.max_range - 120.0).abs() < 1e-4);
        let mole = db
            .cannon_skill(70)
            .expect("slave 70 fires 15748 without a point-1 rider");
        assert_eq!(mole.id, 15748);
        assert!((mole.max_range - 22.0).abs() < 1e-4);
        assert!(
            db.cannon_skill(100).is_none(),
            "the tower deployment 20160 is target type 0, not a shot"
        );
    }
}
