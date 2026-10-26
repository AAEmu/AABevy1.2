//! Swimming of the local player: when it starts and stops, how the body moves in the water, the swim clips and the
//! actor parameters of the database that decide them.
//!
//! **Where the numbers come from.**
//!
//! * `actor_models` (the game database): `height` (the physics capsule, 1.5 m for every playable race), `swim_height` (0.2 m,
//!   how far the top of the capsule sticks out of the water while it swims), the walk / backward / strafe multipliers, the jump
//!   height (1.3 m) and gravity (25 m/s^2).
//! * `x2game.dll` registers these cvars (defaults read from the registration code): `swim_up_speed_mul` 0.7,
//!   `swim_down_speed_mul` 0.5, `swim_back_speed_mul` 0.8, `swim_buoy_speed` 2.0 (the speed a diver floats up at),
//!   `swim_jump_permission_range` 0.2 and `swim_jump_end_depth` 0.2 (m), `swim_jump_speed` 12.
//! * `x2game.dll` (and `crygame.dll`) register `cl_shallowWaterDepthLo` 0.3, `cl_shallowWaterDepthHi` 1.0 and `cl_shallowWaterSpeedMulPlayer`
//!   0.6: wading slows a player down from 0.3 m of water (none) to 1 m (to 0.6 times the speed).
//! * The swim locomotion groups (`lmg_files/fist_mo_swim_run.lmg`) are authored for 3.0 m/s forward and to the side and
//!   2.4 m/s (0.8 times) backwards, which the client's `swim_back_speed_mul` agrees with.
//!
//! **The swim height.** The swim clips are authored with the body about 1.3 m above the entity's origin (`fist_mo_swim_f`: pelvis
//! at 1.34 m, the body lying horizontally; `fist_ba_idle_swim`: upright, head at 1.83 m), so a swimmer's origin hangs about 1.3 m
//! under the surface: `height - swim_height`, the capsule (1.5 m) sticking out of the water by `swim_height` (0.2 m). That is
//! also the depth at which a walking player starts to swim here ([`ActorParams::swim_depth`]); the x2game actor update
//! (`CActor::UpdateSwimStats`) keeps a `relativeWaterLevel` of the origin plus that height against the surface, which is the
//! same quantity, but the stance switch itself was not traced in the binary: the numbers to change when a capture of the real
//! client says otherwise are [`ActorParams::swim_depth`] and [`LEAVE_MARGIN`].

use std::sync::Arc;

use aa_anim::pose::{BoundClip, Rig};
use aa_anim::{Animator, Cal, ClipLibrary};
use aa_world::WaterKind;
use bevy::prelude::*;

use super::anim::{Locomotion, bind_clip, direction_index};
use super::combat::{Acting, Dead};
use super::player::{LocalPlayer, Mover};

/// Forward and sideways swim speed (m/s); the clips are made for it.
pub const BASE_SPEED: f32 = 3.0;
/// `swim_back_speed_mul`: swimming backwards (and diagonally backwards).
pub const BACK_MUL: f32 = 0.8;
/// `swim_up_speed_mul`, `swim_down_speed_mul`: diving and surfacing with Space / X.
pub const UP_MUL: f32 = 0.7;
pub const DOWN_MUL: f32 = 0.5;
/// `swim_buoy_speed`: the speed (m/s) a diver floats back to the surface at.
pub const BUOY_SPEED: f32 = 2.0;
/// A swimmer walks again when the water over the ground is this much shallower than it takes to start swimming (m), so that
/// the shore line does not flicker (`swim_jump_end_depth`).
pub const LEAVE_MARGIN: f32 = 0.2;
/// `swim_jump_permission_range`: a jump from within this distance of the surface leaves the water (m).
pub const JUMP_RANGE: f32 = 0.2;
/// The body is "deep" (the `_deep` clips) when its origin is this far under the swimming height (m).
pub const DEEP: f32 = 0.5;
/// `cl_shallowWaterDepthLo` / `Hi` / `SpeedMulPlayer`.
pub const WADE_LO: f32 = 0.3;
pub const WADE_HI: f32 = 1.0;
pub const WADE_MUL: f32 = 0.6;
/// How hard the water stops a body that fell in (m/s^2), and gravity on a body that sinks in water as a share of gravity.
pub const WATER_DRAG: f32 = 35.0;
pub const WATER_GRAVITY: f32 = 0.3;

