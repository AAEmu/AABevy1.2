//! The 1.2 client's space search before a slave summon item is used (`x2game.dll` `FUN_3942c0e0` ->
//! `FUN_3934a340` -> `FUN_39349a10` -> `FUN_39347fb0`).
//!
//! The client searches before it sends anything: 40 steps at `slaves.spawn_x_offset + i` metres along
//! the player's facing, and at each step three candidates sideways `{0, +spawn_y_offset, -spawn_y_offset}`
//! (the Eznan Cutter 30 / 20, the carts 5 / 5, ...). A candidate passes when
//!
//! 1. ships have terrain at or below the water level there; land vehicles have terrain at most 10 m
//!    above the player;
//! 2. a ray from the player's position 1 m up to the candidate (the water surface for ships) hits
//!    nothing;
//! 3. the slave model's box at the candidate, turned to the player's yaw + pi / 2, overlaps nothing
//!    (ships also test the terrain, land vehicles do not);
//! 4. the 3D distance is at most 80 m.
//!
//! The first candidate that passes wins. When none does the client shows
//! `SLAVE_SPAWN_SHIP_NEED_MORE_SPACE` (error 100, "Summoning requires a larger space.") and sends
//! nothing. An owned slave of the same item toggles the item into a cancel: near it the client sends
//! `CSDespawnSlave`, far from it (over `slaves.spawn_valid_area_range`) it shows
//! `SLAVE_DESPAWN_NEAR_THE_SLAVE` (error 312) and sends nothing. The 1.2 cancel range itself was not
//! recovered from the decompile; 50 m is the database's `spawn_valid_area_range` for the captured
//! boats and carts (needs capture).

use std::sync::Arc;

use aa_net::slave::SlavePositionInfo;
use aa_world::{TerrainQuery, WaterQuery, WorldCollision};
use bevy::prelude::*;

/// Candidates along the facing (`FUN_39347fb0`: `0x27 < local_14`).
pub const STEPS: usize = 40;
/// `DAT_39ac740c` (6400): the 80 m 3D distance cap.
pub const MAX_RANGE: f32 = 80.0;
/// `DAT_3999cce8` (10.0): how far over the player a land vehicle's ground may be.
pub const LAND_CLEARANCE: f32 = 10.0;
/// `_DAT_3999c488` (1.0): the ray starts this far over the player.
pub const RAY_LIFT: f32 = 1.0;
/// The ray stops this far over the spot. The native ray ends on the candidate (the ground for a land
/// vehicle); `terrain_ray` samples the endpoint, where float noise turns a zero clearance into a hit.
pub const RAY_END_LIFT: f32 = 0.2;
/// The fallback cancel range when the database has none.
pub const DEFAULT_VALID_AREA: f32 = 50.0;

/// One `item_summon_slaves` row with its slave (`slaves`) and model box.
#[derive(Clone, Debug, PartialEq)]
pub struct SummonSlave {
    pub slave_id: u32,
    /// `slaves.spawn_x_offset` / `spawn_y_offset`.
    pub spawn_x_offset: f32,
    pub spawn_y_offset: f32,
    /// A sailing ship, boat or fishboat (`slaves.slave_kind_id` 1..=4 or 9): searched on water.
    pub ship: bool,
    pub model_id: u32,
    /// The model's box in model space (game axes, Z up).
    pub half_extents: Vec3,
    pub box_center: Vec3,
    /// `slaves.spawn_valid_area_range`: the 1.2 valid-area limit (50 m in the database).
    pub valid_area_range: f32,
}

/// What a summon-item press does after the client's checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Press {
    /// The search found a spot (`Some`), or there is no ground query to search with (`None`: send the
    /// item's skill as before).
    Send(Option<Vec3>),
    /// An owned slave of the item is near: cancel it with `CSDespawnSlave`.
    Despawn(u32),
    /// Show this `ErrorMessageType` and send nothing.
    Refused(i16),
}

