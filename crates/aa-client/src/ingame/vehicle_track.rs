//! How other players' carts and ships move on screen, as the 1.2 client plays them back.
//!
//! The 1.2 vehicle receive path is not the living-unit one: `FUN_390b2f30` (ships) and
//! `FUN_390c63c0` (land vehicles) turn each `SCOneUnitMovement` into a sample and queue it by the
//! packet's own `time` (`FUN_390b2980` / `FUN_390b2840`). A playback clock runs the delay behind
//! the server (`FUN_390986a0` keeps an offset between the sender's clock and the local clock) and
//! the drawn pose is the interpolation of the two samples around it (`FUN_390ad150` ->
//! `FUN_390a16b0`: position lerp, quaternion nlerp). There is **no extrapolation**: past the
//! newest sample the last pose is held, and 5 s after the last sample the controller stops
//! (`FUN_390b2420`). The clock offset is corrected once per frame (`FUN_390ad150` calls
//! `FUN_39098780`: a 1 ms nudge inside the dead band, `max(1 ms, 0.2 * excess)` outside it).
//! Before sampling, the drawn entity is compared with the newest sample's position; a step over
//! 20 m is a snap, not a glide (`FUN_390b0040`, `DAT_399a3364` = 400.0, squared).
//!
//! Two rules are ship-only in the decompile and are kept here: a packet older than the newest
//! sample but less than 1 s older is dropped (`FUN_390b2f30`), and a zone change restarts the
//! sync.
//!
//! Constants read from the 1.2 image (`x2game_unpacked_data.dll`, base 0x38ff0000):
//! `_DAT_399a2eb8` = 20.0 (clock re-base threshold as a multiple of the delay), `_DAT_399a2ec0`
//! = 0.2 (correction fraction), `DAT_399a28a8` = 0.001 (the 1 ms nudge),
//! `DAT_399a3364` = 400.0. The playback delay itself comes from a runtime object
//! (`DAT_39ae4e90`, capped at 10000 ms in `NetActorMovementController::PrePhysicsUpdate`); the
//! only default in the image is 300 ms when that getter is null, which this port uses: S8-C1.

use std::collections::VecDeque;

use aa_net::model::Movement;
use bevy::math::{Quat, Vec3};

/// Playback runs this far behind the newest sample (s). **S8-C1**: the original reads the delay
/// from a runtime getter (`DAT_39ae4e90`, integer ms capped at 10000). The only default in the
/// image is the 300 ms fallback when the getter is null (`x2game.dll.c:756212`). Capture step:
/// read the getter in the hidden 1.2 client while a vehicle moves, or measure the drawn pose
/// against the packet timestamps on the A side of the two-client run.
pub const DELAY: f64 = 0.3;
/// A vehicle track older than this (s) stops updating; the drawn pose stays where it was
/// (`FUN_390b2420`: `5000 < now - last sample arrival`).
pub const TIMEOUT: f64 = 5.0;
/// A change of drawn position over this (m) is a teleport: snap instead of gliding.
/// **S8-C5**: the 20 m threshold is proven (`DAT_399a3364`); that the snap takes the newest
/// sample and re-bases the clock is inferred (capture: move a ship across more than 20 m in
/// view and record the drawn poses per frame).
pub const SNAP_DISTANCE: f32 = 20.0;
/// The clock may deviate this many delays before it is re-based (`_DAT_399a2eb8` = 20.0).
pub const REBASE_FACTOR: f64 = 20.0;
/// The per-frame nudge inside the dead band (`DAT_399a28a8` = 0.001, the ms-to-seconds factor).
pub const NUDGE: f64 = 0.001;
/// Fraction of the excess corrected per frame (`_DAT_399a2ec0` = 0.2, `FUN_39098780`).
pub const CORRECTION: f64 = 0.2;
/// The dead band around the target deviation (s). **S8-C2**: `FUN_39098780` takes it and the
/// minimum step from its caller (`DAT_39ae4e94` in the 1.2 review), whose runtime values are not
/// captured; the port uses this band and the 1 ms nudge as the minimum step. Capture step: read
/// the getter or record the drawn lag over a long drive and measure the band.
pub const DEAD_BAND: f64 = 0.01;
/// A ship packet older than the newest sample but less than this (ms) older is stale.
pub const STALE_WINDOW_MS: u32 = 1000;

/// One received vehicle / ship movement. `pos` and `rot` are game coordinates (z up), as the
/// packet carries them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehicleSample {
    /// The packet's `time`, ms. Only differences against the local clock count.
    pub time: u32,
    pub pos: Vec3,
    pub rot: Quat,
    /// Helm / wheel state, -1..=1 (a ship's `i8 / 127`, `DAT_399a28dc`).
    pub steering: f32,
    /// -1..=1 (a ship's `i8 / 127`).
    pub throttle: f32,
    /// Ship zone (`FUN_390b2f30` resets the sync when it changes); 0 for a land vehicle.
    pub zone: u16,
    /// A type-3 (ship) body: the stale packet and zone rules apply.
    pub ship: bool,
}

