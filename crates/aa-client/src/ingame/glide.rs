//! Gliders (S14): the back-slot item, the gliding buff, the flight controller, the stowed and opened models and the
//! ordinary movement packets that carry a glider.
//!
//! **What is read from the 1.2 client** (`x2game.dll.c`, base `0x38ff0000`; constants read from the binary):
//!
//! * A glider is a `item_backpacks.backpack_type_id = 2` item; its `items.use_skill_id` casts a skill whose effect applies a
//!   `buffs.gliding = 't'` buff. The flight starts and ends with that buff, not with a key: `ClientBuffMan::ApplyAttrs`
//!   (`FUN_39389440`) calls `FUN_39357920`, and the buff's removal runs `FUN_39389d60`. A backpack change
//!   (`FUN_393220a0`) is the second start path.
//! * `FUN_39357920` reads the three move speeds from unit attributes 7 / 8 / 9 (`GlidingMoveSpeed*`, x 0.001) and the
//!   other 14 values from the buff descriptor; `FUN_3911fc80(1, block)` copies the `0x50` byte block to `actor+0x168`,
//!   sets `smoothed_v = startup_speed`, divides `lift_duration` by `lift_count`, and sets stance 4. This client has no
//!   unit-attribute table, so it uses the buff row's `gliding_move_speed_*` columns; **S14-C6** captures attributes 7/8/9
//!   from the live unit (base + modifiers) to confirm they equal the row.
//! * `FUN_3912d860` is the glide controller (stance 4): elapsed <= `startup_time` uses `move_normal` and climbs at
//!   `+startup_speed` (the decompile flips the sign twice: store at `+0x1b8`, then the velocity write); then the vertical
//!   input (`controller+0x10`) below `-0.1` picks slow, above `0.1` fast, else normal.
//!   A lift (`lift_count > 0`, `elapsed > lift_valid_time`, the same input below `-0.1`, no lift running, a downward ray
//!   of `lift_height` hits) sets the lift timer to `lift_count * lift_duration` and decrements the count; while it runs
//!   the vertical target is `+lift_speed` and the smoothing response is `smooth_time * 0.2`. The vertical velocity is
//!   filtered with `x = 2 * dt / smooth_time` and `1 / (1 + x + 0.48x^2 + 0.235x^3)`.
//! * `FUN_3911fc80(0, 0)` is the stop probe: with `sliding_time > 0` and a downwards velocity, a downward ray of
//!   `land_height + 0.2` that hits sets stance 5 (sliding), a miss sets stance 0; a rising velocity also keeps stance 5.
//!   `CActor::Update` (`FUN_3911feb0`) counts `sliding_time` down and leaves stance 5 at 0.
//! * `FUN_3912b280` is the sliding movement: `move_normal` along the facing, no gravity and no input.
//! * The wire is the ordinary `CSMoveUnit` type 1; there is no glider packet. The stance byte and actor flags while gliding
//!   were not found in a capture yet, so this client sends `stance::FLY` with no actor flag (airborne to the server) and
//!   mark it capture-pending (S14-C1).
//! * The animation graph (`game/animations/graphs/player_full_body.xml`) has `gliding`/`gliding_slow`/`gliding_fast`/
//!   `gliding_sliding`, the turn poses `gliding_{left,right,front_left,...}` (TurnSpeedLH +-1.9, 2..200, -200..-2),
//!   `gliding_start` and `gliding_end` (which fires `CleanupGlider`); the glider model's own `.cal` names
//!   `start`, `loop`, `end`, `endloop`, `left`, `right`, `front`, `back`, `sliding`.
//!
//! **Capture-pending** (concrete capture steps are in `docs/aa-client-movement.md`):
//!
//! * S14-C1: the stance byte and actor flags of the gliding and landing movements.
//! * S14-C2: which keys feed `controller+0x10` (W / S is assumed, as in the old branches, but the decompile only names the
//!   axis).
//! * S14-C3: the movement packet of the first ground contact (this client sends a normal standing packet).
//! * S14-C4: the double-jump deploy gate (`OPTION_ITEM_USE_GLIDE_WITH_DOUBLE_JUMP`, option 57, default off): the height /
//!   freefall gate is unknown; this client asks for the item's skill when airborne in a freefall or 3 m over the terrain.
//! * S14-C5: where the opened model hangs (this client puts it at the character's origin, like the old branch).
//! * S14-C6: the move speeds against the live unit attributes 7/8/9 (the buff row is used here; see above).
//! * S14-C7: a ceiling stops the rise (x2game has no upward ray; cryphysics owns the capsule, so this is a conservative
//!   `WorldCollision` ray at the head, not a captured rule).
//! * S14-C8: `BUFF_GRACE` 0.2 s and `TOUCH_TIMEOUT` 1.0 s are client tolerances, not decompiled timers; capture the buff
//!   removal-to-slide delay. `END_CLIP` 0.5 s is the graph's `gliding_end forceInStateUntil` [R data].

use std::collections::HashMap;
use std::sync::Arc;

use aa_anim::pose::BoundClip;
use aa_anim::{Animator, Cal, ClipLibrary};
use aa_net::model::{slot, stance};
use aa_net::state::{Buffs, Inventory, Units};
use aa_net::{Command, MoveCommand, MoveState, NetCommand, Pos};
use aa_world::{TerrainQuery, WaterQuery, WorldCollision};
use bevy::prelude::*;

use super::anim::{Locomotion, Motion, bind_clip};
use super::camera::OrbitCamera;
use super::combat::Dead;
use super::data::Db;
use super::dev::DevOptions;
use super::player::{
    self, HEIGHT, LocalPlayer, Mover, PlayerInput, RADIUS, STEP_UP, SendState, placement,
};
use super::ship::DeckRide;
use super::ship::deck::{DeckContact, DeckGround, DeckSlide, deck_move, leave_deck, slide_step};
use super::weapons::StowedPack;

/// `DAT_3999cf98`: degrees to radians in the 0x50 block builder (kept for the block's derivation; the rows are loaded
/// directly in [`GlideParams::load_all`]).
#[allow(dead_code)]
pub const DEG_TO_RAD: f32 = 0.017_453_292;
/// `DAT_3999cf30`: milliseconds to seconds for the move speeds.
#[allow(dead_code)]
pub const MS_TO_S: f32 = 0.001;
/// `DAT_3999ccf0` / `DAT_399a2f24`: the tier dead zone of the vertical input.
pub const TIER: f32 = 0.1;
/// `DAT_399a2998`, `DAT_399a2994`: the vertical smoothing filter.
pub const SMOOTH_X2: f32 = 0.48;
pub const SMOOTH_X3: f32 = 0.235;
/// `DAT_3999dd34` (0.2), but read as a scale of the smoothing response while a lift runs (`FUN_3912d860`).
pub const LIFT_SMOOTH_SCALE: f32 = 0.2;
/// `DAT_3999cf90`: the smallest vertical difference the smoothing filter acts on.
pub const SMOOTH_EPS: f32 = 0.01;
/// Seconds after the buff is gone before the stop probe runs (the buff list lags the packets a frame or two).
/// S14-C8: a client tolerance, not a decompiled timer.
const BUFF_GRACE: f32 = 0.2;
/// Seconds a landed glider waits for the server's buff removal before it stops on its own. S14-C8: a local fallback.
const TOUCH_TIMEOUT: f32 = 1.0;
/// The `gliding_end` clip keeps the character busy this long (s): `player_full_body.xml`'s `forceInStateUntil` 0.5.
const END_CLIP: f32 = 0.5;
/// Interval between movement packets while flying (s).
const SEND_INTERVAL: f32 = 0.1;
/// A deploy request waits this long before it may go out again (s).
const DEPLOY_RETRY: f64 = 1.0;
/// Distance under the feet within which the stop probe sees the ground (m): `land_height + 0.2`.
pub const LAND_RAY_MARGIN: f32 = 0.2;

/// The first surface above the head, as a highest allowed feet height for a rise of `rise` (S14-C7: x2game has no ceiling
/// ray; this stands in for the capsule collision cryphysics would do, and is capture-pending).
fn ceiling_limit(collision: &WorldCollision, feet: Vec3, rise: f32) -> Option<f32> {
    if rise <= 0.0 {
        return None;
    }
    collision
        .raycast(feet + Vec3::Z * HEIGHT, Vec3::Z, rise + 0.01)
        .map(|hit| feet.z + (hit.t - 0.01).max(0.0))
}

/// The numbers of one gliding buff, in the 0x50 block's order (`FUN_39357920`, `FUN_3911fc80`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlideParams {
    /// `+0x00` slow move speed (`gliding_move_speed_slow`).
    pub move_slow: f32,
    /// `+0x04` normal move speed and the sliding speed.
    pub move_normal: f32,
    /// `+0x08` fast move speed.
    pub move_fast: f32,
    /// `+0x0c` turn speed (rad/s; the row is degrees).
    pub rotate_speed: f32,
    /// `+0x10` startup time (s).
    pub startup_time: f32,
    /// `+0x14` startup climb (m/s; the block stores it as the starting smoothed vertical speed, and the double sign flip
    /// in `FUN_3912d860` keeps it positive).
    pub startup_speed: f32,
    /// `+0x18` slow fall speed (m/s, downwards).
    pub fall_slow: f32,
    /// `+0x1c` normal fall speed.
    pub fall_normal: f32,
    /// `+0x20` fast fall speed.
    pub fall_fast: f32,
    /// `+0x24` vertical smoothing time (s).
    pub smooth_time: f32,
    /// `+0x28` lifts (i32).
    pub lift_count: i32,
    /// `+0x2c` lift ray length (m).
    pub lift_height: f32,
    /// `+0x30` earliest lift time (s).
    pub lift_valid_time: f32,
    /// `+0x34` lift duration (s, divided by `lift_count` on start).
    pub lift_duration: f32,
    /// `+0x38` upward lift speed (m/s).
    pub lift_speed: f32,
    /// `+0x3c` landing ray length (m; the ray adds [`LAND_RAY_MARGIN`]).
    pub land_height: f32,
    /// `+0x40` sliding time (s).
    pub sliding_time: f32,
}

impl GlideParams {
    /// Buff 1029, the Experimental Glider (quest 1065's reward, item 14677, skill 12373): the row this slice's tests use.
    /// Values from `buffs` id 1029 (`gliding = 't'`).
    #[allow(dead_code)]
    pub const EXPERIMENTAL: GlideParams = GlideParams {
        move_slow: 5.0,
        move_normal: 6.0,
        move_fast: 7.0,
        rotate_speed: 30.0 * DEG_TO_RAD,
        startup_time: 0.7,
        startup_speed: 3.0,
        fall_slow: 1.0,
        fall_normal: 1.5,
        fall_fast: 2.5,
        smooth_time: 1.0,
        lift_count: 2,
        lift_height: 1.5,
        lift_valid_time: 5.0,
        lift_duration: 0.5,
        lift_speed: 3.0,
        land_height: 1.0,
        sliding_time: 1.0,
    };

