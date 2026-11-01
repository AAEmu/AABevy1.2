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
//! The camera aim runs only for a slave whose mount skill is `BallisticPos` (target type 11) or
//! `ArtilleryPos` (15), the stand-in for the native mode 8; a drivable cart with a turret block
//! but only cart skills is not a gunner seat. It uses the game's `OrbitCamera`, never a prewarm
//! or cinema camera.
//!
//! The fire packet follows path A of `FUN_390f3b20`: `CSStartSkill` with the mount skill's own
//! skill id, a `Mount` caster (the cannon slave and `mount_skills.id`), a `Position` target at the
//! impact point of the ballistic solve (`FUN_394322f0`) and `rot` 0. The aim ray starts at the
//! camera and runs through the screen centre over `max(300, max_range)`; its hit is pulled into
//! the skill's horizontal angle cone (`FUN_393594c0`), then the arc is solved from the
//! `item_hand_r` muzzle at the skill's `effect_speed` and traced in 0.1 s steps. The point it sends
//! is the first trace hit, or the camera hit when the arc hits nothing (the decompile leaves
//! `local_24` unchanged). A camera ray that hits nothing is refused with `NO_TARGET` (`result 9`)
//! and the click sends nothing (`FUN_394281b0` returns 0); a `BallisticPos` click with no
//! published `Sight` is refused locally with the same code.
//!
//! The camera ray and the arc trace stop at terrain, static brushes, the sea
//! (`World::ocean_level`) and the drawn unit/ship bodies (`FUN_3917dd80` / `FUN_39429750`,
//! mask 0x307). Units are bounding spheres; a ship hull is an oriented box from its hull-floor
//! bounds and the `ship_models` mass box's keel-to-deck/rail height.
//!
//! Key aim (`FUN_394332d0` -> `FUN_39425580`) only turns an installed turret; it never decides
//! the click's target, which stays the camera-arc point, and it does not need a cannon skill. The
//! type-15 `ArtilleryPos` preview of `FUN_394332d0` is not reproduced because path B (the Lua
//! siege-weapon skill) does not use this client's click (S12-C4).
//!
//! Capture-pending (see `docs/aa-client-units.md`):
//! * S12-C1: the exact `Position` bytes of a live shot (the solve is from the decompile; the
//!   `item_hand_r` muzzle and the `0x80` option flag are inferred).
//! * S12-C2: which unit `CSTurretState` names (the manned slave here).
//! * S12-C3: the camera ray (this client uses the drawn camera's screen-centre ray; the 1.2
//!   camera split past 50 degrees is not reproduced).
//! * S12-C4: the `FUN_39094500` / `+0x74` gate before key aim; the actions are 0/1 pitch and
//!   4/5 yaw.
//! * S12-C7: the bind-relative bone composition when a turret bone has a non-identity bind.

use aa_net::model::{Unit, UnitKind};
use aa_net::{Event, NetCommand, NetEvent, Pos, TurretAim};
use aa_world::collide::Mask;
use aa_world::coords::cry_rot_to_bevy;
use aa_world::stream::WorldInfo;
use aa_world::{TerrainQuery, WorldCollision, bevy_to_cry, cry_to_bevy};
use bevy::prelude::{TransformSystems, *};

use super::cart::wrap_angle;
use super::data::Db;
use super::mount::LocalMount;
use super::units::RemoteUnit;

/// `FUN_39356990`: `DAT_3a169940 + 200 <= now` before one `CSTurretState`.
pub const SEND_INTERVAL_MS: u64 = 200;

/// `FUN_394322f0` @ `0x3943238c`: `comiss 300, range; ja keep 300` makes the aim range
/// `max(300, skills.max_range)` (`0x3999cfe0` = 300), so the ray reaches the 300 m floor even
/// when the skill is shorter ranged.
pub const MAX_AIM_RANGE: f32 = 300.0;

/// `0x399d8d40` = g*dt and `0x399d8d3c` = g*dt/2 with dt 0.1: g = 9.81 m/s².
pub const GRAVITY: f32 = 9.81;

/// The arc trace step (`0x3999ccf0` = 0.1 s).
pub const TRACE_STEP: f32 = 0.1;

/// The `mount_skills` -> `skills` row a gunner's slot fires: the mount skill's own skill id, its
/// projectile speed, its range and the horizontal aim cone
/// (`FUN_396f6390(DAT_3a5a4eb0)` + `+0xbc` / `+0x1b0`, and `FUN_393594c0`'s `+0x17c`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CannonSkill {
    pub id: u32,
    /// `skills.effect_speed` (`+0xbc`), m/s.
    pub speed: f32,
    pub min_range: f32,
    pub max_range: f32,
    /// `skills.target_angle`, degrees: the whole cone, so `target_angle / 2 - 1` is the
    /// horizontal half-angle `FUN_393594c0` clamps the camera hit to.
    pub target_angle: f32,
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
    /// `vehicle_models.velocity`: the old HUD bar rule uses `velocity == 0` to show the point-1
    /// attached skill; a drivable cart keeps its mount skills. It is not the camera-aim gate:
    /// that is a `BallisticPos`/`ArtilleryPos` mount skill (S12-C6).
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
/// gate is the model controller type 3 (`FUN_390b3750` = the `X2::VehicleUnitModel` class), i.e.
/// `models.sub_type = 'VehicleModel'`, which `Db::turret_limits` already requires (S12-C6). The
/// all-zero turret rows are not turrets. A drivable vehicle with a turret block (a catapult or a
/// tank) is a turret seat too; it keeps driving through the cart path and aims with the keys when
/// `vehicle_models.installed_turret` is set (`FUN_39425580`). The camera aim is narrower: it also
/// needs a `BallisticPos`/`ArtilleryPos` mount skill (`Db::has_cannon_skill`), so a cart with
/// only Honk horn is a seat but not a mode-8 gunner.
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
            .is_some_and(|limits| limits.is_aimable())
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