impl VehicleSample {
    /// The sample a movement packet carries, or `None` for the movement types that are not
    /// vehicles (0 / 1 / 4 / 5).
    pub fn from_movement(m: &Movement) -> Option<VehicleSample> {
        if !matches!(m.move_type, 2 | 3) {
            return None;
        }
        let rot = m.rotation_quat()?;
        let pos = Vec3::new(m.pos.x, m.pos.y, m.pos.z);
        let (steering, throttle, zone, ship) = match (&m.vehicle, &m.ship) {
            (Some(v), _) => (v.steering, 0.0, 0, false),
            (_, Some(s)) => (
                f32::from(s.steering) / 127.0,
                f32::from(s.throttle) / 127.0,
                s.zone_id,
                true,
            ),
            _ => return None,
        };
        Some(VehicleSample {
            time: m.time,
            pos,
            rot: Quat::from_array(rot),
            steering,
            throttle,
            zone,
            ship,
        })
    }
}

/// The pose the track draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VehiclePose {
    pub pos: Vec3,
    /// Game coordinates (z up).
    pub rot: Quat,
    pub steering: f32,
    pub throttle: f32,
}

impl VehiclePose {
    fn of(sample: &VehicleSample) -> VehiclePose {
        VehiclePose {
            pos: sample.pos,
            rot: sample.rot,
            steering: sample.steering,
            throttle: sample.throttle,
        }
    }
}

/// What `push` did, for the tests and the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pushed {
    Added,
    /// A ship packet older than the newest sample (by less than 1 s).
    Stale,
    /// The clock jumped; the queue was erased and the offset re-based.
    Rebased,
    /// A ship entered another zone; the queue was erased and the clock re-based.
    ZoneReset,
}

/// The buffered samples of one vehicle or ship and the playback clock.
#[derive(Clone, Debug)]
pub struct VehicleTrack {
    queue: VecDeque<VehicleSample>,
    /// The pose last applied to the drawing.
    display: VehiclePose,
    /// Local seconds + `offset` is the sender's clock; `None` until the first sample.
    offset: Option<f64>,
    /// `(server - local) - offset`, kept near [`DELAY`] by the nudge.
    deviation: f64,
    /// When the last sample arrived (app seconds), for the 5 s timeout.
    last_arrival: Option<f64>,
    /// The newest sample's `time`, for the ship stale check.
    last_time: Option<u32>,
    /// The last ship zone.
    zone: Option<u16>,
}

impl VehicleTrack {
    /// A vehicle that stands at `pos` facing the game rotation `rot` (the announced pose).
    pub fn new(pos: Vec3, rot: Quat) -> VehicleTrack {
        VehicleTrack {
            queue: VecDeque::new(),
            display: VehiclePose {
                pos,
                rot,
                steering: 0.0,
                throttle: 0.0,
            },
            offset: None,
            deviation: 0.0,
            last_arrival: None,
            last_time: None,
            zone: None,
        }
    }

    /// Where the newest accepted sample puts the vehicle.
    pub fn latest_pos(&self) -> Vec3 {
        self.queue
            .back()
            .map_or(self.display.pos, |sample| sample.pos)
    }

    /// Buffered samples (the tests check the queue).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// The clock offset and stored deviation (the tests check the per-frame correction).
    #[cfg(test)]
    pub fn clock(&self) -> (Option<f64>, f64) {
        (self.offset, self.deviation)
    }

    /// A sample arrived at `now` (app seconds).
    pub fn push(&mut self, now: f64, sample: VehicleSample) -> Pushed {
        // The controller was destroyed 5 s after the previous sample: the next one starts a new
        // one (FUN_390b2420), with a fresh clock and queue.
        if self.last_arrival.is_some_and(|last| now - last > TIMEOUT) {
            self.queue.clear();
            self.offset = None;
            self.deviation = 0.0;
            self.last_time = None;
            self.zone = None;
        }
        let mut outcome = Pushed::Added;
        if sample.ship {
            // FUN_390b2f30: a zone change resets the sync instead of queueing across zones.
            // **S8-C3** (inferred): the original compares with the model's zone and shifts the
            // queue; this port uses the last sample's zone and starts the sync over.
            if self.zone.is_some_and(|zone| zone != sample.zone) {
                self.queue.clear();
                self.offset = None;
                self.deviation = 0.0;
                self.last_time = None;
                outcome = Pushed::ZoneReset;
            }
            // FUN_390b2f30: an older packet within 1000 ms of the newest is a duplicate / late
            // copy and is dropped. (The wrap guard `last < time + 1000` is the original's.)
            if let Some(last) = self.last_time
                && sample.time < last
                && last < sample.time.saturating_add(STALE_WINDOW_MS)
            {
                return Pushed::Stale;
            }
            self.zone = Some(sample.zone);
        }

        let server = f64::from(sample.time) / 1000.0;
        let local = now;
        match self.offset {
            None => {
                // FUN_390986a0 first sample: offset = (server - local) - delay.
                self.offset = Some(server - local - DELAY);
                self.deviation = DELAY;
            }
            Some(offset) => {
                let deviation = (server - local) - offset;
                self.deviation = deviation;
                if deviation < 0.0 || deviation > DELAY * REBASE_FACTOR {
                    // A late burst or a clock jump: re-base and erase the queue (the original
                    // returns false and the caller clears it).
                    self.offset = Some(server - local);
                    self.deviation = 0.0;
                    self.queue.clear();
                    outcome = Pushed::Rebased;
                }
            }
        }

        let at = self
            .queue
            .partition_point(|queued| queued.time <= sample.time);
        if at > 0 && self.queue[at - 1].time == sample.time {
            self.queue[at - 1] = sample;
        } else {
            self.queue.insert(at, sample);
        }
        // FUN_390b2980: at most 1000 samples; the oldest is dropped.
        while self.queue.len() > 1000 {
            self.queue.pop_front();
        }
        self.last_time = Some(sample.time);
        self.last_arrival = Some(now);
        outcome
    }

