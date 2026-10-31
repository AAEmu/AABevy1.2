//! Picking a unit in the 3D view: the left click's target and the right click's NPC or vehicle.
//!
//! Small units keep the 1.2-sized sphere at their middle ([`PICK_RADIUS`]); a unit whose drawn
//! model is larger than that sphere (a wagon, a ship) is picked by its model box, turned with the
//! unit. 1.2 allows a click anywhere on the vehicle you ride (2026-10-28 capture case e: a right
//! click on the slave you are seated on sends `CSChangeTarget` 0x02c), which a fixed 1.1 m sphere
//! could not reach on a 4 m wagon.

use aa_net::model::Unit;
use aa_net::state::Units;
use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::ingame::units::RemoteUnit;

/// How near the view ray has to pass a small unit's middle for a click to select it, metres.
const PICK_RADIUS: f32 = 1.1;
/// The middle of the old pick sphere, above the unit's feet (fraction of its scale).
const CENTRE_LIFT: f32 = 0.9;

/// A unit's model box in the Bevy world: the centre, the half extents along its own axes, and the
/// rotation those axes have in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickBox {
    pub centre: Vec3,
    pub half: Vec3,
    pub rotation: Quat,
}

impl PickBox {
    /// Whether the box is larger than the old pick sphere would be for this unit: only then does it
    /// replace the sphere, so a small model keeps exactly the click it had.
    fn replaces_sphere(&self, sphere_radius: f32) -> bool {
        self.half.is_finite() && self.half.max_element() > sphere_radius
    }

    /// How far along `ray` the box is entered, if it is.
    fn hit(&self, ray: &Ray3d) -> Option<f32> {
        let inv = self.rotation.inverse();
        let origin = inv * (ray.origin - self.centre);
        let direction = inv * *ray.direction;
        let mut near = 0.0f32;
        let mut far = f32::INFINITY;
        for axis in 0..3 {
            let (o, d, h) = (origin[axis], direction[axis], self.half[axis]);
            if d.abs() < 1e-6 {
                if o.abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o) / d, (h - o) / d);
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            near = near.max(a);
            far = far.min(b);
            if near > far {
                return None;
            }
        }
        (far >= 0.0).then_some(near.max(0.0))
    }
}

/// The middle of a unit's old pick sphere, in the Bevy world.
fn unit_centre(pos: aa_net::Pos, scale: f32) -> Vec3 {
    aa_world::cry_to_bevy(Vec3::new(
        pos.x,
        pos.y,
        pos.z + CENTRE_LIFT * scale.max(0.5),
    ))
}

/// How far along `ray` the unit is hit: its model box when that is larger than the old sphere,
/// else the sphere. `None` for a miss or a unit behind the ray's origin.
pub(crate) fn unit_hit(ray: &Ray3d, unit: &Unit, model: Option<PickBox>) -> Option<f32> {
    let sphere_radius = PICK_RADIUS * unit.scale.max(0.7);
    if let Some(model) = model.filter(|model| model.replaces_sphere(sphere_radius)) {
        return model.hit(ray);
    }
    let centre = unit_centre(unit.pos, unit.scale);
    let along = (centre - ray.origin).dot(*ray.direction);
    if along <= 0.0 {
        return None;
    }
    ((ray.origin + *ray.direction * along).distance(centre) <= sphere_radius).then_some(along)
}

/// The nearest unit the ray picks among the ones `accept` takes (never the local player), with
/// `model` supplying a drawn model box when the unit has one. A miss is logged at debug level with
/// the nearest unit's name, so an owner report can be checked from the log.
pub(crate) fn pick_unit(
    ray: &Ray3d,
    units: &Units,
    own: u32,
    mut accept: impl FnMut(&Unit) -> bool,
    mut model: impl FnMut(u32) -> Option<PickBox>,
) -> Option<u32> {
    let mut best: Option<(f32, u32)> = None;
    let mut nearest_miss: Option<(f32, String)> = None;
    for unit in units.iter().filter(|unit| unit.obj_id != own) {
        if !accept(unit) {
            continue;
        }
        match unit_hit(ray, unit, model(unit.obj_id)) {
            Some(along) if best.is_none_or(|(best, _)| along < best) => {
                best = Some((along, unit.obj_id))
            }
            Some(_) => {}
            None => {
                let centre = unit_centre(unit.pos, unit.scale);
                let along = (centre - ray.origin).dot(*ray.direction);
                let miss = if along > 0.0 {
                    (ray.origin + *ray.direction * along).distance(centre)
                } else {
                    (Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z) - ray.origin).length()
                };
                if nearest_miss.as_ref().is_none_or(|(near, _)| miss < *near) {
                    nearest_miss = Some((miss, unit.name.clone()));
                }
            }
        }
    }
    if best.is_none()
        && let Some((miss, name)) = nearest_miss
    {
        debug!("hud: pick: no unit under the ray (nearest miss {miss:.2} m: {name})");
    }
    best.map(|(_, obj_id)| obj_id)
}

