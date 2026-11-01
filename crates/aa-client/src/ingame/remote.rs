//! How other units move on screen: the positions the server reports are buffered and played back a little late, so a unit
//! glides along the path the server walked it instead of stepping at every packet or overshooting between them.
//!
//! The server sends an NPC's step every 100 ms (a walking mob: 0.15 m, a chasing one: 0.5 m) and a player's movements at
//! whatever rate that client uses. A [`Track`] keeps the last few reports (`push`, with the arrival time) and a playback
//! clock that runs [`Track::delay`] behind the newest one (`sample`); between two reports the pose is interpolated, past the
//! newest it is extrapolated for a moment while the unit was moving and then held. Report times follow the sender's own
//! clock when that is plausible (`server_ms`), so a late or bunched packet does not show.

use std::collections::VecDeque;

use bevy::math::{Vec2, Vec3};

/// Reports kept.
const KEEP: usize = 8;
/// Playback runs this far behind the newest report (s): at least this, and a bit more than the time between reports.
const MIN_DELAY: f32 = 0.08;
const MAX_DELAY: f32 = 0.35;
/// How long a unit that was moving keeps going on its last velocity when no report arrives (s), then it holds still.
const EXTRAPOLATE: f32 = 0.3;
/// Silence longer than this (s) is a pause: the unit stood still and starts afresh with the next report.
const PAUSE: f32 = 0.5;
/// A step this long (m) between two reports is a teleport, not a walk: there is nothing to glide over.
const TELEPORT: f32 = 12.0;
/// Horizontal speed (m/s) under which a unit counts as standing.
pub const STANDING: f32 = 0.05;

/// What one movement packet says.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Game coordinates (x east, y north, z up).
    pub pos: Vec3,
    /// Facing, radians (0 north, counter-clockwise).
    pub yaw: f32,
    /// The packet's `time` (the sender's clock, ms); only differences count.
    pub server_ms: u32,
    /// In the air (a jump or a fall), by the packet's flags.
    pub airborne: bool,
    /// In combat (the movement flag, the alertness or the stance of the packet).
    pub combat: bool,
}

#[derive(Clone, Copy, Debug)]
struct Snap {
    /// On the track's clock (s).
    t: f32,
    /// When it arrived (app clock, s).
    arrival: f32,
    report: Report,
}

/// What the unit looks like right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: Vec3,
    pub yaw: f32,
    /// Horizontal speed (m/s) along the path it is on; 0 when it stands.
    pub speed: f32,
    /// Direction of travel in the world (unit length) when it moves.
    pub heading: Vec2,
    pub airborne: bool,
    pub combat: bool,
}

/// The buffered reports of one unit and the playback clock.
#[derive(Clone, Debug)]
pub struct Track {
    snaps: VecDeque<Snap>,
    /// Playback time on the track's clock.
    clock: f32,
    /// Average time between reports while the unit moves (s).
    gap: f32,
}

/// Shortest-arc interpolation of angles.
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * t
}

impl Track {
    /// A unit that stands at `pos` facing `yaw`.
    pub fn new(pos: Vec3, yaw: f32) -> Track {
        let report = Report { pos, yaw, server_ms: 0, airborne: false, combat: false };
        Track { snaps: VecDeque::from([Snap { t: 0.0, arrival: f32::NEG_INFINITY, report }]), clock: 0.0, gap: 0.1 }
    }

    /// Where the newest report put the unit.
    pub fn latest(&self) -> Vec3 {
        self.snaps.back().expect("a track always has a report").report.pos
    }

    /// The playback time on the track's clock (s).
    pub fn clock(&self) -> f32 {
        self.clock
    }

    /// How far playback runs behind the newest report (s).
    pub fn delay(&self) -> f32 {
        (self.gap * 1.25).clamp(MIN_DELAY, MAX_DELAY)
    }

    /// A report arrived at `now` (app clock, s).
    pub fn push(&mut self, now: f32, report: Report) {
        self.push_inner(now, report, true);
    }

    /// A report arrived after the caller seeded the track at the current drawn point (a rider
    /// stepping onto another hull): a large step is blended over the delay rather than snapped.
    pub fn push_blend(&mut self, now: f32, report: Report) {
        self.push_inner(now, report, false);
    }