    /// The gliding buffs of the game database (`buffs.gliding`), by buff id.
    pub fn load_all() -> HashMap<u32, GlideParams> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let mut out = HashMap::new();
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        ) else {
            warn!("glide: no game database, no gliding buff is known");
            return out;
        };
        let read = conn
            .prepare(
                "SELECT id, gliding_move_speed_slow, gliding_move_speed_normal, gliding_move_speed_fast, gliding_rotate_speed, \
                 gliding_startup_time, gliding_startup_speed, gliding_fall_speed_slow, gliding_fall_speed_normal, gliding_fall_speed_fast, \
                 gliding_smooth_time, gliding_lift_count, gliding_lift_height, gliding_lift_valid_time, gliding_lift_duration, \
                 gliding_lift_speed, gliding_land_height, gliding_sliding_time FROM buffs WHERE gliding = 't'",
            )
            .and_then(|mut st| {
                st.query_map([], |r| {
                    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                    Ok((
                        r.get::<_, i64>(0)? as u32,
                        GlideParams {
                            move_slow: f(1)?,
                            move_normal: f(2)?,
                            move_fast: f(3)?,
                            rotate_speed: f(4)?.to_radians(),
                            startup_time: f(5)?,
                            startup_speed: f(6)?,
                            fall_slow: f(7)?,
                            fall_normal: f(8)?,
                            fall_fast: f(9)?,
                            smooth_time: f(10)?,
                            lift_count: f(11)?.max(0.0) as i32,
                            lift_height: f(12)?,
                            lift_valid_time: f(13)?,
                            lift_duration: f(14)?,
                            lift_speed: f(15)?,
                            land_height: f(16)?,
                            sliding_time: f(17)?,
                        },
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
            });
        match read {
            Ok(rows) => out.extend(rows),
            Err(e) => warn!("glide: buffs.gliding_*: {e}"),
        }
        out
    }

    /// `FUN_3911fc80`: `lift_duration / lift_count` (the whole count is 1.0 when it is zero).
    pub fn normalized_lift_duration(&self) -> f32 {
        if self.lift_count != 0 {
            self.lift_duration / self.lift_count as f32
        } else {
            self.lift_duration
        }
    }
}

/// The phase of a flight. Stance 4 is [`Phase::Open`] / [`Phase::Gliding`], stance 5 is [`Phase::Sliding`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// `elapsed <= startup_time`: the opening.
    Open,
    Gliding,
    Sliding,
    Done,
}

/// What the player asks of the glider in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// W 1, S -1 (S14-C2: the decompile names the axis, not the keys).
    pub forward: f32,
    /// A (left) 1, D (right) -1.
    pub turn: f32,
}

/// One frame of a flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub phase: Phase,
    /// Ground speed along the facing (m/s).
    pub speed: f32,
    /// Vertical speed (m/s, up is positive).
    pub vz: f32,
    /// Radians a second to turn by.
    pub turn_rate: f32,
    /// A lift is running.
    pub lifting: bool,
    /// The slide ran out.
    pub done: bool,
}

/// The flight state machine: pure numbers, the decompile's `FUN_3912d860` / `FUN_3912b280` / `FUN_3911fc80` / sliding
/// timer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub params: GlideParams,
    pub phase: Phase,
    /// `+0x44` flight elapsed (s).
    pub elapsed: f32,
    /// `+0x48` lift time left (s).
    pub lift_left: f32,
    /// `+0x28` lifts left.
    pub lift_count: i32,
    /// `+0x34` the normalized `lift_duration`.
    pub lift_duration: f32,
    /// `+0x1b8` smoothed vertical velocity.
    pub smoothed_v: f32,
    /// `+0x1bc` smoothed vertical derivative.
    pub smoothed_dv: f32,
    /// `+0x1a8` sliding time left (s).
    pub slide_left: f32,
    /// Horizontal speed of this frame (m/s).
    pub speed: f32,
    /// Vertical speed of this frame (m/s).
    pub vz: f32,
}

impl Flight {
    /// `FUN_3911fc80(1, block)`: the glider opens. `smoothed_v` starts at `+startup_speed` (the decompile's
    /// `actor+0x1b8 = block+0x14`) and `FUN_3912d860`'s startup target is the same `+startup_speed` after its double
    /// sign flip, so the opening climbs at that speed.
    pub fn start(params: GlideParams) -> Flight {
        Flight {
            params,
            phase: Phase::Open,
            elapsed: 0.0,
            lift_left: 0.0,
            lift_count: params.lift_count,
            lift_duration: params.normalized_lift_duration(),
            smoothed_v: params.startup_speed,
            smoothed_dv: 0.0,
            slide_left: 0.0,
            speed: params.move_normal,
            vz: params.startup_speed,
        }
    }

    /// One frame of `FUN_3912d860`. `lift_ray_hit` is the downward ray of `lift_height` under the unit.
    pub fn step(&mut self, dt: f32, input: Input, lift_ray_hit: bool) -> Step {
        let p = self.params;
        self.elapsed += dt;
        self.lift_left -= dt;
        let (move_speed, fall_target) = if self.elapsed <= p.startup_time {
            // `FUN_3912d860` flips the startup speed twice (store then the velocity write), so the opening
            // climbs at `startup_speed`; the tier branches below are the sinks.
            (p.move_normal, p.startup_speed)
        } else if input.forward <= TIER {
            if input.forward >= -TIER {
                (p.move_normal, -p.fall_normal)
            } else {
                (p.move_slow, -p.fall_slow)
            }
        } else {
            (p.move_fast, -p.fall_fast)
        };
        self.speed = move_speed;
        if self.lift_count > 0
            && self.elapsed > p.lift_valid_time
            && input.forward <= -TIER
            && self.lift_left <= 0.0
            && lift_ray_hit
        {
            self.lift_left = self.lift_count as f32 * self.lift_duration;
            self.lift_count -= 1;
        }
        let mut smooth_time = p.smooth_time;
        let mut target = fall_target;
        if self.lift_left > 0.0 {
            smooth_time *= LIFT_SMOOTH_SCALE;
            target = p.lift_speed;
        }
        if (self.smoothed_v - target).abs() >= SMOOTH_EPS {
            if smooth_time <= 0.0 {
                if dt <= 0.0 {
                    self.smoothed_v = target;
                    self.smoothed_dv = 0.0;
                } else {
                    self.smoothed_dv = (target - self.smoothed_v) / dt;
                    self.smoothed_v = target;
                }
            } else {
                let inv = 2.0 / smooth_time;
                let x = inv * dt;
                let den = x * SMOOTH_X2 * x + x + 1.0 + x * SMOOTH_X3 * x * x;
                let scale = 1.0 / den;
                let delta = self.smoothed_v - target;
                let acc = (delta * inv + self.smoothed_dv) * dt;
                self.smoothed_dv = (self.smoothed_dv - acc * inv) * scale;
                self.smoothed_v = (acc + delta) * scale + target;
            }
            self.vz = self.smoothed_v;
        } else {
            self.smoothed_dv = 0.0;
            self.smoothed_v = target;
            self.vz = target;
        }
        self.phase = if self.elapsed <= p.startup_time {
            Phase::Open
        } else {
            Phase::Gliding
        };
        Step {
            phase: self.phase,
            speed: self.speed,
            vz: self.vz,
            turn_rate: if input.turn > 0.0 {
                p.rotate_speed
            } else if input.turn < 0.0 {
                -p.rotate_speed
            } else {
                0.0
            },
            lifting: self.lift_left > 0.0,
            done: false,
        }
    }

    /// `FUN_3911fc80(0, 0)`: the stop probe. `ground` is the distance under the unit when there is a surface. A descending
    /// glider within `land_height + 0.2` slides; a rising one keeps sliding; anything else stops at once (stance 0).
    pub fn stop(&mut self, vertical_speed: f32, ground: Option<f32>) -> Phase {
        // `FUN_3911fc80` only chooses 4/5/0 from the glide states; a running slide keeps its timer.
        if !matches!(self.phase, Phase::Open | Phase::Gliding) {
            return self.phase;
        }
        let near =
            ground.is_some_and(|distance| distance <= self.params.land_height + LAND_RAY_MARGIN);
        let slide = self.params.sliding_time > 0.0
            && (vertical_speed > 0.0 || (vertical_speed <= 0.0 && near));
        if slide {
            self.phase = Phase::Sliding;
            self.slide_left = self.params.sliding_time;
            self.speed = self.params.move_normal;
            self.vz = 0.0;
        } else {
            self.phase = Phase::Done;
            self.speed = 0.0;
            self.vz = 0.0;
        }
        self.phase
    }

    /// The sliding timer of `CActor::Update` and `FUN_3912b280`'s straight movement. Returns true when the slide ran out.
    pub fn slide(&mut self, dt: f32) -> bool {
        if self.phase != Phase::Sliding {
            return true;
        }
        self.slide_left -= dt;
        if self.slide_left < 0.0 {
            self.slide_left = 0.0;
            self.phase = Phase::Done;
            self.speed = 0.0;
            self.vz = 0.0;
            return true;
        }
        self.speed = self.params.move_normal;
        self.vz = 0.0;
        false
    }
}

/// The character and glider poses of the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    Start,
    Idle,
    Fast,
    Slow,
    Left,
    Right,
    FrontLeft,
    FrontRight,
    BackLeft,
    BackRight,
    Slide,
    End,
}

/// The pose for a frame (`player_full_body.xml`: `gliding`, the speed action and TurnSpeedLH).
pub fn pose(flight: &Flight, input: Input) -> Pose {
    match flight.phase {
        Phase::Open => return Pose::Start,
        Phase::Sliding => return Pose::Slide,
        Phase::Done => return Pose::End,
        Phase::Gliding => {}
    }
    let turn = input.turn;
    // TurnSpeedLH 1.9 dead zone, 2..200 / -200..-2 for the side poses (degrees a second).
    let turning = |t: f32| t.abs() * flight.params.rotate_speed.to_degrees() >= 1.9;
    if input.forward > TIER {
        if turning(turn) {
            if turn > 0.0 {
                Pose::FrontLeft
            } else {
                Pose::FrontRight
            }
        } else {
            Pose::Fast
        }
    } else if input.forward < -TIER {
        if turning(turn) {
            if turn > 0.0 {
                Pose::BackLeft
            } else {
                Pose::BackRight
            }
        } else {
            Pose::Slow
        }
    } else if turning(turn) {
        if turn > 0.0 { Pose::Left } else { Pose::Right }
    } else {
        Pose::Idle
    }
}

impl Pose {
    /// The character's `.cal` clip and whether it loops.
    pub fn character_clip(self) -> (&'static str, bool) {
        match self {
            Pose::Start => ("fist_ac_gliding_start", false),
            Pose::Idle => ("fist_ac_gliding_idle", true),
            Pose::Fast => ("fist_ac_gliding_front", true),
            Pose::Slow => ("fist_ac_gliding_back", true),
            Pose::Left => ("fist_ac_gliding_left", true),
            Pose::Right => ("fist_ac_gliding_right", true),
            Pose::FrontLeft => ("fist_ac_gliding_fl", true),
            Pose::FrontRight => ("fist_ac_gliding_fr", true),
            Pose::BackLeft => ("fist_ac_gliding_bl", true),
            Pose::BackRight => ("fist_ac_gliding_br", true),
            Pose::Slide => ("fist_ac_gliding_sliding", true),
            Pose::End => ("fist_ac_gliding_end", false),
        }
    }

    /// The glider model's `.cal` clip (`glider_bx.cal`: start, loop, end, endloop, left, right, front, back, sliding) and
    /// whether it loops. The diagonals take the side clip of the turn.
    pub fn glider_clip(self) -> (&'static str, bool) {
        match self {
            // `FUN_3911d6c0(1)` plays the item model's "loop" when the glider opens (the character plays
            // `fist_ac_gliding_start`); "start" exists in the `.cal` but is not the decompile's attach animation.
            Pose::Start => ("loop", true),
            Pose::Idle => ("loop", true),
            Pose::Fast => ("front", true),
            Pose::Slow => ("back", true),
            Pose::Left | Pose::FrontLeft | Pose::BackLeft => ("left", true),
            Pose::Right | Pose::FrontRight | Pose::BackRight => ("right", true),
            Pose::Slide => ("sliding", true),
            Pose::End => ("endloop", false),
        }
    }
}

