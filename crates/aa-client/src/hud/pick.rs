//! Picking a unit in the 3D view: the left click's target and the right click's NPC or vehicle.
//!
//! A click is a ray through the view. A small unit is picked when the ray passes through the 1.2
//! client's 1.1 m pick sphere around its middle. A vehicle (a [`UnitKind::Slave`]) is also picked
//! by the box around the meshes it is drawn with, because most of a 4 m Farm Wagon lies outside
//! that sphere; 1.2 lets the vehicle you ride be targeted (2026-10-28b capture case e, the
//! `bound` row: a right click on the seated slave sends `CSChangeTarget` 0x02c).
//!
//! The box is only a fallback, and a loose one: it is the union of every drawn mesh, so a ship's
//! masts and sails enlarge it well beyond the hull. A unit whose sphere the ray hits therefore
//! wins over every vehicle box, wherever along the ray (a passenger on a deck is picked from
//! shore, not the ship); the box only decides when no sphere is hit. A box that contains the
//! camera is ignored too (a camera on deck sits inside the ship's box and would otherwise pick the
//! ship on every click), an attached child (a cannon) wins over its parent only while its hit lies
//! inside the parent's span along the ray, and the slave the player rides gets no box at all, only
//! its sphere. Making the box hug the hull would need the mesh geometry or a hull material/mesh
//! marker, neither of which the draw tree carries; the capture's own pick landed on only 4 of 44
//! sweep points (`captures/2026-10-28b/summary.txt:36`, case e), so the exact 1.2 pick shape stays
//! capture-pending.

use std::collections::HashMap;

use aa_fx::FxRun;
use aa_net::model::{Unit, UnitKind};
use aa_net::state::Units;
use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;

use crate::ingame::units::{RemoteDoodad, RemoteUnit, Views};

/// How near the view ray has to pass a small unit's middle for a click to select it, metres.
const PICK_RADIUS: f32 = 1.1;
/// The middle of the old pick sphere, above the unit's feet (fraction of its scale).
const CENTRE_LIFT: f32 = 0.9;
/// The pick-box cache holds at most this many units; it is cleared rather than grown without bound
/// when object ids churn (respawns, teleports).
const CACHE_LIMIT: usize = 1024;
/// How many parent links the child-over-parent preference follows (the attach tree is shallow).
const MAX_ATTACH_DEPTH: usize = 8;

/// A unit's model box in the Bevy world: the centre, the half extents along its own axes, and the
/// rotation those axes have in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PickBox {
    pub centre: Vec3,
    pub half: Vec3,
    pub rotation: Quat,
}

impl PickBox {
    /// Whether `point` is inside the box.
    pub(crate) fn contains(&self, point: Vec3) -> bool {
        let local = self.rotation.inverse() * (point - self.centre);
        local.abs().cmple(self.half).all()
    }

    /// The span of the box along `ray`: how far it is entered and where it is left. `None` when
    /// the ray misses it or the whole box is behind the ray's origin.
    fn span(&self, ray: &Ray3d) -> Option<(f32, f32)> {
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
        (far >= 0.0).then_some((near.max(0.0), far))
    }
}

/// The middle of a unit's old pick sphere, in the Bevy world. `pos` is the unit's world position in
/// game coordinates (the drawn pose for an attached unit, not its parent-local packet position).
fn unit_centre(pos: Vec3, scale: f32) -> Vec3 {
    aa_world::cry_to_bevy(Vec3::new(
        pos.x,
        pos.y,
        pos.z + CENTRE_LIFT * scale.max(0.5),
    ))
}

/// How far along `ray` enters a sphere, `None` for a miss or a sphere behind the ray's origin. This
/// is the entry point, not the distance to the middle, so a sphere and a box compare like with like.
fn sphere_hit(ray: &Ray3d, centre: Vec3, radius: f32) -> Option<f32> {
    let offset = ray.origin - centre;
    let along = -offset.dot(*ray.direction);
    if along <= 0.0 {
        return None;
    }
    let perpendicular2 = offset.length_squared() - along * along;
    let radius2 = radius * radius;
    (perpendicular2 <= radius2).then(|| (along - (radius2 - perpendicular2).sqrt()).max(0.0))
}

/// How far along `ray` it passes the unit's pick-sphere middle, if it passes inside the sphere.
/// This is the middle comparison for readers that already order by middle (the doodad pick); the
/// click pick uses [`sphere_hit`]'s entry.
pub(crate) fn unit_middle_hit(ray: &Ray3d, unit: &Unit, world: Option<Vec3>) -> Option<f32> {
    let at = world.unwrap_or(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z));
    let centre = unit_centre(at, unit.scale);
    let radius = PICK_RADIUS * unit.scale.max(0.7);
    let offset = ray.origin - centre;
    let along = -offset.dot(*ray.direction);
    let perpendicular2 = offset.length_squared() - along * along;
    (along > 0.0 && perpendicular2 <= radius * radius).then_some(along)
}

