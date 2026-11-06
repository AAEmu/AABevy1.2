//! The per-model numbers the cart controller drives with: `vehicle_models` speed, turn rate and
//! inertia, and the wheel layout of the body.
//!
//! `X2::VehicleUnitModel::Init` (`FUN_390f0510`) reads `velocity`, `angVel`, `lin_inertia`,
//! `lin_deaccel_inertia`, `rot_inertia` and `rot_deaccel_inertia` from the model's row, and the wheel
//! positions come from the body's `wheelN` / `wheel_type2N` mounts. Only the Farm Wagon (`vehicle_models`
//! 38) was captured, so [`CartParams::WAGON`] is the fit and every other model scales from it:
//!
//! * `top_speed = velocity` and `yaw_rate_per_steer = angVel` (model 38: 4.0 and 0.4, the capture);
//! * `accel = 1 / lin_inertia` (model 38: 2.5, the captured slope) and `decel = 1 / lin_deaccel_inertia`.
//!   **Inferred** for the other models: no capture shows their inertia at work. A model with no linear
//!   inertia (the wheeled-simulation speedcars) takes the wagon's 1.6 s from rest to top speed.

use bevy::prelude::*;

use super::{ACCEL, TOP_SPEED, TRACK, WHEEL_CENTROID, WHEEL_CONTACTS, WHEELBASE, YAW_RATE_PER_STEER};
use crate::ingame::data::VehicleLook;
use crate::ingame::vehicle;

/// The wagon's seconds from rest to top speed (4.0 m/s at 2.5 m/s^2), the stand-in for a model with
/// no `lin_inertia`.
const WAGON_RAMP_SECONDS: f64 = 1.6;

/// What one land vehicle model drives with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CartParams {
    /// Top forward speed, m/s.
    pub top_speed: f32,
    /// Yaw rate at full steering, rad/s (the wire `angVel.z`).
    pub yaw_rate_per_steer: f32,
    /// Speed gained per second while the throttle asks for more.
    pub accel: f32,
    /// Speed lost per second while the throttle asks for less (coasting, braking).
    pub decel: f32,
    /// Ground-contact probes in model space (x right, y forward): front right, front left, rear right,
    /// rear left.
    pub wheel_contacts: [(f32, f32); 4],
    /// Front-to-rear wheel distance, from `wheel_contacts`.
    pub wheelbase: f32,
    /// Mean left-to-right wheel distance, from `wheel_contacts`.
    pub track: f32,
}

impl CartParams {
    /// The Farm Wagon (`vehicle_models` 38), the captured fit.
    pub const WAGON: CartParams = CartParams {
        top_speed: TOP_SPEED,
        yaw_rate_per_steer: YAW_RATE_PER_STEER,
        accel: ACCEL,
        decel: ACCEL,
        wheel_contacts: WHEEL_CONTACTS,
        wheelbase: WHEELBASE,
        track: TRACK,
    };

    /// The mean of the wheel contacts: the point the fitted ground plane passes through.
    pub fn centroid(&self) -> (f32, f32) {
        if self.wheel_contacts == WHEEL_CONTACTS {
            return WHEEL_CENTROID;
        }
        let n = self.wheel_contacts.len() as f32;
        let (x, y) = self.wheel_contacts.iter().fold((0.0, 0.0), |(x, y), c| (x + c.0, y + c.1));
        (x / n, y / n)
    }

