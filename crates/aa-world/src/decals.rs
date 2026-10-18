//! Static `object.dat` decals: planar quads and the original terrain/static-object projections.
//! All geometry work runs once on placement workers. Receivers are visible LOD0 CGF triangles,
//! not physics proxies; a decal can therefore mark a wall even when its collision is a box.

use std::collections::HashMap;
use std::sync::Arc;

use aa_cry::{CryLoader, Model};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::TerrainQuery;
use crate::coords::{CELL_SIZE, CELL_UNITS, UNIT_SIZE, cell_origin, cry_to_bevy};
use crate::decal_projection::DeferredProjection;
use crate::placement::{CellObjects, Decal, parse_object_dat};
use crate::roads::{RoadAssets, RoadMesh};

/// The native simple decal is raised 1 mm; a few extra millimetres avoid float precision
/// fighting with the separately transformed terrain/brush meshes at world coordinates.
const SURFACE_OFFSET: f32 = 0.005;
// Deferred receivers are copied once on placement workers, with bounded memory/draw cost.
const MAX_DEFERRED_MESH_TRIANGLES: usize = 65_536;
const MAX_DEFERRED_CELL_TRIANGLES: usize = 262_144;
const MAX_DEFERRED_CELL_MESHES: usize = 256;

#[derive(Clone, Copy)]
struct Vertex {
    pos: Vec3,
    normal: Vec3,
}

#[derive(Clone, Copy)]
struct Projector {
    center: Vec3,
    basis: Mat3,
    inverse: Mat3,
    radius: f32,
    deferred: Option<DeferredProjection>,
}

impl Projector {
    fn new(decal: &Decal) -> Option<Self> {
        if !decal.pos.is_finite()
            || !decal.basis.is_finite()
            || !decal.radius.is_finite()
            || decal.radius < 0.01
            || decal.radius > 128.0
            || decal.basis.determinant().abs() < 1e-6
        {
            return None;
        }
        let extent = (decal.basis.x_axis.abs() + decal.basis.y_axis.abs() + decal.basis.z_axis.abs()) * decal.radius;
        if !extent.is_finite() || extent.max_element() > CELL_SIZE {
            return None;
        }
        Some(Self {
            center: decal.pos,
            basis: decal.basis,
            inverse: decal.basis.inverse(),
            radius: decal.radius,
            deferred: None,
        })
    }

    fn deferred(decal: &Decal) -> Option<Self> {
        let native = DeferredProjection::new(decal)?;
        Some(Self {
            center: native.volume_center,
            basis: native.volume,
            inverse: native.volume.inverse(),
            radius: 1.0,
            deferred: Some(native),
        })
    }

    fn local(self, p: Vec3) -> Vec3 {
        self.inverse * (p - self.center)
    }

    fn bounds(self) -> (Vec3, Vec3) {
        let extent = (self.basis.x_axis.abs() + self.basis.y_axis.abs() + self.basis.z_axis.abs()) * self.radius;
        (self.center - extent, self.center + extent)
    }

    /// Native `CDecal` texgen: S = -up / (2r), T = right / (2r), each centred on 0.5.
    fn uv(self, p: Vec3, normal: Vec3) -> Vec2 {
        if let Some(native) = self.deferred {
            return native.uv(p, normal);
        }
        let relative = p - self.center;
        Vec2::new(-self.basis.y_axis.dot(relative), self.basis.x_axis.dot(relative)) / (2.0 * self.radius)
            + Vec2::splat(0.5)
    }
}

fn overlaps(a: (Vec3, Vec3), b: (Vec3, Vec3)) -> bool {
    a.0.cmple(b.1).all() && b.0.cmple(a.1).all()
}

fn clip(mut polygon: Vec<Vertex>, projector: Projector) -> Vec<Vertex> {
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            if polygon.is_empty() {
                return polygon;
            }
            let distance = |v: Vertex| projector.radius - sign * projector.local(v.pos)[axis];
            let mut a = *polygon.last().unwrap();
            let mut da = distance(a);
            let mut result = Vec::with_capacity(polygon.len() + 1);
            for &b in &polygon {
                let db = distance(b);
                if (da >= 0.0) != (db >= 0.0) {
                    let t = da / (da - db);
                    result.push(Vertex {
                        pos: a.pos.lerp(b.pos, t),
                        normal: a.normal.lerp(b.normal, t),
                    });
                }
                if db >= 0.0 {
                    result.push(b)
                }
                (a, da) = (b, db);
            }
            polygon = result;
        }
    }
    polygon
}