/// The unit the ray picks among the ones `accept` takes (never the local player), with `data`
/// giving a unit's world position and drawn model box. A unit whose pick sphere is hit wins over
/// every model box, wherever along the ray; among sphere hits the nearest entry wins. The box only
/// decides when no sphere is hit: its nearest entry wins, and an attached child (a cannon) wins
/// over its parent only while the child's hit lies inside the parent's span along the ray. The
/// slave the player rides (`ridden`) gets no box at all. A miss is logged at debug level with the
/// nearest unit's name, so an owner report can be checked from the log.
pub(crate) fn pick_unit(
    ray: &Ray3d,
    units: &Units,
    own: u32,
    ridden: Option<u32>,
    mut accept: impl FnMut(&Unit) -> bool,
    mut data: impl FnMut(&Unit, &Ray3d) -> (Option<Vec3>, Option<PickBox>),
) -> Option<u32> {
    let mut spheres: Vec<(f32, u32)> = Vec::new();
    // a box hit: entry, exit and the unit
    let mut boxes: Vec<(f32, f32, u32)> = Vec::new();
    let mut nearest_miss: Option<(f32, String)> = None;
    for unit in units.iter().filter(|unit| unit.obj_id != own) {
        if !accept(unit) {
            continue;
        }
        let (at, model) = data(unit, ray);
        let at = at.unwrap_or(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z));
        let sphere = sphere_hit(
            ray,
            unit_centre(at, unit.scale),
            PICK_RADIUS * unit.scale.max(0.7),
        );
        let span = if Some(unit.obj_id) == ridden {
            None
        } else {
            model
        }
        .filter(|model| !model.contains(ray.origin))
        .and_then(|model| model.span(ray));
        if let Some(along) = sphere {
            spheres.push((along, unit.obj_id));
        }
        if let Some((near, far)) = span {
            boxes.push((near, far, unit.obj_id));
        }
        if sphere.is_none() && span.is_none() {
            let centre = unit_centre(at, unit.scale);
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
    // a passenger on a deck or in a wagon bed is inside the vehicle's loose box; the sphere is
    // what the click aimed at
    if let Some((_, chosen)) = spheres.iter().min_by(|a, b| a.0.total_cmp(&b.0)) {
        return Some(*chosen);
    }
    let Some(&(near, far, mut chosen)) = boxes.iter().min_by(|a, b| a.0.total_cmp(&b.0)) else {
        if let Some((miss, name)) = nearest_miss {
            debug!("hud: pick: no unit under the ray (nearest miss {miss:.2} m: {name})");
        }
        return None;
    };
    let mut span = (near, far);
    // a cannon on a ship is inside the ship's box, so its entry is farther; the click still means
    // the cannon the player aimed at, but only while the child is inside the parent's span (a
    // cannon far behind the clicked bow is not what the click aims at)
    for _ in 0..MAX_ATTACH_DEPTH {
        let Some((child_near, child_far, child)) = boxes
            .iter()
            .filter(|(child_near, _, id)| {
                *id != chosen
                    && *child_near >= span.0
                    && *child_near <= span.1
                    && units.get(*id).and_then(crate::ingame::units::unit_parent) == Some(chosen)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .copied()
        else {
            break;
        };
        chosen = child;
        span = (child_near, child_far);
    }
    Some(chosen)
}

/// The current world pose of units, for readers outside the view systems (examples and tests have
/// no drawn roots, and read the parent-composed packet pose instead).
#[derive(SystemParam)]
pub(crate) struct UnitPoses<'w, 's> {
    views: Option<Res<'w, Views>>,
    transforms: Query<'w, 's, &'static Transform>,
    parents: Query<'w, 's, &'static ChildOf>,
}

impl UnitPoses<'_, '_> {
    /// The unit's game-coordinate position and yaw: the drawn root's current transform chain when
    /// it has one (so an attached cannon is at its world position), else the parent-composed packet
    /// pose.
    pub(crate) fn get(&self, unit: &Unit, units: &Units) -> Option<(Vec3, f32)> {
        match self.views.as_deref() {
            Some(views) => crate::ingame::units::unit_world_pose(
                unit,
                units,
                views,
                &self.transforms,
                &self.parents,
            ),
            None => crate::ingame::units::unit_world_pose_in(unit, units),
        }
    }
}

/// The pick's view inputs together, for a system at Bevy's 16-parameter limit (`talk_input`).
#[derive(SystemParam)]
pub(crate) struct PickWorld<'w, 's> {
    poses: UnitPoses<'w, 's>,
    boxes: ModelBoxes<'w, 's>,
}

impl PickWorld<'_, '_> {
    /// The unit's world pose, as [`UnitPoses::get`].
    pub(crate) fn pose(&self, unit: &Unit, units: &Units) -> Option<(Vec3, f32)> {
        self.poses.get(unit, units)
    }

    /// A unit's world pose and culled model box for one pick, as [`pick_unit`] takes them.
    pub(crate) fn data(
        &mut self,
        unit: &Unit,
        units: &Units,
        ray: &Ray3d,
    ) -> (Option<Vec3>, Option<PickBox>) {
        let pose = self.poses.get(unit, units).map(|(pos, _)| pos);
        (pose, self.boxes.get_near(unit, ray))
    }
}

/// The draw roots that are another object, not part of the vehicle whose box is measured.
type ForeignRoot = Or<(With<RemoteUnit>, With<RemoteDoodad>, With<FxRun>)>;

/// A cached vehicle box, in the drawn root's local frame: the root and model it was measured
/// from, and the box the root's current pose is applied to on every read.
#[derive(Clone, Copy)]
struct CachedBox {
    entity: Entity,
    model_id: u32,
    local: PickBox,
}

/// The drawn model boxes of the vehicles in the view. Measured once from the meshes' [`Aabb`]s
/// (plates measure heights the same way) in the root's local frame, cached per unit until the root
/// or its model changes, and composed with the root's current `Transform` chain on every read, so
/// a moving or turning vehicle is picked where it is now. Only a fully built tree is cached: while
/// any mesh still lacks its bounds the box is not ready.
#[derive(SystemParam)]
pub(crate) struct ModelBoxes<'w, 's> {
    views: Option<Res<'w, Views>>,
    roots: Query<'w, 's, (Entity, &'static RemoteUnit)>,
    globals: Query<'w, 's, &'static GlobalTransform>,
    transforms: Query<'w, 's, &'static Transform>,
    parents: Query<'w, 's, &'static ChildOf>,
    children: Query<'w, 's, &'static Children>,
    meshes: Query<'w, 's, (), With<Mesh3d>>,
    /// Skinned meshes are drawn without an `Aabb` (aa-cry gives them none), so they are not
    /// counted when checking that the tree is fully built.
    skinned: Query<'w, 's, (), With<SkinnedMesh>>,
    bounds: Query<'w, 's, (&'static Aabb, &'static GlobalTransform)>,
    /// Another unit's root, a doodad or an effect: its meshes belong to that object, not this box.
    foreign: Query<'w, 's, (), ForeignRoot>,
    cache: Local<'s, HashMap<u32, CachedBox>>,
}

impl ModelBoxes<'_, '_> {
    /// The entity drawing the unit and the model its tree was built from, in O(1) through
    /// [`Views`] when it is present.
    fn entity_for(&self, obj_id: u32) -> Option<(Entity, u32)> {
        let tracked = self
            .views
            .as_deref()
            .and_then(|views| views.unit_entity(obj_id))
            .filter(|entity| self.roots.contains(*entity));
        if let Some(entity) = tracked {
            let (_, remote) = self.roots.get(entity).ok()?;
            return Some((entity, remote.model_id));
        }
        self.roots
            .iter()
            .find(|(_, unit)| unit.obj_id == obj_id)
            .map(|(entity, unit)| (entity, unit.model_id))
    }

    /// The model box of a vehicle, if it is drawn and its tree is fully built. The box is measured
    /// in the root's local frame and placed by the root's current `Transform` chain (not last
    /// frame's `GlobalTransform`). A unit that is not a vehicle has no box at all (its tree's
    /// Aabbs are weapons, attachments or effects).
    pub(crate) fn get(&mut self, unit: &Unit) -> Option<PickBox> {
        if !matches!(unit.kind, UnitKind::Slave { .. }) {
            return None;
        }
        let (entity, model_id) = self.entity_for(unit.obj_id)?;
        let current =
            crate::ingame::units::current_transform(entity, &self.transforms, &self.parents)?;
        let local = match self.cache.get(&unit.obj_id) {
            Some(cached) if cached.entity == entity && cached.model_id == model_id => cached.local,
            _ => {
                let root = *self.globals.get(entity).ok()?;
                let local = self.measure(entity, &root)?;
                if self.cache.len() >= CACHE_LIMIT {
                    self.cache.clear();
                }
                self.cache.insert(
                    unit.obj_id,
                    CachedBox {
                        entity,
                        model_id,
                        local,
                    },
                );
                local
            }
        };
        Some(PickBox {
            centre: current.transform_point(local.centre),
            half: local.half * current.scale.abs(),
            rotation: current.rotation,
        })
    }

    /// The union of the meshes' bounds in `root`'s local frame, or `None` while a mesh of the tree
    /// still lacks its bounds (`Aabb` is added when the mesh asset is loaded, so a box measured
    /// before that would miss part of the model).
    fn measure(&self, root: Entity, root_global: &GlobalTransform) -> Option<PickBox> {
        let to_root = root_global.affine().inverse();
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        let mut meshes = 0;
        let mut measured = 0;
        let mut stack = Vec::new();
        // the root has no Aabb of its own: start at its children
        if let Ok(children) = self.children.get(root) {
            stack.extend(children.iter());
        }
        while let Some(entity) = stack.pop() {
            // a child unit (ship cannon), a parented doodad or an fx aura is another object
            if self.foreign.contains(entity) {
                continue;
            }
            let is_mesh = self.meshes.contains(entity) && !self.skinned.contains(entity);
            if is_mesh {
                meshes += 1;
            }
            if let Ok((aabb, global)) = self.bounds.get(entity) {
                if is_mesh {
                    measured += 1;
                }
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
        if !any || measured < meshes {
            return None;
        }
        Some(PickBox {
            centre: (lo + hi) * 0.5,
            half: (hi - lo) * 0.5,
            rotation: Quat::IDENTITY,
        })
    }

    /// [`ModelBoxes::get`] with a cheap cull: a box whose bounding sphere the ray cannot reach is
    /// not needed. A box whose middle is behind the ray's origin still counts while its reach
    /// crosses the origin (a camera just in front of the hull). A cold (uncached) box is measured
    /// once and then culled this way.
    pub(crate) fn get_near(&mut self, unit: &Unit, ray: &Ray3d) -> Option<PickBox> {
        let model = self.get(unit)?;
        let along = (model.centre - ray.origin).dot(*ray.direction);
        let reach = model.half.length() + PICK_RADIUS * unit.scale.max(0.7);
        if along + reach <= 0.0 {
            return None;
        }
        ((ray.origin + *ray.direction * along).distance(model.centre) <= reach).then_some(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::model::{UnitKind, UnitStateAttachments};
    use bevy::transform::TransformPlugin;

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

    /// The same unit with its packet position written as the given Bevy-world point.
    fn unit_at_bevy(pos: Vec3) -> Unit {
        let mut unit = unit_at(Vec3::ZERO);
        unit.pos = aa_net::Pos::new(pos.x, -pos.z, pos.y);
        unit
    }

    fn npc_at_bevy(obj_id: u32, pos: Vec3) -> Unit {
        let mut unit = unit_at_bevy(pos);
        unit.obj_id = obj_id;
        unit.name = format!("Deck NPC {obj_id}");
        unit.kind = UnitKind::Npc {
            template_id: 1,
            owner_id: 0,
        };
        unit.model_id = 0;
        unit
    }

    fn character_at_bevy(obj_id: u32, pos: Vec3) -> Unit {
        let mut unit = unit_at_bevy(pos);
        unit.obj_id = obj_id;
        unit.name = format!("Deck Player {obj_id}");
        unit.kind = UnitKind::Character { char_id: obj_id };
        unit.model_id = 0;
        unit
    }

    fn units_with(units: &[Unit]) -> Units {
        let mut all = Units::default();
        for unit in units {
            all.by_id.insert(unit.obj_id, unit.clone());
        }
        all
    }

    fn world_raw(unit: &Unit) -> Option<Vec3> {
        Some(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z))
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
        let units = units_with(&[unit]);
        let wagon = Some(wagon_box(0.0));
        // 1.8 m behind the origin: outside the old 1.1 m sphere, inside the model box
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.0, 10.0, 1.8)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), wagon)
            ),
            Some(20)
        );
        // 1.5 m to its side: outside the 1 m half width
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.5, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), wagon)
            ),
            None
        );
    }

    #[test]
    fn a_yawed_wagon_is_picked_along_its_turned_axes() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&[unit]);
        let yawed = Some(wagon_box(std::f32::consts::FRAC_PI_2));
        // a quarter turn: the local +z end now points along world +x
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.8, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), yawed)
            ),
            Some(20)
        );
        // the same point is now beside the wagon, and the old end is beside it too
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.0, 10.0, 1.8)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), yawed)
            ),
            None
        );
    }

    #[test]
    fn a_small_model_keeps_the_old_sphere() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&[unit]);
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
                None,
                |_| true,
                |unit, _| (world_raw(unit), small)
            ),
            Some(20)
        );
        // 1.2 m: outside the sphere as before
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.2, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), small)
            ),
            None
        );
    }

    #[test]
    fn a_camera_inside_a_vehicle_box_picks_the_unit_on_deck() {
        let ship = unit_at(Vec3::ZERO);
        let npc = npc_at_bevy(21, Vec3::new(0.5, 0.0, 0.5));
        let units = units_with(&[ship, npc]);
        let ship_box = PickBox {
            centre: Vec3::new(0.0, 1.0, 0.0),
            half: Vec3::new(10.0, 5.0, 10.0),
            rotation: Quat::IDENTITY,
        };
        let ray = down(Vec3::new(0.5, 3.0, 0.5));
        assert!(
            ship_box.contains(ray.origin),
            "the camera stands inside the ship's box"
        );
        assert_eq!(
            pick_unit(
                &ray,
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), (unit.obj_id == 20).then_some(ship_box))
            ),
            Some(21)
        );
    }

    #[test]
    fn a_sphere_hit_wins_over_a_vehicle_box_wherever_it_lies_on_the_ray() {
        // the wagon's own pick sphere misses the ray (2.4 m to its middle); only its box is hit
        let wagon = unit_at_bevy(Vec3::new(3.0, 0.0, 0.0));
        let npc = npc_at_bevy(21, Vec3::new(0.6, 0.0, 0.0));
        let units = units_with(&[wagon, npc]);
        // the box is entered at 8.5 m (top 1.5 m, ray from y=10); the NPC sphere's middle is
        // 9.1 m away but its surface is entered at 8.18 m, and the sphere wins anyway
        let wagon_box = PickBox {
            centre: Vec3::new(3.0, 0.75, 0.0),
            half: Vec3::new(3.0, 0.75, 2.0),
            rotation: Quat::IDENTITY,
        };
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.6, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), (unit.obj_id == 20).then_some(wagon_box))
            ),
            Some(21)
        );
    }

    #[test]
    fn an_attached_cannons_sphere_wins_over_the_ship_box_around_it() {
        let ship = unit_at(Vec3::ZERO);
        // 1.3 m off the ship's middle, so the ship's own sphere misses; the cannon's sphere hits
        let mut cannon = unit_at_bevy(Vec3::new(1.3, 0.0, -0.9));
        cannon.obj_id = 21;
        cannon.name = "Ship Cannon".into();
        cannon.state_attachments = Some(UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(20),
            bonding_obj_id: None,
        });
        let units = units_with(&[ship, cannon]);
        let ship_box = PickBox {
            centre: Vec3::new(0.0, 1.0, 0.0),
            half: Vec3::new(2.0, 0.5, 2.0),
            rotation: Quat::IDENTITY,
        };
        // ship box entry 8.5 m, cannon sphere entry 8.47 m: the cannon wins
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.3, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), (unit.obj_id == 20).then_some(ship_box))
            ),
            Some(21)
        );
    }

    #[test]
    fn an_attached_child_box_wins_inside_the_parents_span() {
        let ship = unit_at(Vec3::ZERO);
        // middle (0, 4, 0): the sphere misses the ray at y 2.5, the 1 m box is hit
        let mut cannon = unit_at_bevy(Vec3::new(0.0, 3.1, 0.0));
        cannon.obj_id = 21;
        cannon.name = "Deck Cannon".into();
        cannon.state_attachments = Some(UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(20),
            bonding_obj_id: None,
        });
        let units = units_with(&[ship, cannon]);
        let ship_box = PickBox {
            centre: Vec3::new(0.0, 1.0, 0.0),
            half: Vec3::new(2.0, 2.0, 2.0),
            rotation: Quat::IDENTITY,
        };
        let cannon_box = PickBox {
            centre: Vec3::new(0.0, 2.5, 0.0),
            half: Vec3::new(0.5, 0.5, 0.5),
            rotation: Quat::IDENTITY,
        };
        // the ship is entered at 8 m and left at 12 m; the cannon at 9.5 m, inside that span
        let ray = Ray3d::new(
            Vec3::new(0.0, 2.5, -10.0),
            Dir3::new(Vec3::Z).expect("forward"),
        );
        assert_eq!(
            pick_unit(
                &ray,
                &units,
                0,
                None,
                |_| true,
                |unit, _| (
                    world_raw(unit),
                    match unit.obj_id {
                        20 => Some(ship_box),
                        21 => Some(cannon_box),
                        _ => None,
                    }
                )
            ),
            Some(21)
        );
    }

    #[test]
    fn an_npc_in_the_wagon_bed_beats_the_wagon_box_from_outside() {
        let wagon = unit_at(Vec3::ZERO);
        let npc = npc_at_bevy(21, Vec3::new(0.5, 0.0, 0.5));
        let units = units_with(&[wagon, npc]);
        let wagon_box = PickBox {
            centre: Vec3::new(0.0, 1.5, 0.0),
            half: Vec3::new(3.0, 1.5, 3.0),
            rotation: Quat::IDENTITY,
        };
        let ray = down(Vec3::new(0.5, 10.0, 0.5));
        assert!(
            !wagon_box.contains(ray.origin),
            "the camera stands outside the wagon"
        );
        // the box is entered at 7 m, the NPC's sphere at 8 m: the sphere still wins
        assert_eq!(
            pick_unit(
                &ray,
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), (unit.obj_id == 20).then_some(wagon_box))
            ),
            Some(21)
        );
    }

    #[test]
    fn a_player_on_a_ship_deck_beats_the_ship_box_from_shore() {
        let ship = unit_at(Vec3::ZERO);
        let player = character_at_bevy(31, Vec3::new(0.5, 0.0, 0.5));
        let units = units_with(&[ship, player]);
        let ship_box = PickBox {
            centre: Vec3::new(0.0, 1.5, 0.0),
            half: Vec3::new(3.0, 1.5, 3.0),
            rotation: Quat::IDENTITY,
        };
        let ray = down(Vec3::new(0.5, 10.0, 0.5));
        assert!(!ship_box.contains(ray.origin), "the camera stands on shore");
        assert_eq!(
            pick_unit(
                &ray,
                &units,
                0,
                None,
                |_| true,
                |unit, _| (world_raw(unit), (unit.obj_id == 20).then_some(ship_box))
            ),
            Some(31)
        );
    }

    #[test]
    fn a_seated_player_targets_a_mob_and_the_wagon_body_but_not_its_loose_box() {
        let wagon = unit_at(Vec3::ZERO);
        let mob = npc_at_bevy(21, Vec3::new(0.0, 0.0, -1.5));
        let units = units_with(&[wagon, mob]);
        let wagon_box = PickBox {
            centre: Vec3::new(0.0, 0.5, 0.0),
            half: Vec3::new(2.0, 0.5, 3.0),
            rotation: Quat::IDENTITY,
        };
        let data =
            |unit: &Unit, _: &Ray3d| (world_raw(unit), (unit.obj_id == 20).then_some(wagon_box));
        // a mob ahead is a sphere hit, not the ridden wagon's box
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.0, 10.0, -1.5)),
                &units,
                0,
                Some(20),
                |_| true,
                data
            ),
            Some(21)
        );
        // the body's middle is inside the ridden wagon's own sphere
        assert_eq!(
            pick_unit(
                &down(Vec3::new(0.5, 10.0, 0.5)),
                &units,
                0,
                Some(20),
                |_| true,
                data
            ),
            Some(20)
        );
        // a point only the loose box covers is not the ridden wagon
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.5, 10.0, 0.0)),
                &units,
                0,
                Some(20),
                |_| true,
                data
            ),
            None
        );
    }

    #[test]
    fn a_stern_cannon_behind_a_bow_click_does_not_win() {
        let ship = unit_at(Vec3::ZERO);
        // middle (0, 4, 5): its sphere misses the ray at y 2.5, its box (2 m tall) is hit
        let mut cannon = unit_at_bevy(Vec3::new(0.0, 3.1, 5.0));
        cannon.obj_id = 21;
        cannon.name = "Stern Cannon".into();
        cannon.state_attachments = Some(UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(20),
            bonding_obj_id: None,
        });
        let units = units_with(&[ship, cannon]);
        let ship_box = PickBox {
            centre: Vec3::new(0.0, 1.0, 0.0),
            half: Vec3::new(2.0, 2.0, 2.0),
            rotation: Quat::IDENTITY,
        };
        let cannon_box = PickBox {
            centre: Vec3::new(0.0, 4.0, 5.0),
            half: Vec3::new(0.5, 2.0, 0.5),
            rotation: Quat::IDENTITY,
        };
        // the ship is entered at 8 m and left at 12 m; the cannon is entered at 14.5 m, behind
        // the whole ship box, so the bow click is the ship
        let ray = Ray3d::new(
            Vec3::new(0.0, 2.5, -10.0),
            Dir3::new(Vec3::Z).expect("forward"),
        );
        assert_eq!(
            pick_unit(
                &ray,
                &units,
                0,
                None,
                |_| true,
                |unit, _| (
                    world_raw(unit),
                    match unit.obj_id {
                        20 => Some(ship_box),
                        21 => Some(cannon_box),
                        _ => None,
                    }
                )
            ),
            Some(20)
        );
    }

    #[derive(Resource, Default)]
    struct PickOut(Option<u32>);

    #[test]
    fn a_cannon_on_a_yawed_ship_is_picked_at_its_world_position() {
        let mut units = Units::default();
        let mut ship = unit_at(Vec3::ZERO);
        ship.obj_id = 20;
        ship.rotation = [0.0, 0.0, 0.0];
        let mut cannon = unit_at(Vec3::new(5.0, 0.0, 0.0));
        cannon.obj_id = 21;
        cannon.state_attachments = Some(UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(20),
            bonding_obj_id: None,
        });
        units.by_id.insert(20, ship.clone());
        units.by_id.insert(21, cannon.clone());

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .init_resource::<Views>()
            .insert_resource(units)
            .init_resource::<PickOut>()
            .add_systems(Update, pick_cannon_from_world_pose);
        let ship_root = app
            .world_mut()
            .spawn((
                Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                RemoteUnit::announced(&ship),
            ))
            .id();
        let cannon_root = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
                RemoteUnit::announced(&cannon),
                ChildOf(ship_root),
            ))
            .id();
        {
            let mut views = app.world_mut().resource_mut::<Views>();
            views.track_unit_entity(20, ship_root);
            views.track_unit_entity(21, cannon_root);
        }
        app.update();
        // a quarter turn puts the cannon's local +x at world -z: the pick centre is (0, 0.9, -5)
        assert_eq!(app.world().resource::<PickOut>().0, Some(21));
    }

    fn pick_cannon_from_world_pose(poses: UnitPoses, units: Res<Units>, mut out: ResMut<PickOut>) {
        let ray = down(Vec3::new(0.0, 10.0, -5.0));
        out.0 = pick_unit(
            &ray,
            &units,
            0,
            None,
            |_| true,
            |unit, _| (poses.get(unit, &units).map(|(pos, _)| pos), None),
        );
    }

    #[derive(Resource)]
    struct CachedUnit(Unit);

    #[derive(Resource, Default)]
    struct BoxOut(Option<PickBox>);

    fn read_box(mut boxes: ModelBoxes, unit: Res<CachedUnit>, mut out: ResMut<BoxOut>) {
        out.0 = boxes.get(&unit.0);
    }

    fn box_app(unit: Unit) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .init_resource::<Views>()
            .insert_resource(CachedUnit(unit))
            .init_resource::<BoxOut>()
            .add_systems(Update, read_box);
        app
    }

    #[test]
    fn a_vehicle_box_ignores_child_units_effects_and_doodads() {
        let unit = unit_at(Vec3::ZERO);
        let mut child_unit = unit_at(Vec3::new(100.0, 0.0, 0.0));
        child_unit.obj_id = 21;
        child_unit.state_attachments = Some(UnitStateAttachments {
            first_point: 1,
            first_obj_id: Some(20),
            bonding_obj_id: None,
        });
        let mut app = box_app(unit.clone());
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&unit)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(20, root);
        app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(root),
        ));
        // a child unit, an fx aura and a parented doodad, each with a box of its own
        let huge = Aabb::from_min_max(Vec3::splat(-100.0), Vec3::splat(100.0));
        app.world_mut().spawn((
            huge,
            Transform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
            GlobalTransform::from_translation(Vec3::new(100.0, 0.0, 0.0)),
            RemoteUnit::announced(&child_unit),
            ChildOf(root),
        ));
        app.world_mut().spawn((
            huge,
            Transform::from_translation(Vec3::new(0.0, 100.0, 0.0)),
            GlobalTransform::from_translation(Vec3::new(0.0, 100.0, 0.0)),
            FxRun::default(),
            ChildOf(root),
        ));
        app.world_mut().spawn((
            huge,
            Transform::from_translation(Vec3::new(0.0, 0.0, 100.0)),
            GlobalTransform::from_translation(Vec3::new(0.0, 0.0, 100.0)),
            RemoteDoodad { obj_id: 7 },
            ChildOf(root),
        ));
        app.update();
        let model = app.world().resource::<BoxOut>().0.expect("the hull's box");
        assert_eq!(model.centre, Vec3::ZERO);
        assert_eq!(model.half, Vec3::splat(1.0));
    }

    #[test]
    fn a_character_has_no_box_from_its_weapon_aabb() {
        let mut character = unit_at(Vec3::ZERO);
        character.obj_id = 30;
        character.kind = UnitKind::Character { char_id: 47 };
        let mut app = box_app(character.clone());
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&character)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(30, root);
        app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-0.2), Vec3::splat(0.2)),
            Transform::from_translation(Vec3::new(0.5, 1.0, 0.0)),
            GlobalTransform::default(),
            ChildOf(root),
        ));
        app.update();
        assert_eq!(app.world().resource::<BoxOut>().0, None);
    }

    #[test]
    fn a_moved_and_turned_root_composes_its_box_from_the_current_pose() {
        let unit = unit_at(Vec3::ZERO);
        let units = units_with(&[unit.clone()]);
        let mut app = box_app(unit.clone());
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&unit)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(20, root);
        app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-2.0), Vec3::splat(2.0)),
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(root),
        ));
        app.update();
        assert_eq!(
            app.world().resource::<BoxOut>().0.expect("the hull").centre,
            Vec3::ZERO
        );
        // move and turn the root after the box was measured and cached
        {
            let mut transform = app.world_mut().get_mut::<Transform>(root).unwrap();
            transform.translation = Vec3::new(20.0, 0.0, 0.0);
            transform.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        }
        app.update();
        let model = app.world().resource::<BoxOut>().0.expect("the moved box");
        assert!(
            model.centre.abs_diff_eq(Vec3::new(20.0, 0.0, 0.0), 1e-4),
            "the box follows the root: {model:?}"
        );
        let data = |_: &Unit, _: &Ray3d| (None, Some(model));
        assert_eq!(
            pick_unit(
                &down(Vec3::new(21.5, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                data
            ),
            Some(20),
            "the new place is picked"
        );
        assert_eq!(
            pick_unit(
                &down(Vec3::new(1.5, 10.0, 0.0)),
                &units,
                0,
                None,
                |_| true,
                data
            ),
            None,
            "the old place is not picked"
        );
    }

    #[test]
    fn a_partial_model_is_not_measured_or_cached() {
        let unit = unit_at(Vec3::ZERO);
        let mut app = box_app(unit.clone());
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&unit)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(20, root);
        app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            Transform::default(),
            GlobalTransform::default(),
            Mesh3d::default(),
            ChildOf(root),
        ));
        // a second mesh whose asset (and so its Aabb) has not loaded yet
        let later = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Transform::from_translation(Vec3::new(10.0, 0.0, 0.0)),
                ChildOf(root),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<BoxOut>().0,
            None,
            "a box measured without the second mesh would be too small"
        );
        app.world_mut().entity_mut(later).insert((
            Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
            GlobalTransform::from_translation(Vec3::new(10.0, 0.0, 0.0)),
        ));
        app.update();
        let model = app.world().resource::<BoxOut>().0.expect("the full box");
        assert_eq!(model.half, Vec3::new(6.0, 1.0, 1.0));
        // now that the tree is complete the box is cached
        app.world_mut()
            .entity_mut(later)
            .insert(Aabb::from_min_max(Vec3::splat(-2.0), Vec3::splat(2.0)));
        app.update();
        assert_eq!(
            app.world()
                .resource::<BoxOut>()
                .0
                .expect("the cached box")
                .half,
            Vec3::new(6.0, 1.0, 1.0)
        );
    }

    #[derive(Resource, Default)]
    struct NearOut(Option<PickBox>);

    fn read_near(mut boxes: ModelBoxes, unit: Res<CachedUnit>, mut out: ResMut<NearOut>) {
        let ray = Ray3d::new(
            Vec3::new(0.0, 0.0, 1.0),
            Dir3::new(Vec3::Z).expect("forward"),
        );
        out.0 = boxes.get_near(&unit.0, &ray);
    }

    #[test]
    fn a_box_with_its_middle_just_behind_the_camera_is_still_culled_in() {
        let unit = unit_at(Vec3::ZERO);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .init_resource::<Views>()
            .insert_resource(CachedUnit(unit.clone()))
            .init_resource::<NearOut>()
            .add_systems(Update, read_near);
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&unit)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(20, root);
        app.world_mut().spawn((
            Aabb::from_min_max(Vec3::splat(-2.0), Vec3::splat(2.0)),
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(root),
        ));
        app.update();
        assert!(
            app.world().resource::<NearOut>().0.is_some(),
            "the box crosses the ray's origin; only its middle is behind"
        );
    }

    #[test]
    fn a_pick_box_is_cached_until_the_model_changes() {
        let unit = unit_at(Vec3::ZERO);
        let mut app = box_app(unit.clone());
        let root = app
            .world_mut()
            .spawn((Transform::default(), RemoteUnit::announced(&unit)))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(20, root);
        let hull = app
            .world_mut()
            .spawn((
                Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0)),
                Transform::default(),
                GlobalTransform::default(),
                ChildOf(root),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<BoxOut>().0.expect("the hull").half,
            Vec3::splat(1.0)
        );
        // a changed mesh with the same model is not remeasured (the cache)
        app.world_mut()
            .entity_mut(hull)
            .insert(Aabb::from_min_max(Vec3::splat(-2.0), Vec3::splat(2.0)));
        app.update();
        assert_eq!(
            app.world().resource::<BoxOut>().0.expect("the hull").half,
            Vec3::splat(1.0)
        );
        // a model change invalidates it
        app.world_mut().resource_mut::<CachedUnit>().0.model_id = 999;
        let mut changed = unit;
        changed.model_id = 999;
        app.world_mut()
            .entity_mut(root)
            .insert(RemoteUnit::announced(&changed));
        app.update();
        assert_eq!(
            app.world().resource::<BoxOut>().0.expect("the hull").half,
            Vec3::splat(2.0)
        );
    }
}