    /// The row of the vehicle model `models.id` `model_id`, with the wheel layout of `look`'s body when
    /// it has one; the wagon's layout otherwise. `None` when the database or the row is missing.
    pub fn load(model_id: u32, look: Option<&VehicleLook>) -> Option<CartParams> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .ok()?;
        let (velocity, ang_vel, lin, deaccel): (f64, f64, f64, f64) = conn
            .query_row(
                "SELECT v.velocity, v.angVel, v.lin_inertia, v.lin_deaccel_inertia \
                 FROM models m JOIN vehicle_models v ON v.id = m.sub_id AND m.sub_type = 'VehicleModel' \
                 WHERE m.id = ?1",
                [model_id],
                |r| {
                    let f = |i: usize| -> rusqlite::Result<f64> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0)) };
                    Ok((f(0)?, f(1)?, f(2)?, f(3)?))
                },
            )
            .ok()?;
        let mounts: Vec<Vec3> = look.map(vehicle::wheel_mounts).unwrap_or_default();
        Some(Self::from_row(velocity, ang_vel, lin, deaccel, &mounts))
    }

    /// Builds the parameters from a `vehicle_models` row and the body's wheel mounts. A row with no
    /// speed or turn rate keeps the wagon's, and a body without an axle pair keeps the wagon's layout.
    pub fn from_row(velocity: f64, ang_vel: f64, lin_inertia: f64, lin_deaccel_inertia: f64, mounts: &[Vec3]) -> CartParams {
        let wagon = Self::WAGON;
        let top = if velocity > 0.0 { velocity } else { f64::from(wagon.top_speed) };
        let rate = |inertia: f64| if inertia > 0.0 { 1.0 / inertia } else { top / WAGON_RAMP_SECONDS };
        let accel = rate(lin_inertia) as f32;
        let (contacts, wheelbase, track) = layout(mounts).unwrap_or((wagon.wheel_contacts, wagon.wheelbase, wagon.track));
        CartParams {
            top_speed: top as f32,
            yaw_rate_per_steer: if ang_vel > 0.0 { ang_vel as f32 } else { wagon.yaw_rate_per_steer },
            accel,
            decel: if lin_deaccel_inertia > 0.0 { rate(lin_deaccel_inertia) as f32 } else { accel },
            wheel_contacts: contacts,
            wheelbase,
            track,
        }
    }
}