    /// Moves the offset a step toward keeping `deviation` at [`DELAY`] (`FUN_39098780`, called
    /// from `FUN_390ad150` once per frame): a 1 ms nudge inside [`DEAD_BAND`], else
    /// `max(1 ms, 0.2 * excess)`. `DAT_399a28a8` is the ms-to-seconds factor.
    fn correct(&mut self) {
        let Some(offset) = self.offset.as_mut() else {
            return;
        };
        let error = self.deviation - DELAY;
        if error == 0.0 {
            return;
        }
        let excess = (error.abs() - DEAD_BAND).max(0.0);
        let step = if excess > 0.0 {
            (excess * CORRECTION).max(NUDGE)
        } else {
            NUDGE
        }
        .min(error.abs());
        *offset += step.copysign(error);
        self.deviation -= step.copysign(error);
    }

    /// The pose to draw at `now` (app seconds): interpolate around the playback time, hold when
    /// the queue runs dry, freeze after the 5 s timeout, snap over [`SNAP_DISTANCE`].
    pub fn sample(&mut self, now: f64) -> VehiclePose {
        let Some(last_arrival) = self.last_arrival else {
            return self.display;
        };
        if now - last_arrival > TIMEOUT {
            // FUN_390b2420 destroyed the controller; the drawing keeps its last pose.
            return self.display;
        }
        // FUN_39098780 is called per frame from FUN_390ad150, not per packet.
        self.correct();
        let Some(offset) = self.offset else {
            return self.display;
        };
        let Some(newest) = self.queue.back().copied() else {
            return self.display;
        };
        // FUN_390b0040 compares the drawn entity with the model (newest sample) position before
        // sampling. A larger step is a teleport and snaps at once, even while the playback
        // clock is still behind the new sample.
        if self.display.pos.distance(newest.pos) > SNAP_DISTANCE {
            self.display = VehiclePose::of(&newest);
            self.offset = Some(f64::from(newest.time) / 1000.0 - now);
            self.deviation = 0.0;
            self.queue.retain(|sample| sample.time == newest.time);
            return self.display;
        }
        let play = (now + offset) * 1000.0;
        let Some(pose) = self.pose_at(play) else {
            // Before the first sample's time: the entity keeps its pose (FUN_390ad150 returns
            // false).
            return self.display;
        };
        self.display = pose;
        pose
    }

    /// The interpolated pose at `play` on the sender's clock (ms), or `None` while the playback
    /// time is before the first sample.
    fn pose_at(&mut self, play: f64) -> Option<VehiclePose> {
        let first = *self.queue.front()?;
        if play < f64::from(first.time) {
            return None;
        }
        // FUN_390ad150 erased the samples the playback had passed, keeping the bracketing pair.
        while self.queue.len() > 2 && f64::from(self.queue[1].time) <= play {
            self.queue.pop_front();
        }
        let a = *self.queue.front()?;
        if play >= f64::from(self.queue.back()?.time) {
            // Past the newest sample: hold it (no extrapolation; `FUN_390a16b0` gets t = 0).
            return Some(VehiclePose::of(self.queue.back()?));
        }
        let b = *self.queue.get(1)?;
        let span = f64::from(b.time) - f64::from(a.time);
        let f = if span > 0.0 {
            ((play - f64::from(a.time)) / span).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        Some(VehiclePose {
            pos: a.pos.lerp(b.pos, f),
            // FUN_390a16b0: nlerp with the sign flipped to the short way, then normalise.
            rot: nlerp(a.rot, b.rot, f),
            steering: a.steering + (b.steering - a.steering) * f,
            throttle: a.throttle + (b.throttle - a.throttle) * f,
        })
    }
}

/// Quaternion linear interpolation on the short arc, normalised (`FUN_390a16b0`).
pub fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    (a * (1.0 - t) + b * t).normalize()
}

#[cfg(test)]
mod tests;
