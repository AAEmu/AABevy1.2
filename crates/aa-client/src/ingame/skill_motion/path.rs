//! The paths skills move units along, as plain math (game coordinates: x east, y north, z up).
//!
//! * [`leap`]: a Leap skill controller (`skill_controllers.kind_id` 2). The server's `LeapArc` (`controllers.rs` in the AAEmu
//!   server, from the zone's `0x39231780`) is the reference: the unit goes in a straight line over the ground at a constant speed
//!   `max(min speed, distance / duration)` (at most 50 m/s) to a point `offset` metres past the target on the line from the unit
//!   to it (a negative offset stops short of it; with the unit itself as the target it is `offset` metres along its facing), and
//!   when the controller has a launch angle the height follows a parabola that starts that much above the line and lands on the
//!   end point. Charge is angle 0 (a straight dash), Drop Back and Overwhelm are arcs.
//! * [`dash`]: kind 4, a straight run `offset` metres towards (or away from) the target.
//! * [`knock`]: a push (`KnockBack` special effect, `value1` mm and `value2` degrees): a parabola away from the source that lands
//!   `distance` metres on.

use bevy::math::{Vec2, Vec3};

/// Gravity of the actor model (`physics_living_gravity`).
pub const GRAVITY: f32 = 25.0;
/// The fastest a controller moves a unit (m/s).
const MAX_SPEED: f32 = 50.0;
/// Seconds a controller without a duration takes (the server's fallback).
const DEFAULT_DURATION: f32 = 0.5;
/// The highest launch angle a leap adds (radians, `LeapArc`).
const MAX_PITCH: f32 = 80.0 * std::f32::consts::PI / 180.0;

/// A row of `skill_controllers`: the kind and `value1` ..= `value13` (index 0 is `value1`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Controller {
    pub id: u32,
    pub kind: u32,
    pub v: [i32; 13],
    /// `start_anim_id`: the clip the caster plays.
    pub start_anim: u32,
}

impl Controller {
    /// Which way the controller may move the unit (`value7`).
    pub fn direction(&self) -> Direction {
        match self.v[6] {
            1 => Direction::Forward,
            2 => Direction::Backward,
            _ => Direction::Both,
        }
    }

    /// The unit turns to its direction of travel (`value9`).
    pub fn rotates(&self) -> bool {
        self.v[8] == 1
    }
}

/// `value7` of a leap or dash: only towards the target, only away from it, or either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Both,
    Forward,
    Backward,
}

/// A move at constant horizontal speed with a parabolic height: `at(t)` for `t` in `0..=eta`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub from: Vec3,
    /// Where it ends (height: where the ground should be).
    pub to: Vec3,
    /// Horizontal unit direction of travel.
    pub dir: Vec2,
    /// Horizontal distance (m).
    pub dist: f32,
    /// Horizontal speed (m/s).
    pub hs: f32,
    /// Initial vertical speed and acceleration.
    pub vs: f32,
    pub az: f32,
    /// Seconds it takes.
    pub eta: f32,
}

impl Arc {
    /// The position `t` seconds after the start (clamped to the end).
    pub fn at(&self, t: f32) -> Vec3 {
        if self.eta <= 0.0 {
            return self.to;
        }
        let t = t.clamp(0.0, self.eta);
        let xy = Vec2::new(self.from.x, self.from.y) + self.dir * self.hs * t;
        Vec3::new(xy.x, xy.y, self.from.z + self.vs * t + 0.5 * self.az * t * t)
    }

    /// The highest point above the straight line from start to end (m): 0 for a dash.
    pub fn lift(&self) -> f32 {
        (0..=8).map(|i| {
            let t = self.eta * i as f32 / 8.0;
            let line = self.from.z + (self.to.z - self.from.z) * (t / self.eta.max(1e-4));
            self.at(t).z - line
        }).fold(0.0, f32::max)
    }
}

/// Horizontal distance.
fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.y - b.y).length()
}

/// Unit vector of a game yaw (0 faces north, counter-clockwise).
pub fn forward(yaw: f32) -> Vec2 {
    Vec2::new(-yaw.sin(), yaw.cos())
}

/// Where a leap controller ends. `target`: the reference unit's position (`None`: the unit itself, then it goes along its facing).
/// `None` when the direction rule keeps the unit where it is.
pub fn leap_end(c: &Controller, owner: Vec3, yaw: f32, target: Option<Vec3>) -> Option<Vec3> {
    let offset = c.v[3] as f32 * 0.001;
    let end = match target {
        None => {
            let f = forward(yaw);
            Vec3::new(owner.x + f.x * offset, owner.y + f.y * offset, owner.z)
        }
        Some(t) => {
            let delta = t - owner;
            let dir = delta.normalize_or_zero();
            let end = t + dir * offset;
            // `ApplyDirectionConstraint`: only towards the target / only away from it
            let (before, after) = (flat(owner, t), flat(end, t));
            match c.direction() {
                Direction::Forward if after > before => return None,
                Direction::Backward if after < before => return None,
                _ => end,
            }
        }
    };
    (flat(owner, end) >= 0.01).then_some(end)
}