#[derive(Default)]
struct Geometry {
    pos: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    indices: Vec<u32>,
    over_budget: bool,
}

impl Geometry {
    fn polygon(&mut self, polygon: &[Vertex], projector: Projector) {
        if polygon.len() < 3 {
            return;
        }
        if projector.deferred.is_some() && self.indices.len() / 3 + polygon.len() - 2 > MAX_DEFERRED_MESH_TRIANGLES {
            self.over_budget = true;
            return;
        }
        let base = self.pos.len() as u32;
        for v in polygon {
            let normal = v.normal.normalize_or_zero();
            self.pos.push(cry_to_bevy(v.pos + normal * SURFACE_OFFSET).to_array());
            self.normals.push(cry_to_bevy(normal).to_array());
            self.uv.push(projector.uv(v.pos, normal).to_array());
        }
        for i in 1..polygon.len() - 1 {
            if (polygon[i].pos - polygon[0].pos)
                .cross(polygon[i + 1].pos - polygon[0].pos)
                .length_squared()
                > 1e-12
            {
                self.indices.extend([base, base + i as u32, base + i as u32 + 1]);
            }
        }
    }

    fn triangle(&mut self, triangle: [Vertex; 3], projector: Projector) {
        let face = (triangle[1].pos - triangle[0].pos).cross(triangle[2].pos - triangle[0].pos);
        // The native receiver walk rejects faces pointing away from the projection direction.
        if projector.deferred.is_none() && face.dot(projector.basis.z_axis) <= 1e-7 {
            return;
        }
        self.polygon(&clip(triangle.to_vec(), projector), projector);
    }

    fn mesh(self) -> Option<Mesh> {
        if self.over_budget || self.indices.is_empty() {
            return None;
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uv);
        mesh.insert_indices(Indices::U32(self.indices));
        let _ = mesh.generate_tangents();
        Some(mesh)
    }
}

fn planar(projector: Projector) -> Option<Mesh> {
    let right = projector.basis.x_axis * projector.radius;
    let up = projector.basis.y_axis * projector.radius;
    let normal = projector.basis.z_axis.normalize_or_zero();
    let mut geometry = Geometry::default();
    let vertices = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| Vertex {
        pos: projector.center + right * x + up * y,
        normal,
    });
    geometry.polygon(&vertices, projector);
    geometry.mesh()
}

fn terrain_projector(mut projector: Projector, cell: (i32, i32), terrain: &TerrainQuery) -> Option<Projector> {
    let origin = cell_origin(cell.0, cell.1);
    let height = terrain.height_at(origin.x + projector.center.x, origin.y + projector.center.y)?;
    let delta = projector.center.z - height;
    // `CDecalRenderNode::CreateDecalOnTerrain`: only a decal close enough to the ground
    // is projected, with the height delta removed from its radius.
    if delta >= projector.radius || delta <= -0.5 {
        return None;
    }
    projector.center.z = height;
    projector.radius -= delta;
    Some(projector)
}