/// Terrain, water and collision as the search sees them. The real adapter is [`WorldSummon`]; the
/// tests use a synthetic one.
pub trait SummonSpace: Send + Sync {
    fn terrain_height(&self, x: f32, y: f32) -> Option<f32>;
    fn water_level(&self, x: f32, y: f32) -> Option<f32>;
    fn ray_blocked(&self, from: Vec3, to: Vec3) -> bool;
    fn box_blocked(&self, center: Vec3, yaw: f32, half: Vec3, terrain: bool) -> bool;
    /// Whether the ground queries exist at all. Without terrain and collision the client cannot run the
    /// search and must not refuse a summon it cannot judge.
    fn has_world(&self) -> bool {
        true
    }
}

/// The app's world queries behind [`SummonSpace`] (`None` where the resource is absent).
#[derive(Clone, Default)]
pub struct WorldSummon {
    pub terrain: Option<TerrainQuery>,
    pub water: Option<WaterQuery>,
    pub collision: Option<WorldCollision>,
}

/// A [`SummonSpace`] a test can put in the app instead of the world's queries.
#[derive(Resource, Clone)]
pub struct SummonWorld(pub Arc<dyn SummonSpace>);

impl SummonSpace for WorldSummon {
    fn terrain_height(&self, x: f32, y: f32) -> Option<f32> {
        self.terrain
            .as_ref()
            .or_else(|| self.collision.as_ref().map(WorldCollision::terrain))
            .and_then(|t| t.height_at(x, y))
    }

    fn water_level(&self, x: f32, y: f32) -> Option<f32> {
        // The volume query also wants a z for its floor; anything over the surface is fine.
        self.water
            .as_ref()
            .and_then(|w| w.water_at(Vec3::new(x, y, 1.0e6)))
            .map(|(height, _)| height)
    }

    fn ray_blocked(&self, from: Vec3, to: Vec3) -> bool {
        let Some(collision) = &self.collision else {
            return false;
        };
        let delta = to - from;
        let length = delta.length();
        (length > 1.0e-4) && collision.raycast(from, delta, length).is_some()
    }

    fn box_blocked(&self, center: Vec3, yaw: f32, half: Vec3, terrain: bool) -> bool {
        if terrain && terrain_overlaps_box(center, yaw, half, |x, y| self.terrain_height(x, y)) {
            return true;
        }
        let Some(collision) = &self.collision else {
            return false;
        };
        let reach = half.length().max(0.1);
        collision
            .triangles_near(center, reach, aa_world::collide::Mask::ALL)
            .iter()
            .any(|tri| tri_box_overlap(*tri, center, yaw, half))
    }

    fn has_world(&self) -> bool {
        self.terrain.is_some() || self.collision.is_some()
    }
}

/// Whether the terrain reaches the box: a point at or over the box's bottom blocks it, even when it is
/// over the top (a quay or a cliff over the summon spot), as the engine's terrain box query does.
pub fn terrain_overlaps_box(
    center: Vec3,
    yaw: f32,
    half: Vec3,
    height: impl Fn(f32, f32) -> Option<f32>,
) -> bool {
    for (sx, sy) in [
        (0.0, 0.0),
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
    ] {
        let p = center + rotate(Vec3::new(sx * half.x, sy * half.y, 0.0), yaw);
        if let Some(ground) = height(p.x, p.y)
            && ground >= center.z - half.z
        {
            return true;
        }
    }
    false
}

/// The box the search places at a candidate (the model's box turned to the player's yaw + pi / 2).
fn box_at(center: Vec3, yaw: f32, offset: Vec3) -> Vec3 {
    center + rotate(offset, yaw)
}

/// Rotate `v` about Z (game axes, Z up) by `yaw`.
fn rotate(v: Vec3, yaw: f32) -> Vec3 {
    let (sin, cos) = yaw.sin_cos();
    Vec3::new(cos * v.x - sin * v.y, sin * v.x + cos * v.y, v.z)
}

