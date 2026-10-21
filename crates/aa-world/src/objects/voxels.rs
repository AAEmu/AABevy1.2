//! Native compiled voxel Terrain base pass. Projected Terrain.Layer detail is separate.

use aa_cry::CryAssets;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use std::collections::HashMap;

use crate::placement::PlacedVoxel;

mod cover;
mod layers;
mod material;
mod normals;
pub use material::{VoxelExtension, VoxelMaterial, VoxelMaterialPlugin, VoxelParams};

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

/// One native projected diffuse layer; separate visibility control from the base receiver.
#[derive(Component)]
pub struct VoxelDetail {
    pub cell: (i32, i32),
    pub slot: usize,
    pub surface: u8,
    pub axis: u8,
}

pub(crate) struct PreparedVoxel {
    mesh: Mesh,
    slot: usize,
    triangles: usize,
    distance: f32,
    cover: Option<cover::Node>,
    layers: Vec<layers::Draw>,
}

#[derive(Default)]
pub(crate) struct PreparedCell {
    meshes: Vec<PreparedVoxel>,
    covers: HashMap<cover::Node, Image>,
    layers: layers::Layers,
    unsupported: usize,
}

pub(crate) struct CellVoxels {
    pub entities: Vec<Entity>,
    pub triangles: usize,
    pub unsupported: usize,
    pub base_count: usize,
    pub layer_count: usize,
    pub layers_unsupported: usize,
    meshes: Vec<Handle<Mesh>>,
    materials: Vec<Handle<VoxelMaterial>>,
    images: Vec<Handle<Image>>,
}

impl CellVoxels {
    pub fn release(
        self,
        commands: &mut Commands,
        cry: &mut CryAssets,
        materials: &mut Assets<VoxelMaterial>,
    ) {
        for entity in self.entities {
            commands.entity(entity).despawn();
        }
        for mesh in self.meshes {
            cry.meshes.remove(mesh.id());
        }
        for material in self.materials {
            materials.remove(material.id());
        }
        for image in self.images {
            cry.images.remove(image.id());
        }
    }

    /// Like [`CellVoxels::release`] but directly on a [`World`], for a runtime world switch.
    pub(crate) fn release_world(&mut self, world: &mut World) {
        for entity in self.entities.drain(..) {
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
        if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
            for mesh in self.meshes.drain(..) {
                meshes.remove(mesh.id());
            }
        }
        if let Some(mut materials) = world.get_resource_mut::<Assets<VoxelMaterial>>() {
            for material in self.materials.drain(..) {
                materials.remove(material.id());
            }
        }
        if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
            for image in self.images.drain(..) {
                images.remove(image.id());
            }
        }
    }
}

fn mesh(
    voxel: &PlacedVoxel,
    source: aa_cry::mesh::MeshData,
    cover: Option<cover::Node>,
) -> Option<Mesh> {
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
    let uvs: Vec<_> = source
        .positions
        .iter()
        .map(|p| cover.map_or(Vec2::ZERO, |c| c.uv(transform.transform_point3(*p))))
        .collect();
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices)),
    )
}

pub(crate) fn prepare_cell(world: &str, cell: (i32, i32), voxels: &[PlacedVoxel]) -> PreparedCell {
    let mut out = PreparedCell::default();
    if !voxels.is_empty() {
        out.layers = layers::Layers::load(world, cell);
    }
    let (mut decoded, mut bytes) = (0usize, 0usize);
    // Read once on the bounded placement worker, only if a mesh actually needs cover.
    let mut cover_bytes = None;
    let mut cover_attempted = false;
    for (slot, voxel) in voxels.iter().enumerate() {
        let size = crate::placement::voxel_decoded_size(&voxel.compiled);
        if out.meshes.len() >= MAX_CELL_MESHES
            || size.is_none_or(|n| n > MAX_CELL_DECODED_BYTES.saturating_sub(decoded))
        {
            out.unsupported += 1;
            continue;
        }
        decoded += size.unwrap();
        let Some((source, ids)) = voxel.compiled_surface_mesh() else {
            out.unsupported += 1;
            continue;
        };
        let cover = if source.colors.iter().any(|c| c[3] == 0) {
            if !cover_attempted {
                cover_attempted = true;
                cover_bytes = aa_pak::Pak::global()
                    .read(&format!(
                        "game/worlds/{world}/cells/{:03}_{:03}/client/terrain/cover.ctc",
                        cell.0, cell.1
                    ))
                    .ok();
            }
            let selected = cover_bytes
                .as_deref()
                .and_then(|bytes| crate::cover::Cover::parse(bytes).ok())
                .and_then(|cover| {
                    let node = cover::select(&cover, &voxel.bbox)?;
                    if !out.covers.contains_key(&node) {
                        out.covers.insert(node, cover::image(&cover, node)?);
                    }
                    Some(node)
                });
            let Some(node) = selected else {
                out.unsupported += 1;
                continue;
            };
            Some(node)
        } else {
            None
        };
        let Some(mesh) = mesh(voxel, source.clone(), cover) else {
            out.unsupported += 1;
            continue;
        };
        let indices = mesh.indices().unwrap().len();
        let size = mesh.count_vertices() * 48 + indices * 4;
        if size > MAX_CELL_MESH_BYTES.saturating_sub(bytes) {
            out.unsupported += 1;
            continue;
        }
        bytes += size;
        let layers = out.layers.prepare(voxel, &source, &ids, &mesh);
        out.meshes.push(PreparedVoxel {
            mesh,
            slot,
            triangles: indices / 3,
            distance: voxel.max_view_dist,
            cover,
            layers,
        });
    }
    out
}