    fn push_inner(&mut self, now: f32, report: Report, permit_teleport: bool) {
        let last = *self.snaps.back().expect("a track always has a report");
        let arrival_gap = if last.arrival.is_finite() { now - last.arrival } else { f32::INFINITY };
        let teleport = permit_teleport
            && Vec2::new(report.pos.x - last.report.pos.x, report.pos.y - last.report.pos.y).length() > TELEPORT;
        if teleport {
            // nothing to glide over: the unit is simply somewhere else
            self.snaps.clear();
            self.snaps.push_back(Snap { t: 0.0, arrival: now, report });
            self.clock = 0.0;
            return;
        }
        // the sender's clock spaces the reports evenly whatever the network did, when it agrees with ours
        let server_gap = report.server_ms.wrapping_sub(last.report.server_ms) as f32 / 1000.0;
        let step = if (0.001..=1.0).contains(&server_gap) && (server_gap - arrival_gap).abs() < 0.5 { server_gap } else { arrival_gap.clamp(0.001, PAUSE * 2.0) };
        let moved = Vec2::new(report.pos.x - last.report.pos.x, report.pos.y - last.report.pos.y).length() > 0.005;
        if moved && step < PAUSE {
            self.gap += (step - self.gap) * 0.3;
        }
        if arrival_gap > PAUSE {
            // it stood still until now: hold the old pose until just before this report, and start playback right behind it
            let hold = (self.gap).min(step);
            let t = last.t + step.max(hold);
            self.snaps.push_back(Snap { t: t - hold, arrival: now, report: Report { server_ms: report.server_ms, ..last.report } });
            self.snaps.push_back(Snap { t, arrival: now, report });
            self.clock = t - self.delay();
        } else {
            self.snaps.push_back(Snap { t: last.t + step, arrival: now, report });
        }
        while self.snaps.len() > KEEP {
            self.snaps.pop_front();
        }
    }

    /// Moves the playback clock on by `dt` and returns the pose.
    pub fn sample(&mut self, dt: f32) -> Pose {
        let latest = self.snaps.back().expect("a track always has a report").t;
        // steer the clock to `delay` behind the newest report: faster when behind, slower when ahead, a jump when far behind
        let err = latest - self.delay() - self.clock;
        if err > 0.6 {
            self.clock = latest - self.delay();
        } else {
            self.clock += dt * (1.0 + err).clamp(0.5, 1.5);
        }
        self.pose_at(self.clock)
    }

    /// The pose at `clock` on the track's clock.
    pub fn pose_at(&self, clock: f32) -> Pose {
        let n = self.snaps.len();
        let first = self.snaps[0];
        let last = self.snaps[n - 1];
        let combat = last.report.combat;
        let horizontal = |a: &Report, b: &Report, dt: f32| {
            let d = Vec2::new(b.pos.x - a.pos.x, b.pos.y - a.pos.y);
            let speed = if dt > 1e-4 { d.length() / dt } else { 0.0 };
            (speed, d.normalize_or_zero())
        };
        if clock <= first.t || n == 1 {
            return Pose { pos: first.report.pos, yaw: first.report.yaw, speed: 0.0, heading: Vec2::ZERO, airborne: first.report.airborne, combat };
        }
        if clock >= last.t {
            // past the newest report: carry on for a moment if the unit was moving, else hold
            let prev = self.snaps[n - 2];
            let (speed, heading) = horizontal(&prev.report, &last.report, last.t - prev.t);
            let over = clock - last.t;
            if speed > STANDING && over < EXTRAPOLATE {
                let v = (last.report.pos - prev.report.pos) / (last.t - prev.t).max(1e-4);
                let v = if last.report.airborne { v } else { Vec3::new(v.x, v.y, 0.0) };
                return Pose { pos: last.report.pos + v * over, yaw: last.report.yaw, speed, heading, airborne: last.report.airborne, combat };
            }
            return Pose { pos: last.report.pos, yaw: last.report.yaw, speed: 0.0, heading: Vec2::ZERO, airborne: last.report.airborne, combat };
        }
        let i = (0..n - 1).find(|&i| self.snaps[i + 1].t >= clock).unwrap_or(n - 2);
        let (a, b) = (self.snaps[i], self.snaps[i + 1]);
        let span = b.t - a.t;
        let f = if span > 1e-5 { ((clock - a.t) / span).clamp(0.0, 1.0) } else { 1.0 };
        let (speed, heading) = horizontal(&a.report, &b.report, span);
        Pose {
            pos: a.report.pos.lerp(b.report.pos, f),
            yaw: lerp_angle(a.report.yaw, b.report.yaw, f),
            speed,
            heading,
            airborne: if f < 0.5 { a.report.airborne } else { b.report.airborne },
            combat,
        }
    }
}