/// The four contacts (front right, front left, rear right, rear left), wheelbase and track of a set of
/// wheel mounts: the mounts split at the middle of their length into a front and a rear axle, and the
/// widest pair of each is the contact pair. `None` when there is no axle pair to span a plane.
fn layout(mounts: &[Vec3]) -> Option<([(f32, f32); 4], f32, f32)> {
    let lo = mounts.iter().map(|m| m.y).fold(f32::MAX, f32::min);
    let hi = mounts.iter().map(|m| m.y).fold(f32::MIN, f32::max);
    let mid = (lo + hi) * 0.5;
    let axle = |front: bool| -> Option<((f32, f32), (f32, f32))> {
        let side = mounts.iter().filter(|m| (m.y > mid) == front);
        let right = side.clone().max_by(|a, b| a.x.total_cmp(&b.x))?;
        let left = side.min_by(|a, b| a.x.total_cmp(&b.x))?;
        Some(((right.x, right.y), (left.x, left.y)))
    };
    let (front_right, front_left) = axle(true)?;
    let (rear_right, rear_left) = axle(false)?;
    let wheelbase = (front_right.1 + front_left.1 - rear_right.1 - rear_left.1) * 0.5;
    let track = (front_right.0 - front_left.0 + rear_right.0 - rear_left.0) * 0.5;
    (wheelbase > 0.1 && track > 0.1).then_some(([front_right, front_left, rear_right, rear_left], wheelbase, track))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wagon_mounts() -> Vec<Vec3> {
        WHEEL_CONTACTS.iter().map(|&(x, y)| Vec3::new(x, y, 0.4)).collect()
    }

    #[test]
    fn the_wagon_row_gives_the_captured_fit() {
        // vehicle_models 38: velocity 4.0, angVel 0.4, lin_inertia 0.4, lin_deaccel_inertia 0.4
        let p = CartParams::from_row(4.0, 0.4, 0.4, 0.4, &wagon_mounts());
        assert_eq!(p.top_speed, TOP_SPEED);
        assert_eq!(p.yaw_rate_per_steer, YAW_RATE_PER_STEER);
        assert_eq!(p.accel, ACCEL);
        assert_eq!(p.decel, ACCEL);
        assert_eq!(p.wheel_contacts, WHEEL_CONTACTS);
        assert!((p.wheelbase - WHEELBASE).abs() < 1e-4 && (p.track - TRACK).abs() < 1e-4);
    }

    #[test]
    fn the_centroid_is_the_contacts_mean() {
        let derived = CartParams { wheel_contacts: [(1.0, 2.0), (-1.0, 2.0), (1.0, -4.0), (-1.0, -4.0)], ..CartParams::WAGON };
        assert_eq!(derived.centroid(), (0.0, -1.0));
        let mean = WHEEL_CONTACTS.iter().fold((0.0, 0.0), |a, c| (a.0 + c.0 / 4.0, a.1 + c.1 / 4.0));
        assert!((mean.0 - WHEEL_CENTROID.0).abs() < 1e-4 && (mean.1 - WHEEL_CENTROID.1).abs() < 1e-4);
        assert_eq!(CartParams::WAGON.centroid(), WHEEL_CENTROID);
    }

    #[test]
    fn a_model_without_mounts_keeps_the_wagon_layout() {
        let p = CartParams::from_row(5.0, 0.4, 0.4, 0.4, &[]);
        assert_eq!(p.wheel_contacts, WHEEL_CONTACTS);
        assert_eq!(p.top_speed, 5.0);
        // a single wheel spans no plane
        let one = CartParams::from_row(5.0, 0.4, 0.4, 0.4, &[Vec3::new(0.5, 1.0, 0.0)]);
        assert_eq!(one.wheel_contacts, WHEEL_CONTACTS);
    }

    #[test]
    fn a_model_with_no_inertia_takes_the_wagon_ramp_time() {
        let p = CartParams::from_row(15.0, 1.0, 0.0, 0.0, &[]);
        assert!((p.accel - 15.0 / 1.6).abs() < 1e-5);
        assert_eq!(p.decel, p.accel);
        assert_eq!(p.yaw_rate_per_steer, 1.0);
    }

    #[test]
    fn deceleration_follows_its_own_inertia() {
        // vehicle_models 17: lin_inertia 0.3, lin_deaccel_inertia 0.1
        let p = CartParams::from_row(13.0, 1.0, 0.3, 0.1, &[]);
        assert!((p.accel - 1.0 / 0.3).abs() < 1e-5 && (p.decel - 10.0).abs() < 1e-5);
    }

    #[test]
    fn six_wheels_pick_the_outer_axle_pairs() {
        let mounts = [(0.9, 2.0), (-0.9, 2.0), (1.0, 0.0), (-1.0, 0.0), (1.1, -2.0), (-1.1, -2.0)]
            .map(|(x, y)| Vec3::new(x, y, 0.0));
        let (contacts, wheelbase, track) = layout(&mounts).unwrap();
        assert_eq!(contacts[0], (0.9, 2.0));
        assert_eq!(contacts[3], (-1.1, -2.0));
        assert!(wheelbase > 3.9 && (track - 2.0).abs() < 0.3);
    }

    /// Models 38, 65 and 45 read their speeds from the game database (skips without it).
    #[test]
    fn the_database_rows_set_each_models_speed() {
        let Some(wagon) = CartParams::load(1008, None) else { return };
        assert_eq!(wagon.top_speed, 4.0);
        assert_eq!(wagon.accel, ACCEL);
        assert_eq!(wagon.yaw_rate_per_steer, YAW_RATE_PER_STEER);
        assert_eq!(CartParams::load(1553, None).unwrap().top_speed, 5.0);
        assert_eq!(CartParams::load(1293, None).unwrap().top_speed, 15.0);
    }

    /// The wagon body's mounts give back [`WHEEL_CONTACTS`] (skips without the pak and the database).
    #[test]
    fn the_wagon_bodys_mounts_equal_the_constants() {
        let Some(db) = crate::ingame::data::Db::open() else { return };
        let Some(look) = db.vehicle_look(1008) else { return };
        if vehicle::wheel_mounts(&look).is_empty() {
            return;
        }
        let Some(p) = CartParams::load(1008, Some(&look)) else { return };
        for (got, want) in p.wheel_contacts.iter().zip(WHEEL_CONTACTS) {
            assert!((got.0 - want.0).abs() < 1e-3 && (got.1 - want.1).abs() < 1e-3, "{got:?} vs {want:?}");
        }
    }
}