/// The flight the local player is in.
#[derive(Clone, Copy, Debug)]
pub struct Active {
    pub flight: Flight,
    pub buff_id: u32,
    pub buff_index: u32,
    pub pose: Pose,
    /// The glider touched the ground and a standing movement went out; waiting for the server's buff removal.
    pub touched: bool,
    pub touched_for: f32,
    /// Seconds the gliding buff has been missing.
    pub missing: f32,
    /// The ship deck the glider touched, if any: the touched wait and the slide run in its frame.
    pub deck: Option<DeckContact>,
}

/// The local player's glider: the flight, the numbers of every gliding buff and the deploy retry.
#[derive(Resource, Default)]
pub struct Gliding {
    pub active: Option<Active>,
    /// Seconds of the `gliding_end` clip left (`CleanupGlider` follows it).
    pub ending: f32,
    /// The buff whose flight ended while it was still in the list: it must leave before it can start another flight.
    ended: Option<u32>,
    params: Option<HashMap<u32, GlideParams>>,
    /// When the last deploy request went out (s).
    asked: Option<f64>,
    since_sent: f32,
    sent_first: bool,
}

impl Gliding {
    /// The player is moved by the glider controller.
    pub fn flying(&self) -> bool {
        self.active.is_some()
    }

    /// The character or its `gliding_end` clip is still shown.
    pub fn shown(&self) -> bool {
        self.active.is_some() || self.ending > 0.0
    }

    /// The numbers of a gliding buff (the table is loaded once).
    pub fn params_of(&mut self, buff: u32) -> Option<GlideParams> {
        self.params
            .get_or_insert_with(GlideParams::load_all)
            .get(&buff)
            .copied()
    }

    fn end_flight(&mut self, buff_id: u32) {
        self.active = None;
        self.ending = END_CLIP;
        self.ended = Some(buff_id);
        self.since_sent = 0.0;
        self.sent_first = false;
    }
}

/// The glider model of the local player, spawned with the flight.
#[derive(Component)]
pub struct GliderModel;

/// A glider model that belongs to another player's character.
#[derive(Component)]
pub struct RemoteGliderModel;

/// The glider model's `.cal` and the pose it last played.
#[derive(Component)]
pub struct GliderPlay {
    cal: Option<Arc<Cal>>,
    playing: Option<Pose>,
    /// The pose whose clip is missing from the model: do not retry it every frame.
    failed: Option<Pose>,
}

/// The character's gliding clips, bound to its rig.
#[derive(Component, Default)]
pub struct GlideClips {
    bound: HashMap<&'static str, Option<BoundClip>>,
    playing: Option<Pose>,
    was: bool,
    end_played: bool,
}

/// Another player's character under a gliding buff.
#[derive(Component)]
pub struct RemoteGlide {
    pub buff_id: u32,
    pub params: GlideParams,
    pub age: f32,
    pub speed: f32,
    pub pose: Pose,
    glider: Option<Entity>,
    /// The pose the character's clip was last started for (the glider part has its own [`GliderPlay`]).
    character_playing: Option<Pose>,
    bound: HashMap<&'static str, Option<BoundClip>>,
}

/// `OPTION_ITEM_USE_GLIDE_WITH_DOUBLE_JUMP` (57, the 1.2 cvar `glider_start_with_double_jump`, default 0).
pub const DOUBLE_JUMP_OPTION: u32 = 57;

/// Whether the double-jump deploy option is on (default 0: off).
pub fn double_jump_on(saved: Option<f64>) -> bool {
    saved
        .or_else(|| crate::settings::native_default(DOUBLE_JUMP_OPTION))
        .is_some_and(|value| value != 0.0)
}

/// Whether a jump press in the air may ask for the glider (S14-C4: the gate is inferred).
pub fn may_deploy(
    option_on: bool,
    airborne: bool,
    swimming: bool,
    freefall: bool,
    height: f32,
    flying: bool,
) -> bool {
    option_on
        && airborne
        && !swimming
        && !flying
        && (freefall || height >= super::jump::FALL_START_HEIGHT_JUMPED)
}

/// The gliding buff on a unit: `(buff id, index)`.
pub fn gliding_buff(
    buffs: &[aa_net::state::BuffEntry],
    mut is_gliding: impl FnMut(u32) -> bool,
) -> Option<(u32, u32)> {
    buffs
        .iter()
        .find(|entry| is_gliding(entry.buff.buff_id))
        .map(|entry| (entry.buff.buff_id, entry.buff.index))
}

/// Starts and ends the flight with the own gliding buff and runs the stop probe when it goes.
#[allow(clippy::too_many_arguments)]
pub fn track_flight(
    time: Res<Time>,
    buffs: Option<Res<Buffs>>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    deck: DeckGround,
    mount: Option<Res<super::mount::LocalMount>>,
    mut commands: Commands,
    mut gliding: ResMut<Gliding>,
    mut state: ResMut<SendState>,
    player: Query<(Entity, &LocalPlayer, &Mover, Has<Dead>)>,
) {
    let dt = time.delta_secs().min(0.1);
    let Ok((entity, local, mover, dead)) = player.single() else {
        gliding.active = None;
        state.glide_owns = false;
        return;
    };
    if dead {
        gliding.active = None;
        state.glide_owns = false;
        return;
    }
    if mount.as_deref().is_some_and(|mount| mount.0.is_some()) {
        if let Some(active) = gliding.active.take() {
            gliding.ended = Some(active.buff_id);
        }
        state.glide_owns = false;
        return;
    }
    let own: &[aa_net::state::BuffEntry] = buffs.as_deref().map_or(&[], |b| b.of(local.obj_id));
    let found = {
        let g = &mut *gliding;
        let table = g.params.get_or_insert_with(GlideParams::load_all);
        gliding_buff(own, |id| table.contains_key(&id))
    };
    if found.is_none() {
        gliding.ended = None;
    }
    if let Some(mut active) = gliding.active {
        match found {
            Some((id, index)) => {
                active.missing = 0.0;
                active.buff_id = id;
                active.buff_index = index;
            }
            None => active.missing += dt,
        }
        let timeout = active.touched && active.touched_for > TOUCH_TIMEOUT;
        let can_stop = matches!(active.flight.phase, Phase::Open | Phase::Gliding);
        if can_stop && (active.missing >= BUFF_GRACE || timeout) {
            let mut deck_contact = deck.contact_within(
                mover.pos,
                mover.vz,
                dt,
                active.flight.params.land_height + LAND_RAY_MARGIN,
            );
            if let Some(contact) = deck_contact.as_mut() {
                contact.facing = mover.facing - contact.yaw;
            }
            let floor = deck_contact
                .map(|contact| contact.world().z)
                .or_else(|| player::support_at(&terrain, &collision, mover.pos, 0.05));
            let distance = floor.map(|floor| mover.pos.z - floor);
            let phase = active.flight.stop(mover.vz, distance);
            if phase == Phase::Sliding {
                active.pose = Pose::Slide;
                if let Some(contact) = deck_contact {
                    active.deck = Some(contact);
                    commands.entity(entity).insert(DeckRide::new(
                        contact.ship,
                        contact.local,
                        contact.facing,
                        contact.yaw,
                    ));
                    info!(
                        "glide: buff {} gone, sliding {:.2} s on ship {} at local ({:.1}, {:.1}, {:.1})",
                        active.buff_id,
                        active.flight.params.sliding_time,
                        contact.ship,
                        contact.local.x,
                        contact.local.y,
                        contact.local.z
                    );
                } else {
                    info!(
                        "glide: buff {} gone, sliding {:.2} s on the ground ({:.2} m)",
                        active.buff_id,
                        active.flight.params.sliding_time,
                        distance.unwrap_or(0.0)
                    );
                }
            } else {
                info!(
                    "glide: buff {} gone, folding in the air at {:.1} m",
                    active.buff_id,
                    distance.unwrap_or(f32::INFINITY)
                );
                gliding.end_flight(active.buff_id);
                state.glide_owns = false;
                return;
            }
        }
        gliding.active = Some(active);
        state.glide_owns = true;
        return;
    }
    let Some((buff_id, buff_index)) = found else {
        return;
    };
    if gliding.ended == Some(buff_id) {
        return;
    }
    let Some(params) = gliding.params_of(buff_id) else {
        return;
    };
    let flight = Flight::start(params);
    info!(
        "glide: buff {buff_id} opens the glider (move {:.0}/{:.0}/{:.0}, fall {:.1}/{:.1}/{:.1}, {:.2} s startup)",
        params.move_slow,
        params.move_normal,
        params.move_fast,
        params.fall_slow,
        params.fall_normal,
        params.fall_fast,
        params.startup_time
    );
    // the flight owns the character: a ship deck ride (a moving frame) is let go
    commands.entity(entity).remove::<super::ship::DeckRide>();
    gliding.active = Some(Active {
        flight,
        buff_id,
        buff_index,
        pose: Pose::Start,
        touched: false,
        touched_for: 0.0,
        missing: 0.0,
        deck: None,
    });
    gliding.ending = 0.0;
    gliding.since_sent = SEND_INTERVAL;
    gliding.sent_first = false;
    state.glide_owns = true;
}

/// A second jump in the air uses the equipped glider's item skill (option 57; S14-C4).
#[allow(clippy::too_many_arguments)]
pub fn request_deploy(
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    inventory: Option<Res<Inventory>>,
    db: Option<Res<Db>>,
    terrain: Res<TerrainQuery>,
    state: Res<SendState>,
    mount: Option<Res<super::mount::LocalMount>>,
    mut gliding: ResMut<Gliding>,
    player: Query<(&Mover, Has<Dead>), With<LocalPlayer>>,
    mut out: MessageWriter<NetCommand>,
) {
    let Ok((mover, dead)) = player.single() else {
        return;
    };
    if dead
        || typing.is_some_and(|f| f.0)
        || gliding.flying()
        || mount.as_deref().is_some_and(|mount| mount.0.is_some())
        || !actions.just_pressed("jump", 0)
    {
        return;
    }
    let now = time.elapsed_secs_f64();
    if gliding
        .asked
        .is_some_and(|asked| now - asked < DEPLOY_RETRY)
    {
        return;
    }
    let ground = terrain
        .ground_at(mover.pos.x, mover.pos.y)
        .map_or(mover.pos.z, |g| g.height);
    let airborne = !mover.grounded && mover.swim.is_none();
    if !may_deploy(
        double_jump_on(crate::settings::option_number("n:57")),
        airborne,
        mover.swim.is_some(),
        state.in_freefall(),
        mover.pos.z - ground,
        false,
    ) {
        return;
    }
    let Some(item) = inventory
        .as_deref()
        .and_then(|i| i.equipment.get(slot::BACKPACK))
    else {
        return;
    };
    let Some(glider) = db
        .as_deref()
        .and_then(|db| db.glider(item.template_id))
        .filter(|g| g.use_skill != 0)
    else {
        return;
    };
    gliding.asked = Some(now);
    info!(
        "glide: second jump in the air, glider skill {} (item {})",
        glider.use_skill, item.template_id
    );
    out.write(NetCommand(Command::UseItem {
        item_id: item.id,
        template_id: item.template_id,
        skill_id: glider.use_skill,
        target_obj_id: 0,
    }));
}