/// The direction of travel `heading` (world, x east, y north) for a unit facing `yaw`, as the locomotion clips want it:
/// x to the right, y forward.
pub fn local_dir(heading: Vec2, yaw: f32) -> Vec2 {
    let forward = Vec2::new(-yaw.sin(), yaw.cos());
    let right = Vec2::new(yaw.cos(), yaw.sin());
    Vec2::new(heading.dot(right), heading.dot(forward))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(x: f32, ms: u32) -> Report {
        Report { pos: Vec3::new(x, 0.0, 0.0), yaw: 0.0, server_ms: ms, airborne: false, combat: false }
    }

    /// Plays back a unit that walks `speed` m/s along x, reported every 100 ms, with arrival times jittered by `jitter` (s,
    /// alternating sign); returns the displayed x every 16 ms frame after the first second.
    fn play(speed: f32, jitter: f32, seconds: f32) -> Vec<f32> {
        let mut track = Track::new(Vec3::ZERO, 0.0);
        let (mut out, mut next, mut sent) = (Vec::new(), 0, 0);
        let frame = 0.016;
        let mut now = 0.0f32;
        while now < seconds {
            // reports whose (jittered) arrival time has come
            loop {
                let t = sent as f32 * 0.1;
                let arrives = t + if sent % 2 == 0 { jitter } else { -jitter };
                if arrives > now {
                    break;
                }
                track.push(now, report(speed * t, (1000 + sent * 100) as u32));
                sent += 1;
            }
            let pose = track.sample(frame);
            if now > 1.0 {
                out.push(pose.pos.x);
            }
            next += 1;
            now = next as f32 * frame;
        }
        out
    }

    #[test]
    fn a_steady_walk_is_played_back_smoothly() {
        for jitter in [0.0, 0.03] {
            let xs = play(1.5, jitter, 6.0);
            let steps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
            let (min, max) = steps.iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            // 1.5 m/s at 62 frames/s is 24 mm a frame; no frame stands still or jumps (also with 30 ms of jitter)
            assert!(min > 0.010 && max < 0.040, "jitter {jitter}: steps from {min} to {max}");
            let speed = (xs[xs.len() - 1] - xs[0]) / (xs.len() as f32 * 0.016);
            assert!((speed - 1.5).abs() < 0.1, "played at {speed} m/s");
        }
    }

    #[test]
    fn a_unit_stops_where_the_server_stopped_it() {
        let mut track = Track::new(Vec3::ZERO, 0.0);
        let mut now = 0.0;
        for i in 0..10u32 {
            track.push(now, report(i as f32 * 0.15, 1000 + i * 100));
            now += 0.1;
        }
        // the reports stop at x = 1.35: it keeps going for at most the extrapolation, then holds
        let mut last = track.sample(0.016);
        for _ in 0..200 {
            last = track.sample(0.016);
        }
        assert!((last.pos.x - 1.35).abs() < 1e-4, "{}", last.pos.x);
        assert_eq!(last.speed, 0.0);
    }

    #[test]
    fn a_unit_that_starts_again_after_a_pause_does_not_slide() {
        let mut track = Track::new(Vec3::new(10.0, 0.0, 0.0), 0.0);
        track.push(5.0, report(10.0, 1000));
        for _ in 0..60 {
            track.sample(0.016);
        }
        // two seconds later it moves on: a step of 0.15 m must not be played over two seconds
        let (mut now, mut moved_at) = (5.0, None);
        for i in 0..8u32 {
            now = 7.0 + i as f32 * 0.1;
            track.push(now, report(10.0 + (i + 1) as f32 * 0.15, 3000 + i * 100));
        }
        for frame in 0..60 {
            let pose = track.sample(0.016);
            if pose.pos.x > 10.05 && moved_at.is_none() {
                moved_at = Some(frame);
            }
        }
        assert!(moved_at.is_some_and(|f| f < 30), "it began to move after {moved_at:?} frames");
        let _ = now;
    }

    #[test]
    fn a_teleport_is_not_glided_over() {
        let mut track = Track::new(Vec3::ZERO, 0.0);
        track.push(1.0, report(0.1, 1000));
        track.push(1.1, report(500.0, 1100));
        let pose = track.sample(0.016);
        assert_eq!(pose.pos.x, 500.0);
    }

    #[test]
    fn angles_take_the_short_way() {
        let a = 3.0;
        let b = -3.0;
        let mid = lerp_angle(a, b, 0.5);
        // half way from 3 to -3 radians the short way round is across pi
        assert!((mid.abs() - std::f32::consts::PI).abs() < 0.01, "{mid}");
        assert!((lerp_angle(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn travel_direction_relative_to_the_facing() {
        // facing north (yaw 0): moving north is forward, moving east is to the right
        assert!((local_dir(Vec2::Y, 0.0) - Vec2::Y).length() < 1e-5);
        assert!((local_dir(Vec2::X, 0.0) - Vec2::X).length() < 1e-5);
        // facing east (yaw -90 degrees): moving east is forward, moving north is to the left
        let yaw = -std::f32::consts::FRAC_PI_2;
        assert!((local_dir(Vec2::X, yaw) - Vec2::Y).length() < 1e-5);
        assert!((local_dir(Vec2::Y, yaw) + Vec2::X).length() < 1e-5);
        // moving backwards
        assert!((local_dir(-Vec2::Y, 0.0) + Vec2::Y).length() < 1e-5);
    }
}