/// The 1.2 trace (`FUN_394322f0` loop): 0.1 s segments until `flight`, the first hit of the
/// segment test (`FUN_39429750`), or the camera hit when nothing is hit (`local_24` is left
/// unchanged then).
pub fn trace_impact(
    launch: &Ballistic,
    fallback: Vec3,
    mut segment_hit: impl FnMut(Vec3, Vec3) -> Option<Vec3>,
) -> Vec3 {
    let horizontal = Vec3::new(launch.velocity.x, 0.0, launch.velocity.z);
    let mut point = launch.start;
    let mut climb = launch.velocity.y;
    let mut elapsed = 0.0;
    while elapsed <= launch.flight {
        let next = point
            + horizontal * TRACE_STEP
            + Vec3::Y * (climb * TRACE_STEP - 0.5 * GRAVITY * TRACE_STEP * TRACE_STEP);
        if let Some(hit) = segment_hit(point, next) {
            return hit;
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

/// `FUN_393594c0`: pull the camera hit into the horizontal cone of half-angle
/// `target_angle / 2 - 1 deg` around the slave's facing, keeping the hit's height. The 1.2 code
/// rotates the facing by `min(acos(dot), half-angle)`, so a hit already inside the cone is kept
/// and an outside hit lands on the cone edge at the same horizontal distance. `target_angle` 0
/// or outside 0..=360 is the 1.2 early return (the hit unchanged).
pub fn cone_clamp(origin: Vec3, forward: Vec3, hit: Vec3, target_angle: f32) -> Vec3 {
    if !target_angle.is_finite() || !(0.0..=360.0).contains(&target_angle) || target_angle == 0.0 {
        return hit;
    }
    let half = (target_angle * 0.5 - 1.0).to_radians();
    if half <= 0.0 {
        return hit;
    }
    let forward = Vec3::new(forward.x, 0.0, forward.z);
    let delta = hit - origin;
    let flat = Vec3::new(delta.x, 0.0, delta.z);
    let length = flat.length();
    if length <= 1e-4 {
        return hit;
    }
    let Some(forward) = forward.try_normalize() else {
        return hit;
    };
    let direction = flat / length;
    let angle = forward.dot(direction).clamp(-1.0, 1.0).acos();
    if angle <= half {
        return hit;
    }
    let side = forward.cross(direction).y;
    let side = if side < 0.0 { -1.0 } else { 1.0 };
    let turned = Quat::from_rotation_y(side * half) * forward;
    origin + turned * length + Vec3::Y * delta.y
}

/// A drawn body the aim rays stop at (`FUN_3917dd80` living entities, `FUN_39429750` rigid
/// objects). Units are spheres; a ship hull is the oriented box of its hull floor (the deck
/// footprint) with the `ship_models` mass box's vertical range, keel to deck/rail (S12-C8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AimBody {
    Sphere {
        centre: Vec3,
        radius: f32,
    },
    Box {
        centre: Vec3,
        half: Vec3,
        rotation: Quat,
    },
}

/// Where the camera ray and the arc trace stop: terrain and static brushes, the sea
/// (`World::ocean_level`) and the drawn bodies. All points are Bevy world space.
#[derive(Clone, Copy, Default)]
pub struct AimWorld<'a> {
    pub terrain: Option<&'a TerrainQuery>,
    pub collision: Option<&'a WorldCollision>,
    /// `World::ocean_level`; `None` for a world without a sea.
    pub ocean: Option<f32>,
    pub bodies: &'a [AimBody],
}

impl AimWorld<'_> {
    /// The first point a ray segment touches (`FUN_3917dd80` / `FUN_39429750`).
    pub fn segment(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let delta = to - from;
        let length = delta.length();
        if !length.is_finite() || length <= 1e-4 {
            return None;
        }
        let direction = delta / length;
        let mut best: Option<(f32, Vec3)> = None;
        let mut offer = |distance: f32, point: Vec3| {
            if distance >= 0.0 && distance <= length && best.is_none_or(|(best, _)| distance < best)
            {
                best = Some((distance, point));
            }
        };
        let mut collision_hit = false;
        if let Some(collision) = self.collision {
            // The 1.2 ray masks are 0x307 (static/sleeping/rigid 7, terrain 0x100, water 0x200);
            // `Mask::ALL` is this world's terrain plus its static brushes.
            let origin = bevy_to_cry(from);
            let dir = bevy_to_cry(direction);
            if let Some(hit) = collision.raycast_mask(origin, dir, length, Mask::ALL) {
                collision_hit = true;
                offer(hit.t, cry_to_bevy(hit.point));
            }
        }
        // The height-march is the fallback for terrain outside the collision cells (and for a
        // world built without collision); a collision hit already covers the built terrain.
        if let Some(terrain) = self.terrain.filter(|_| !collision_hit) {
            let (start, end) = (bevy_to_cry(from), bevy_to_cry(to));
            if let Some(point) =
                terrain_crossing(start, end, |x, y| terrain.ground_at(x, y).map(|g| g.height))
            {
                offer((cry_to_bevy(point) - from).length(), cry_to_bevy(point));
            }
        }
        if let Some(level) = self.ocean {
            let (start, end) = (bevy_to_cry(from), bevy_to_cry(to));
            if let Some(point) = water_crossing(start, end, level) {
                offer((cry_to_bevy(point) - from).length(), cry_to_bevy(point));
            }
        }
        for body in self.bodies {
            let distance = match *body {
                AimBody::Sphere { centre, radius } => {
                    sphere_hit(from, direction, length, centre, radius)
                }
                AimBody::Box {
                    centre,
                    half,
                    rotation,
                } => box_hit(from, direction, length, centre, half, rotation),
            };
            if let Some(distance) = distance {
                offer(distance, from + direction * distance);
            }
        }
        best.map(|(_, point)| point)
    }

    /// [`AimWorld::segment`] from `origin` along `direction` for `range` metres.
    pub fn ray(&self, origin: Vec3, direction: Vec3, range: f32) -> Option<Vec3> {
        let direction = direction.normalize_or_zero();
        if direction == Vec3::ZERO || !range.is_finite() || range <= 0.0 {
            return None;
        }
        self.segment(origin, origin + direction * range)
    }
}