/// The flight: keys, turning, the move, the ground and the movement packets. Runs before `player::control_player`, which
/// leaves the character alone while [`SendState::glide_owns`] is set.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn control_glide(
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    dev: Res<DevOptions>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    water: Res<WaterQuery>,
    deck: DeckGround,
    mount: Option<Res<super::mount::LocalMount>>,
    mut gliding: ResMut<Gliding>,
    mut state: ResMut<SendState>,
    mut input_state: ResMut<PlayerInput>,
    mut cameras: Query<&mut OrbitCamera>,
    mut player: Query<(
        Entity,
        &LocalPlayer,
        &mut Mover,
        &mut Transform,
        &mut Motion,
        Has<Dead>,
    )>,
    mut out: MessageWriter<NetCommand>,
    mut commands: Commands,
) {
    let dt = time.delta_secs().min(0.1);
    if gliding.ending > 0.0 {
        gliding.ending = (gliding.ending - dt).max(0.0);
    }
    let Ok((entity, _local, mut mover, mut transform, mut motion, dead)) = player.single_mut()
    else {
        gliding.active = None;
        state.glide_owns = false;
        return;
    };
    if dead {
        gliding.active = None;
        state.glide_owns = false;
        return;
    }
    if mount.as_deref().is_some_and(|mount| mount.0.is_some()) {
        if let Some(active) = gliding.active.take() {
            gliding.ended = Some(active.buff_id);
        }
        state.glide_owns = false;
        return;
    }
    let Some(mut active) = gliding.active else {
        state.glide_owns = false;
        return;
    };
    let typing = typing.is_some_and(|f| f.0);
    let t = state.since_spawn();
    let pressed =
        |action: &str, c: char| (!typing && actions.pressed(action, 0)) || dev.scripted(t, c);
    let input = Input {
        forward: pressed("moveforward", 'W') as i32 as f32 - pressed("moveback", 'S') as i32 as f32,
        turn: pressed("turnleft", 'A') as i32 as f32 - pressed("turnright", 'D') as i32 as f32,
    };
    state.glide_owns = true;
    // stance 5: straight at the normal speed until the sliding timer runs out
    if active.flight.phase == Phase::Sliding {
        let done = active.flight.slide(dt);
        active.pose = Pose::Slide;
        let old = mover.pos;
        if let Some(contact) = active.deck.as_mut() {
            let Some((root, yaw)) = deck.pose(contact.ship) else {
                // the hull left the view: the slide falls from the last known point and never sends a deck move again
                let facing = contact.facing;
                leave_deck(
                    &mut commands,
                    entity,
                    &mut mover,
                    &mut transform,
                    &mut motion,
                    &mut input_state,
                    contact.root,
                    contact.yaw,
                    facing,
                    contact.local,
                    Vec2::ZERO,
                    0.0,
                    Vec3::ZERO,
                    true,
                    true,
                    &mut state,
                    None,
                );
                mover.grounded = false;
                mover.vz = 0.0;
                info!(
                    "glide: ship {} gone during the slide, falling",
                    contact.ship
                );
                gliding.end_flight(active.buff_id);
                state.glide_owns = false;
                return;
            };
            contact.root = root;
            contact.yaw = yaw;
            mover.facing = yaw + contact.facing;
            let local_facing = contact.facing;
            let floor = deck.ship_floor(contact.ship);
            match slide_step(
                floor.as_deref(),
                &collision,
                root,
                yaw,
                contact.local,
                local_facing,
                active.flight.speed,
                dt,
            ) {
                DeckSlide::Moved(local) => {
                    contact.local = local;
                    let pos = contact.world();
                    mover.pos = pos;
                    mover.vz = 0.0;
                    mover.grounded = true;
                    mover.velocity = if dt > 0.0 {
                        (pos - old) / dt
                    } else {
                        Vec3::ZERO
                    };
                    *transform = placement(pos, mover.facing, 1.0);
                    *motion = Motion::default();
                    *input_state = PlayerInput {
                        moving: true,
                        jumped: false,
                        turned: false,
                    };
                    gliding.since_sent += dt;
                    if gliding.since_sent >= SEND_INTERVAL || !gliding.sent_first {
                        let command = deck_move(
                            contact.ship,
                            contact.local,
                            local_facing,
                            pos,
                            mover.facing,
                            true,
                            MoveState::Ground,
                            [0, 127, 0],
                        );
                        send_command(&mut out, &mut gliding, command);
                    }
                    if done {
                        info!(
                            "glide: the slide is over on ship {} at local ({:.1}, {:.1}, {:.1})",
                            contact.ship, contact.local.x, contact.local.y, contact.local.z
                        );
                        // deck_control resumes next frame: leave the ride at the slide's end, not the touchdown point
                        commands.entity(entity).insert(DeckRide::new(
                            contact.ship,
                            contact.local,
                            contact.facing,
                            contact.yaw,
                        ));
                        let command = deck_move(
                            contact.ship,
                            contact.local,
                            local_facing,
                            pos,
                            mover.facing,
                            false,
                            MoveState::Ground,
                            [0; 3],
                        );
                        send_command(&mut out, &mut gliding, command);
                        gliding.end_flight(active.buff_id);
                        state.glide_owns = false;
                    } else {
                        gliding.active = Some(active);
                    }
                    return;
                }
                DeckSlide::Drop { local, ground } => {
                    contact.local = local;
                    let pos = contact.world();
                    mover.pos = pos;
                    mover.vz = 0.0;
                    mover.grounded = false;
                    mover.velocity = Vec3::ZERO;
                    *transform = placement(pos, mover.facing, 1.0);
                    *motion = Motion::default();
                    *input_state = PlayerInput::default();
                    commands.entity(entity).insert(DeckRide::falling(
                        contact.ship,
                        contact.local,
                        ground,
                        contact.facing,
                        contact.yaw,
                    ));
                    info!(
                        "glide: the slide stepped off a {:.1} m deck on ship {}, falling",
                        contact.local.z - ground,
                        contact.ship
                    );
                    gliding.end_flight(active.buff_id);
                    state.glide_owns = false;
                    return;
                }
                DeckSlide::Blocked => {
                    let pos = contact.world();
                    mover.pos = pos;
                    mover.vz = 0.0;
                    mover.grounded = true;
                    mover.velocity = Vec3::ZERO;
                    *transform = placement(pos, mover.facing, 1.0);
                    *motion = Motion::default();
                    *input_state = PlayerInput::default();
                    commands.entity(entity).insert(DeckRide::new(
                        contact.ship,
                        contact.local,
                        contact.facing,
                        contact.yaw,
                    ));
                    let command = deck_move(
                        contact.ship,
                        contact.local,
                        local_facing,
                        pos,
                        mover.facing,
                        false,
                        MoveState::Ground,
                        [0; 3],
                    );
                    send_command(&mut out, &mut gliding, command);
                    info!(
                        "glide: the slide hit a riser or wall on ship {} at local ({:.1}, {:.1}, {:.1})",
                        contact.ship, contact.local.x, contact.local.y, contact.local.z
                    );
                    gliding.end_flight(active.buff_id);
                    state.glide_owns = false;
                    return;
                }
                DeckSlide::OffHull {
                    local,
                    local_velocity,
                } => {
                    leave_deck(
                        &mut commands,
                        entity,
                        &mut mover,
                        &mut transform,
                        &mut motion,
                        &mut input_state,
                        root,
                        yaw,
                        local_facing,
                        local,
                        local_velocity,
                        0.0,
                        deck.ship_velocity(contact.ship),
                        true,
                        true,
                        &mut state,
                        None,
                    );
                    // leave_deck carried the ship's velocity and removed the ride; this is a normal fall, not a jump
                    mover.grounded = false;
                    mover.vz = 0.0;
                    info!(
                        "glide: the slide left ship {} at local ({:.1}, {:.1}, {:.1}), falling",
                        contact.ship, local.x, local.y, local.z
                    );
                    gliding.end_flight(active.buff_id);
                    state.glide_owns = false;
                    return;
                }
            }
        }
        let ahead = player::forward(mover.facing) * active.flight.speed * dt;
        let mut pos = collision.slide(old, ahead, RADIUS, HEIGHT, STEP_UP).pos;
        if let Some(floor) = player::support_at(
            &terrain,
            &collision,
            Vec3::new(pos.x, pos.y, old.z),
            STEP_UP,
        ) && pos.z <= floor + 0.05
        {
            pos.z = floor;
        }
        mover.pos = pos;
        mover.vz = 0.0;
        mover.grounded = true;
        mover.velocity = if dt > 0.0 {
            (pos - old) / dt
        } else {
            Vec3::ZERO
        };
        *transform = placement(pos, mover.facing, 1.0);
        *motion = Motion::default();
        *input_state = PlayerInput {
            moving: true,
            jumped: false,
            turned: false,
        };
        gliding.since_sent += dt;
        if gliding.since_sent >= SEND_INTERVAL || !gliding.sent_first {
            send_move(
                &mut out,
                &mut gliding,
                pos,
                mover.facing,
                true,
                stance::RELAXED,
                mover.velocity,
                [0; 3],
                MoveState::Ground,
            );
        }
        if done {
            info!(
                "glide: the slide is over at ({:.1}, {:.1}, {:.1})",
                pos.x, pos.y, pos.z
            );
            send_move(
                &mut out,
                &mut gliding,
                pos,
                mover.facing,
                false,
                stance::RELAXED,
                Vec3::ZERO,
                [0; 3],
                MoveState::Ground,
            );
            gliding.end_flight(active.buff_id);
            state.glide_owns = false;
        } else {
            gliding.active = Some(active);
        }
        return;
    }
    // on the ground, waiting for the server to remove the remove-on-land buff
    if active.touched {
        active.touched_for += dt;
        if active.touched_for > TOUCH_TIMEOUT {
            let phase = active.flight.stop(0.0, Some(0.0));
            if phase == Phase::Done {
                gliding.end_flight(active.buff_id);
                state.glide_owns = false;
                return;
            }
        }
        if let Some(contact) = active.deck.as_mut() {
            let Some((root, yaw)) = deck.pose(contact.ship) else {
                let facing = contact.facing;
                leave_deck(
                    &mut commands,
                    entity,
                    &mut mover,
                    &mut transform,
                    &mut motion,
                    &mut input_state,
                    contact.root,
                    contact.yaw,
                    facing,
                    contact.local,
                    Vec2::ZERO,
                    0.0,
                    Vec3::ZERO,
                    true,
                    true,
                    &mut state,
                    None,
                );
                mover.grounded = false;
                mover.vz = 0.0;
                info!(
                    "glide: ship {} gone while waiting to land, falling",
                    contact.ship
                );
                gliding.end_flight(active.buff_id);
                state.glide_owns = false;
                return;
            };
            contact.root = root;
            contact.yaw = yaw;
            mover.facing = yaw + contact.facing;
            let pos = contact.world();
            mover.pos = pos;
            mover.vz = 0.0;
            mover.grounded = true;
            mover.velocity = Vec3::ZERO;
            *transform = placement(pos, mover.facing, 1.0);
        } else {
            mover.velocity = Vec3::ZERO;
        }
        gliding.active = Some(active);
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        return;
    }
    let old = mover.pos;
    let deck_floor = deck.floor_at(old, mover.vz, dt);
    let floor = deck_floor.or_else(|| player::support_at(&terrain, &collision, old, 0.05));
    let lift_ray_hit = floor
        .is_some_and(|floor| old.z >= floor && old.z - floor <= active.flight.params.lift_height);
    let step = active.flight.step(dt, input, lift_ray_hit);
    let before_facing = mover.facing;
    let d = step.turn_rate * dt;
    mover.facing += d;
    if let Ok(mut orbit) = cameras.single_mut() {
        orbit.yaw += d;
    }
    let ahead = player::forward(mover.facing) * step.speed * dt;
    let moved = collision.slide(old, ahead, RADIUS, HEIGHT, STEP_UP).pos;
    let mut pos = Vec3::new(moved.x, moved.y, old.z + step.vz * dt);
    let mut touched = false;
    let mut deck_contact = deck.contact(Vec3::new(pos.x, pos.y, old.z), step.vz, dt);
    if let Some(contact) = deck_contact.as_mut() {
        contact.facing = mover.facing - contact.yaw;
    }
    let landing_floor = deck_contact.map(|contact| contact.world().z).or_else(|| {
        player::support_at(
            &terrain,
            &collision,
            Vec3::new(pos.x, pos.y, old.z.max(pos.z)),
            0.05,
        )
    });
    if step.vz <= 0.0
        && let Some(floor) = landing_floor
        && pos.z <= floor
    {
        pos.z = floor;
        touched = true;
    }
    if !touched
        && step.vz <= 0.0
        && let Some((surface, _)) = water.water_at(pos)
        && pos.z < surface
    {
        pos.z = surface;
        touched = true;
    }
    // 1.2's capsule collision is cryphysics, not an x2game ray; the client has no ceiling pass on foot either.
    // Stop the rise at the first surface above the head (S14-C7: conservative, capture-pending).
    let mut ceiling = false;
    if step.vz > 0.0
        && let Some(max_z) = ceiling_limit(&collision, old, pos.z - old.z)
        && pos.z > max_z
    {
        pos.z = max_z;
        ceiling = true;
    }
    let velocity = if dt > 0.0 {
        (pos - old) / dt
    } else {
        Vec3::ZERO
    };
    mover.velocity = velocity;
    mover.pos = pos;
    mover.vz = if touched || ceiling { 0.0 } else { step.vz };
    mover.grounded = touched;
    *transform = placement(pos, mover.facing, 1.0);
    *motion = Motion::default();
    *input_state = PlayerInput {
        moving: true,
        jumped: false,
        turned: (mover.facing - before_facing).abs() > 1e-4,
    };
    active.pose = pose(&active.flight, input);
    if touched {
        active.touched = true;
        active.touched_for = 0.0;
        active.deck = deck_contact;
        gliding.active = Some(active);
        gliding.since_sent = 0.0;
        gliding.sent_first = false;
        if let Some(contact) = deck_contact {
            // the same deck ride board_deck gives a walker: the touched wait and the slide stay in the hull frame
            commands.entity(entity).insert(DeckRide::new(
                contact.ship,
                contact.local,
                contact.facing,
                contact.yaw,
            ));
            info!(
                "glide: touched ship {} at local ({:.1}, {:.1}, {:.1}), boarding the deck",
                contact.ship, contact.local.x, contact.local.y, contact.local.z
            );
            let command = deck_move(
                contact.ship,
                contact.local,
                mover.facing - contact.yaw,
                pos,
                mover.facing,
                false,
                MoveState::Ground,
                [0; 3],
            );
            send_command(&mut out, &mut gliding, command);
        } else {
            info!(
                "glide: touched the ground at ({:.1}, {:.1}, {:.1}), asking the server to land",
                pos.x, pos.y, pos.z
            );
            // S14-C3: the first grounded movement (stance and flags not captured yet)
            send_move(
                &mut out,
                &mut gliding,
                pos,
                mover.facing,
                false,
                stance::RELAXED,
                Vec3::ZERO,
                [0; 3],
                MoveState::Ground,
            );
        }
        return;
    }
    gliding.since_sent += dt;
    let due = gliding.since_sent >= SEND_INTERVAL || !gliding.sent_first;
    if due {
        let delta = [0, (input.forward * 127.0).round() as i8, 0];
        send_move(
            &mut out,
            &mut gliding,
            pos,
            mover.facing,
            true,
            stance::FLY,
            velocity,
            delta,
            MoveState::Fall,
        );
    }
    gliding.active = Some(active);
}