pub(crate) fn spawn_cell(
    commands: &mut Commands,
    cry: &mut CryAssets,
    materials: &mut Assets<VoxelMaterial>,
    cell: (i32, i32),
    prepared: PreparedCell,
    distance: f32,
) -> CellVoxels {
    let mut material_by_cover = HashMap::new();
    let mut images = Vec::new();
    let used_covers: std::collections::HashSet<_> =
        prepared.meshes.iter().filter_map(|m| m.cover).collect();
    let covers: HashMap<_, _> = prepared
        .covers
        .into_iter()
        .filter(|(node, _)| used_covers.contains(node))
        .map(|(node, image)| {
            let image = cry.images.add(image);
            images.push(image.clone());
            (node, image)
        })
        .collect();
    let mut layer_data = prepared.layers;
    let used_textures: std::collections::HashSet<_> = prepared
        .meshes
        .iter()
        .flat_map(|m| &m.layers)
        .flat_map(|layer| {
            let definition = &layer_data.definitions[&layer.key];
            std::iter::once(definition.texture.clone()).chain(
                definition.normal.iter().flat_map(|n| {
                    std::iter::once(n.texture.clone()).chain(n.detail.iter().cloned())
                }),
            )
        })
        .collect();
    layer_data
        .textures
        .retain(|path, _| used_textures.contains(path));
    let textures: HashMap<_, _> = layer_data
        .textures
        .drain()
        .map(|(path, image)| {
            let image = cry.images.add(image);
            images.push(image.clone());
            (path, image)
        })
        .collect();
    for mesh in &prepared.meshes {
        material_by_cover
            .entry((mesh.cover, None))
            .or_insert_with(|| {
                let cover = mesh.cover.map(|key| covers[&key].clone());
                materials.add(VoxelMaterial {
                    base: StandardMaterial {
                        base_color: Color::WHITE,
                        perceptual_roughness: 1.0,
                        reflectance: 0.0,
                        ..default()
                    },
                    extension: VoxelExtension::new(cover),
                })
            });
        for layer in &mesh.layers {
            material_by_cover
                .entry((mesh.cover, Some(layer.key)))
                .or_insert_with(|| {
                    let definition = &layer_data.definitions[&layer.key];
                    let mut extension =
                        VoxelExtension::new(mesh.cover.map(|key| covers[&key].clone()));
                    extension.diffuse = Some(textures[&definition.texture].clone());
                    extension.params.flags.y = 1.0;
                    extension.params.flags.z = definition.strength;
                    extension.params.flags.w = definition.distance;
                    extension.params.u = definition.u;
                    extension.params.v = definition.v;
                    extension.params.origin =
                        crate::cry_to_bevy(crate::coords::cell_origin(cell.0, cell.1)).extend(0.0);
                    extension.params.colour = definition.colour.extend(1.0);
                    if let Some(normal) = &definition.normal {
                        extension.normal = Some(textures[&normal.texture].clone());
                        extension.detail_normal =
                            normal.detail.as_ref().map(|path| textures[path].clone());
                        extension.params.normals = Vec4::new(
                            normal.mode,
                            if normal.detail.is_some() { 1.0 } else { 0.0 },
                            normal.detail_scale,
                            0.0,
                        );
                        extension.params.bump_u = normal.u;
                        extension.params.bump_v = normal.v;
                        extension.params.detail_u = normal.detail_u;
                        extension.params.detail_v = normal.detail_v;
                    }
                    materials.add(VoxelMaterial {
                        base: StandardMaterial {
                            base_color: Color::WHITE,
                            perceptual_roughness: 1.0,
                            reflectance: 0.0,
                            alpha_mode: AlphaMode::Blend,
                            // Preserve native surface/projection pass order before deferred decals/world glass.
                            depth_bias: -900_000.0
                                + (layer.key.0 as f32 * 3.0 + layer.key.1 as f32) * 4096.0,
                            ..default()
                        },
                        extension,
                    })
                });
        }
    }
    let mut out = CellVoxels {
        entities: Vec::new(),
        meshes: Vec::new(),
        materials: material_by_cover.values().cloned().collect(),
        images,
        unsupported: prepared.unsupported,
        base_count: prepared.meshes.len(),
        layer_count: layer_data.draws,
        layers_unsupported: layer_data.unsupported,
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
                    Name::new("Native voxel Terrain base"),
                    VoxelSurface {
                        cell,
                        slot: prepared.slot,
                        triangles: prepared.triangles,
                    },
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material_by_cover[&(prepared.cover, None)].clone()),
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
        for layer in prepared.layers {
            let mesh = cry.meshes.add(layer.mesh);
            out.entities.push(
                commands
                    .spawn((
                        Name::new("Native voxel projected diffuse layer"),
                        VoxelDetail {
                            cell,
                            slot: prepared.slot,
                            surface: layer.key.0,
                            axis: layer.key.1,
                        },
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(
                            material_by_cover[&(prepared.cover, Some(layer.key))].clone(),
                        ),
                        Transform::from_translation(crate::cry_to_bevy(
                            crate::coords::cell_origin(cell.0, cell.1),
                        )),
                        VisibilityRange {
                            start_margin: 0.0..0.0,
                            end_margin: view..view,
                            use_aabb: true,
                        },
                        bevy::light::NotShadowCaster,
                    ))
                    .id(),
            );
            out.meshes.push(mesh);
        }
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
        let cell = prepare_cell("main_world", (20, 8), &objects.voxels);
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
        let bounded = prepare_cell("main_world", (20, 8), &repeated);
        assert_eq!(bounded.meshes.len(), MAX_CELL_MESHES);
        assert_eq!(bounded.unsupported, 1);
    }

    #[test]
    fn original_mixed_cover_meshes_share_exact_tile_and_keep_interpolated_selector() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 21, 8);
        let cell = prepare_cell("main_world", (21, 8), &objects.voxels);
        assert_eq!(
            (cell.meshes.len(), cell.unsupported, cell.covers.len()),
            (2, 0, 1)
        );
        assert!(
            cell.meshes.iter().all(|m| !m.layers.is_empty()),
            "both native neighbour slots retain projected detail despite their compiled Z overlap"
        );
        assert_eq!(cell.layers.unsupported, 0);
        let cover = &cell.covers[&cover::Node {
            level: 4,
            x: 13,
            y: 0,
        }];
        assert_eq!(
            (
                cover.width(),
                cover.height(),
                cover.texture_descriptor.mip_level_count
            ),
            (128, 128, 1)
        );
        assert_eq!(cover.data.as_ref().unwrap().len(), 8192);
        assert_eq!(
            cover.texture_descriptor.format,
            bevy::render::render_resource::TextureFormat::Bc1RgbaUnorm
        );
        let Some(VertexAttributeValues::Float32x4(colours)) =
            cell.meshes[0].mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!()
        };
        assert!(colours.iter().any(|c| c[3] == 0.0) && colours.iter().any(|c| c[3] == 1.0));
        let Some(VertexAttributeValues::Float32x2(uvs)) =
            cell.meshes[0].mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!()
        };
        assert!(uvs.iter().all(|uv| {
            uv.iter()
                .all(|v| *v >= 0.5 / 128.0 - 0.001 && *v <= 127.5 / 128.0 + 0.001)
        }));
        // Missing source texture must not silently become a baked-colour or flat fallback.
        let missing = prepare_cell("missing_voxel_fixture", (21, 8), &objects.voxels);
        assert_eq!(
            (
                missing.meshes.len(),
                missing.unsupported,
                missing.covers.len()
            ),
            (0, 2, 0)
        );
    }
}