/// The path of a leap controller from `owner` to `end` (see the module documentation). `anim_speed` scales the duration when the
/// controller says so (`value12`).
pub fn leap(c: &Controller, owner: Vec3, end: Vec3, anim_speed: f32) -> Option<Arc> {
    let dist = flat(owner, end);
    if dist < 0.01 {
        return None;
    }
    let scale = if c.v[11] == 1 { anim_speed.max(0.001) } else { 1.0 };
    let duration = if c.v[2] > 0 { c.v[2] as f32 * 0.001 / scale } else { DEFAULT_DURATION };
    let hs = (c.v[1] as f32 * scale).max((dist / duration).clamp(0.0, MAX_SPEED));
    let angle = (c.v[0] as f32).to_radians();
    let dz = end.z - owner.z;
    let vs = (((dz / dist).atan() + angle).min(MAX_PITCH)).tan() * hs;
    let eta = dist / hs;
    let az = if angle > 0.0 && eta > 0.0 { (2.0 * (dz - eta * vs) / (eta * eta)).clamp(-3000.0, 3000.0) } else { 0.0 };
    let d = Vec2::new(end.x - owner.x, end.y - owner.y) / dist;
    Some(Arc { from: owner, to: end, dir: d, dist, hs, vs, az, eta })
}

/// A dash controller (kind 4): `offset` metres towards the target (away from it for `Backward`), over the controller's duration.
pub fn dash(c: &Controller, owner: Vec3, yaw: f32, target: Option<Vec3>) -> Option<Arc> {
    let offset = c.v[3] as f32 * 0.001;
    let toward = match target {
        Some(t) if flat(owner, t) > 0.01 => Vec2::new(t.x - owner.x, t.y - owner.y).normalize(),
        _ => forward(yaw),
    };
    let toward = if c.direction() == Direction::Backward { -toward } else { toward };
    let end = Vec3::new(owner.x + toward.x * offset, owner.y + toward.y * offset, owner.z);
    leap(&Controller { v: { let mut v = c.v; v[0] = 0; v[1] = 0; v }, ..*c }, owner, end, 1.0)
}