/// The first spot the 1.2 search accepts, or `None` (the caller shows the space error).
pub fn find_spot(
    slave: &SummonSlave,
    player: Vec3,
    yaw: f32,
    space: &dyn SummonSpace,
) -> Option<Vec3> {
    let forward = Vec2::new(-yaw.sin(), yaw.cos());
    // `FUN_39347fb0` uses (-local_94, local_90): the +spawn_y_offset candidate is the player's left.
    let left = Vec2::new(-yaw.cos(), -yaw.sin());
    let box_yaw = yaw + std::f32::consts::FRAC_PI_2;
    for step in 0..STEPS {
        let distance = slave.spawn_x_offset + step as f32;
        for lateral in [0.0, slave.spawn_y_offset, -slave.spawn_y_offset] {
            let x = player.x + forward.x * distance + left.x * lateral;
            let y = player.y + forward.y * distance + left.y * lateral;
            let Some(ground) = space.terrain_height(x, y) else {
                continue;
            };
            let z = if slave.ship {
                let Some(water) = space.water_level(x, y) else {
                    continue;
                };
                if ground > water {
                    continue;
                }
                water
            } else {
                if ground - player.z > LAND_CLEARANCE {
                    continue;
                }
                ground
            };
            let spot = Vec3::new(x, y, z);
            if space.ray_blocked(player + Vec3::Z * RAY_LIFT, spot + Vec3::Z * RAY_END_LIFT) {
                continue;
            }
            let box_center = box_at(spot, box_yaw, slave.box_center);
            if space.box_blocked(box_center, box_yaw, slave.half_extents, slave.ship) {
                continue;
            }
            if player.distance(spot) > MAX_RANGE {
                continue;
            }
            return Some(spot);
        }
    }
    None
}

/// What using the summon item does now: cancel an owned slave of the same item, search for a spot, or
/// refuse with one of the two 1.2 messages.
pub fn press(
    slave: &SummonSlave,
    existing: Option<&SlavePositionInfo>,
    player: Vec3,
    yaw: f32,
    space: &dyn SummonSpace,
) -> Press {
    if let Some(existing) = existing.filter(|e| e.template_id == slave.slave_id) {
        let range = if slave.valid_area_range > 0.0 {
            slave.valid_area_range
        } else {
            DEFAULT_VALID_AREA
        };
        let pos = existing.position();
        if player.distance(Vec3::new(pos.x, pos.y, pos.z)) <= range {
            return Press::Despawn(existing.obj_id);
        }
        return Press::Refused(312);
    }
    if !space.has_world() {
        return Press::Send(None);
    }
    match find_spot(slave, player, yaw, space) {
        Some(spot) => Press::Send(Some(spot)),
        None => Press::Refused(100),
    }
}

