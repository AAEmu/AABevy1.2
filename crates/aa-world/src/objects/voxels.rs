//! Native compiled voxel Terrain base pass. Projected Terrain.Layer detail is separate.

use aa_cry::CryAssets;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::placement::PlacedVoxel;

const MAX_CELL_DECODED_BYTES: usize = 64 * 1024 * 1024;
const MAX_CELL_MESH_BYTES: usize = 32 * 1024 * 1024;
const MAX_CELL_MESHES: usize = 32;

/// One native baked-colour base mesh; useful for diagnostics/visibility controls.
#[derive(Component)]
pub struct VoxelSurface {
    pub cell: (i32, i32),
    pub slot: usize,
    pub triangles: usize,
}

pub(crate) struct PreparedVoxel {
    mesh: Mesh,
    slot: usize,
    triangles: usize,
    distance: f32,
}

#[derive(Default)]
pub(crate) struct PreparedCell {
    meshes: Vec<PreparedVoxel>,
    unsupported: usize,
}

pub(crate) struct CellVoxels {
    pub entities: Vec<Entity>,
    pub triangles: usize,
    pub unsupported: usize,
    meshes: Vec<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

impl CellVoxels {
    pub fn release(self, commands: &mut Commands, cry: &mut CryAssets) {
        for entity in self.entities {
            commands.entity(entity).despawn();
        }
        for mesh in self.meshes {
            cry.meshes.remove(mesh.id());
        }
        if let Some(material) = self.material {
            cry.materials.remove(material.id());
        }
    }
}

fn mesh(voxel: &PlacedVoxel) -> Option<Mesh> {
    let source = voxel.compiled_base_mesh()?;
    let transform = voxel.mat4();
    let normal = Mat3::from_mat4(transform).inverse().transpose();
    let positions: Vec<_> = source
        .positions
        .iter()
        .map(|p| crate::cry_to_bevy(transform.transform_point3(*p)))
        .collect();
    let normals: Vec<_> = source
        .normals
        .iter()
        .map(|n| crate::cry_to_bevy((normal * *n).normalize_or_zero()))
        .collect();
    if positions.iter().chain(&normals).any(|v| !v.is_finite()) {
        return None;
    }
    let mut indices = source.indices;
    if transform.determinant() < 0.0 {
        for triangle in indices.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }
    // The original Terrain shader says voxel vertex colour is linear. Do not sRGB-decode it.
    let colours: Vec<[f32; 4]> = source
        .colors
        .iter()
        .map(|c| c.map(|v| v as f32 / 255.0))
        .collect();
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours)
        .with_inserted_indices(Indices::U32(indices)),
    )
}

pub(crate) fn prepare_cell(voxels: &[PlacedVoxel]) -> PreparedCell {
    let mut out = PreparedCell::default();
    let (mut decoded, mut bytes) = (0usize, 0usize);
    for (slot, voxel) in voxels.iter().enumerate() {
        let size = crate::placement::voxel_decoded_size(&voxel.compiled);
        if out.meshes.len() >= MAX_CELL_MESHES
            || size.is_none_or(|n| n > MAX_CELL_DECODED_BYTES.saturating_sub(decoded))
        {
            out.unsupported += 1;
            continue;
        }
        decoded += size.unwrap();
        let Some(mesh) = mesh(voxel) else {
            out.unsupported += 1;
            continue;
        };
        let indices = mesh.indices().unwrap().len();
        let size = mesh.count_vertices() * 40 + indices * 4;
        if size > MAX_CELL_MESH_BYTES.saturating_sub(bytes) {
            out.unsupported += 1;
            continue;
        }
        bytes += size;
        out.meshes.push(PreparedVoxel {
            mesh,
            slot,
            triangles: indices / 3,
            distance: voxel.max_view_dist,
        });
    }
    out
}

pub(crate) fn spawn_cell(
    commands: &mut Commands,
    cry: &mut CryAssets,
    cell: (i32, i32),
    prepared: PreparedCell,
    distance: f32,
) -> CellVoxels {
    let material = (!prepared.meshes.is_empty()).then(|| {
        cry.materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            ..default()
        })
    });
    let mut out = CellVoxels {
        entities: Vec::new(),
        meshes: Vec::new(),
        material,
        unsupported: prepared.unsupported,
        triangles: 0,
    };
    for prepared in prepared.meshes {
        let mesh = cry.meshes.add(prepared.mesh);
        let view = if prepared.distance.is_finite() && prepared.distance > 1.0 {
            prepared.distance.min(distance)
        } else {
            distance
        };
        out.entities.push(
            commands
                .spawn((
                    Name::new("Native voxel baked-colour base"),
                    VoxelSurface {
                        cell,
                        slot: prepared.slot,
                        triangles: prepared.triangles,
                    },
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(out.material.as_ref().unwrap().clone()),
                    Transform::from_translation(crate::cry_to_bevy(crate::coords::cell_origin(
                        cell.0, cell.1,
                    ))),
                    VisibilityRange {
                        start_margin: 0.0..0.0,
                        end_margin: view..view,
                        use_aabb: true,
                    },
                ))
                .id(),
        );
        out.meshes.push(mesh);
        out.triangles += prepared.triangles;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn original_cave_base_meshes_keep_authored_colour_transform_and_resource_bounds() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 20, 8);
        let cell = prepare_cell(&objects.voxels);
        assert_eq!(cell.unsupported, 0);
        assert_eq!(cell.meshes.len(), 7);
        assert_eq!(
            cell.meshes.iter().map(|m| m.triangles).sum::<usize>(),
            45963
        );
        let mesh = &cell.meshes[0].mesh;
        let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("linear colours")
        };
        assert_eq!(colours[0], [21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0, 1.0]);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions")
        };
        // Cell-local transformed coordinates: the global cell origin belongs only to the entity.
        assert!(
            positions
                .iter()
                .all(|p| p[0] >= 832.0 && p[0] < 896.0 && p[2] <= -832.0 && p[2] > -896.0)
        );
        let repeated: Vec<_> =
            std::iter::repeat_n(objects.voxels[0].clone(), MAX_CELL_MESHES + 1).collect();
        let bounded = prepare_cell(&repeated);
        assert_eq!(bounded.meshes.len(), MAX_CELL_MESHES);
        assert_eq!(bounded.unsupported, 1);
    }
}