/// A push of `distance` metres along `dir` that launches at `angle_deg` (a flat push, angle 0, still hops a little). The flight
/// takes the time of that projectile under [`GRAVITY`], at least 0.3 s so a short push is seen.
pub fn knock(from: Vec3, dir: Vec2, distance: f32, angle_deg: f32) -> Arc {
    let theta = angle_deg.abs().clamp(20.0, 70.0).to_radians();
    let v = (distance * GRAVITY / (2.0 * theta).sin()).sqrt();
    let eta = (2.0 * v * theta.sin() / GRAVITY).clamp(0.3, 1.0);
    let dir = dir.normalize_or_zero();
    Arc {
        from,
        to: from + Vec3::new(dir.x * distance, dir.y * distance, 0.0),
        dir,
        dist: distance,
        hs: distance / eta,
        vs: 0.5 * GRAVITY * eta,
        az: -GRAVITY,
        eta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Charge (controller 6779, plot event 5100): 400 ms, at least 10 m/s, ends 0.8 m before the target, forward only.
    fn charge() -> Controller {
        let mut v = [0; 13];
        v[1] = 10;
        v[2] = 400;
        v[3] = -800;
        v[4..10].copy_from_slice(&[1, 1, 1, 1, 1, 1]);
        Controller { id: 6779, kind: 2, v, start_anim: 174 }
    }

    /// Drop Back (controller 8074): a 15 degree arc 10 m backwards in 700 ms, at least 14 m/s.
    fn drop_back() -> Controller {
        let mut v = [0; 13];
        v[0] = 15;
        v[1] = 14;
        v[2] = 700;
        v[3] = -10000;
        Controller { id: 8074, kind: 2, v, start_anim: 167 }
    }

    #[test]
    fn charge_stops_short_of_the_target_and_runs_fast() {
        let owner = Vec3::new(100.0, 100.0, 10.0);
        let target = Vec3::new(100.0, 108.0, 10.0);
        let c = charge();
        let end = leap_end(&c, owner, 0.0, Some(target)).unwrap();
        assert!((end - Vec3::new(100.0, 107.2, 10.0)).length() < 1e-4, "{end}");
        let arc = leap(&c, owner, end, 1.0).unwrap();
        // 7.2 m in 0.4 s is 18 m/s, above the 10 m/s minimum
        assert!((arc.hs - 18.0).abs() < 1e-3 && (arc.eta - 0.4).abs() < 1e-4, "{arc:?}");
        assert!((arc.at(0.2) - Vec3::new(100.0, 103.6, 10.0)).length() < 1e-3);
        assert!((arc.at(5.0) - end).length() < 1e-3);
        assert!(arc.lift() < 1e-3, "a charge is a straight line");
    }

    #[test]
    fn a_close_target_is_reached_at_the_minimum_speed_and_a_closer_one_not_at_all() {
        let owner = Vec3::ZERO;
        let c = charge();
        let end = leap_end(&c, owner, 0.0, Some(Vec3::new(0.0, 3.0, 0.0))).unwrap();
        let arc = leap(&c, owner, end, 1.0).unwrap();
        assert!((arc.hs - 10.0).abs() < 1e-4 && (arc.eta - 0.22).abs() < 1e-3, "{arc:?}");
        // already within 0.8 m of the target: forward only means it stays
        assert!(leap_end(&c, owner, 0.0, Some(Vec3::new(0.0, 0.5, 0.0))).is_none());
    }

    #[test]
    fn drop_back_jumps_backwards_along_an_arc() {
        let owner = Vec3::new(50.0, 50.0, 5.0);
        // facing north (yaw 0): ten metres backwards is south
        let end = leap_end(&drop_back(), owner, 0.0, None).unwrap();
        assert!((end - Vec3::new(50.0, 40.0, 5.0)).length() < 1e-3, "{end}");
        let arc = leap(&drop_back(), owner, end, 1.0).unwrap();
        assert!((arc.hs - 14.2857).abs() < 0.01 && arc.eta < 0.71, "{arc:?}");
        let top = arc.lift();
        assert!(top > 0.5 && top < 3.0, "{top}");
        assert!((arc.at(arc.eta).z - 5.0).abs() < 1e-3, "lands where it started: {}", arc.at(arc.eta).z);
        // facing east (yaw -90 degrees): backwards is west
        let end = leap_end(&drop_back(), owner, -std::f32::consts::FRAC_PI_2, None).unwrap();
        assert!((end - Vec3::new(40.0, 50.0, 5.0)).length() < 1e-3, "{end}");
    }

    #[test]
    fn lasso_pulls_the_target_to_just_before_the_caster() {
        // Lasso (controller 5515): 30 degrees, 40 m/s minimum, 500 ms, ends 1 m short of the reference (the caster)
        let mut v = [0; 13];
        v[0] = 30;
        v[1] = 40;
        v[2] = 500;
        v[3] = -1000;
        let c = Controller { id: 5515, kind: 2, v, start_anim: 49 };
        let victim = Vec3::new(0.0, 12.0, 0.0);
        let caster = Vec3::new(0.0, 0.0, 0.0);
        let end = leap_end(&c, victim, 0.0, Some(caster)).unwrap();
        assert!((end - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4, "{end}");
        let arc = leap(&c, victim, end, 1.0).unwrap();
        assert!((arc.hs - 40.0).abs() < 1e-3);
        assert!(arc.lift() > 0.2);
    }

    #[test]
    fn a_knock_lands_on_the_distance_it_asks() {
        let a = knock(Vec3::new(0.0, 0.0, 3.0), Vec2::new(0.0, 2.0), 3.0, 40.0);
        let end = a.at(a.eta);
        assert!((end.x).abs() < 1e-4 && (end.y - 3.0).abs() < 1e-3 && (end.z - 3.0).abs() < 1e-3, "{end}");
        assert!(a.eta >= 0.3 && a.eta <= 1.0);
        assert!(a.at(a.eta * 0.5).z > 3.2, "it goes up");
        // a short flat push still takes a moment and stays low
        let small = knock(Vec3::ZERO, Vec2::X, 0.3, 0.0);
        assert!((small.eta - 0.3).abs() < 1e-4 && small.lift() < 0.4);
    }

    #[test]
    fn a_blink_is_at_its_end_at_once() {
        let a = Arc { from: Vec3::ZERO, to: Vec3::new(3.0, 4.0, 5.0), dir: Vec2::ZERO, dist: 0.0, hs: 0.0, vs: 0.0, az: 0.0, eta: 0.0 };
        assert_eq!(a.at(0.0), Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn a_dash_goes_straight_and_backwards_when_told() {
        let mut v = [0; 13];
        v[2] = 500;
        v[3] = 4000;
        let c = Controller { id: 1, kind: 4, v, start_anim: 0 };
        let a = dash(&c, Vec3::ZERO, 0.0, Some(Vec3::new(0.0, 10.0, 0.0))).unwrap();
        assert!((a.to - Vec3::new(0.0, 4.0, 0.0)).length() < 1e-4 && (a.eta - 0.5).abs() < 1e-4, "{a:?}");
        let mut back = c;
        back.v[6] = 2;
        let a = dash(&back, Vec3::ZERO, 0.0, Some(Vec3::new(0.0, 10.0, 0.0))).unwrap();
        assert!((a.to - Vec3::new(0.0, -4.0, 0.0)).length() < 1e-4);
    }
}