fn terrain_mesh(projector: Projector, cell: (i32, i32), terrain: &TerrainQuery) -> Option<Mesh> {
    let (lo, hi) = projector.bounds();
    if projector.deferred.is_some()
        && ((hi.x / UNIT_SIZE).ceil() - (lo.x / UNIT_SIZE).floor())
            * ((hi.y / UNIT_SIZE).ceil() - (lo.y / UNIT_SIZE).floor())
            > (MAX_DEFERRED_MESH_TRIANGLES / 2) as f32
    {
        warn!(
            "deferred decal terrain receiver exceeds geometry budget at {:?}",
            projector.center
        );
        return None;
    }
    let mut heights = HashMap::new();
    let mut geometry = Geometry::default();
    for iy in (lo.y / UNIT_SIZE).floor() as i32..(hi.y / UNIT_SIZE).ceil() as i32 {
        for ix in (lo.x / UNIT_SIZE).floor() as i32..(hi.x / UNIT_SIZE).ceil() as i32 {
            let delta = (ix.div_euclid(CELL_UNITS as i32), iy.div_euclid(CELL_UNITS as i32));
            let Some(h) = heights
                .entry(delta)
                .or_insert_with(|| terrain.cell(cell.0 + delta.0, cell.1 + delta.1))
                .as_ref()
            else {
                continue;
            };
            let (x, y) = (
                ix.rem_euclid(CELL_UNITS as i32) as usize,
                iy.rem_euclid(CELL_UNITS as i32) as usize,
            );
            if h.quad_is_hole(x, y, 1) {
                continue;
            }
            let q = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(dx, dy)| Vertex {
                pos: Vec3::new(
                    (ix + dx as i32) as f32 * UNIT_SIZE,
                    (iy + dy as i32) as f32 * UNIT_SIZE,
                    h.at(x + dx, y + dy),
                ),
                normal: Vec3::from(h.normal(x + dx, y + dy)),
            });
            geometry.triangle([q[0], q[1], q[3]], projector);
            geometry.triangle([q[1], q[2], q[3]], projector);
            if geometry.over_budget {
                warn!(
                    "deferred decal terrain receiver exceeds geometry budget at {:?}",
                    projector.center
                );
                return None;
            }
        }
    }
    geometry.mesh()
}

type Models = HashMap<String, Option<Arc<Model>>>;

fn receiver_mesh(
    projector: Projector,
    sources: &[(Vec3, &CellObjects)],
    loader: &CryLoader,
    models: &mut Models,
) -> Option<Mesh> {
    let mut geometry = Geometry::default();
    let bounds = projector.bounds();
    for &(offset, objects) in sources {
        for brush in &objects.brushes {
            let brush_bounds = (
                Vec3::from_slice(&brush.bbox[..3]) + offset,
                Vec3::from_slice(&brush.bbox[3..]) + offset,
            );
            if !overlaps(bounds, brush_bounds) {
                continue;
            }
            let Some(path) = objects.models.get(brush.model as usize).filter(|p| !p.is_empty()) else {
                continue;
            };
            let Some(model) = models
                .entry(path.clone())
                .or_insert_with(|| Model::load(path).ok().map(Arc::new))
                .clone()
            else {
                continue;
            };
            let material = objects
                .materials
                .get(brush.material as usize)
                .filter(|m| !m.is_empty())
                .cloned()
                .or_else(|| aa_cry::scene::find_mtl(path, &model));
            let material = material.and_then(|p| {
                let p = if p.ends_with(".mtl") { p } else { format!("{p}.mtl") };
                loader.mtl(&p)
            });
            for node in model.lod_nodes(0) {
                let mesh = &model.meshes[model.nodes[node].mesh.unwrap()];
                let transform = Mat4::from_translation(offset) * brush.mat4() * Mat4::from(model.world_transform(node));
                if !transform.is_finite() || transform.determinant().abs() < 1e-9 {
                    continue;
                }
                let normal_transform = transform.inverse().transpose();
                let mirrored = transform.determinant() < 0.0;
                for subset in &mesh.subsets {
                    let receiver_material = material.as_ref().and_then(|m| m.sub(subset.material_id as usize));
                    if receiver_material.is_some_and(|m| m.no_draw() || m.shader.eq_ignore_ascii_case("cut")) {
                        continue;
                    }
                    // Native deferred decals affect opaque depth-buffer receivers. Their
                    // alpha-cutout/translucent masks cannot be recovered from bare triangles.
                    if projector.deferred.is_some()
                        && receiver_material.is_none_or(|m| {
                            aa_cry::shade::to_standard_material(m, |_, _| None)
                                .is_none_or(|m| m.alpha_mode != AlphaMode::Opaque)
                        })
                    {
                        continue;
                    }
                    let Some(indices) = mesh
                        .indices
                        .get(subset.first_index as usize..(subset.first_index + subset.index_count) as usize)
                    else {
                        continue;
                    };
                    for tri in indices.chunks_exact(3) {
                        let mut tri = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                        if mirrored {
                            tri.swap(1, 2)
                        }
                        if tri.iter().any(|&i| i >= mesh.positions.len()) {
                            continue;
                        }
                        let face = (mesh.positions[tri[1]] - mesh.positions[tri[0]])
                            .cross(mesh.positions[tri[2]] - mesh.positions[tri[0]])
                            .normalize_or_zero();
                        let triangle = tri.map(|i| Vertex {
                            pos: transform.transform_point3(mesh.positions[i]),
                            normal: normal_transform
                                .transform_vector3(mesh.normals.get(i).copied().unwrap_or(face))
                                .normalize_or_zero(),
                        });
                        let lo = triangle.iter().fold(Vec3::splat(f32::INFINITY), |a, b| a.min(b.pos));
                        let hi = triangle
                            .iter()
                            .fold(Vec3::splat(f32::NEG_INFINITY), |a, b| a.max(b.pos));
                        if overlaps(bounds, (lo, hi)) {
                            geometry.triangle(triangle, projector);
                            if projector.deferred.is_some() && receiver_material.is_some_and(|m| m.two_sided()) {
                                let mut reverse = [triangle[0], triangle[2], triangle[1]];
                                for vertex in &mut reverse {
                                    vertex.normal = -vertex.normal;
                                }
                                geometry.triangle(reverse, projector);
                            }
                            if geometry.over_budget {
                                warn!(
                                    "deferred decal brush receiver exceeds geometry budget at {:?}",
                                    projector.center
                                );
                                return None;
                            }
                        }
                    }
                }
            }
        }
    }
    geometry.mesh()
}