/// The `[0, 1]` overlap test of a triangle and an axis-aligned box in the box's frame (SAT).
pub fn tri_box_overlap(tri: [Vec3; 3], center: Vec3, yaw: f32, half: Vec3) -> bool {
    let (sin, cos) = yaw.sin_cos();
    let local = |p: Vec3| {
        let d = p - center;
        Vec3::new(cos * d.x + sin * d.y, -sin * d.x + cos * d.y, d.z)
    };
    let [a, b, c] = [local(tri[0]), local(tri[1]), local(tri[2])];
    let min = a.min(b).min(c);
    let max = a.max(b).max(c);
    if min.x > half.x
        || max.x < -half.x
        || min.y > half.y
        || max.y < -half.y
        || min.z > half.z
        || max.z < -half.z
    {
        return false;
    }
    let normal = (b - a).cross(c - a);
    if normal.length_squared() > 1.0e-12 {
        let distance = normal.dot(a);
        let reach = half.x * normal.x.abs() + half.y * normal.y.abs() + half.z * normal.z.abs();
        if distance.abs() > reach {
            return false;
        }
    }
    for edge in [b - a, c - b, a - c] {
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let separate = axis.cross(edge);
            if separate.length_squared() < 1.0e-12 {
                continue;
            }
            let [pa, pb, pc] = [a.dot(separate), b.dot(separate), c.dot(separate)];
            let reach =
                half.x * separate.x.abs() + half.y * separate.y.abs() + half.z * separate.z.abs();
            if pa.min(pb).min(pc) > reach || pa.max(pb).max(pc) < -reach {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat terrain, an optional water rectangle and scripted blockages.
    struct Fake {
        terrain: f32,
        water: Option<(f32, f32, f32, f32, f32)>,
        blocked_ray: Option<Vec3>,
        blocked_box: Option<Vec3>,
        ray_from: std::sync::Mutex<Option<Vec3>>,
        box_terrain: std::sync::Mutex<Option<bool>>,
    }

    impl Fake {
        fn land(terrain: f32) -> Fake {
            Fake {
                terrain,
                water: None,
                blocked_ray: None,
                blocked_box: None,
                ray_from: std::sync::Mutex::new(None),
                box_terrain: std::sync::Mutex::new(None),
            }
        }

        fn sea(terrain: f32, water: f32) -> Fake {
            Fake {
                terrain,
                water: Some((water, -1000.0, -1000.0, 1000.0, 1000.0)),
                blocked_ray: None,
                blocked_box: None,
                ray_from: std::sync::Mutex::new(None),
                box_terrain: std::sync::Mutex::new(None),
            }
        }

        fn blocks(mut self, at: Vec3) -> Fake {
            self.blocked_box = Some(at);
            self
        }

        fn blocks_ray(mut self, at: Vec3) -> Fake {
            self.blocked_ray = Some(at);
            self
        }
    }

    impl SummonSpace for Fake {
        fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
            Some(self.terrain)
        }

        fn water_level(&self, x: f32, y: f32) -> Option<f32> {
            self.water
                .filter(|(_, x0, y0, x1, y1)| x >= *x0 && x <= *x1 && y >= *y0 && y <= *y1)
                .map(|w| w.0)
        }

        fn ray_blocked(&self, from: Vec3, to: Vec3) -> bool {
            *self.ray_from.lock().unwrap() = Some(from);
            self.blocked_ray.is_some_and(|at| (to - at).length() < 0.01)
        }

        fn box_blocked(&self, center: Vec3, _yaw: f32, _half: Vec3, terrain: bool) -> bool {
            *self.box_terrain.lock().unwrap() = Some(terrain);
            self.blocked_box
                .is_some_and(|at| (center - at).length() < 0.01)
        }
    }

    fn slave(x: f32, y: f32, ship: bool) -> SummonSlave {
        SummonSlave {
            slave_id: 21,
            spawn_x_offset: x,
            spawn_y_offset: y,
            ship,
            model_id: 393,
            half_extents: Vec3::new(3.0, 15.0, 7.0),
            box_center: Vec3::ZERO,
            valid_area_range: 50.0,
        }
    }

    #[test]
    fn the_first_candidate_is_straight_ahead_and_first_pass_wins() {
        let space = Fake::land(0.0);
        let spot = find_spot(&slave(5.0, 5.0, false), Vec3::ZERO, 0.0, &space).unwrap();
        assert_eq!(spot, Vec3::new(0.0, 5.0, 0.0));
        // yaw + 90 degrees turns north into west, 0 north counter-clockwise
        let spot = find_spot(
            &slave(5.0, 5.0, false),
            Vec3::ZERO,
            std::f32::consts::FRAC_PI_2,
            &space,
        )
        .unwrap();
        assert!(
            (spot - Vec3::new(-5.0, 0.0, 0.0)).length() < 1.0e-4,
            "{spot}"
        );
    }

    #[test]
    fn the_lateral_order_is_zero_then_plus_then_minus() {
        // the first candidate is 20 m up: a land vehicle may not spawn there, the +y (left) one is fine
        struct Terrain(std::sync::Mutex<usize>);
        impl SummonSpace for Terrain {
            fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
                let mut n = self.0.lock().unwrap();
                let value = *n;
                *n += 1;
                Some(if value == 0 { 20.0 } else { 0.0 })
            }
            fn water_level(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
            fn ray_blocked(&self, _from: Vec3, _to: Vec3) -> bool {
                false
            }
            fn box_blocked(&self, _center: Vec3, _yaw: f32, _half: Vec3, _terrain: bool) -> bool {
                false
            }
        }
        let terrain = Terrain(std::sync::Mutex::new(0));
        let spot = find_spot(&slave(5.0, 2.0, false), Vec3::ZERO, 0.0, &terrain).unwrap();
        assert_eq!(
            spot,
            Vec3::new(-2.0, 5.0, 0.0),
            "+spawn_y_offset is the player's left, before -spawn_y_offset"
        );
    }

    #[test]
    fn a_land_vehicle_needs_ground_within_ten_metres() {
        let high = Fake::land(10.5);
        assert!(find_spot(&slave(0.0, 0.0, false), Vec3::ZERO, 0.0, &high).is_none());
        let edge = Fake::land(10.0);
        assert!(find_spot(&slave(0.0, 0.0, false), Vec3::ZERO, 0.0, &edge).is_some());
        // the height is measured from the player, not the sea
        assert!(
            find_spot(
                &slave(0.0, 0.0, false),
                Vec3::new(0.0, 0.0, 5.0),
                0.0,
                &Fake::land(15.0)
            )
            .is_some()
        );
    }

    #[test]
    fn a_ship_needs_water_at_or_over_the_ground_and_spawns_on_the_surface() {
        let sea = Fake::sea(-20.0, 4.0);
        let spot = find_spot(&slave(10.0, 0.0, true), Vec3::ZERO, 0.0, &sea).unwrap();
        assert_eq!(spot, Vec3::new(0.0, 10.0, 4.0), "z is the water level");
        assert!(
            find_spot(&slave(10.0, 0.0, true), Vec3::ZERO, 0.0, &Fake::land(0.0)).is_none(),
            "no water"
        );
        assert!(
            find_spot(
                &slave(10.0, 0.0, true),
                Vec3::ZERO,
                0.0,
                &Fake::sea(5.0, 4.0)
            )
            .is_none(),
            "ground over water"
        );
        assert!(
            find_spot(
                &slave(10.0, 0.0, true),
                Vec3::ZERO,
                0.0,
                &Fake::sea(4.0, 4.0)
            )
            .is_some(),
            "ground at the level is fine"
        );
    }

    #[test]
    fn a_ray_from_one_metre_over_the_player_blocks_a_candidate() {
        // the ray ends 0.2 m over the ground candidate
        let blocked = Vec3::new(0.0, 5.0, RAY_END_LIFT);
        let space = Fake::land(0.0).blocks_ray(blocked);
        let spot = find_spot(&slave(5.0, 5.0, false), Vec3::ZERO, 0.0, &space).unwrap();
        assert_eq!(
            spot,
            Vec3::new(-5.0, 5.0, 0.0),
            "the next lateral candidate"
        );
        assert_eq!(
            *space.ray_from.lock().unwrap(),
            Some(Vec3::new(0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn a_land_candidate_passes_when_the_ray_ends_on_the_ground() {
        // `terrain_ray` samples the endpoint: a zero clearance at x/y ~15000 is a hit or a miss by
        // float noise. The port stops the ray 0.2 m over the spot, so the result is deterministic.
        struct Endpoint;
        impl SummonSpace for Endpoint {
            fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
                Some(0.0)
            }
            fn water_level(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
            fn ray_blocked(&self, _from: Vec3, to: Vec3) -> bool {
                to.z <= 0.0
            }
            fn box_blocked(&self, _center: Vec3, _yaw: f32, _half: Vec3, _terrain: bool) -> bool {
                false
            }
        }
        let spot = find_spot(
            &slave(5.0, 5.0, false),
            Vec3::new(15000.0, 15000.0, 0.0),
            0.0,
            &Endpoint,
        )
        .expect("the flat land candidate must pass at large coordinates");
        assert_eq!(spot, Vec3::new(15000.0, 15005.0, 0.0));
    }

    #[test]
    fn the_model_box_blocks_a_candidate_and_ships_also_test_the_terrain() {
        let blocked = Vec3::new(0.0, 5.0, 0.0);
        let space = Fake::land(0.0).blocks(blocked);
        assert_eq!(
            find_spot(&slave(5.0, 5.0, false), Vec3::ZERO, 0.0, &space).unwrap(),
            Vec3::new(-5.0, 5.0, 0.0)
        );
        assert_eq!(
            *space.box_terrain.lock().unwrap(),
            Some(false),
            "land vehicles do not test the terrain"
        );
        let space = Fake::sea(-20.0, 0.0).blocks(Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(
            find_spot(&slave(5.0, 5.0, true), Vec3::ZERO, 0.0, &space).unwrap(),
            Vec3::new(-5.0, 5.0, 0.0)
        );
        assert_eq!(
            *space.box_terrain.lock().unwrap(),
            Some(true),
            "ships also test the terrain"
        );
    }

    #[test]
    fn the_box_is_placed_at_the_player_yaw_plus_a_quarter_turn() {
        struct Recording(std::sync::Mutex<Option<f32>>);
        impl SummonSpace for Recording {
            fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
                Some(0.0)
            }
            fn water_level(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
            fn ray_blocked(&self, _from: Vec3, _to: Vec3) -> bool {
                false
            }
            fn box_blocked(&self, _center: Vec3, yaw: f32, _half: Vec3, _terrain: bool) -> bool {
                *self.0.lock().unwrap() = Some(yaw);
                false
            }
        }
        let space = Recording(std::sync::Mutex::new(None));
        find_spot(&slave(0.0, 0.0, false), Vec3::ZERO, 0.5, &space);
        assert!(
            (space.0.lock().unwrap().unwrap() - (0.5 + std::f32::consts::FRAC_PI_2)).abs() < 1.0e-6
        );
    }

    #[test]
    fn the_distance_cap_is_eighty_metres() {
        assert!(find_spot(&slave(80.0, 0.0, false), Vec3::ZERO, 0.0, &Fake::land(0.0)).is_some());
        assert!(find_spot(&slave(80.01, 0.0, false), Vec3::ZERO, 0.0, &Fake::land(0.0)).is_none());
        // the cap is 3D: a ship 79 m ahead with the water 15 m over the player is over 80 m away
        assert!(
            find_spot(
                &slave(79.0, 0.0, true),
                Vec3::ZERO,
                0.0,
                &Fake::sea(-30.0, 15.0)
            )
            .is_none()
        );
    }

    #[test]
    fn the_search_tries_forty_steps() {
        // offset 50: the fortieth candidate is 89 m away, over the cap; none pass
        assert!(find_spot(&slave(50.0, 0.0, false), Vec3::ZERO, 0.0, &Fake::land(0.0)).is_some());
        assert!(find_spot(&slave(81.0, 0.0, false), Vec3::ZERO, 0.0, &Fake::land(0.0)).is_none());
        // the first step inside the cap wins
        assert_eq!(
            find_spot(&slave(50.0, 0.0, false), Vec3::ZERO, 0.0, &Fake::land(0.0)).unwrap(),
            Vec3::new(0.0, 50.0, 0.0)
        );
    }

    #[test]
    fn an_owned_slave_of_the_item_nearby_is_a_cancel_and_far_away_is_refused() {
        let existing = SlavePositionInfo {
            obj_id: 77,
            tl_id: 1,
            name: "Eznan Cutter".into(),
            template_id: 21,
            hp: 1,
            max_hp: 2,
            x: 0,
            y: 0,
            z: 0.0,
        };
        let existing_near = SlavePositionInfo {
            x: aa_net::wire::coord_to_long(10.0),
            ..existing.clone()
        };
        assert_eq!(
            press(
                &slave(5.0, 5.0, true),
                Some(&existing_near),
                Vec3::ZERO,
                0.0,
                &Fake::sea(-20.0, 0.0)
            ),
            Press::Despawn(77)
        );
        let far = SlavePositionInfo {
            x: aa_net::wire::coord_to_long(200.0),
            ..existing.clone()
        };
        assert_eq!(
            press(
                &slave(5.0, 5.0, true),
                Some(&far),
                Vec3::ZERO,
                0.0,
                &Fake::sea(-20.0, 0.0)
            ),
            Press::Refused(312)
        );
        // another slave template leaves the summon path alone
        let other = SlavePositionInfo {
            template_id: 22,
            ..existing
        };
        assert_eq!(
            press(
                &slave(5.0, 5.0, true),
                Some(&other),
                Vec3::ZERO,
                0.0,
                &Fake::sea(-20.0, 0.0)
            ),
            Press::Send(Some(Vec3::new(0.0, 5.0, 0.0)))
        );
    }

    #[test]
    fn no_spot_is_error_100() {
        assert_eq!(
            press(
                &slave(5.0, 5.0, true),
                None,
                Vec3::ZERO,
                0.0,
                &Fake::land(0.0)
            ),
            Press::Refused(100)
        );
    }

    #[test]
    fn no_ground_queries_send_instead_of_refusing_every_candidate() {
        struct NoWorld;
        impl SummonSpace for NoWorld {
            fn terrain_height(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
            fn water_level(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
            fn ray_blocked(&self, _from: Vec3, _to: Vec3) -> bool {
                false
            }
            fn box_blocked(&self, _center: Vec3, _yaw: f32, _half: Vec3, _terrain: bool) -> bool {
                false
            }
            fn has_world(&self) -> bool {
                false
            }
        }
        assert_eq!(
            press(&slave(5.0, 5.0, true), None, Vec3::ZERO, 0.0, &NoWorld),
            Press::Send(None)
        );
    }

    #[test]
    fn terrain_over_the_box_top_blocks_it_too() {
        // a ship box from z 8 to 12 at the water surface: a quay at 20 m is still ground over it
        let center = Vec3::new(0.0, 0.0, 10.0);
        let half = Vec3::new(1.0, 1.0, 2.0);
        assert!(terrain_overlaps_box(center, 0.0, half, |_, _| Some(20.0)));
        assert!(terrain_overlaps_box(center, 0.0, half, |_, _| Some(8.0)));
        assert!(!terrain_overlaps_box(center, 0.0, half, |_, _| Some(7.9)));
        assert!(!terrain_overlaps_box(center, 0.0, half, |_, _| None));
    }

    #[test]
    #[ignore = "requires the read-only game database and pak"]
    fn the_captured_cutter_item_maps_to_slave_21() {
        let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
        if !std::path::Path::new(&pak).exists() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let game = super::super::data::GameData::load(&path).expect("game data");
        let slave = game
            .summon_slaves
            .get(&5223)
            .expect("Eznan Cutter summon item");
        assert_eq!(
            (
                slave.slave_id,
                slave.ship,
                slave.spawn_x_offset,
                slave.spawn_y_offset
            ),
            (21, true, 30.0, 20.0)
        );
        assert!(
            slave.half_extents.x > 0.0 && slave.half_extents.y > 0.0 && slave.half_extents.z > 0.0,
            "{:?}",
            slave.half_extents
        );
        // Carriage 715 and the two other slave-kind-1 rows 1197 / 26873 have VehicleModel models:
        // the client's ship test is the model type, not the slave kind.
        for item in [715, 1197, 26873] {
            let vehicle = game
                .summon_slaves
                .get(&item)
                .unwrap_or_else(|| panic!("summon item {item}"));
            assert!(!vehicle.ship, "item {item} is a land vehicle");
        }
    }

    #[test]
    fn a_triangle_inside_the_box_overlaps_and_one_outside_does_not() {
        let half = Vec3::new(1.0, 1.0, 1.0);
        let inside = [
            Vec3::new(-0.2, -0.2, 0.0),
            Vec3::new(0.2, -0.2, 0.0),
            Vec3::new(0.0, 0.2, 0.0),
        ];
        assert!(tri_box_overlap(inside, Vec3::ZERO, 0.0, half));
        let outside = [
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(2.5, 1.0, 0.0),
        ];
        assert!(!tri_box_overlap(outside, Vec3::ZERO, 0.0, half));
        // a long triangle crossing the box, all of its corners outside
        let crossing = [
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 5.0),
        ];
        assert!(tri_box_overlap(crossing, Vec3::ZERO, 0.0, half));
        // turning the box by 45 degrees covers a corner the axis-aligned box does not
        let corner = [
            Vec3::new(1.1, 0.1, 0.0),
            Vec3::new(1.3, 0.1, 0.0),
            Vec3::new(1.1, 0.3, 0.0),
        ];
        assert!(!tri_box_overlap(corner, Vec3::ZERO, 0.0, half));
        assert!(tri_box_overlap(
            corner,
            Vec3::ZERO,
            std::f32::consts::FRAC_PI_4,
            half
        ));
    }
}