/// Where a game-space segment first passes under the terrain height `ground` (marching at 2 m
/// and bisecting the crossing). The collision ray is exact where its cells are built; this is
/// the fallback for terrain outside them.
pub fn terrain_crossing(
    from: Vec3,
    to: Vec3,
    ground: impl Fn(f32, f32) -> Option<f32>,
) -> Option<Vec3> {
    if !from.is_finite() || !to.is_finite() || from == to {
        return None;
    }
    if ground(from.x, from.y).is_some_and(|height| from.z <= height) {
        return Some(from);
    }
    let steps = ((from.distance(to) / 2.0).ceil() as usize).max(1);
    let mut prev = from;
    for step in 1..=steps {
        let point = from.lerp(to, step as f32 / steps as f32);
        if ground(point.x, point.y).is_some_and(|height| point.z <= height) {
            let (mut low, mut high) = (0.0, 1.0);
            for _ in 0..12 {
                let middle = 0.5 * (low + high);
                let probe = prev.lerp(point, middle);
                if ground(probe.x, probe.y).is_some_and(|height| probe.z > height) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return Some(prev.lerp(point, high));
        }
        prev = point;
    }
    None
}

/// Where a game-space segment crosses the sea plane, if it starts above it and ends at or below.
pub fn water_crossing(from: Vec3, to: Vec3, level: f32) -> Option<Vec3> {
    if !from.is_finite() || !to.is_finite() || from.z <= level || to.z > level {
        return None;
    }
    let t = (level - from.z) / (to.z - from.z);
    Some(from.lerp(to, t))
}

/// How far along a normalized `direction` a ray first meets a sphere. A ray that starts inside
/// the sphere does not hit it (the caster's own body is not an obstacle).
fn sphere_hit(origin: Vec3, direction: Vec3, max: f32, centre: Vec3, radius: f32) -> Option<f32> {
    if radius <= 0.0 {
        return None;
    }
    let offset = origin - centre;
    let closest = offset.dot(direction);
    let outside = offset.length_squared() - radius * radius;
    if outside <= 0.0 {
        return None;
    }
    let discriminant = closest * closest - outside;
    if discriminant < 0.0 {
        return None;
    }
    let distance = -closest - discriminant.sqrt();
    (distance >= 0.0 && distance <= max).then_some(distance)
}

/// How far along a normalized `direction` a ray first meets an oriented box. A ray that starts
/// inside the box does not hit it (the caster's own body is not an obstacle), as for spheres.
fn box_hit(
    origin: Vec3,
    direction: Vec3,
    max: f32,
    centre: Vec3,
    half: Vec3,
    rotation: Quat,
) -> Option<f32> {
    let half = half.abs().max(Vec3::splat(1e-3));
    let inverse = rotation.inverse();
    let local_origin = inverse * (origin - centre);
    let local_direction = inverse * direction;
    if local_origin.abs().cmple(half).all() {
        return None;
    }
    let (mut near, mut far) = (0.0f32, max);
    for axis in 0..3 {
        let offset = local_origin[axis];
        let direction = local_direction[axis];
        let half = half[axis];
        if direction.abs() < 1e-6 {
            if offset < -half || offset > half {
                return None;
            }
            continue;
        }
        let (mut first, mut second) = ((-half - offset) / direction, (half - offset) / direction);
        if first > second {
            std::mem::swap(&mut first, &mut second);
        }
        near = near.max(first);
        far = far.min(second);
        if near > far {
            return None;
        }
    }
    (near >= 0.0 && near <= max).then_some(near)
}

/// The preview's refusal (`CanUse` with flags 0x442): the ray failure is `NO_TARGET`; a solved
/// point outside `skills.min_range ..= max_range` is the same range code the server uses.
/// `origin` is the slave's position, the caster position the server measures from.
pub fn range_refusal(hit: Option<Vec3>, origin: Vec3, cannon: Option<CannonSkill>) -> Option<u8> {
    let hit = hit?;
    let cannon = cannon?;
    let distance = (hit - origin).length();
    if distance < cannon.min_range {
        Some(aa_net::skill::result::TOO_CLOSE_RANGE)
    } else if cannon.max_range > 0.0 && distance > cannon.max_range {
        Some(aa_net::skill::result::TOO_FAR_RANGE)
    } else {
        None
    }
}

/// `FUN_394322f0` from the camera ray to the click's `Position`: the ray hit is cone-clamped,
/// the arc is solved from the muzzle to it and traced, and the refusal is `FUN_394281b0`'s
/// `NO_TARGET` or the range preview. `aim` is the clamped solved aim (camera aim otherwise).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimResult {
    pub camera: TurretAim,
    pub aim: TurretAim,
    pub impact: Vec3,
    pub hit: Option<Vec3>,
    pub refusal: Option<u8>,
}

/// The whole 1.2 camera aim and ballistic solve, with no ECS: `local_aim` calls this with the
/// drawn camera ray and the frame's world.
#[allow(clippy::too_many_arguments)]
pub fn solve_aim(
    root: &GlobalTransform,
    muzzle: Vec3,
    ray_origin: Vec3,
    ray_direction: Vec3,
    limits: TurretLimits,
    cannon: Option<CannonSkill>,
    world: &AimWorld,
) -> Option<AimResult> {
    let camera = limits.clamp(camera_aim(root, ray_direction)?);
    let range = cannon.map_or(MAX_AIM_RANGE, |cannon| aim_range(cannon.max_range));
    let raw = world.ray(ray_origin, ray_direction, range);
    let hit = raw.map(|hit| {
        cannon.map_or(hit, |cannon| {
            cone_clamp(
                root.translation(),
                *root.forward(),
                hit,
                cannon.target_angle,
            )
        })
    });
    let refusal = if raw.is_none() {
        Some(aa_net::skill::result::NO_TARGET)
    } else {
        range_refusal(hit, root.translation(), cannon)
    };
    let (impact, solved) = match (refusal, hit, cannon) {
        (None, Some(hit), Some(cannon)) => match ballistic_aim(root, muzzle, hit, cannon.speed) {
            Some((ballistic, aim)) => (
                trace_impact(&ballistic, hit, |from, to| world.segment(from, to)),
                Some(aim),
            ),
            None => (hit, None),
        },
        (None, Some(hit), None) => (hit, None),
        _ => (raw.unwrap_or(ray_origin), None),
    };
    Some(AimResult {
        camera,
        aim: solved.map_or(camera, |aim| limits.clamp(aim)),
        impact,
        hit,
        refusal,
    })
}