/// Writes one movement and resets the flight's send timer.
fn send_command(out: &mut MessageWriter<NetCommand>, gliding: &mut Gliding, command: MoveCommand) {
    out.write(NetCommand(Command::Move(command)));
    gliding.since_sent = 0.0;
    gliding.sent_first = true;
}

/// Writes one movement and resets the flight's send timer.
#[allow(clippy::too_many_arguments)]
fn send_move(
    out: &mut MessageWriter<NetCommand>,
    gliding: &mut Gliding,
    pos: Vec3,
    yaw: f32,
    moving: bool,
    stance: u8,
    velocity: Vec3,
    delta: [i8; 3],
    state: MoveState,
) {
    let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
    let cmd = MoveCommand {
        pos: Pos::new(pos.x, pos.y, pos.z),
        yaw,
        moving,
        stance,
        velocity: [
            to_wire(velocity.x),
            to_wire(velocity.y),
            to_wire(velocity.z),
        ],
        delta_movement: delta,
        ..MoveCommand::default()
    }
    .with_state(state);
    send_command(out, gliding, cmd);
}

/// Plays the gliding clips on the local character and gives the animator back to the locomotion afterwards.
#[allow(clippy::type_complexity)]
pub fn drive_glide(
    gliding: Res<Gliding>,
    library: Res<ClipLibrary>,
    mut commands: Commands,
    mut player: Query<
        (
            Entity,
            &mut Animator,
            &mut Locomotion,
            Option<&mut GlideClips>,
            Option<&super::combat::Acting>,
            Has<Dead>,
        ),
        With<LocalPlayer>,
    >,
) {
    let Ok((entity, mut animator, mut loco, clips, acting, dead)) = player.single_mut() else {
        return;
    };
    let Some(mut clips) = clips else {
        commands.entity(entity).insert(GlideClips::default());
        return;
    };
    let pose = gliding
        .active
        .map(|active| active.pose)
        .or_else(|| (gliding.ending > 0.0).then_some(Pose::End));
    let Some(pose) = pose else {
        if clips.was {
            clips.was = false;
            clips.playing = None;
            clips.end_played = false;
            loco.invalidate();
        }
        return;
    };
    clips.was = true;
    if dead || acting.is_some_and(|a| !a.layered) {
        return;
    }
    if clips.playing == Some(pose) && pose != Pose::End {
        return;
    }
    if pose == Pose::End && clips.end_played {
        return;
    }
    let (name, looped) = pose.character_clip();
    let clip = clips
        .bound
        .entry(name)
        .or_insert_with(|| bind_clip(&animator.rig, &loco.clips.cal, &library, name))
        .clone();
    let clip = clip.or_else(|| {
        let (fallback, _) = Pose::Idle.character_clip();
        clips
            .bound
            .entry(fallback)
            .or_insert_with(|| bind_clip(&animator.rig, &loco.clips.cal, &library, fallback))
            .clone()
    });
    if let Some(clip) = clip {
        let fade = if matches!(pose, Pose::Start | Pose::End) {
            0.1
        } else {
            0.25
        };
        animator.play(clip, looped, fade);
        animator.random_idle = None;
    }
    clips.playing = Some(pose);
    if pose == Pose::End {
        clips.end_played = true;
    }
}