/// What the database says about the player's actor model (`actor_models`).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ActorParams {
    /// Height of the physics capsule (m).
    pub height: f32,
    /// How far the top of the capsule sticks out of the water while swimming (m).
    pub swim_height: f32,
    /// Speed multipliers of walking (`game_walk_multiplier`), backward running and walking, and strafing.
    pub walk: f32,
    pub backward: f32,
    pub walk_backward: f32,
    pub strafe: f32,
    pub walk_strafe: f32,
    /// Jump height (m) and gravity (m/s^2).
    pub jump_height: f32,
    pub gravity: f32,
}

impl Default for ActorParams {
    /// The Nuian male's row.
    fn default() -> Self {
        ActorParams { height: 1.5, swim_height: 0.2, walk: 0.45, backward: 0.65, walk_backward: 0.2925, strafe: 1.0, walk_strafe: 0.45, jump_height: 1.3, gravity: 25.0 }
    }
}

impl ActorParams {
    /// Water over the ground at which a walking player starts to swim (m): the origin of a swimmer hangs this far under the
    /// surface (see the module documentation).
    pub fn swim_depth(&self) -> f32 {
        self.height - self.swim_height
    }

    /// Vertical speed of a jump: `v = sqrt(2 g h)`.
    pub fn jump_speed(&self) -> f32 {
        (2.0 * self.gravity * self.jump_height).sqrt()
    }

    /// The row of the actor model of `models.id` `model_id`; `None` when the database or the row is missing.
    pub fn load(model_id: u32) -> Option<ActorParams> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI).ok()?;
        let d = ActorParams::default();
        conn.query_row(
            "SELECT a.height, a.swim_height, a.game_walk_multiplier, a.game_backward_multiplier, a.game_walk_backward_multiplier, a.game_strafe_multiplier, \
             a.game_walk_strafe_multiplier, a.game_jump_height, a.physics_living_gravity \
             FROM models m JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel' WHERE m.id = ?1",
            [model_id],
            |r| {
                let f = |i: usize, default: f32| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.map_or(default, |v| v as f32)) };
                Ok(ActorParams {
                    height: f(0, d.height)?,
                    swim_height: f(1, d.swim_height)?,
                    walk: f(2, d.walk)?,
                    backward: f(3, d.backward)?,
                    walk_backward: f(4, d.walk_backward)?,
                    strafe: f(5, d.strafe)?,
                    walk_strafe: f(6, d.walk_strafe)?,
                    jump_height: f(7, d.jump_height)?,
                    gravity: f(8, d.gravity)?,
                })
            },
        )
        .ok()
    }
}

/// A player in the water: the surface and what it does there. `Mover::swim` is `Some` while it swims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swim {
    pub surface: f32,
    pub kind: WaterKind,
    /// The pressed direction relative to the facing (x right, y forward, unit length or zero) and the ground speed (m/s).
    pub local_dir: Vec2,
    pub speed: f32,
    /// -1 diving, 1 surfacing, 0 not pressing.
    pub vertical: f32,
    /// The body is diving (see [`DEEP`]).
    pub deep: bool,
}

/// Water at the player: the depth over the ground it would stand on and the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wet {
    pub surface: f32,
    pub kind: WaterKind,
    /// Surface minus ground (m), never negative.
    pub depth: f32,
}

/// Slowdown of wading (`cl_shallowWaterSpeedMulPlayer`): 1 up to 0.3 m of water, 0.6 from 1 m, linear between.
pub fn wading_factor(depth: f32) -> f32 {
    let t = ((depth - WADE_LO) / (WADE_HI - WADE_LO)).clamp(0.0, 1.0);
    1.0 + (WADE_MUL - 1.0) * t
}

/// Does a player start swimming? `feet` is its height, `wet` the water there, `swimming` what it does now and `enter` the depth
/// that makes a walking player swim ([`ActorParams::swim_depth`]).
pub fn should_swim(swimming: bool, feet: f32, wet: Option<Wet>, enter: f32) -> bool {
    let Some(w) = wet else { return false };
    if swimming {
        // it keeps swimming until the ground comes up to LEAVE_MARGIN above the starting depth; a body thrown out of the
        // water leaves it
        w.depth >= enter - LEAVE_MARGIN && feet <= w.surface + 0.6
    } else {
        // the feet have to be in the water (a jump off a bridge enters it at the surface)
        w.depth >= enter && feet <= w.surface
    }
}

/// Speed (m/s) in the local frame (x right, y forward) of a swimmer pressing `dir`.
pub fn local_velocity(dir: Vec2) -> Vec2 {
    let d = dir.normalize_or_zero();
    let speed = if d.y < -0.1 { BASE_SPEED * BACK_MUL } else { BASE_SPEED };
    d * speed
}