/// `FUN_394322f0`: the camera ray runs `max(300, skills.max_range)`, so the 300 m floor is kept
/// when the skill is shorter ranged (`0x3943238c`).
pub fn aim_range(max_range: f32) -> f32 {
    if max_range.is_finite() {
        max_range.max(0.0).max(MAX_AIM_RANGE)
    } else {
        MAX_AIM_RANGE
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
#[allow(clippy::type_complexity)]
pub struct TurretQueries<'w, 's> {
    views: Query<
        'w,
        's,
        (&'static Camera, &'static GlobalTransform),
        (
            With<Camera3d>,
            With<crate::ingame::camera::OrbitCamera>,
            Without<crate::hud::ModelViewCamera>,
        ),
    >,
    roots: Query<'w, 's, (Entity, &'static RemoteUnit, &'static GlobalTransform)>,
    children: Query<'w, 's, &'static Children>,
    animators: Query<'w, 's, &'static aa_anim::Animator>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    targets: Query<'w, 's, &'static mut TurretTarget>,
    guns: Query<'w, 's, &'static mut TurretGun>,
}

/// The world resources of [`local_aim`], grouped to stay inside the system parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct AimResources<'w> {
    terrain: Option<Res<'w, TerrainQuery>>,
    collision: Option<Res<'w, WorldCollision>>,
    /// `World::ocean_level` (`worlds/<name>/world.g`).
    world: Option<Res<'w, WorldInfo>>,
    player: Option<Res<'w, aa_net::state::Player>>,
}

/// The turret aim of the local gunner: every frame the game camera's screen-centre ray is
/// traced and solved ballistically (`FUN_394322f0`), clamped to the cannon's
/// `vehicle_models` limits, sent through the 200 ms change-only rule, and published as
/// [`Sight`] for the HUD. The camera half runs only for a `BallisticPos`/`ArtilleryPos` mount
/// skill (mode 8); the installed-turret keys run independently and send no [`Sight`].
#[allow(clippy::too_many_arguments)]
pub fn local_aim(
    mut commands: Commands,
    seat: Res<LocalMount>,
    units: Res<aa_net::state::Units>,
    db: Option<Res<Db>>,
    world: AimResources,
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
    // TODO(#58 child units): use `units::current_transform` instead of the drawn
    // `GlobalTransform` once the child-units branch lands, so a cannon aboard a moving ship
    // aims from its parent's transform instead of the replicated one.
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
    let pitch = actions.as_ref().map_or(0.0, |a| {
        f32::from(i8::from(a.pressed("moveforward", 0)) - i8::from(a.pressed("moveback", 0)))
    });
    let yaw = actions.as_ref().map_or(0.0, |a| {
        f32::from(i8::from(a.pressed("turnleft", 0)) - i8::from(a.pressed("turnright", 0)))
    });
    // Key aim only turns an installed turret (`FUN_39425580`, the `VehicleModel` controller
    // type 3 with `installed_turret`); it is independent of the camera-aim mode.
    let keyed = keys_apply(limits, pitch, yaw);
    // The camera aim is the native mode 8, entered by a `BallisticPos` (11) or `ArtilleryPos`
    // (15) mount skill. A drivable VehicleModel with a turret block but only cart skills (the
    // cotton wagon) is not a gunner seat: it must not aim or send `CSTurretState`.
    let camera_seat = db.has_cannon_skill(template_id);
    if !camera_seat && !keyed {
        return;
    }
    let cannon = db.cannon_skill(template_id);
    let mut solved = None;
    if camera_seat {
        // The muzzle is the slave bone `item_hand_r` (`FUN_390b8210`); a model without it falls
        // back to the unit position `FUN_390b3850` returns (S12-C1).
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
            .map_or_else(|| root.translation(), |bone| bone.translation());
        // `FUN_394281b0` / `FUN_3917dd80`: the screen-centre ray starts at the camera matrix
        // translation and runs over `max(300, max_range)`.
        let Some((camera, view)) = queries.views.iter().find(|(camera, _)| camera.is_active) else {
            return;
        };
        let Some(viewport) = camera.logical_viewport_size() else {
            return;
        };
        let Ok(ray) = camera.viewport_to_world(view, viewport * 0.5) else {
            return;
        };
        // `FUN_3917dd80` reads the camera matrix translation, not the near-plane point
        // `viewport_to_world` returns; the range is measured from here.
        let ray_origin = view.translation();
        let direction = *ray.direction;
        let mut skip = vec![slave];
        if let Some(own) = world.player.as_ref().and_then(|p| p.spawn.as_ref()) {
            skip.push(own.obj_id);
        }
        if let Some(parent) = unit.attached_to {
            skip.push(parent.obj_id);
        }
        let bodies = aim_bodies(&units, &queries.roots, db, &skip);
        let ocean = world
            .world
            .as_ref()
            .map(|info| info.0.ocean_level)
            .filter(|level| *level > 0.0);
        let aim_world = AimWorld {
            terrain: world.terrain.as_deref(),
            collision: world.collision.as_deref(),
            ocean,
            bodies: &bodies,
        };
        let Some(result) = solve_aim(
            root, start, ray_origin, direction, limits, cannon, &aim_world,
        ) else {
            return;
        };
        solved = Some(result);
    }
    // A key-only installed turret starts from its current aim; a camera seat starts from the
    // clamped camera aim.
    let initial = solved.map_or_else(
        || {
            queries
                .targets
                .get(entity)
                .ok()
                .map(|target| target.0)
                .or(unit.turret)
                .unwrap_or_default()
        },
        |solved| solved.camera,
    );
    let key = if keyed {
        let next = key_step(
            key_aim.current(slave, initial),
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
    let aim = key
        .or_else(|| solved.map(|solved| solved.aim))
        .unwrap_or(initial);
    // The turret is turned when the camera aim preview passes (`CanUse` 0x442), and always by
    // the installed-turret keys. A failed preview publishes the refusal and sends no fire.
    let refused = solved.is_some_and(|solved| solved.refusal.is_some());
    if !refused || keyed {
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
    }
    if let Some(solved) = solved {
        let cry = bevy_to_cry(solved.impact);
        commands.insert_resource(Sight {
            slave,
            aim,
            pos: Pos::new(cry.x, cry.y, cry.z),
            skill: cannon.map_or(0, |cannon| cannon.id),
            refusal: solved.refusal,
        });
    }
}

/// The drawn bodies (`FUN_3917dd80` living entities, `FUN_39429750` rigid objects) of the other
/// units: a sphere over a unit, and an oriented hull box for a ship. The manned slave, the local
/// player and the slave's parent are skipped, as the 1.2 ray ignores its own caster.
fn aim_bodies(
    units: &aa_net::state::Units,
    roots: &Query<(Entity, &RemoteUnit, &GlobalTransform)>,
    db: &Db,
    skip: &[u32],
) -> Vec<AimBody> {
    let mut bodies = Vec::new();
    for (_, remote, root) in roots.iter() {
        let Some(unit) = units.by_id.get(&remote.obj_id) else {
            continue;
        };
        if skip.contains(&unit.obj_id) {
            continue;
        }
        bodies.push(aim_body(unit, root.translation(), db));
    }
    bodies
}

/// The aim volume of one drawn unit. A ship hull uses its hull-floor XY bounds (the deck
/// footprint) and the `ship_models` mass box's vertical extent (keel to deck/rail), both in the
/// ship frame and turned by the unit's yaw. The old bounding sphere reached about ten metres past
/// the hull side, so a broadside shot's `Position` landed short of the server's small hit area
/// (S12-C8).
fn aim_body(unit: &Unit, root: Vec3, db: &Db) -> AimBody {
    let scale = unit.scale.max(0.5);
    if !(matches!(unit.kind, UnitKind::Slave { .. }) && db.is_ship(unit.model_id)) {
        return AimBody::Sphere {
            centre: root + Vec3::Y * (0.9 * scale),
            radius: 1.1 * scale,
        };
    }
    let (min, max) = db
        .ship_bounds(unit.model_id)
        .unwrap_or((Vec2::splat(-2.0), Vec2::splat(2.0)));
    // `ship_models.mass_center_z +/- mass_box_size_z / 2`, the hull's keel-to-rail range
    // relative to the hull root; a row without a box keeps a conservative 2 m below / 4 m above.
    let (low, high) = db.ship_hull_height(unit.model_id).unwrap_or((-2.0, 4.0));
    let yaw = Quat::from_rotation_z(unit.yaw());
    let middle = yaw * Vec3::new((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, 0.0);
    let centre = cry_to_bevy(
        Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z) + middle + Vec3::Z * ((low + high) * 0.5),
    );
    let half = cry_to_bevy(Vec3::new(
        (max.x - min.x) * 0.5,
        (max.y - min.y) * 0.5,
        (high - low) * 0.5,
    ))
    .abs()
    .max(Vec3::splat(0.5));
    AimBody::Box {
        centre,
        half,
        rotation: cry_rot_to_bevy(yaw),
    }
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

    /// Every kind-7/8 slave row through the same gate `track_local_mount` uses: a
    /// `models.sub_type = 'VehicleModel'` row with a non-zero turret range is a turret seat,
    /// drivable or not (the controller type 3, S12-C6); an all-zero or unreadable row is not.
    #[test]
    #[ignore = "requires AA_DB"]
    fn the_real_kind_7_8_vehicle_models_with_a_turret_block_are_seats() {
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
            let has_block = db
                .turret_limits(model)
                .is_some_and(|limits| limits.is_aimable());
            if has_block {
                aimable += 1;
                assert!(
                    seat,
                    "slave {id} {name} (kind {kind}, model {model}) is a turret seat"
                );
            } else {
                assert!(
                    !seat,
                    "slave {id} {name} (kind {kind}, model {model}) has no turret range"
                );
            }
            if velocity.is_some_and(|v| v > 0.0) {
                drivable += 1;
                assert!(
                    seat == has_block,
                    "slave {id} {name} (kind {kind}, model {model}) drives"
                );
            }
        }
        assert!(
            aimable >= 20 && drivable >= 15,
            "aimable {aimable}, drivable {drivable}"
        );
        // The galleon cannons (kind 8 and ground kind 7) aim; the trebuchet and tank are turret
        // seats too (they keep driving); an all-zero human cannon is not.
        assert!(turret_seat_model(8, 117, &db), "ship cannon");
        assert!(turret_seat_model(7, 117, &db), "ground cannon");
        assert!(turret_seat_model(7, 1535, &db), "galleon cannon model 61");
        assert!(
            turret_seat_model(7, 2, &db),
            "trebuchet (installed, velocity 2.5)"
        );
        assert!(
            turret_seat_model(7, 892, &db),
            "tank (installed, velocity 3.5)"
        );
        assert!(
            turret_seat_model(7, 1008, &db),
            "cotton cart (VehicleModel)"
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
        let slope = |x: f32, _y: f32| Some(0.5 * x);
        let impact = trace_impact(&ballistic, hit, |from, to| {
            terrain_crossing(bevy_to_cry(from), bevy_to_cry(to), slope).map(cry_to_bevy)
        });
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

    /// `FUN_394322f0` @ `0x3943238c`: `comiss 300, range; ja keep 300` makes the ray
    /// `max(300, skills.max_range)`.
    #[test]
    fn the_aim_range_is_the_skill_range_with_a_300_metre_floor() {
        assert_eq!(aim_range(100.0), 300.0);
        assert_eq!(aim_range(1000.0), 1000.0);
        assert_eq!(aim_range(0.0), 300.0);
        assert_eq!(aim_range(f32::NAN), 300.0);
    }

    /// `FUN_393594c0`: the camera hit is pulled into the horizontal cone of half-angle
    /// `target_angle / 2 - 1 deg`; a hit inside the cone is left exactly as it was, and the
    /// height is kept.
    #[test]
    fn the_camera_hit_is_pulled_into_the_skill_angle_cone() {
        let origin = Vec3::ZERO;
        let forward = Vec3::NEG_Z;
        let direction = Quat::from_rotation_y((-60f32).to_radians()) * forward;
        let original = origin + direction * 100.0 + Vec3::Y * 7.0;
        let clamped = cone_clamp(origin, forward, original, 50.0);
        assert!((clamped.y - 7.0).abs() < 1e-5, "the height is kept");
        let flat = Vec3::new(clamped.x, 0.0, clamped.z);
        assert!(
            (flat.length() - 100.0).abs() < 1e-3,
            "the horizontal distance is kept"
        );
        let angle = forward
            .dot(flat.normalize())
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        assert!((angle - 24.0).abs() < 1e-3, "clamped to {angle} deg");
        assert!(angle < 25.0);
        // The other side keeps its sign: the hit moves to the other cone edge.
        let other = origin + (Quat::from_rotation_y(60f32.to_radians()) * forward) * 100.0;
        let other = cone_clamp(origin, forward, other, 50.0);
        let side = forward
            .cross(Vec3::new(other.x, 0.0, other.z).normalize())
            .y;
        assert!(side > 0.0, "the other side stays on its side");
        assert!(
            (forward
                .dot(Vec3::new(other.x, 0.0, other.z).normalize())
                .acos()
                .to_degrees()
                - 24.0)
                .abs()
                < 1e-3
        );

        // 10 degrees off is already inside the 24 degree half-cone and is not moved.
        let inside = origin + (Quat::from_rotation_y((-10f32).to_radians()) * forward) * 30.0;
        assert_eq!(cone_clamp(origin, forward, inside, 50.0), inside);
        // 0 disables the clamp (the 1.2 early return).
        assert_eq!(cone_clamp(origin, forward, original, 0.0), original);
    }

    /// `FUN_3917dd80` / `FUN_39429750`: the sea is an obstacle for the camera ray.
    #[test]
    fn the_aim_ray_stops_at_the_ocean() {
        let world = AimWorld {
            ocean: Some(5.0),
            ..AimWorld::default()
        };
        let hit = world
            .ray(Vec3::new(0.0, 20.0, 0.0), Vec3::NEG_Y, 100.0)
            .expect("the ray reaches the sea");
        assert!((hit.y - 5.0).abs() < 1e-4, "{hit:?}");
        assert!(
            world
                .ray(Vec3::new(0.0, 20.0, 0.0), Vec3::Y, 100.0)
                .is_none(),
            "upward"
        );
    }

    /// The arc trace uses the same segment test: it stops at the sea, not at the fallback.
    #[test]
    fn the_arc_trace_stops_at_the_ocean() {
        let world = AimWorld {
            ocean: Some(5.0),
            ..AimWorld::default()
        };
        let ballistic = Ballistic {
            start: Vec3::new(0.0, 20.0, 0.0),
            velocity: Vec3::new(10.0, 0.0, 0.0),
            flight: 10.0,
            len: 100.0,
        };
        let fallback = Vec3::new(100.0, 100.0, 0.0);
        let impact = trace_impact(&ballistic, fallback, |from, to| world.segment(from, to));
        assert!((impact.y - 5.0).abs() < 1e-3, "{impact:?}");
        assert!(impact.x > 0.0 && impact.x < 100.0, "{impact:?}");
    }

    /// `FUN_394281b0` returns 0 when the screen ray hits nothing: result 9 and no solved point.
    #[test]
    fn a_screen_ray_that_hits_nothing_is_no_target() {
        let root = GlobalTransform::default();
        let cannon = CannonSkill {
            id: 17742,
            speed: 25.0,
            min_range: 10.0,
            max_range: 100.0,
            target_angle: 50.0,
        };
        let world = AimWorld::default();
        let result = solve_aim(
            &root,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 10.0, 0.0),
            Vec3::new(0.0, -1.0, -1.0),
            vm10(),
            Some(cannon),
            &world,
        )
        .expect("a horizontal camera direction");
        assert_eq!(
            result.refusal,
            Some(aa_net::skill::result::NO_TARGET),
            "FUN_394281b0 answers 0"
        );
        assert_eq!(result.impact, Vec3::new(0.0, 10.0, 0.0));
        assert!(result.hit.is_none());
    }

    /// The solve's published `Position` is the traced arc impact, not the camera hit: a body on
    /// the arc before the camera's target is the impact. (The `local_aim` system test below
    /// drives the same rule through the ECS.)
    #[test]
    fn the_solve_publishes_the_traced_impact_not_the_camera_hit() {
        let root = GlobalTransform::default();
        let muzzle = Vec3::new(0.0, 10.0, 0.0);
        let camera = Vec3::new(0.0, 100.0, 0.0);
        let target = Vec3::new(100.0, 50.0, 0.0);
        let camera_hit =
            camera + (target - camera).normalize() * ((target - camera).length() - 1.0);
        let along_arc = Vec3::new(50.0, 35.6, 0.0);
        let bodies = [
            AimBody::Sphere {
                centre: target,
                radius: 1.0,
            },
            AimBody::Sphere {
                centre: along_arc,
                radius: 2.0,
            },
        ];
        let world = AimWorld {
            bodies: &bodies,
            ..AimWorld::default()
        };
        let cannon = CannonSkill {
            id: 17742,
            speed: 50.0,
            min_range: 0.0,
            max_range: 500.0,
            target_angle: 0.0,
        };
        let result = solve_aim(
            &root,
            muzzle,
            camera,
            (target - camera).normalize(),
            vm10(),
            Some(cannon),
            &world,
        )
        .expect("a camera ray");
        assert!(
            (result.hit.expect("the camera ray hit") - camera_hit).length() < 0.2,
            "the camera ray hit the far body"
        );
        assert!(
            (result.impact - along_arc).length() <= 2.5,
            "the impact is the arc's body, not the camera hit: {:?}",
            result.impact
        );
        assert!(
            (result.impact - camera_hit).length() > 20.0,
            "a code that published the camera hit would fail here"
        );
    }

    /// The real `local_aim` system publishes the trace's body, not the camera ray's target: a
    /// camera ray to a far body and a second body on the arc before it. This drives the system
    /// with a drawn camera and a fixture database, so the `Sight` it inserts is checked, not the
    /// solve helper alone.
    #[test]
    fn local_aim_system_publishes_the_traced_impact() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "aa-cannon-aim-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        {
            let conn = rusqlite::Connection::open(&path).expect("fixture database");
            conn.execute_batch(
                "CREATE TABLE models(id INTEGER PRIMARY KEY, sub_type TEXT, sub_id INTEGER);
                 CREATE TABLE vehicle_models(id INTEGER PRIMARY KEY,
                     turret_pitch_angle_min REAL, turret_pitch_angle_max REAL,
                     turret_yaw_angle_min REAL, turret_yaw_angle_max REAL,
                     turret_yaw_angvel REAL, turret_pitch_angvel REAL,
                     velocity REAL, installed_turret TEXT);
                 CREATE TABLE slave_mount_skills(id INTEGER PRIMARY KEY, slave_id INTEGER, mount_skill_id INTEGER);
                 CREATE TABLE mount_skills(id INTEGER PRIMARY KEY, skill_id INTEGER);
                 CREATE TABLE skills(id INTEGER PRIMARY KEY, effect_speed REAL, min_range REAL,
                     max_range REAL, target_angle REAL, target_type_id INTEGER);
                 INSERT INTO models VALUES (117, 'VehicleModel', 10);
                 INSERT INTO vehicle_models VALUES (10, -20, 35, -22, 22, 35, 20, 0, 'f');
                 INSERT INTO slave_mount_skills VALUES (72, 77, 116);
                 INSERT INTO mount_skills VALUES (116, 17742);
                 INSERT INTO skills VALUES (17742, 25, 10, 100, 50, 11);",
            )
            .expect("fixture rows");
        }
        let db = Db::open_path(path.to_str().expect("utf-8 temp path")).expect("cannon fixture db");

        let gunner = crate::hud::fixture_spawn().unit;
        let mut cannon = gunner.clone();
        cannon.obj_id = 77;
        cannon.kind = UnitKind::Slave {
            id: 7,
            tl_id: 1,
            template_id: 77,
            summoner_id: 0,
        };
        cannon.model_id = 117;
        cannon.pos = Pos::new(0.0, 0.0, 0.0);
        cannon.scale = 1.0;
        let root = Transform::from_xyz(0.0, 0.0, 0.0);

        let mut far = gunner.clone();
        far.obj_id = 1;
        far.kind = UnitKind::Npc {
            template_id: 1,
            owner_id: 0,
        };
        far.pos = Pos::new(0.0, 30.0, 30.0);
        let far_transform = Transform::from_xyz(0.0, 30.0, -30.0);

        let mut arc_body = gunner;
        arc_body.obj_id = 2;
        arc_body.kind = UnitKind::Npc {
            template_id: 1,
            owner_id: 0,
        };
        arc_body.pos = Pos::new(0.0, 21.5, 19.1);
        arc_body.scale = 1.82;
        let arc_transform = Transform::from_xyz(0.0, 17.46, -21.5);

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<aa_net::state::Units>()
            .init_resource::<LocalMount>()
            .init_resource::<TurretSend>()
            .init_resource::<KeyAim>()
            .insert_resource(db)
            .add_message::<NetCommand>()
            .add_systems(Update, local_aim);
        {
            let mut units = app.world_mut().resource_mut::<aa_net::state::Units>();
            units.by_id.insert(77, cannon.clone());
            units.by_id.insert(1, far.clone());
            units.by_id.insert(2, arc_body.clone());
        }
        let camera_pose = Transform::from_translation(Vec3::new(0.0, 50.0, 20.0))
            .looking_at(Vec3::new(0.0, 30.9, -30.0), Vec3::Y);
        let mut camera = Camera {
            viewport: Some(bevy::camera::Viewport {
                physical_position: UVec2::ZERO,
                physical_size: UVec2::new(1280, 720),
                depth: 0.0..1.0,
            }),
            ..Default::default()
        };
        // The near-plane matrix only shapes the direction through the viewport centre; the ray
        // origin is the camera translation (`local_aim`).
        camera.computed.clip_from_view = Mat4::IDENTITY;
        camera.computed.target_info = Some(bevy::camera::RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            scale_factor: 1.0,
        });
        app.world_mut().spawn((
            Camera3d::default(),
            crate::ingame::camera::OrbitCamera::default(),
            camera,
            camera_pose,
            GlobalTransform::from(camera_pose),
        ));
        app.world_mut().spawn((
            RemoteUnit::announced(&cannon),
            root,
            GlobalTransform::from(root),
        ));
        app.world_mut().spawn((
            RemoteUnit::announced(&far),
            far_transform,
            GlobalTransform::from(far_transform),
        ));
        app.world_mut().spawn((
            RemoteUnit::announced(&arc_body),
            arc_transform,
            GlobalTransform::from(arc_transform),
        ));
        app.world_mut()
            .insert_resource(LocalMount(Some(crate::ingame::mount::LocalSeat {
                mount: 77,
                point: 1,
                drives: true,
                cart: false,
                ship: false,
                turret: true,
                speed: Default::default(),
                lift: 0.0,
            })));
        app.update();

        let sight = app.world().resource::<Sight>();
        let impact = cry_to_bevy(Vec3::new(sight.pos.x, sight.pos.y, sight.pos.z));
        let arc = arc_transform.translation + Vec3::Y * (0.9 * 1.82);
        let far = far_transform.translation + Vec3::Y * 0.9;
        assert!(
            impact.distance(arc) <= 2.5,
            "the published point is the traced body: {impact:?} vs {arc:?}"
        );
        assert!(
            impact.distance(far) > 10.0,
            "a code that published the camera hit would fail here: {impact:?} vs {far:?}"
        );
        assert_eq!(sight.refusal, None, "the preview passed");
        drop(app);
        let _ = std::fs::remove_file(path);
    }

    /// A temporary database with one kind-7/8 slave row and its point-1 mount skill. `target`
    /// is the `skills.target_type_id` (11 `BallisticPos`, 15 `ArtilleryPos`, 0 a cart skill).
    struct FixtureDb {
        path: std::path::PathBuf,
        db: Db,
    }

    impl FixtureDb {
        fn new(template: u32, model: u32, target: u32, velocity: f32, installed: bool) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "aa-cannon-seat-{}-{}.sqlite3",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_file(&path);
            {
                let conn = rusqlite::Connection::open(&path).expect("fixture database");
                conn.execute_batch(&format!(
                    "CREATE TABLE models(id INTEGER PRIMARY KEY, sub_type TEXT, sub_id INTEGER);
                     CREATE TABLE vehicle_models(id INTEGER PRIMARY KEY,
                         turret_pitch_angle_min REAL, turret_pitch_angle_max REAL,
                         turret_yaw_angle_min REAL, turret_yaw_angle_max REAL,
                         turret_yaw_angvel REAL, turret_pitch_angvel REAL,
                         velocity REAL, installed_turret TEXT);
                     CREATE TABLE slave_mount_skills(id INTEGER PRIMARY KEY, slave_id INTEGER, mount_skill_id INTEGER);
                     CREATE TABLE mount_skills(id INTEGER PRIMARY KEY, skill_id INTEGER);
                     CREATE TABLE skills(id INTEGER PRIMARY KEY, effect_speed REAL, min_range REAL,
                         max_range REAL, target_angle REAL, target_type_id INTEGER);
                     INSERT INTO models VALUES ({model}, 'VehicleModel', 10);
                     INSERT INTO vehicle_models VALUES (10, -20, 35, -22, 22, 35, 20, {velocity}, '{installed}');
                     INSERT INTO slave_mount_skills VALUES (72, {template}, 116);
                     INSERT INTO mount_skills VALUES (116, 17742);
                     INSERT INTO skills VALUES (17742, 25, 10, 100, 50, {target});"
                ))
                .expect("fixture rows");
            }
            let db =
                Db::open_path(path.to_str().expect("utf-8 temp path")).expect("fixture database");
            Self { path, db }
        }
    }

    impl Drop for FixtureDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// The `local_aim` app of a manned slave and a far unit the camera ray can hit.
    fn aim_app(db: &Db, cannon: &Unit, orbit: bool) -> (App, Entity) {
        let gunner = crate::hud::fixture_spawn().unit;
        let mut far = gunner;
        far.obj_id = 1;
        far.kind = UnitKind::Npc {
            template_id: 1,
            owner_id: 0,
        };
        far.pos = Pos::new(0.0, 30.0, 30.0);
        let far_transform = Transform::from_xyz(0.0, 30.0, -30.0);
        let root = Transform::from_translation(cry_to_bevy(Vec3::new(
            cannon.pos.x,
            cannon.pos.y,
            cannon.pos.z,
        )));
        let camera_pose = Transform::from_translation(Vec3::new(0.0, 50.0, 20.0))
            .looking_at(Vec3::new(0.0, 30.9, -30.0), Vec3::Y);
        let mut camera = Camera {
            viewport: Some(bevy::camera::Viewport {
                physical_position: UVec2::ZERO,
                physical_size: UVec2::new(1280, 720),
                depth: 0.0..1.0,
            }),
            ..Default::default()
        };
        camera.computed.clip_from_view = Mat4::IDENTITY;
        camera.computed.target_info = Some(bevy::camera::RenderTargetInfo {
            physical_size: UVec2::new(1280, 720),
            scale_factor: 1.0,
        });
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(250),
            ))
            .init_resource::<aa_net::state::Units>()
            .init_resource::<LocalMount>()
            .init_resource::<TurretSend>()
            .init_resource::<KeyAim>()
            .init_resource::<crate::settings::Actions>()
            .insert_resource(db.clone())
            .add_message::<NetCommand>()
            .add_systems(Update, local_aim);
        {
            let mut units = app.world_mut().resource_mut::<aa_net::state::Units>();
            units.by_id.insert(cannon.obj_id, cannon.clone());
            units.by_id.insert(far.obj_id, far.clone());
        }
        let mut camera_commands = app.world_mut().spawn((
            Camera3d::default(),
            camera,
            camera_pose,
            GlobalTransform::from(camera_pose),
        ));
        if orbit {
            camera_commands.insert(crate::ingame::camera::OrbitCamera::default());
        }
        let camera_entity = camera_commands.id();
        app.world_mut().spawn((
            RemoteUnit::announced(cannon),
            root,
            GlobalTransform::from(root),
        ));
        app.world_mut().spawn((
            RemoteUnit::announced(&far),
            far_transform,
            GlobalTransform::from(far_transform),
        ));
        app.world_mut()
            .insert_resource(LocalMount(Some(crate::ingame::mount::LocalSeat {
                mount: cannon.obj_id,
                point: 1,
                drives: true,
                cart: false,
                ship: false,
                turret: true,
                speed: Default::default(),
                lift: 0.0,
            })));
        (app, camera_entity)
    }

    fn turret_sends(app: &mut App) -> Vec<(f32, f32)> {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| match command {
                aa_net::Command::TurretState { pitch, yaw, .. } => Some((pitch, yaw)),
                _ => None,
            })
            .collect()
    }

    fn cannon_unit(obj_id: u32, template: u32, model: u32) -> Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = obj_id;
        unit.kind = UnitKind::Slave {
            id: 7,
            tl_id: 1,
            template_id: template,
            summoner_id: 0,
        };
        unit.model_id = model;
        unit.pos = Pos::new(0.0, 0.0, 0.0);
        unit.scale = 1.0;
        unit
    }

    /// A hull box built from the deck footprint and the keel-to-rail range: a broadside ray at
    /// deck height hits the side, a downward ray hits the deck, and the arc trace stops at the
    /// same side. The old hull sphere stopped about ten metres short of the side and fifteen
    /// metres above the deck.
    #[test]
    fn a_ship_hull_aims_as_an_oriented_box() {
        use crate::ingame::ship::floor::ShipFloor;

        const MODEL: u32 = 900;
        let quad = [
            [
                Vec3::new(-4.0, -10.0, 2.0),
                Vec3::new(4.0, -10.0, 2.0),
                Vec3::new(4.0, 10.0, 2.0),
            ],
            [
                Vec3::new(-4.0, -10.0, 2.0),
                Vec3::new(4.0, 10.0, 2.0),
                Vec3::new(-4.0, 10.0, 2.0),
            ],
        ];
        let Some(db) = Db::ship_floor_fixture(MODEL, std::sync::Arc::new(ShipFloor::new(quad)))
        else {
            return;
        };
        db.seed_ship_hull_height(MODEL, -3.0, 2.0);
        let ship = cannon_unit(5, 15, MODEL);
        let body = aim_body(&ship, Vec3::ZERO, &db);
        let world = AimWorld {
            bodies: std::slice::from_ref(&body),
            ..AimWorld::default()
        };

        // A broadside ray at deck height hits the hull side (x = 4) within half a metre.
        let hit = world
            .ray(Vec3::new(20.0, 1.5, 0.0), Vec3::NEG_X, 40.0)
            .expect("the hull side");
        assert!(
            (hit.x - 4.0).abs() < 0.5,
            "the hull side is at x 4, got {hit:?}"
        );
        assert!(
            hit.x < 10.0,
            "a hull sphere would stop ten metres out: {hit:?}"
        );

        // A downward ray hits the deck (y = 2), not the air above it.
        let hit = world
            .ray(Vec3::new(0.0, 20.0, 0.0), Vec3::NEG_Y, 40.0)
            .expect("the deck");
        assert!((hit.y - 2.0).abs() < 0.5, "the deck is at y 2, got {hit:?}");
        assert!(
            hit.y < 5.0,
            "a hull sphere would stop above the mast: {hit:?}"
        );

        // The arc trace uses the same segment test and stops at the hull side.
        let ballistic = Ballistic {
            start: Vec3::new(20.0, 0.0, 0.0),
            velocity: Vec3::new(-20.0, 4.0, 0.0),
            flight: 1.0,
            len: 21.0,
        };
        let impact = trace_impact(&ballistic, Vec3::new(-100.0, 0.0, 0.0), |from, to| {
            world.segment(from, to)
        });
        assert!(
            (impact.x - 4.0).abs() < 0.5,
            "the trace stops at the hull side, got {impact:?}"
        );

        // The box is turned by the unit yaw: a quarter turn puts the 8 m width across Bevy z,
        // so a ray down +z hits the side at z 4, not at the unrotated 10 m length.
        let mut turned = ship;
        turned.rotation[2] = std::f32::consts::FRAC_PI_2;
        let body = aim_body(&turned, Vec3::ZERO, &db);
        let world = AimWorld {
            bodies: std::slice::from_ref(&body),
            ..AimWorld::default()
        };
        let hit = world
            .ray(Vec3::new(0.0, 1.5, 20.0), Vec3::NEG_Z, 40.0)
            .expect("the turned hull side");
        assert!(
            (hit.z - 4.0).abs() < 0.5,
            "a quarter turn keeps the hull side at 4, got {hit:?}"
        );
    }

    /// The cotton wagon (a drivable `VehicleModel` with a turret block but only Honk horn) is
    /// not a native mode-8 gunner seat: no `Sight`, no `CSTurretState`. Without the cannon-skill
    /// gate the camera ray hits the far body and both appear.
    #[test]
    fn a_drivable_cart_does_not_aim_or_send_turret_state() {
        let fixture = FixtureDb::new(73, 1008, 0, 4.0, false);
        let wagon = cannon_unit(73, 73, 1008);
        let (mut app, _) = aim_app(&fixture.db, &wagon, true);
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_none(),
            "a cart with only a Honk horn skill is not a gunner seat"
        );
        assert!(
            turret_sends(&mut app).is_empty(),
            "the cart driver sends no CSTurretState"
        );
    }

    /// `ArtilleryPos` (15, a tank's skill) is the other mode-8 entry: it runs the camera aim
    /// and sends turret state even though `Db::cannon_skill` only names the BallisticPos shot.
    #[test]
    fn an_artillery_pos_skill_runs_the_camera_aim() {
        let fixture = FixtureDb::new(53, 892, 15, 3.5, true);
        let tank = cannon_unit(53, 53, 892);
        let (mut app, _) = aim_app(&fixture.db, &tank, true);
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_some(),
            "an ArtilleryPos mount skill enters the camera aim"
        );
        assert!(
            !turret_sends(&mut app).is_empty(),
            "the camera aim sends CSTurretState"
        );
    }

    /// A prewarm/cinema camera is not the aim camera: only the game's `OrbitCamera` is.
    #[test]
    fn only_the_orbiting_game_camera_aims_the_cannon() {
        let fixture = FixtureDb::new(77, 117, 11, 0.0, false);
        let cannon = cannon_unit(77, 77, 117);
        let (mut app, camera) = aim_app(&fixture.db, &cannon, false);
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_none(),
            "a prewarm or cinema camera must not be picked"
        );
        app.world_mut()
            .entity_mut(camera)
            .insert(crate::ingame::camera::OrbitCamera::default());
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_some(),
            "the game camera aims the cannon"
        );
    }

    /// Key aim stays the `VehicleModel`/`installed_turret` path: a model without a cannon skill
    /// still turns and sends `CSTurretState`, but publishes no camera `Sight`.
    #[test]
    fn an_installed_turret_without_a_cannon_skill_still_key_aims() {
        let fixture = FixtureDb::new(2, 2, 0, 2.5, true);
        let turret = cannon_unit(2, 2, 2);
        let (mut app, _) = aim_app(&fixture.db, &turret, true);
        app.world_mut()
            .resource_mut::<crate::settings::Actions>()
            .hold_for_test("moveforward");
        // The first update has a zero delta in Bevy; the second steps at the model rate.
        app.update();
        app.update();
        assert!(
            app.world().get_resource::<Sight>().is_none(),
            "no BallisticPos/ArtilleryPos skill, no camera aim"
        );
        let sends = turret_sends(&mut app);
        let send = sends.last().unwrap_or_else(|| panic!("{sends:?}"));
        assert!(
            send.0 > 0.0,
            "the key stepped the pitch at the model rate: {sends:?}"
        );
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

    /// The native mode-8 stand-in: a `BallisticPos` (11) or `ArtilleryPos` (15) mount skill
    /// makes a slave a camera-aim gunner seat. The cotton wagon 73 only has Honk horn (type 0);
    /// the trebuchet 2 and the tower cannon 100 have no gunner skill (their key aim stays).
    #[test]
    #[ignore = "requires AA_DB"]
    fn the_real_cannon_skill_gate_excludes_carts_and_plain_vehicles() {
        let Some(db) = Db::open() else { return };
        assert!(db.has_cannon_skill(10), "ship cannon 10 is BallisticPos");
        assert!(db.has_cannon_skill(77), "cannon 77 is BallisticPos");
        assert!(db.has_cannon_skill(53), "tank 53 is ArtilleryPos");
        assert!(!db.has_cannon_skill(73), "the cotton wagon only honks");
        assert!(!db.has_cannon_skill(2), "the trebuchet is not a shot slot");
        assert!(
            !db.has_cannon_skill(100),
            "the tower deployment is not a shot"
        );
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
        assert!((sample.min_range - 10.0).abs() < 1e-4);
        assert!((sample.max_range - 100.0).abs() < 1e-4);
        assert!((sample.target_angle - 50.0).abs() < 1e-4);
        let galleon = db.cannon_skill(10).expect("slave 10 fires 10473");
        assert_eq!(galleon.id, 10473);
        assert!((galleon.speed - 35.0).abs() < 1e-4);
        assert!((galleon.min_range - 4.0).abs() < 1e-4);
        assert!((galleon.max_range - 120.0).abs() < 1e-4);
        assert!((galleon.target_angle - 60.0).abs() < 1e-4);
        let mole = db
            .cannon_skill(70)
            .expect("slave 70 fires 15748 without a point-1 rider");
        assert_eq!(mole.id, 15748);
        assert!((mole.max_range - 22.0).abs() < 1e-4);
        assert!((mole.target_angle - 60.0).abs() < 1e-4);
        // The siege thrower (slave 1) has three ammo throws; the first gunner row wins, and a
        // caller that knows the slot can ask for it. All three share the ballistic data here.
        let first = db.cannon_skill(1).expect("slave 1 throws");
        assert_eq!((first.id, first.target_angle), (15515, 45.0));
        let water = db.cannon_skill_for(1, Some(63)).expect("mount skill 63");
        assert_eq!(water.id, 15580);
        assert!((water.speed - first.speed).abs() < 1e-4);
        assert!((water.max_range - first.max_range).abs() < 1e-4);
        assert!(
            db.cannon_skill(100).is_none(),
            "the tower deployment 20160 is target type 0, not a shot"
        );
    }
}