/// Geometry is prepared once on the existing bounded placement workers. Native deferred
/// diffuse/opacity projection uses opaque terrain / brush receivers and per-fragment texgen.
pub(crate) fn prepare_cell(
    world: &str,
    objects: &CellObjects,
    cell: (i32, i32),
    terrain: &TerrainQuery,
    loader: &CryLoader,
    assets: &RoadAssets,
) -> Vec<RoadMesh> {
    let deferred_enabled = std::env::var_os("AA_NO_DEFERRED_DECALS").is_none();
    let mut neighbours = HashMap::new();
    for decal in objects.decals.iter().filter(|d| {
        ((!d.deferred && matches!(d.projection, 1 | 3))
            || (deferred_enabled && (d.deferred || matches!(d.projection, 4..=7))))
            && objects
                .materials
                .get(d.material as usize)
                .is_some_and(|p| !p.is_empty())
    }) {
        let Some(p) = (if decal.deferred || matches!(decal.projection, 4..=7) {
            Projector::deferred(decal)
        } else {
            Projector::new(decal)
        }) else {
            continue;
        };
        let (lo, hi) = p.bounds();
        for dy in (lo.y / CELL_SIZE).floor() as i32..=(hi.y / CELL_SIZE).floor() as i32 {
            for dx in (lo.x / CELL_SIZE).floor() as i32..=(hi.x / CELL_SIZE).floor() as i32 {
                if (dx, dy) == (0, 0) {
                    continue;
                }
                neighbours.entry((dx, dy)).or_insert_with(|| {
                    let path = format!(
                        "{}/client/object.dat",
                        crate::world::cell_dir(world, cell.0 + dx, cell.1 + dy)
                    );
                    aa_pak::Pak::global()
                        .read(&path)
                        .ok()
                        .and_then(|d| parse_object_dat(&d).ok())
                        .unwrap_or_default()
                });
            }
        }
    }
    let mut sources = vec![(Vec3::ZERO, objects)];
    sources.extend(
        neighbours
            .iter()
            .map(|(&(dx, dy), o)| (Vec3::new(dx as f32 * CELL_SIZE, dy as f32 * CELL_SIZE, 0.0), o)),
    );
    let mut models = Models::new();
    let mut out = Vec::new();
    let (mut deferred_meshes, mut deferred_triangles) = (0, 0);
    let mut warned_cell_budget = false;
    for decal in &objects.decals {
        let Some(path) = objects.materials.get(decal.material as usize).filter(|p| !p.is_empty()) else {
            continue;
        };
        if decal.deferred || matches!(decal.projection, 4..=7) {
            if !deferred_enabled {
                continue;
            }
            if deferred_meshes >= MAX_DEFERRED_CELL_MESHES || deferred_triangles >= MAX_DEFERRED_CELL_TRIANGLES {
                if !warned_cell_budget {
                    warn!("deferred decal cell receiver budget exceeded at {cell:?}");
                    warned_cell_budget = true;
                }
                continue;
            }
            let Some(projector) = Projector::deferred(decal) else {
                continue;
            };
            let native = projector.deferred.unwrap();
            let (lo, hi) = projector.bounds();
            let radius = (hi - lo).length() * 0.5;
            for mesh in [
                terrain_mesh(projector, cell, terrain),
                receiver_mesh(projector, &sources, loader, &mut models),
            ]
            .into_iter()
            .flatten()
            {
                let triangles = mesh.indices().map_or(0, |i| i.len() / 3);
                if deferred_meshes >= MAX_DEFERRED_CELL_MESHES
                    || deferred_triangles + triangles > MAX_DEFERRED_CELL_TRIANGLES
                {
                    if !warned_cell_budget {
                        warn!("deferred decal cell receiver budget exceeded at {cell:?}");
                        warned_cell_budget = true;
                    }
                    continue;
                }
                if let Some(mut mesh) = assets.decal(
                    loader,
                    path,
                    mesh,
                    decal.sort_priority,
                    radius,
                    decal.max_view_dist,
                    Vec4::ZERO,
                ) {
                    mesh.deferred = Some(native.params(cell_origin(cell.0, cell.1), SURFACE_OFFSET));
                    out.push(mesh);
                    deferred_meshes += 1;
                    deferred_triangles += triangles;
                }
            }
            continue;
        }
        let Some(projector) = Projector::new(decal) else {
            continue;
        };
        let mut push = |mesh: Option<Mesh>, p: Projector, fade: bool, forward_overlay: bool| {
            let Some(mesh) = mesh else { return };
            let attenuation = if fade {
                cry_to_bevy(p.center + cell_origin(cell.0, cell.1)).extend(p.radius.recip())
            } else {
                Vec4::ZERO
            };
            if let Some(mut mesh) = assets.decal(
                loader,
                path,
                mesh,
                decal.sort_priority,
                p.radius,
                decal.max_view_dist,
                attenuation,
            ) {
                mesh.forward_overlay = forward_overlay;
                out.push(mesh)
            }
        };
        if decal.projection == 0 {
            push(planar(projector), projector, false, true);
        }
        if matches!(decal.projection, 1 | 3) {
            push(receiver_mesh(projector, &sources, loader, &mut models), projector, true, true);
        }
        if matches!(decal.projection, 2 | 3)
            && let Some(p) = terrain_projector(projector, cell, terrain)
        {
            push(terrain_mesh(p, cell, terrain), p, true, false);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_cry::scene::CryCache;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::VertexAttributeValues;
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;

    fn loader() -> CryLoader {
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<Image>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(Assets::<SkinnedMeshInverseBindposes>::default());
        world.insert_resource(CryCache::default());
        world
            .run_system_once(|mut cry: aa_cry::CryAssets| cry.loader())
            .unwrap()
    }

    fn projector() -> Projector {
        Projector {
            center: Vec3::new(10.0, 20.0, 30.0),
            basis: Mat3::IDENTITY,
            inverse: Mat3::IDENTITY,
            radius: 2.0,
            deferred: None,
        }
    }

    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        let Some(VertexAttributeValues::Float32x3(values)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("positions")
        };
        values.iter().map(|&p| crate::bevy_to_cry(Vec3::from(p))).collect()
    }

    #[test]
    fn static_receiver_clip_preserves_the_wall_and_native_uv_orientation() {
        // A wall facing +Y; project a rotated decal onto it. The input triangle extends well
        // outside the footprint and has a nonzero world translation.
        let basis = Mat3::from_cols(Vec3::X, Vec3::NEG_Z, Vec3::Y);
        let p = Projector {
            basis,
            inverse: basis.inverse(),
            ..projector()
        };
        let triangle = [
            Vec3::new(4.0, 20.0, 24.0),
            Vec3::new(4.0, 20.0, 36.0),
            Vec3::new(16.0, 20.0, 30.0),
        ]
        .map(|pos| Vertex { pos, normal: Vec3::Y });
        let mut geometry = Geometry::default();
        geometry.triangle(triangle, p);
        let mesh = geometry.mesh().unwrap();
        for v in positions(&mesh) {
            assert!((v.y - 20.0 - SURFACE_OFFSET).abs() < 1e-5);
            let q = p.local(v);
            assert!(q.x.abs() <= 2.00001 && q.y.abs() <= 2.00001);
        }
        assert_eq!(p.uv(p.center - basis.y_axis * 2.0, Vec3::Y), Vec2::new(1.0, 0.5));
        assert_eq!(p.uv(p.center + basis.x_axis * 2.0, Vec3::Y), Vec2::new(0.5, 1.0));
        let mut back = Geometry::default();
        back.triangle([triangle[0], triangle[2], triangle[1]], p);
        assert!(back.mesh().is_none(), "back faces must not receive a forward decal");
    }

    #[test]
    fn planar_decal_is_a_rotated_quad_with_full_coverage() {
        let mesh = planar(projector()).unwrap();
        assert_eq!(mesh.indices().unwrap().len(), 6);
        assert_eq!(positions(&mesh).len(), 4);
        let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
            panic!("uv")
        };
        assert!(uv.iter().all(|p| p.iter().all(|v| *v == 0.0 || *v == 1.0)));
    }

    #[test]
    fn real_deferred_and_wrapped_receivers_and_materials() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 2, 29);
        assert_eq!(objects.decals.iter().filter(|d| d.deferred).count(), 81);
        let terrain = TerrainQuery::new("main_world");
        let mut world = World::new();
        world.insert_resource(Assets::<Mesh>::default());
        world.insert_resource(Assets::<Image>::default());
        world.insert_resource(Assets::<StandardMaterial>::default());
        world.insert_resource(Assets::<crate::roads::RoadMaterial>::default());
        world.insert_resource(Assets::<SkinnedMeshInverseBindposes>::default());
        world.insert_resource(CryCache::default());
        let loader = world
            .run_system_once(|mut cry: aa_cry::CryAssets| cry.loader())
            .unwrap();
        let prepared = prepare_cell(
            "main_world",
            &objects,
            (2, 29),
            &terrain,
            &loader,
            &RoadAssets::default(),
        );
        let mut deferred: Vec<_> = prepared.into_iter().filter(|m| m.deferred.is_some()).collect();
        assert_eq!(
            deferred.len(),
            81,
            "all original siegefield decals must hit real receivers within budget"
        );
        let triangles: usize = deferred.iter().map(|m| m.mesh.indices().unwrap().len() / 3).sum();
        eprintln!("native siegefield: {} meshes, {} triangles", deferred.len(), triangles);
        assert!(triangles < 200_000, "bounded real-fixture geometry");
        for mesh in &deferred {
            assert!(positions(&mesh.mesh).iter().all(|p| p.is_finite()));
            assert!(mesh.mesh.indices().unwrap().len() / 3 <= MAX_DEFERRED_MESH_TRIANGLES);
        }
        let first = deferred.remove(0);
        let second_index = deferred
            .iter()
            .position(|m| m.deferred.unwrap().center != first.deferred.unwrap().center)
            .unwrap();
        let second = deferred.swap_remove(second_index);
        world
            .run_system_once(
                move |mut cry: aa_cry::CryAssets, mut materials: ResMut<Assets<crate::roads::RoadMaterial>>| {
                    let a = first.material(&mut cry, &mut materials).unwrap();
                    let b = second.material(&mut cry, &mut materials).unwrap();
                    assert_ne!(
                        a, b,
                        "native placement uniforms must not reuse a sort-priority-only material"
                    );
                    assert!(materials.get(&a).unwrap().base.normal_map_texture.is_none());
                    assert_eq!(materials.get(&a).unwrap().base.reflectance, 0.0);
                },
            )
            .unwrap();
        let wrapped = crate::placement::load_cell_objects("main_world", 23, 10);
        let authored = wrapped.decals.iter().find(|d| d.projection == 4).unwrap();
        assert_eq!(
            wrapped.materials[authored.material as usize],
            "game/materials/aa_terrain/decals/tr_decal_spider_400b"
        );
        let prepared = prepare_cell(
            "main_world",
            &wrapped,
            (23, 10),
            &terrain,
            &loader,
            &RoadAssets::default(),
        );
        let receivers: Vec<_> = prepared
            .iter()
            .filter(|m| m.deferred.is_some_and(|p| p.center.w == 2.0))
            .collect();
        assert_eq!(
            receivers.len(),
            2,
            "original wrapped spiderweb hits terrain and an opaque brush"
        );
        assert!(receivers.iter().all(|m| m.mesh.indices().unwrap().len() > 0));
        let planar: Vec<_> = prepared.iter().filter(|m| m.forward_overlay && m.attenuation.w == 0.0).collect();
        assert_eq!(planar.len(), 3, "three original planar Illum+DECAL overlays");
        assert!(planar.iter().all(|m| m.early_forward_overlay()));
        assert!(prepared.iter().filter(|m| m.deferred.is_some()).all(|m| !m.early_forward_overlay()));
    }

    #[test]
    fn original_visarea_decals_reach_supported_receivers() {
        if !crate::pak_available() {
            return;
        }
        let terrain = TerrainQuery::new("main_world");
        let loader = loader();
        // Corrected native height/surface decoding and topology change the 12,15 terrain receivers.
        for (cell, expected) in [((8, 13), 18), ((12, 15), 43), ((20, 8), 4)] {
            let path = format!("{}/client/visareas.dat", crate::world::cell_dir("main_world", cell.0, cell.1));
            let data = aa_pak::Pak::global().read(&path).unwrap();
            let mut objects = crate::placement::load_cell_objects("main_world", cell.0, cell.1);
            let interior = crate::placement::parse_visareas_dat(&data).unwrap();
            // Keep the merged independent material table and just the interior decals.
            let start = objects.decals.len() - interior.decals.len();
            objects.decals.drain(..start);
            let prepared = prepare_cell("main_world", &objects, cell, &terrain, &loader, &RoadAssets::default());
            println!("interior {cell:?}: {} source decals, {} supported receiver meshes", objects.decals.len(), prepared.len());
            assert_eq!(prepared.len(), expected);
            for mesh in &prepared {
                let positions = positions(&mesh.mesh);
                assert!(positions.iter().all(|p| p.is_finite()));
                assert!(mesh.mesh.indices().unwrap().len() / 3 <= MAX_DEFERRED_MESH_TRIANGLES);
            }
        }
    }

    #[test]
    fn real_forward_decals_use_terrain_and_visible_brush_receivers() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 18, 7);
        assert_eq!(objects.decals.len(), 44);
        assert_eq!(
            objects
                .decals
                .iter()
                .filter(|d| d.projection == 3 && !d.deferred)
                .count(),
            20
        );
        let terrain = TerrainQuery::new("main_world");
        let loader = loader();
        let mut models = Models::new();
        let mut receivers = 0;
        for decal in objects.decals.iter().filter(|d| d.projection == 3) {
            let p = Projector::new(decal).unwrap();
            if let Some(mesh) = receiver_mesh(p, &[(Vec3::ZERO, &objects)], &loader, &mut models) {
                println!(
                    "static receiver at {:?}, {} triangles",
                    decal.pos,
                    mesh.indices().unwrap().len() / 3
                );
                assert!(mesh.indices().unwrap().len() >= 3);
                receivers += 1;
                for v in positions(&mesh) {
                    let q = p.local(v);
                    assert!(q.abs().max_element() <= p.radius + 0.01);
                }
            }
        }
        assert_eq!(
            receivers, 5,
            "combined projections mark five visible CGF receivers in this original cell"
        );
        let prepared = prepare_cell(
            "main_world",
            &objects,
            (18, 7),
            &terrain,
            &loader,
            &RoadAssets::default(),
        );
        assert_eq!(prepared.len(), 48, "43 terrain + five static receiver meshes");
        assert_eq!(prepared.iter().filter(|m| m.early_forward_overlay()).count(), 5);
        assert_eq!(prepared.iter().filter(|m| !m.forward_overlay).count(), 43,
            "terrain portions of combined projections keep their native water-path boundary");
        assert!(prepared.iter().all(|m| m.attenuation.w > 0.0));
        println!(
            "real decal fixture: {} projected meshes, {receivers} static receivers",
            prepared.len()
        );
    }
}