/// The vertical speed (m/s) a swimmer wants: `vertical` is -1 (X), 0 or 1 (Space); `z` its height, `rest` the height the body
/// floats at (surface minus swim height).
pub fn wanted_vz(vertical: f32, z: f32, rest: f32) -> f32 {
    if vertical < 0.0 {
        -BASE_SPEED * DOWN_MUL
    } else if vertical > 0.0 && z < rest - 0.05 {
        BASE_SPEED * UP_MUL
    } else {
        // buoyancy: float to the resting height, slowing down near it
        ((rest - z) * 4.0).clamp(-BUOY_SPEED, BUOY_SPEED)
    }
}

/// Moves `vz` towards `wanted` by at most `accel * dt`.
pub fn approach(vz: f32, wanted: f32, accel: f32, dt: f32) -> f32 {
    vz + (wanted - vz).clamp(-accel * dt, accel * dt)
}

// ------------------------------------------------------------------------------------------------------------ clips

/// Direction suffixes of the 8-way clips, the order of [`direction_index`].
const DIRS: [&str; 8] = ["f", "fl", "fr", "l", "r", "b", "bl", "br"];

/// The swim clips of the player's model (`fist_ba_idle_swim`, `fist_mo_swim_<dir>` and `_deep`, `fist_mo_swim_up` / `down`).
#[derive(Component, Clone)]
pub struct SwimClips {
    idle: BoundClip,
    swim: [Option<BoundClip>; 8],
    deep: [Option<BoundClip>; 8],
    up: Option<BoundClip>,
    down: Option<BoundClip>,
    /// Ground speed (m/s) the forward clips cover per second of playback.
    swim_speed: f32,
    deep_speed: f32,
}

impl SwimClips {
    /// Binds the swim clips of the model whose skeleton and `.cal` are given; `None` when it has no swim idle.
    pub fn load(skeleton: &str, cal: &str, clips: &ClipLibrary) -> Option<SwimClips> {
        let rig = aa_anim::pose::shared_rig(skeleton).ok()?;
        let cal = Cal::shared(cal).ok()?;
        let bind = |name: &str| bind_clip(&rig, &cal, clips, name);
        let idle = bind("fist_ba_idle_swim")?;
        let dirs = |suffix: &str| -> [Option<BoundClip>; 8] { std::array::from_fn(|i| bind(&format!("fist_mo_swim_{}{suffix}", DIRS[i]))) };
        let (swim, deep) = (dirs(""), dirs("_deep"));
        let speed = |c: &[Option<BoundClip>; 8]| c[0].as_ref().map_or(0.0, |c| ground_speed(&rig, c));
        let (swim_speed, deep_speed) = (speed(&swim), speed(&deep));
        debug!("swim clips: forward {swim_speed:.2} m/s, deep {deep_speed:.2} m/s");
        Some(SwimClips { idle, swim, deep, up: bind("fist_mo_swim_up"), down: bind("fist_mo_swim_down"), swim_speed, deep_speed })
    }

    /// The clip for a swimmer and the ground speed it covers per second of playback (0: no speed matching).
    fn pick(&self, swim: &Swim) -> (&BoundClip, f32) {
        let moving = swim.speed > 0.2 && swim.local_dir != Vec2::ZERO;
        if moving {
            let i = direction_index(swim.local_dir);
            let (set, speed) = if swim.deep { (&self.deep, self.deep_speed) } else { (&self.swim, self.swim_speed) };
            if let Some(c) = set[i].as_ref().or(set[0].as_ref()).or(self.swim[0].as_ref()) {
                return (c, speed);
            }
        } else if swim.vertical > 0.0 && let Some(c) = &self.up {
            return (c, 0.0);
        } else if swim.vertical < 0.0 && let Some(c) = &self.down {
            return (c, 0.0);
        }
        (&self.idle, 0.0)
    }
}

/// Metres per second the root bone travels in a forward clip.
fn ground_speed(rig: &Rig, clip: &BoundClip) -> f32 {
    let root = rig.skeleton.bones.get(rig.root).map(|b| b.controller_id);
    let Some(track) = root.and_then(|c| clip.clip.track(c)) else { return 0.0 };
    let (Some(a), Some(b)) = (track.sample_pos(0.0), track.sample_pos(clip.clip.duration)) else { return 0.0 };
    let travelled = Vec2::new(b.x - a.x, b.y - a.y).length() * clip.options.position_scale;
    if clip.clip.duration > 0.0 { travelled / clip.clip.duration } else { 0.0 }
}