/// The drawn model boxes of the units in the view, in their root's frame and turned with it. Built
/// from the meshes' [`Aabb`]s as they are drawn (plates measure heights the same way).
#[derive(SystemParam)]
pub(crate) struct ModelBoxes<'w, 's> {
    roots: Query<'w, 's, (Entity, &'static RemoteUnit, &'static GlobalTransform)>,
    children: Query<'w, 's, &'static Children>,
    bounds: Query<'w, 's, (&'static Aabb, &'static GlobalTransform)>,
}

impl ModelBoxes<'_, '_> {
    /// The model box of the unit `obj_id`, if it is drawn and has bounds yet.
    pub(crate) fn get(&self, obj_id: u32) -> Option<PickBox> {
        let (root_entity, _, root) = self
            .roots
            .iter()
            .find(|(_, unit, _)| unit.obj_id == obj_id)?;
        let to_root = root.affine().inverse();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        let mut stack = Vec::new();
        // the root has no Aabb of its own: start at its children
        if let Ok(children) = self.children.get(root_entity) {
            stack.extend(children.iter());
        }
        while let Some(entity) = stack.pop() {
            if let Ok((aabb, global)) = self.bounds.get(entity) {
                let (centre, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                let transform = to_root * global.affine();
                for sx in [-1.0, 1.0] {
                    for sy in [-1.0, 1.0] {
                        for sz in [-1.0, 1.0] {
                            let corner =
                                transform.transform_point3(centre + half * Vec3::new(sx, sy, sz));
                            lo = lo.min(corner);
                            hi = hi.max(corner);
                            any = true;
                        }
                    }
                }
            }
            if let Ok(children) = self.children.get(entity) {
                stack.extend(children.iter());
            }
        }
        if !any {
            return None;
        }
        Some(PickBox {
            centre: root.transform_point((lo + hi) * 0.5),
            half: (hi - lo) * 0.5 * root.scale().abs(),
            rotation: root.rotation(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::model::UnitKind;

    fn unit_at(pos: Vec3) -> Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = 20;
        unit.name = "Farm Wagon".into();
        unit.kind = UnitKind::Slave {
            id: 700,
            tl_id: 700,
            template_id: 60,
            summoner_id: 100,
        };
        unit.model_id = 1008;
        unit.pos = aa_net::Pos::new(pos.x, pos.y, pos.z);
        unit
    }

    fn units_with(unit: &Unit) -> Units {
        let mut units = Units::default();
        units.by_id.insert(unit.obj_id, unit.clone());
        units
    }

    /// The Farm Wagon's box for the helper: a 4 m long, 2 m wide, 2.4 m high body.
    fn wagon_box(yaw: f32) -> PickBox {
        PickBox {
            centre: Vec3::new(0.0, 1.2, 0.0),
            half: Vec3::new(1.0, 1.2, 2.0),
            rotation: Quat::from_rotation_y(yaw),
        }
    }

    fn down(origin: Vec3) -> Ray3d {
        Ray3d::new(origin, Dir3::new(Vec3::NEG_Y).expect("down"))
    }

    #[test]
    fn a_ray_on_the_wagons_rear_hits_and_one_beside_it_misses() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&unit);
        let wagon = Some(wagon_box(0.0));
        // 1.8 m behind the origin: outside the old 1.1 m sphere, inside the model box
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.0, 10.0, 1.8)),
                &units,
                0,
                |_| true,
                |_| wagon
            ),
            Some(20)
        );
        // 1.5 m to its side: outside the 1 m half width
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.5, 10.0, 0.0)),
                &units,
                0,
                |_| true,
                |_| wagon
            ),
            None
        );
    }

    #[test]
    fn a_yawed_wagon_is_picked_along_its_turned_axes() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&unit);
        let yawed = Some(wagon_box(std::f32::consts::FRAC_PI_2));
        // a quarter turn: the local +z end now points along world +x
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.8, 10.0, 0.0)),
                &units,
                0,
                |_| true,
                |_| yawed
            ),
            Some(20)
        );
        // the same point is now beside the wagon, and the old end is beside it too
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.0, 10.0, 1.8)),
                &units,
                0,
                |_| true,
                |_| yawed
            ),
            None
        );
    }

    #[test]
    fn a_small_model_keeps_the_old_sphere() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&unit);
        let small = Some(PickBox {
            centre: Vec3::new(0.0, 0.9, 0.0),
            half: Vec3::new(0.3, 0.9, 0.3),
            rotation: Quat::IDENTITY,
        });
        // 1.0 m from the middle: the box misses, the 1.1 m sphere still picks
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.0, 10.0, 0.0)),
                &units,
                0,
                |_| true,
                |_| small
            ),
            Some(20)
        );
        // 1.2 m: outside the sphere as before
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.2, 10.0, 0.0)),
                &units,
                0,
                |_| true,
                |_| small
            ),
            None
        );
    }
}