/// Spawns the opened glider while the local player flies and hides the stowed one; removes it afterwards.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn show_glider(
    gliding: Res<Gliding>,
    inventory: Option<Res<Inventory>>,
    db: Option<Res<Db>>,
    library: Res<ClipLibrary>,
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    player: Query<Entity, With<LocalPlayer>>,
    mut models: Query<
        (Entity, &mut Animator, &mut GliderPlay),
        (With<GliderModel>, Without<RemoteGliderModel>),
    >,
    children: Query<&Children>,
    mut stowed: Query<&mut Visibility, With<StowedPack>>,
) {
    let shown = gliding.shown();
    let Ok(owner) = player.single() else { return };
    for descendant in children.iter_descendants(owner) {
        if let Ok(mut visibility) = stowed.get_mut(descendant) {
            let wanted = if shown {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
    let have = !models.is_empty();
    if !shown {
        for (entity, ..) in &models {
            commands.entity(entity).try_despawn();
        }
        return;
    }
    let pose = gliding
        .active
        .map(|active| active.pose)
        .unwrap_or(Pose::End);
    if !have {
        let model = inventory
            .as_deref()
            .and_then(|i| i.equipment.get(slot::BACKPACK))
            .map(|item| item.template_id)
            .and_then(|template| db.as_deref()?.glider(template))
            .map(|glider| glider.model);
        let Some(path) = model else { return };
        let Some(root) = super::creature::spawn(
            &mut commands,
            &mut cry,
            &library,
            &path,
            None,
            Transform::IDENTITY,
        ) else {
            debug!("glide: cannot draw the glider {path}");
            return;
        };
        commands
            .entity(root)
            .remove::<Locomotion>()
            .remove::<Motion>();
        commands.entity(root).insert((
            GliderModel,
            GliderPlay {
                cal: glider_cal(&path),
                playing: None,
                failed: None,
            },
            ChildOf(owner),
        ));
        return;
    }
    for (_, mut animator, mut play) in &mut models {
        if play.playing == Some(pose) {
            continue;
        }
        let (name, looped) = pose.glider_clip();
        if let Some(clip) = play
            .cal
            .as_ref()
            .and_then(|cal| bind_clip(&animator.rig, cal, &library, name))
        {
            animator.play(clip, looped, 0.15);
            animator.random_idle = None;
        }
        play.playing = Some(pose);
    }
}

/// The `.cal` of a glider model (`.chr`, or the `.cdf` that names one).
fn glider_cal(path: &str) -> Option<Arc<Cal>> {
    let chr = if path.ends_with(".cdf") {
        aa_anim::Cdf::load(path).ok()?.model
    } else {
        path.to_string()
    };
    Cal::shared(&format!("{}.cal", chr.trim_end_matches(".chr"))).ok()
}

/// The pose of another player's glider from the movement it reports (its keys are not on the wire).
pub fn remote_pose(params: &GlideParams, age: f32, speed: f32) -> Pose {
    if age < params.startup_time {
        return Pose::Start;
    }
    if speed > (params.move_normal + params.move_fast) * 0.5 {
        Pose::Fast
    } else if speed < (params.move_normal + params.move_slow) * 0.5 {
        Pose::Slow
    } else {
        Pose::Idle
    }
}

/// Follows the buff lists of the drawn characters: one under a gliding buff gets a [`RemoteGlide`], its [`Motion`] is
/// emptied and its stowed glider hidden; the buff's removal puts everything back.
#[allow(clippy::type_complexity)]
pub fn track_remote_gliders(
    time: Res<Time>,
    buffs: Option<Res<Buffs>>,
    mut gliding: ResMut<Gliding>,
    mut commands: Commands,
    children: Query<&Children>,
    mut packs: Query<&mut Visibility, With<StowedPack>>,
    mut units: Query<(
        Entity,
        &super::units::RemoteUnit,
        &mut Motion,
        Option<&mut Locomotion>,
        Option<&mut RemoteGlide>,
    )>,
) {
    let Some(buffs) = buffs else { return };
    let dt = time.delta_secs().min(0.1);
    for (entity, unit, mut motion, loco, glide) in &mut units {
        if !unit.is_character() {
            continue;
        }
        let found = {
            let g = &mut *gliding;
            let table = g.params.get_or_insert_with(GlideParams::load_all);
            buffs
                .of(unit.obj_id)
                .iter()
                .find(|entry| table.contains_key(&entry.buff.buff_id))
                .map(|entry| (entry.buff.buff_id, table[&entry.buff.buff_id]))
        };
        let mut set_packs = |flying: bool| {
            for descendant in children.iter_descendants(entity) {
                if let Ok(mut visibility) = packs.get_mut(descendant) {
                    let wanted = if flying {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                    if *visibility != wanted {
                        *visibility = wanted;
                    }
                }
            }
        };
        match (found, glide) {
            (Some((buff_id, params)), None) => {
                commands.entity(entity).insert(RemoteGlide {
                    buff_id,
                    params,
                    age: 0.0,
                    speed: unit.speed(),
                    pose: Pose::Start,
                    glider: None,
                    character_playing: None,
                    bound: HashMap::new(),
                });
            }
            (Some((buff_id, params)), Some(mut glide)) => {
                glide.buff_id = buff_id;
                glide.params = params;
                glide.age += dt;
                glide.speed = unit.speed();
                glide.pose = remote_pose(&params, glide.age, glide.speed);
                *motion = Motion::default();
                set_packs(true);
            }
            (None, Some(glide)) => {
                if let Some(glider) = glide.glider.filter(|glider| *glider != Entity::PLACEHOLDER) {
                    commands.entity(glider).try_despawn();
                }
                commands.entity(entity).remove::<RemoteGlide>();
                if let Some(mut loco) = loco {
                    loco.invalidate();
                }
                set_packs(false);
            }
            (None, None) => {}
        }
    }
}

/// Draws the glider of another player's character and plays the gliding clips on both.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn draw_remote_gliders(
    library: Res<ClipLibrary>,
    units: Option<Res<Units>>,
    db: Option<Res<Db>>,
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    mut flyers: Query<
        (
            Entity,
            &super::units::RemoteUnit,
            &mut Animator,
            Option<&Locomotion>,
            &mut RemoteGlide,
        ),
        (Without<GliderModel>, Without<RemoteGliderModel>),
    >,
    mut gliders: Query<(&mut Animator, &mut GliderPlay), With<RemoteGliderModel>>,
) {
    for (entity, unit, mut animator, loco, mut glide) in &mut flyers {
        if glide.glider.is_none() {
            let model = units
                .as_deref()
                .and_then(|units| units.by_id.get(&unit.obj_id))
                .and_then(|unit| unit.equipment.get(slot::BACKPACK))
                .map(|item| item.template_id)
                .and_then(|template| db.as_deref()?.glider(template))
                .map(|glider| glider.model);
            if let Some(path) = model {
                if let Some(root) = super::creature::spawn(
                    &mut commands,
                    &mut cry,
                    &library,
                    &path,
                    None,
                    Transform::IDENTITY,
                ) {
                    commands
                        .entity(root)
                        .remove::<Locomotion>()
                        .remove::<Motion>();
                    commands.entity(root).insert((
                        GliderModel,
                        RemoteGliderModel,
                        GliderPlay {
                            cal: glider_cal(&path),
                            playing: None,
                            failed: None,
                        },
                        ChildOf(entity),
                    ));
                    glide.glider = Some(root);
                } else {
                    // no model this frame: do not look it up again every frame
                    glide.glider = Some(Entity::PLACEHOLDER);
                }
            } else {
                glide.glider = Some(Entity::PLACEHOLDER);
            }
        }
        let pose = glide.pose;
        if glide.character_playing != Some(pose) {
            if let Some(loco) = loco {
                let (name, looped) = pose.character_clip();
                let g = &mut *glide;
                let clip = g
                    .bound
                    .entry(name)
                    .or_insert_with(|| bind_clip(&animator.rig, &loco.clips.cal, &library, name))
                    .clone();
                if let Some(clip) = clip {
                    animator.play(clip, looped, 0.25);
                    animator.random_idle = None;
                }
            }
            glide.character_playing = Some(pose);
        }
        // the model is spawned through Commands, so it exists a frame later: keep trying its clip until it has
        // one, rather than recording the pose and never opening it (the frame that skipped it)
        if let Some(glider) = glide.glider
            && let Ok((mut glider_animator, mut play)) = gliders.get_mut(glider)
            && play.playing != Some(pose)
            && play.failed != Some(pose)
            && let Some(cal) = play.cal.as_ref()
        {
            let (name, looped) = pose.glider_clip();
            if let Some(clip) = bind_clip(&glider_animator.rig, cal, &library, name) {
                glider_animator.play(clip, looped, 0.15);
                glider_animator.random_idle = None;
                play.playing = Some(pose);
            } else {
                // a model without that clip: remember the failure for this pose, try the next pose only
                play.failed = Some(pose);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(flight: &mut Flight, seconds: f32, input: Input, lift_ray_hit: bool) -> Step {
        let dt = 1.0 / 60.0;
        let mut last = flight.step(dt, input, lift_ray_hit);
        for _ in 1..(seconds / dt) as usize {
            last = flight.step(dt, input, lift_ray_hit);
        }
        last
    }

    #[test]
    fn the_experimental_glider_rows_match_the_database() {
        let params = GlideParams::EXPERIMENTAL;
        assert_eq!(
            (params.move_slow, params.move_normal, params.move_fast),
            (5.0, 6.0, 7.0)
        );
        assert_eq!(
            (params.fall_slow, params.fall_normal, params.fall_fast),
            (1.0, 1.5, 2.5)
        );
        assert!((params.rotate_speed.to_degrees() - 30.0).abs() < 1e-4);
        assert_eq!((params.startup_time, params.startup_speed), (0.7, 3.0));
        assert_eq!(
            (
                params.lift_count,
                params.lift_height,
                params.lift_valid_time
            ),
            (2, 1.5, 5.0)
        );
        assert_eq!((params.lift_duration, params.lift_speed), (0.5, 3.0));
        assert_eq!((params.land_height, params.sliding_time), (1.0, 1.0));
    }

    #[test]
    fn the_live_database_has_the_experimental_glider_buff() {
        let all = GlideParams::load_all();
        if all.is_empty() {
            return; // no AA_DB: the fixture above is the data
        }
        assert_eq!(all.len(), 55, "the 1.2 database's gliding buffs");
        assert_eq!(all.get(&1029), Some(&GlideParams::EXPERIMENTAL));
        assert_eq!(all[&3583].move_normal, 7.0);
        assert!((all[&3583].rotate_speed.to_degrees() - 90.0).abs() < 1e-4);
    }

    #[test]
    fn the_opening_holds_the_normal_speed_and_settles_on_the_normal_sink() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        let step = run(&mut flight, 0.5, Input::default(), false);
        assert_eq!(step.phase, Phase::Open);
        assert!(
            (step.speed - 6.0).abs() < 1e-6,
            "the opening moves at the normal speed: {}",
            step.speed
        );
        assert!(
            (step.vz - 3.0).abs() < 1e-6,
            "the opening climbs at startup_speed: {} (FUN_3912d860's double sign flip)",
            step.vz
        );
        let step = run(&mut flight, 4.0, Input::default(), false);
        assert_eq!(step.phase, Phase::Gliding);
        assert!(
            (step.speed - 6.0).abs() < 1e-6,
            "normal speed {}",
            step.speed
        );
        assert!(
            (step.vz + 1.5).abs() < 0.05,
            "normal sink {} (data: 1.5 m/s)",
            step.vz
        );
    }

    #[test]
    fn w_flies_fast_and_s_slow_with_the_data_fall_speeds() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        let fast = run(
            &mut flight,
            6.0,
            Input {
                forward: 1.0,
                ..Input::default()
            },
            false,
        );
        assert!(
            (fast.speed - 7.0).abs() < 1e-6 && (fast.vz + 2.5).abs() < 0.02,
            "{fast:?}"
        );
        let slow = run(
            &mut flight,
            6.0,
            Input {
                forward: -1.0,
                ..Input::default()
            },
            false,
        );
        assert!(
            (slow.speed - 5.0).abs() < 1e-6 && (slow.vz + 1.0).abs() < 0.02,
            "{slow:?}"
        );
    }

    #[test]
    fn the_turn_rate_is_the_buffs_degrees_a_second() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        run(&mut flight, 2.0, Input::default(), false);
        let left = flight.step(
            1.0 / 60.0,
            Input {
                turn: 1.0,
                ..Input::default()
            },
            false,
        );
        let right = flight.step(
            1.0 / 60.0,
            Input {
                turn: -1.0,
                ..Input::default()
            },
            false,
        );
        assert!((left.turn_rate.to_degrees() - 30.0).abs() < 1e-4);
        assert!((right.turn_rate.to_degrees() + 30.0).abs() < 1e-4);
    }

    #[test]
    fn a_lift_needs_the_ray_and_uses_count_times_duration() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        // before lift_valid_time (5 s) nothing happens
        let early = flight.step(
            1.0 / 60.0,
            Input {
                forward: -1.0,
                ..Input::default()
            },
            true,
        );
        assert!(!early.lifting);
        run(&mut flight, 6.0, Input::default(), false);
        let started = flight.step(
            1.0 / 60.0,
            Input {
                forward: -1.0,
                ..Input::default()
            },
            true,
        );
        assert!(started.lifting);
        // count (2) * normalized duration (0.25) = the row's 0.5 s for the first lift
        assert!(
            (flight.lift_left - 0.5).abs() < 1e-5,
            "{}",
            flight.lift_left
        );
        assert_eq!(flight.lift_count, 1);
        let mut raised = 0.0;
        for _ in 0..40 {
            let step = flight.step(1.0 / 60.0, Input::default(), true);
            if step.vz > 0.0 {
                raised += step.vz / 60.0;
            }
        }
        assert!(raised > 0.1, "the first lift rises: {raised} m");
        // without the ray the next press does nothing
        run(&mut flight, 1.0, Input::default(), false);
        let no_ray = flight.step(
            1.0 / 60.0,
            Input {
                forward: -1.0,
                ..Input::default()
            },
            false,
        );
        assert!(!no_ray.lifting);
        assert_eq!(flight.lift_count, 1);
    }

    #[test]
    fn the_stop_probe_slides_near_the_ground_and_otherwise_folds() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        run(&mut flight, 2.0, Input::default(), false);
        assert_eq!(flight.stop(-1.5, Some(1.2)), Phase::Sliding);
        assert!((flight.slide_left - 1.0).abs() < 1e-6);
        // stop is idempotent: a second probe must not restart the slide timer
        flight.slide(1.0 / 60.0);
        let left = flight.slide_left;
        assert_eq!(flight.stop(-1.5, Some(1.2)), Phase::Sliding);
        assert!((flight.slide_left - left).abs() < 1e-6);
        let mut finished = false;
        for _ in 0..80 {
            finished = flight.slide(1.0 / 60.0);
        }
        assert!(finished && flight.phase == Phase::Done);
        assert_eq!(flight.stop(-1.5, Some(1.2)), Phase::Done);
        let mut high = Flight::start(GlideParams::EXPERIMENTAL);
        run(&mut high, 2.0, Input::default(), false);
        assert_eq!(high.stop(-1.5, Some(1.21)), Phase::Done);
        assert_eq!(high.stop(-1.5, Some(1.21)), Phase::Done);
    }

    #[test]
    fn the_smoothing_filter_matches_the_decompile_step() {
        // smooth_time 1.0, dt 0.1 -> x = 0.2; den = 1 + 0.2 + 0.48*0.04 + 0.235*0.008
        let den = 1.0 + 0.2 + SMOOTH_X2 * 0.04 + SMOOTH_X3 * 0.008;
        let mut flight = Flight::start(GlideParams {
            smooth_time: 1.0,
            ..GlideParams::EXPERIMENTAL
        });
        flight.smoothed_v = 3.0;
        flight.smoothed_dv = 0.0;
        flight.elapsed = 10.0; // past the opening: the target is -1.5
        flight.step(0.1, Input::default(), false);
        let inv = 2.0;
        let acc = ((-1.5f32) - 3.0) * 0.0 + (3.0 - (-1.5)) * inv * 0.1; // (delta*inv + dv) * dt
        let expected = (acc + (3.0 - (-1.5))) / den - 1.5;
        assert!(
            (flight.smoothed_v - expected).abs() < 1e-4,
            "{} vs {}",
            flight.smoothed_v,
            expected
        );
    }

    #[test]
    fn the_pose_follows_the_graph_actions_and_turn_speed() {
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        assert_eq!(pose(&flight, Input::default()), Pose::Start);
        run(&mut flight, 1.0, Input::default(), false);
        assert_eq!(pose(&flight, Input::default()), Pose::Idle);
        assert_eq!(
            pose(
                &flight,
                Input {
                    forward: 1.0,
                    ..Input::default()
                }
            ),
            Pose::Fast
        );
        assert_eq!(
            pose(
                &flight,
                Input {
                    forward: -1.0,
                    ..Input::default()
                }
            ),
            Pose::Slow
        );
        assert_eq!(
            pose(
                &flight,
                Input {
                    turn: 1.0,
                    ..Input::default()
                }
            ),
            Pose::Left
        );
        assert_eq!(
            pose(
                &flight,
                Input {
                    forward: 1.0,
                    turn: -1.0,
                    ..Input::default()
                }
            ),
            Pose::FrontRight
        );
        assert_eq!(flight.stop(-1.0, Some(0.0)), Phase::Sliding);
        assert_eq!(pose(&flight, Input::default()), Pose::Slide);
        // `FUN_3911d6c0(1)` opens the item model with "loop"; the character's start is separate.
        assert_eq!(
            Pose::Start.character_clip(),
            ("fist_ac_gliding_start", false)
        );
        assert_eq!(Pose::Start.glider_clip(), ("loop", true));
    }

    #[test]
    fn the_quest_glider_item_is_a_backpack_type_2_glider() {
        let Some(db) = Db::open() else { return };
        let glider = db.glider(14677).expect("item 14677 (Experimental Glider)");
        assert_eq!(glider.use_skill, 12373);
        assert!(glider.model.ends_with("glider_bx.chr"), "{}", glider.model);
        assert!(
            db.backpack(14677)
                .is_some_and(|stowed| stowed.path.ends_with("glider_b_back.cgf"))
        );
        assert!(db.glider(5660).is_none(), "a sword is not a glider");
    }

    #[test]
    fn the_buff_tracking_starts_the_flight_from_the_gliding_buff() {
        if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
            return;
        }
        use std::time::Duration;
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Gliding>()
            .init_resource::<SendState>()
            .init_resource::<Buffs>()
            .init_resource::<crate::ingame::units::Views>()
            .insert_resource(TerrainQuery::new("__glide_test__"))
            .insert_resource(WorldCollision::new(
                "__glide_test__",
                TerrainQuery::new("__glide_test__"),
            ))
            .add_systems(Update, track_flight);
        app.world_mut().spawn((
            LocalPlayer { obj_id: 7 },
            Mover {
                pos: Vec3::new(0.0, 0.0, 50.0),
                facing: 0.0,
                vz: -1.0,
                grounded: false,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
        ));
        app.world_mut().resource_mut::<Buffs>().by_unit.insert(
            7,
            vec![aa_net::state::BuffEntry {
                buff: aa_net::buff::Buff {
                    index: 3,
                    buff_id: 1029,
                    duration_ms: 60_000,
                    ..Default::default()
                },
                received: web_time::Instant::now(),
            }],
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.1));
        app.update();
        let gliding = app.world().resource::<Gliding>();
        let active = gliding.active.expect("the buff starts the flight");
        assert_eq!((active.buff_id, active.buff_index), (1029, 3));
        assert_eq!(active.flight.params, GlideParams::EXPERIMENTAL);
    }

    #[test]
    fn the_flight_system_sends_a_type_1_gliding_movement() {
        if !aa_cry::pak_available() {
            return;
        }
        use std::time::Duration;
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<crate::settings::Actions>()
            .init_resource::<PlayerInput>()
            .init_resource::<crate::ingame::units::Views>()
            .insert_resource(DevOptions::from_args())
            .insert_resource(TerrainQuery::new("__glide_test__"))
            .insert_resource(WorldCollision::new(
                "__glide_test__",
                TerrainQuery::new("__glide_test__"),
            ))
            .insert_resource(WaterQuery::new("__glide_test__"))
            .insert_resource(Gliding {
                active: Some(Active {
                    flight: Flight::start(GlideParams::EXPERIMENTAL),
                    buff_id: 1029,
                    buff_index: 3,
                    pose: Pose::Start,
                    touched: false,
                    touched_for: 0.0,
                    missing: 0.0,
                    deck: None,
                }),
                ..Default::default()
            })
            .init_resource::<SendState>()
            .add_message::<NetCommand>()
            .add_systems(Update, control_glide);
        app.world_mut().spawn((
            LocalPlayer { obj_id: 7 },
            Mover {
                pos: Vec3::new(0.0, 0.0, 50.0),
                facing: 0.0,
                vz: 0.0,
                grounded: false,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            Motion::default(),
        ));
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.1));
        app.update();
        let commands: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|m| m.0)
            .collect();
        let Some(Command::Move(command)) = commands.first() else {
            panic!("no movement: {commands:?}")
        };
        assert_eq!(command.stance, stance::FLY);
        assert_eq!(command.state, MoveState::Fall);
        assert!(command.moving);
        assert!(command.pos.z > 0.0);
        assert!(app.world().resource::<SendState>().glide_owns);
    }

    #[test]
    fn the_double_jump_gate_is_off_by_default_and_needs_height() {
        assert!(!double_jump_on(None));
        assert!(double_jump_on(Some(1.0)));
        assert!(!may_deploy(false, true, false, true, 50.0, false));
        assert!(may_deploy(true, true, false, true, 1.0, false));
        assert!(may_deploy(true, true, false, false, 3.0, false));
        assert!(!may_deploy(true, true, false, false, 2.9, false));
        assert!(!may_deploy(true, false, false, true, 50.0, false));
        assert!(!may_deploy(true, true, true, true, 50.0, false));
        assert!(!may_deploy(true, true, false, true, 50.0, true));
    }
    fn tick(app: &mut App, dt: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(dt));
        app.update();
    }

    /// The systems and resources `control_glide` needs; the tests add the deck or terrain they exercise.
    fn glide_app(active: Active, pos: Vec3) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<crate::settings::Actions>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<PlayerInput>()
            .init_resource::<SendState>()
            .init_resource::<crate::ingame::LocalJump>()
            .init_resource::<crate::ingame::LocalMount>()
            .init_resource::<crate::ingame::units::Views>()
            .insert_resource(DevOptions::from_args())
            .insert_resource(TerrainQuery::new("__glide_test__"))
            .insert_resource(WorldCollision::new(
                "__glide_test__",
                TerrainQuery::new("__glide_test__"),
            ))
            .insert_resource(WaterQuery::new("__glide_test__"))
            .insert_resource(Gliding {
                active: Some(active),
                ..Default::default()
            })
            .add_message::<NetCommand>()
            .add_systems(Update, (control_glide, player::control_player).chain());
        app.world_mut().resource_mut::<SendState>().settle_now();
        app.world_mut().spawn(OrbitCamera::default());
        let player = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 7 },
                Mover {
                    pos,
                    facing: 0.0,
                    vz: -1.5,
                    grounded: false,
                    velocity: Vec3::ZERO,
                    swim: None,
                    wading: 0.0,
                },
                Transform::default(),
                Motion::default(),
            ))
            .id();
        (app, player)
    }

    fn ship_unit(obj_id: u32, model_id: u32, root: Vec3, yaw: f32) -> aa_net::model::Unit {
        let mut movement =
            aa_net::model::Movement::unit(aa_net::Pos::new(root.x, root.y, root.z), yaw, 0);
        movement.move_type = 3;
        aa_net::model::Unit {
            obj_id,
            name: "Hull".into(),
            kind: aa_net::model::UnitKind::Slave {
                id: 1,
                tl_id: 2,
                template_id: 15,
                summoner_id: 3,
            },
            owner_name: String::new(),
            pos: aa_net::Pos::new(root.x, root.y, root.z),
            rotation: [0.0, 0.0, yaw],
            scale: 1.0,
            level: 1,
            model_id,
            equipment: Default::default(),
            model_params: Default::default(),
            hp: 100,
            mp: 0,
            max_hp: 100,
            max_mp: 0,
            dead: false,
            race_gender: 0,
            movement: Some(movement),
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
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        }
    }

    #[test]
    fn the_slide_branch_sets_the_slide_pose() {
        if !aa_cry::pak_available() {
            return;
        }
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        assert_eq!(flight.stop(-1.0, Some(0.0)), Phase::Sliding);
        let active = Active {
            flight,
            buff_id: 1029,
            buff_index: 1,
            pose: Pose::Idle,
            touched: false,
            touched_for: 0.0,
            missing: 0.0,
            deck: None,
        };
        let (mut app, _player) = glide_app(active, Vec3::new(0.0, 0.0, 50.0));
        tick(&mut app, 1.0 / 60.0);
        assert_eq!(
            app.world().resource::<Gliding>().active.map(|a| a.pose),
            Some(Pose::Slide),
            "the slide branch must switch the character and glider to the slide pose"
        );
    }

    #[test]
    fn a_moving_hull_keeps_the_glider_landing_in_the_ship_frame() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::deck::world_to_ship;
        use super::super::ship::floor::ShipFloor;
        const SHIP: u32 = 8911;
        const MODEL: u32 = 129;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -10.0, 1.0),
                Vec3::new(2.0, -10.0, 1.0),
                Vec3::new(2.0, 10.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -10.0, 1.0),
                Vec3::new(2.0, 10.0, 1.0),
                Vec3::new(-2.0, 10.0, 1.0),
            ],
        ]));
        let Some(db) = Db::ship_floor_fixture(MODEL, floor) else {
            return;
        };
        let root = Vec3::new(100.0, 200.0, 10.0);
        let mut units = Units::default();
        units.by_id.insert(SHIP, ship_unit(SHIP, MODEL, root, 0.0));
        let contact = DeckContact {
            ship: SHIP,
            local: Vec3::new(0.0, -7.2, 1.0),
            root,
            yaw: 0.0,
            facing: 0.0,
        };
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        flight.elapsed = 10.0;
        flight.smoothed_v = -1.5;
        flight.smoothed_dv = 0.0;
        flight.vz = -1.5;
        let active = Active {
            flight,
            buff_id: 1029,
            buff_index: 1,
            pose: Pose::Idle,
            touched: false,
            touched_for: 0.0,
            missing: 0.0,
            deck: None,
        };
        let (mut app, player) = glide_app(active, contact.world() + Vec3::Z * 0.1);
        app.insert_resource(db);
        app.insert_resource(units);
        tick(&mut app, 0.1);
        assert!(
            app.world().get::<DeckRide>(player).is_some(),
            "the touchdown must board the deck"
        );
        let contact = app
            .world()
            .resource::<Gliding>()
            .active
            .and_then(|active| active.deck)
            .expect("the touchdown stores the hull contact");
        assert!(
            (contact.local - Vec3::new(0.0, -6.6, 1.0)).length() < 0.05,
            "the touchdown point was {:?}",
            contact.local
        );
        {
            let mut gliding = app.world_mut().resource_mut::<Gliding>();
            let active = gliding.active.as_mut().unwrap();
            assert_eq!(active.flight.stop(-1.0, Some(0.0)), Phase::Sliding);
            active.touched = true;
        }
        let start = app.world().get::<Mover>(player).unwrap().pos;
        for i in 0..11 {
            if i < 10 {
                app.world_mut()
                    .resource_mut::<Units>()
                    .by_id
                    .get_mut(&SHIP)
                    .unwrap()
                    .pos
                    .y += 0.5;
            }
            tick(&mut app, 0.1);
            if let Some(active) = app.world().resource::<Gliding>().active {
                assert_eq!(active.pose, Pose::Slide);
            }
        }
        let world = app.world().get::<Mover>(player).unwrap().pos;
        let moved = Vec3::new(100.0, 205.0, 10.0);
        let local = world_to_ship(moved, 0.0, world);
        let ride = app
            .world()
            .get::<DeckRide>(player)
            .expect("the slide end must leave the deck ride to deck_control");
        assert!(
            ride.local().y > contact.local.y + 4.0,
            "the slide did not walk the ship-local point: {:?} from {:?}",
            ride.local(),
            contact.local
        );
        assert!(
            (local - ride.local()).length() < 1e-3,
            "the ride and the world position disagree: {local:?} vs {:?}",
            ride.local()
        );
        assert!(
            (world - start).length() >= 4.99,
            "the player did not follow the hull's 5 m: {world:?} from {start:?}"
        );
    }

    #[test]
    fn the_deferred_remote_glider_starts_its_clip_once_the_model_exists() {
        if !aa_cry::pak_available() {
            return;
        }
        let model = "objects/env/06_unit/06_glider/glider_bx.chr";
        let Some(cal) = glider_cal(model) else {
            return;
        };
        let Ok(rig) = aa_anim::pose::shared_rig(model) else {
            return;
        };
        let mut app = App::new();
        app.init_resource::<ClipLibrary>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .init_resource::<aa_cry::scene::CryCache>()
            .add_systems(Update, draw_remote_gliders);
        let glider = app
            .world_mut()
            .spawn((
                GliderPlay {
                    cal: Some(cal),
                    playing: None,
                    failed: None,
                },
                RemoteGliderModel,
                Animator::new(rig.clone(), Vec::new()),
            ))
            .id();
        app.world_mut().spawn((
            crate::ingame::units::RemoteUnit::fixture(7, Vec3::ZERO),
            Animator::new(
                Arc::new(aa_anim::pose::Rig::new(aa_cry::Skeleton::default())),
                Vec::new(),
            ),
            RemoteGlide {
                buff_id: 1029,
                params: GlideParams::EXPERIMENTAL,
                age: 1.0,
                speed: 0.0,
                pose: Pose::Start,
                glider: Some(glider),
                // the bug's state: the character clip was recorded while the model was still queued
                character_playing: Some(Pose::Start),
                bound: HashMap::new(),
            },
        ));
        app.update();
        assert_eq!(
            app.world().get::<GliderPlay>(glider).unwrap().playing,
            Some(Pose::Start),
            "the glider clip must start on the frame its entity exists, even after the character clip"
        );
    }
    /// A glider already in its slide on a synthetic hull, with the deck ride the touchdown would have inserted.
    fn deck_slide_app(
        floor: Arc<super::super::ship::floor::ShipFloor>,
        local: Vec3,
        root: Vec3,
    ) -> Option<(App, Entity, DeckContact)> {
        const SHIP: u32 = 8911;
        const MODEL: u32 = 129;
        let db = Db::ship_floor_fixture(MODEL, floor)?;
        let mut units = Units::default();
        units.by_id.insert(SHIP, ship_unit(SHIP, MODEL, root, 0.0));
        let contact = DeckContact {
            ship: SHIP,
            local,
            root,
            yaw: 0.0,
            facing: 0.0,
        };
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        flight.elapsed = 10.0;
        flight.smoothed_v = -1.5;
        flight.smoothed_dv = 0.0;
        flight.vz = -1.5;
        assert_eq!(flight.stop(-1.0, Some(0.0)), Phase::Sliding);
        let active = Active {
            flight,
            buff_id: 1029,
            buff_index: 1,
            pose: Pose::Slide,
            touched: false,
            touched_for: 0.0,
            missing: 0.0,
            deck: Some(contact),
        };
        let (mut app, player) = glide_app(active, contact.world());
        app.insert_resource(db);
        app.insert_resource(units);
        app.world_mut().entity_mut(player).insert(DeckRide::new(
            contact.ship,
            contact.local,
            0.0,
            contact.yaw,
        ));
        Some((app, player, contact))
    }

    #[test]
    fn a_deck_slide_off_the_edge_falls_instead_of_standing_in_the_air() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::floor::ShipFloor;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -2.0, 1.0),
                Vec3::new(2.0, -2.0, 1.0),
                Vec3::new(2.0, 2.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -2.0, 1.0),
                Vec3::new(2.0, 2.0, 1.0),
                Vec3::new(-2.0, 2.0, 1.0),
            ],
        ]));
        let Some((mut app, player, _contact)) = deck_slide_app(
            floor,
            Vec3::new(0.0, 1.5, 1.0),
            Vec3::new(100.0, 200.0, 10.0),
        ) else {
            return;
        };
        for _ in 0..20 {
            tick(&mut app, 0.1);
            if app.world().resource::<Gliding>().active.is_none() {
                break;
            }
        }
        assert!(
            app.world().resource::<Gliding>().active.is_none(),
            "the slide off the edge did not end"
        );
        assert!(
            app.world().get::<DeckRide>(player).is_none(),
            "the edge fall kept the deck ride"
        );
        assert!(
            !app.world().get::<Mover>(player).unwrap().grounded,
            "the edge fall left the player standing in the air"
        );
    }

    #[test]
    fn a_deck_slide_into_a_cabin_stops_at_the_wall() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::floor::ShipFloor;
        let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
            [
                [
                    Vec3::new(x0, y0, z),
                    Vec3::new(x1, y0, z),
                    Vec3::new(x1, y1, z),
                ],
                [
                    Vec3::new(x0, y0, z),
                    Vec3::new(x1, y1, z),
                    Vec3::new(x0, y1, z),
                ],
            ]
        };
        let floor = Arc::new(ShipFloor::new(
            quad(-2.0, 2.0, -3.0, 3.0, 1.0)
                .into_iter()
                .chain(quad(-2.0, 2.0, 1.5, 3.0, 3.0)),
        ));
        let Some((mut app, player, _contact)) = deck_slide_app(
            floor,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(100.0, 200.0, 10.0),
        ) else {
            return;
        };
        for _ in 0..20 {
            tick(&mut app, 0.1);
            if app.world().resource::<Gliding>().active.is_none() {
                break;
            }
        }
        assert!(
            app.world().resource::<Gliding>().active.is_none(),
            "the slide into the cabin did not stop"
        );
        let rider = app
            .world()
            .get::<DeckRide>(player)
            .expect("a blocked slide stays on the deck");
        assert!(
            rider.local().y < 1.5,
            "the blocked slide stepped into the riser: {:?}",
            rider.local()
        );
        assert!(
            app.world().get::<Mover>(player).unwrap().grounded,
            "the blocked slide left the deck"
        );
    }

    #[test]
    fn a_ship_that_disappears_during_the_slide_drops_the_glider() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::floor::ShipFloor;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
                Vec3::new(-2.0, 5.0, 1.0),
            ],
        ]));
        let Some((mut app, player, _contact)) = deck_slide_app(
            floor,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(100.0, 200.0, 10.0),
        ) else {
            return;
        };
        app.world_mut().resource_mut::<Units>().by_id.clear();
        tick(&mut app, 0.1);
        assert!(
            app.world().resource::<Gliding>().active.is_none(),
            "the glide stayed active after the ship disappeared"
        );
        assert!(
            app.world().get::<DeckRide>(player).is_none(),
            "the disappeared ship kept the deck ride"
        );
        assert!(
            !app.world().get::<Mover>(player).unwrap().grounded,
            "the disappeared ship left the player standing"
        );
    }

    #[test]
    fn a_ship_that_disappears_during_the_landing_wait_drops_the_glider() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::floor::ShipFloor;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
                Vec3::new(-2.0, 5.0, 1.0),
            ],
        ]));
        let Some((mut app, player, _contact)) = deck_slide_app(
            floor,
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(100.0, 200.0, 10.0),
        ) else {
            return;
        };
        {
            let mut gliding = app.world_mut().resource_mut::<Gliding>();
            let active = gliding.active.as_mut().unwrap();
            active.flight.phase = Phase::Gliding;
            active.touched = true;
        }
        app.world_mut().resource_mut::<Units>().by_id.clear();
        tick(&mut app, 0.1);
        assert!(
            app.world().resource::<Gliding>().active.is_none(),
            "the glide stayed active after the ship disappeared during the wait"
        );
        assert!(
            app.world().get::<DeckRide>(player).is_none(),
            "the disappeared ship kept the deck ride"
        );
        assert!(
            !app.world().get::<Mover>(player).unwrap().grounded,
            "the disappeared ship left the player standing"
        );
    }

    #[test]
    fn a_buff_lost_above_a_deck_starts_the_slide_on_the_hull() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::floor::ShipFloor;
        const SHIP: u32 = 8911;
        const MODEL: u32 = 129;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -5.0, 1.0),
                Vec3::new(2.0, 5.0, 1.0),
                Vec3::new(-2.0, 5.0, 1.0),
            ],
        ]));
        let Some(db) = Db::ship_floor_fixture(MODEL, floor) else {
            return;
        };
        let root = Vec3::new(100.0, 200.0, 10.0);
        let mut units = Units::default();
        units.by_id.insert(SHIP, ship_unit(SHIP, MODEL, root, 0.0));
        let contact = DeckContact {
            ship: SHIP,
            local: Vec3::new(0.0, 0.0, 1.0),
            root,
            yaw: 0.0,
            facing: 0.0,
        };
        let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
        flight.elapsed = 10.0;
        flight.smoothed_v = -1.5;
        flight.smoothed_dv = 0.0;
        flight.vz = -1.5;
        let active = Active {
            flight,
            buff_id: 1029,
            buff_index: 1,
            pose: Pose::Idle,
            touched: false,
            touched_for: 0.0,
            missing: 0.0,
            deck: None,
        };
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Buffs>()
            .init_resource::<SendState>()
            .init_resource::<crate::ingame::units::Views>()
            .insert_resource(TerrainQuery::new("__glide_test__"))
            .insert_resource(WorldCollision::new(
                "__glide_test__",
                TerrainQuery::new("__glide_test__"),
            ))
            .insert_resource(db)
            .insert_resource(units)
            .insert_resource(Gliding {
                active: Some(active),
                ..Default::default()
            })
            .add_systems(Update, track_flight);
        let player = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 7 },
                Mover {
                    pos: contact.world() + Vec3::Z,
                    facing: 0.0,
                    vz: -1.5,
                    grounded: false,
                    velocity: Vec3::ZERO,
                    swim: None,
                    wading: 0.0,
                },
            ))
            .id();
        for _ in 0..3 {
            tick(&mut app, 0.1);
        }
        let active = app
            .world()
            .resource::<Gliding>()
            .active
            .expect("the buff loss within reach of the deck starts the slide");
        assert_eq!(active.flight.phase, Phase::Sliding);
        assert_eq!(active.deck.map(|c| c.ship), Some(SHIP));
        assert!(
            app.world().get::<DeckRide>(player).is_some(),
            "the slide on the deck did not take the ride"
        );
    }
    #[test]
    fn a_hull_turn_carries_the_glider_slide_facing() {
        if !aa_cry::pak_available() {
            return;
        }
        use super::super::ship::deck::world_to_ship;
        use super::super::ship::floor::ShipFloor;
        const SHIP: u32 = 8911;
        let floor = Arc::new(ShipFloor::new([
            [
                Vec3::new(-2.0, -10.0, 1.0),
                Vec3::new(2.0, -10.0, 1.0),
                Vec3::new(2.0, 10.0, 1.0),
            ],
            [
                Vec3::new(-2.0, -10.0, 1.0),
                Vec3::new(2.0, 10.0, 1.0),
                Vec3::new(-2.0, 10.0, 1.0),
            ],
        ]));
        let root = Vec3::new(100.0, 200.0, 10.0);
        let Some((mut app, player, _contact)) =
            deck_slide_app(floor, Vec3::new(0.0, -6.0, 1.0), root)
        else {
            return;
        };
        let turn = 30f32.to_radians();
        for i in 0..11 {
            if i == 2 {
                app.world_mut()
                    .resource_mut::<Units>()
                    .by_id
                    .get_mut(&SHIP)
                    .unwrap()
                    .rotation[2] = turn;
            }
            tick(&mut app, 0.1);
        }
        let world = app.world().get::<Mover>(player).unwrap().pos;
        let local = world_to_ship(root, turn, world);
        let facing = app.world().get::<Mover>(player).unwrap().facing;
        assert!(
            local.x.abs() < 0.05,
            "the slide veered in the hull frame after the ship turned: {local:?}"
        );
        assert!(
            (facing - turn).abs() < 1e-3,
            "the player's facing did not turn with the hull: {facing} vs {turn}"
        );
    }
}