/// Plays the swim clips on the local player while it swims, after `anim::drive_locomotion` (which sees a standing player: the
/// player's `Motion` is empty in the water) and hands the animator back when it stops (`Locomotion::invalidate`).
pub fn drive_swim(
    mut player: Query<(&Mover, &mut Animator, &mut Locomotion, &SwimClips, Option<&Acting>, Has<Dead>), With<LocalPlayer>>,
    mut was_swimming: Local<bool>,
) {
    let Ok((mover, mut animator, mut loco, clips, acting, dead)) = player.single_mut() else { return };
    if !animator.starts_enabled() { return; }
    let Some(swim) = mover.swim else {
        if *was_swimming {
            *was_swimming = false;
            loco.invalidate();
        }
        return;
    };
    *was_swimming = true;
    // an action on the whole body (a cast, a swing) owns the clip
    if dead || acting.is_some_and(|a| !a.layered) {
        return;
    }
    let (clip, clip_speed) = clips.pick(&swim);
    let playing = animator.current.as_ref().is_some_and(|p| Arc::ptr_eq(&p.clip.clip, &clip.clip));
    if !playing {
        animator.play(clip.clone(), true, 0.3);
        animator.random_idle = None;
    }
    animator.speed = if clip_speed > 0.1 { (swim.speed / clip_speed).clamp(0.4, 2.0) } else { 1.0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wet(depth: f32) -> Option<Wet> {
        Some(Wet { surface: 100.0, kind: WaterKind::Lake, depth })
    }

    #[test]
    fn wading_slows_down_between_30_cm_and_a_metre() {
        assert_eq!(wading_factor(0.0), 1.0);
        assert_eq!(wading_factor(0.3), 1.0);
        assert!((wading_factor(0.65) - 0.8).abs() < 1e-5);
        assert!((wading_factor(1.0) - 0.6).abs() < 1e-5);
        assert!((wading_factor(5.0) - 0.6).abs() < 1e-5);
    }

    #[test]
    fn swimming_starts_deep_and_stops_shallow() {
        let enter = ActorParams::default().swim_depth();
        assert!((enter - 1.3).abs() < 1e-6);
        // standing in 1.2 m of water walks, 1.4 m swims; once swimming it stays until the water is shallower than 1.1 m
        assert!(!should_swim(false, 98.8, wet(1.2), enter));
        assert!(should_swim(false, 98.6, wet(1.4), enter));
        assert!(should_swim(true, 98.7, wet(1.2), enter));
        assert!(!should_swim(true, 98.9, wet(1.0), enter));
        // no water, no swimming; a jump over the water does not swim until the feet are in it
        assert!(!should_swim(true, 99.0, None, enter));
        assert!(!should_swim(false, 104.0, wet(6.0), enter));
        assert!(should_swim(false, 99.9, wet(6.0), enter));
        // thrown high out of the water
        assert!(!should_swim(true, 101.0, wet(6.0), enter));
    }

    #[test]
    fn swim_speed_depends_on_the_direction() {
        assert_eq!(local_velocity(Vec2::ZERO), Vec2::ZERO);
        assert!((local_velocity(Vec2::Y).length() - 3.0).abs() < 1e-5);
        assert!((local_velocity(Vec2::X).length() - 3.0).abs() < 1e-5);
        assert!((local_velocity(-Vec2::Y).length() - 2.4).abs() < 1e-5);
        assert!((local_velocity(Vec2::new(1.0, -1.0)).length() - 2.4).abs() < 1e-5);
    }

    #[test]
    fn a_swimmer_floats_up_dives_and_surfaces() {
        // 1 m under the resting height floats up at the buoyancy speed, level with it stays
        assert_eq!(wanted_vz(0.0, 98.8, 99.8), BUOY_SPEED);
        assert!(wanted_vz(0.0, 99.8, 99.8).abs() < 1e-5);
        assert!(wanted_vz(0.0, 100.3, 99.8) < 0.0);
        assert_eq!(wanted_vz(-1.0, 99.8, 99.8), -1.5);
        assert!((wanted_vz(1.0, 98.0, 99.8) - 2.1).abs() < 1e-5);
        // pressing up at the surface does not push the body out of it
        assert!(wanted_vz(1.0, 99.8, 99.8).abs() < 1e-5);
        assert_eq!(approach(0.0, 10.0, 20.0, 0.1), 2.0);
        assert_eq!(approach(-8.0, 0.0, 35.0, 0.1), -4.5);
    }

    /// Depths (m, one digit per 2 m, `.` dry) around the Nuian spawn pond (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn pond_depths() {
        let (terrain, water) = (aa_world::TerrainQuery::new("main_world"), aa_world::WaterQuery::new("main_world"));
        let (cx, cy) = (15_578.0f32, 15_382.0f32);
        for j in (-20..=20).rev() {
            let row: String = (-20..=20)
                .map(|i| {
                    let (x, y) = (cx + i as f32 * 2.0, cy + j as f32 * 2.0);
                    let Some(h) = terrain.height_at(x, y) else { return '?' };
                    match water.water_at(Vec3::new(x, y, h)) {
                        Some((surface, _)) if surface > h => char::from_digit(((surface - h) * 2.0).round().clamp(0.0, 9.0) as u32, 10).unwrap_or('+'),
                        _ => '.',
                    }
                })
                .collect();
            println!("{row}");
        }
        println!("each digit: depth in half metres (2 = 1 m, 9 = 4.5 m or more)");
    }

    /// Ground height and water depth along a line (`AA_SCAN=x0,y0,x1,y1,step`, `--ignored --nocapture`): to find a beach.
    #[test]
    #[ignore]
    fn transect() {
        let Some(v) = std::env::var("AA_SCAN").ok().map(|v| v.split(',').filter_map(|n| n.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5) else { return };
        let (terrain, water) = (aa_world::TerrainQuery::new("main_world"), aa_world::WaterQuery::new("main_world"));
        let n = ((Vec2::new(v[2], v[3]) - Vec2::new(v[0], v[1])).length() / v[4]).ceil() as usize;
        for i in 0..=n {
            let p = Vec2::new(v[0], v[1]).lerp(Vec2::new(v[2], v[3]), i as f32 / n as f32);
            let Some(h) = terrain.height_at(p.x, p.y) else { continue };
            let w = water.water_at(Vec3::new(p.x, p.y, h));
            println!("{:.1},{:.1} ground {h:.2} water {:?} normal_z {:.2}", p.x, p.y, w.map(|(s, k)| (s, s - h, k)), terrain.normal_at(p.x, p.y).map_or(0.0, |n| n.z));
        }
    }

    /// Where the head, the pelvis and the feet are in the swim clips and in the standing idle (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn swim_poses() {
        use aa_anim::pose::SampleOptions;
        let (skeleton, cal) = ("objects/characters/nuian/male/nude/nu_m_base.chr", "objects/characters/nuian/male/nude/nu_m_base.cal");
        let Ok(rig) = aa_anim::pose::shared_rig(skeleton) else { return };
        let cal = Cal::shared(cal).unwrap();
        let clips = ClipLibrary::default();
        for name in ["fist_ba_relaxed_idle", "fist_ba_idle_swim", "fist_mo_swim_f", "fist_mo_swim_l", "fist_mo_swim_up", "fist_mo_swim_f_deep", "fist_mo_normal_run_f"] {
            let Some(clip) = bind_clip(&rig, &cal, &clips, name) else { println!("{name}: missing"); continue };
            for t in [0.0, clip.clip.duration * 0.5] {
                let mut pose = rig.bind.clone();
                pose.sample(&rig, &clip.clip, &clip.binding, t, SampleOptions { lock_root_motion: false, ..clip.options });
                let m = pose.model_space(&rig);
                let find = |n: &str| rig.skeleton.bones.iter().position(|b| b.name.eq_ignore_ascii_case(n)).map(|i| m[i].1);
                let f = |v: Option<Vec3>| v.map_or("-".to_string(), |v| format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z));
                println!("{name:22} t={t:.2}/{:.2}s pelvis {} head {} lfoot {} rfoot {} root {}", clip.clip.duration, f(find("Bip01 Pelvis")), f(find("Bip01 Head")), f(find("Bip01 L Foot")), f(find("Bip01 R Foot")), f(m.get(rig.root).map(|r| r.1)));
            }
        }
    }

    #[test]
    fn the_actor_row_of_a_nuian_is_loaded() {
        if !std::path::Path::new(aa_anim::db::DEFAULT_DB).exists() {
            return;
        }
        // models.id 10 is the Nuian male (actor_models.id 1)
        let p = ActorParams::load(10).expect("row");
        assert_eq!(p.height, 1.5);
        assert!((p.swim_height - 0.2).abs() < 1e-6);
        assert!((p.jump_speed() - 8.06).abs() < 0.01, "{}", p.jump_speed());
    }
}
