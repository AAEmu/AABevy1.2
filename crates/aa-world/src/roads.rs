//! Terrain roads from the cell octree. The original spline edges clip the same two-metre
//! terrain triangles as the ground renderer. UVs run along the spline (U) and across it (V).
//! Meshes, DDS decoding and the separate opacity map are prepared on placement workers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aa_cry::material::{Material, TextureMap, texture_pak_path};
use aa_cry::texture::{DdsOptions, load_dds};
use aa_cry::{CryAssets, CryLoader};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use crate::coords::{CELL_SIZE, CELL_UNITS, UNIT_SIZE, cry_to_bevy};
use crate::decal_projection::DeferredParams;
use crate::placement::{CellObjects, Road};
use crate::terrain_query::TerrainQuery;

/// Original engine `fRoadTerrainZOffset` (prevents depth fighting with the ground).
const HEIGHT_OFFSET: f32 = 0.06;

pub type RoadMaterial = ExtendedMaterial<StandardMaterial, RoadExtension>;

#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
pub struct OpacityParams {
    mask: Vec4,
    /// Rows of the opacity map's own UV transform; `uv_x.w` enables the texture.
    uv_x: Vec4,
    uv_y: Vec4,
    /// Static projected decals: world centre and inverse spherical fade radius. Roads use zero.
    attenuation: Vec4,
    projection: DeferredParams,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(RoadKey)]
pub struct RoadExtension {
    #[uniform(100)]
    params: OpacityParams,
    #[texture(101)]
    #[sampler(102)]
    opacity: Option<Handle<Image>>,
    // CPU pipeline key only: the large transparent sort offset must not bias raster depth.
    early_surface: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RoadKey(bool);

impl From<&RoadExtension> for RoadKey {
    fn from(extension: &RoadExtension) -> Self {
        Self(extension.early_surface || extension.params.projection.center.w > 0.0)
    }
}

impl MaterialExtension for RoadExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/roads.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.0
            && let Some(depth) = descriptor.depth_stencil.as_mut()
        {
            // StandardMaterial couples CPU transparent order and raster depth bias. Native
            // roads/decals precede world transparency; their sort offset must not move depth.
            depth.bias.constant = 0;
        }
        Ok(())
    }
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "roads.wesl");
    app.add_plugins(MaterialPlugin::<RoadMaterial>::default());
}

struct MaterialData {
    source: Arc<Material>,
    /// Consumed once on the render thread; only handles remain in the cache afterwards.
    images: Mutex<Option<HashMap<(String, bool), Image>>>,
    handles: Mutex<HashMap<u8, Handle<RoadMaterial>>>,
    textures: Mutex<HashMap<(String, bool), Handle<Image>>>,
}

type MaterialSlot = Arc<OnceLock<Option<MaterialData>>>;

/// Shared by placement workers so neighbouring cells decode a road material once.
#[derive(Clone, Default)]
pub(crate) struct RoadAssets(Arc<Mutex<HashMap<(String, bool), MaterialSlot>>>);

pub(crate) struct RoadMesh {
    pub mesh: Mesh,
    material: MaterialSlot,
    pub priority: u8,
    pub radius: f32,
    pub distance: f32,
    pub attenuation: Vec4,
    pub deferred: Option<DeferredParams>,
    source_road: bool,
}

impl RoadAssets {
    fn material(&self, loader: &CryLoader, path: &str, clamp: bool) -> MaterialSlot {
        let slot = self
            .0
            .lock()
            .unwrap()
            .entry((path.to_string(), clamp))
            .or_default()
            .clone();
        slot.get_or_init(|| prepare_material(loader, path, clamp));
        slot
    }

    pub(crate) fn decal(
        &self,
        loader: &CryLoader,
        path: &str,
        mesh: Mesh,
        priority: u8,
        radius: f32,
        distance: f32,
        attenuation: Vec4,
    ) -> Option<RoadMesh> {
        if path.is_empty() {
            return None;
        }
        let material = self.material(loader, path, true);
        material.get()?.as_ref()?;
        Some(RoadMesh {
            mesh,
            material,
            priority,
            radius,
            distance,
            attenuation,
            deferred: None,
            source_road: false,
        })
    }
}

impl RoadMesh {
    /// Native type13 ordinary Illum uses list5. Legacy decals/custom shaders retain their pass.
    pub(crate) fn early_surface(&self) -> bool {
        self.source_road && self.material.get().and_then(Option::as_ref)
            .is_some_and(|data| data.source.shader.eq_ignore_ascii_case("Illum"))
    }

    /// Moves prepared images into Bevy once; subsequent road objects reuse the handles.
    pub fn material(&self, cry: &mut CryAssets, materials: &mut Assets<RoadMaterial>) -> Option<Handle<RoadMaterial>> {
        let data = self.material.get()?.as_ref()?;
        let mut handles = data.handles.lock().unwrap();
        if self.attenuation.w == 0.0
            && self.deferred.is_none()
            && let Some(handle) = handles.get(&self.priority)
        {
            return Some(handle.clone());
        }
        let mut textures = data.textures.lock().unwrap();
        if let Some(images) = data.images.lock().unwrap().take() {
            textures.extend(images.into_iter().map(|(key, image)| (key, cry.images.add(image))));
        }
        let mut material = aa_cry::shade::to_standard_material(&data.source, |path, srgb| {
            textures.get(&(texture_pak_path(path), srgb)).cloned()
        })?;
        // Vertex alpha fades the true ends of a spline; the diffuse alpha and separate opacity
        // texture soften its banks. Masking this loses the original soft transition.
        if !matches!(material.alpha_mode, AlphaMode::Add) {
            material.alpha_mode = AlphaMode::Blend;
        }
        let early_surface = self.early_surface();
        material.depth_bias = if early_surface { -600_000.0 } else { 0.0 } + self.priority as f32;
        let opacity = data
            .source
            .texture(&TextureMap::Opacity)
            .and_then(|t| textures.get(&(t.pak_path(), false)).cloned());
        let mut params = opacity_params(&data.source, opacity.is_some());
        params.attenuation = self.attenuation;
        if let Some(mut projection) = self.deferred {
            material.alpha_mode = AlphaMode::Blend;
            // Native DeferredDecalPass precedes translucent surfaces. Keep the overlay between
            // the sky (-1,000,000) and world transparency; preserve authored priority locally.
            material.depth_bias = -500_000.0 + self.priority as f32;
            let color = data.source.diffuse;
            material.base_color = Color::linear_rgba(color.x, color.y, color.z, data.source.opacity);
            projection.front.w = data.source.opacity;
            projection.depth.w = data.source.alpha_test;
            params.projection = projection;
            params.mask = Vec4::W;
            params.uv_x = Vec4::new(1.0, 0.0, 0.0, opacity.is_some() as u8 as f32);
            params.uv_y = Vec4::new(0.0, 1.0, 0.0, 0.0);
            // This supported slice changes diffuse/opacity on receiver geometry. Native MRT
            // normal/specular accumulation is deliberately not approximated with new normals.
            material.normal_map_texture = None;
            material.reflectance = 0.0;
            material.metallic = 0.0;
            material.perceptual_roughness = 1.0;
            material.emissive = LinearRgba::BLACK;
            material.emissive_texture = None;
            material.unlit = false;
            material.uv_transform = bevy::math::Affine2::IDENTITY;
            material.cull_mode = Some(bevy::render::render_resource::Face::Back);
            material.double_sided = false;
        }
        let extension = RoadExtension { params, opacity, early_surface };
        let handle = materials.add(ExtendedMaterial {
            base: material,
            extension,
        });
        if self.attenuation.w == 0.0 && self.deferred.is_none() {
            handles.insert(self.priority, handle.clone());
        }
        Some(handle)
    }
}

fn prepare_material(loader: &CryLoader, path: &str, clamp: bool) -> Option<MaterialData> {
    let path = if path.to_ascii_lowercase().ends_with(".mtl") {
        path.to_string()
    } else {
        format!("{path}.mtl")
    };
    let source = loader.mtl(&path)?;
    if source.no_draw() {
        return None;
    }
    let mut images = HashMap::new();
    for (map, srgb) in [
        (TextureMap::Diffuse, true),
        (TextureMap::Normal, false),
        (TextureMap::Bump, false),
        (TextureMap::Opacity, false),
    ] {
        let Some(texture) = source.texture(&map) else { continue };
        let key = texture.pak_path();
        let image = loader
            .resolve_texture(&key)
            .and_then(|path| aa_pak::Pak::global().read(&path).ok())
            .and_then(|bytes| load_dds(&bytes, DdsOptions { srgb, ..default() }).ok());
        if let Some(mut image) = image {
            if clamp {
                let mut sampler = aa_cry::texture::repeat_sampler();
                sampler.address_mode_u = bevy::image::ImageAddressMode::ClampToEdge;
                sampler.address_mode_v = bevy::image::ImageAddressMode::ClampToEdge;
                image.sampler = bevy::image::ImageSampler::Descriptor(sampler);
            }
            images.insert((key, srgb), image);
        }
    }
    Some(MaterialData {
        source,
        images: Mutex::new(Some(images)),
        textures: Mutex::new(HashMap::new()),
        handles: Mutex::new(HashMap::new()),
    })
}

/// `Illum` multiplies diffuse alpha by `dot(opacityChannelMask, opacityMap)`. Its UV
/// transform is independent of diffuse (11 of the 15 road masks use different tiling).
fn opacity_params(source: &Material, enabled: bool) -> OpacityParams {
    let mask: Vec<f32> = source
        .public_param("opacityChannelMask")
        .unwrap_or("0,0,0,1")
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let mask: [f32; 4] = mask.try_into().unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let tm = source
        .texture(&TextureMap::Opacity)
        .and_then(|t| t.tex_mod.clone())
        .unwrap_or_default();
    let transform = bevy::math::Affine2::from_scale_angle_translation(
        Vec2::new(tm.tile_u, tm.tile_v),
        tm.rotate_w.to_radians(),
        Vec2::new(tm.offset_u, tm.offset_v),
    );
    OpacityParams {
        mask: Vec4::from_array(mask),
        uv_x: Vec4::new(
            transform.matrix2.x_axis.x,
            transform.matrix2.y_axis.x,
            transform.translation.x,
            enabled as u8 as f32,
        ),
        uv_y: Vec4::new(
            transform.matrix2.x_axis.y,
            transform.matrix2.y_axis.y,
            transform.translation.y,
            0.0,
        ),
        attenuation: Vec4::ZERO,
        projection: DeferredParams::default(),
    }
}

pub(crate) fn prepare_cell(
    objects: &CellObjects,
    cell: (i32, i32),
    terrain: &TerrainQuery,
    loader: &CryLoader,
    assets: &RoadAssets,
) -> Vec<RoadMesh> {
    let mut out = Vec::new();
    for road in &objects.roads {
        let Some(path) = objects.materials.get(road.material as usize) else {
            continue;
        };
        let Some(mesh) = road_mesh(road, cell, terrain) else {
            continue;
        };
        let material = assets.material(loader, path, false);
        if material.get().is_none_or(|m| m.is_none()) {
            continue;
        }
        let min = Vec3::from_slice(&road.bbox[..3]);
        let max = Vec3::from_slice(&road.bbox[3..]);
        out.push(RoadMesh {
            mesh,
            material,
            priority: road.sort_priority,
            radius: (max - min).length() * 0.5,
            distance: road.max_view_dist,
            attenuation: Vec4::ZERO,
            deferred: None,
            source_road: true,
        });
    }
    out
}

#[derive(Clone, Copy)]
struct Vertex {
    pos: Vec3,
    normal: Vec3,
}

/// A normalized 2D inward-facing plane. Roads project vertically onto terrain.
#[derive(Clone, Copy)]
struct Edge {
    normal: Vec2,
    offset: f32,
}

impl Edge {
    fn new(a: Vec2, b: Vec2, inside: Vec2) -> Option<Self> {
        let mut normal = (b - a).perp().try_normalize()?;
        if normal.dot(inside - a) < 0.0 {
            normal = -normal;
        }
        Some(Self {
            normal,
            offset: -normal.dot(a),
        })
    }

    fn distance(self, pos: Vec3) -> f32 {
        self.normal.dot(pos.truncate()) + self.offset
    }
}

fn clip(mut polygon: Vec<Vertex>, edges: &[Edge; 4]) -> Vec<Vertex> {
    for &edge in edges {
        if polygon.is_empty() {
            break;
        }
        let mut result = Vec::with_capacity(polygon.len() + 1);
        let mut a = *polygon.last().unwrap();
        let mut da = edge.distance(a.pos);
        for &b in &polygon {
            let db = edge.distance(b.pos);
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                result.push(Vertex {
                    pos: a.pos.lerp(b.pos, t),
                    normal: a.normal.lerp(b.normal, t),
                });
            }
            if db >= 0.0 {
                result.push(b);
            }
            (a, da) = (b, db);
        }
        polygon = result;
    }
    polygon
}

fn road_mesh(road: &Road, cell: (i32, i32), terrain: &TerrainQuery) -> Option<Mesh> {
    let mut cells = HashMap::new();
    build_mesh(road, |ix, iy| {
        let (dx, dy) = (ix.div_euclid(CELL_UNITS as i32), iy.div_euclid(CELL_UNITS as i32));
        let heights = cells
            .entry((dx, dy))
            .or_insert_with(|| terrain.cell(cell.0 + dx, cell.1 + dy))
            .as_ref()?;
        let (lx, ly) = (
            ix.rem_euclid(CELL_UNITS as i32) as usize,
            iy.rem_euclid(CELL_UNITS as i32) as usize,
        );
        Some(std::array::from_fn(|i| {
            let (ox, oy) = [(0, 0), (1, 0), (1, 1), (0, 1)][i];
            Vertex {
                pos: Vec3::new(
                    (ix + ox as i32) as f32 * UNIT_SIZE,
                    (iy + oy as i32) as f32 * UNIT_SIZE,
                    heights.at(lx + ox, ly + oy),
                ),
                normal: Vec3::from(heights.normal(lx + ox, ly + oy)),
            }
        }))
    })
}

fn build_mesh(road: &Road, mut quad: impl FnMut(i32, i32) -> Option<[Vertex; 4]>) -> Option<Mesh> {
    if road.points.len() < 4
        || road.points.len() % 2 != 0
        || !road.points.iter().all(|p| p.is_finite())
        || !road
            .tex_coords
            .iter()
            .chain(&road.global_tex_coords)
            .all(|v| v.is_finite())
    {
        return None;
    }
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uv: Vec<[f32; 2]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices = Vec::new();
    let segments = road.points.len() / 2 - 1;
    let uv_step = (road.tex_coords[1] - road.tex_coords[0]) / segments as f32;
    for segment in 0..segments {
        let p = &road.points[segment * 2..segment * 2 + 4];
        let center = (p[0] + p[1] + p[2] + p[3]).truncate() * 0.25;
        let Some(edges) = (|| {
            Some([
                Edge::new(p[0].truncate(), p[1].truncate(), center)?,
                Edge::new(p[2].truncate(), p[3].truncate(), center)?,
                Edge::new(p[0].truncate(), p[2].truncate(), center)?,
                Edge::new(p[1].truncate(), p[3].truncate(), center)?,
            ])
        })() else {
            continue;
        };
        let min = p.iter().fold(Vec3::splat(f32::INFINITY), |a, &b| a.min(b));
        let max = p.iter().fold(Vec3::splat(f32::NEG_INFINITY), |a, &b| a.max(b));
        // A normal baked road is tens of metres wide; reject corrupt bounds before iteration.
        if (max.x - min.x) * (max.y - min.y) > CELL_SIZE * CELL_SIZE {
            continue;
        }
        let u0 = road.tex_coords[0] + uv_step * segment as f32;
        let u1 = u0 + uv_step;
        for iy in (min.y / UNIT_SIZE).floor() as i32..(max.y / UNIT_SIZE).ceil() as i32 {
            for ix in (min.x / UNIT_SIZE).floor() as i32..(max.x / UNIT_SIZE).ceil() as i32 {
                let Some(q) = quad(ix, iy) else { continue };
                for triangle in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    let polygon = clip(triangle.to_vec(), &edges);
                    if polygon.len() < 3 {
                        continue;
                    }
                    let base = positions.len() as u32;
                    for vertex in &polygon {
                        let distances: [f32; 4] = edges.map(|e| e.distance(vertex.pos).max(0.0));
                        let t = distances[0] / (distances[0] + distances[1]).max(1e-6);
                        let v = distances[2] / (distances[2] + distances[3]).max(1e-6);
                        let alpha = if (u0 - road.global_tex_coords[0]).abs() < 0.01 {
                            t
                        } else if (u1 - road.global_tex_coords[1]).abs() < 0.01 {
                            1.0 - t
                        } else {
                            1.0
                        };
                        positions.push(cry_to_bevy(vertex.pos + Vec3::Z * HEIGHT_OFFSET).to_array());
                        normals.push(cry_to_bevy(vertex.normal.normalize_or_zero()).to_array());
                        uv.push([u0.abs() * (1.0 - t) + u1.abs() * t, v]);
                        colors.push([1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0)]);
                    }
                    for i in 1..polygon.len() - 1 {
                        let a = polygon[i].pos - polygon[0].pos;
                        let b = polygon[i + 1].pos - polygon[0].pos;
                        if a.truncate().perp_dot(b.truncate()) > 1e-7 {
                            indices.extend([base, base + i as u32, base + i as u32 + 1]);
                        }
                    }
                }
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    let _ = mesh.generate_tangents();
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn road() -> Road {
        Road {
            bbox: [0.0, 0.0, 0.0, 8.0, 4.0, 5.0],
            max_view_dist: 500.0,
            flags: 0,
            tex_coords: [2.0, 4.0],
            global_tex_coords: [0.0, 6.0],
            material: 0,
            sort_priority: 0,
            // Deliberately above the terrain: roads must project onto the terrain triangles.
            points: vec![
                Vec3::new(0.3, 0.4, 100.0),
                Vec3::new(0.3, 4.4, 100.0),
                Vec3::new(8.3, 0.4, 100.0),
                Vec3::new(8.3, 4.4, 100.0),
            ],
        }
    }

    fn sloped_quad(ix: i32, iy: i32) -> Option<[Vertex; 4]> {
        Some(std::array::from_fn(|i| {
            let (dx, dy) = [(0, 0), (1, 0), (1, 1), (0, 1)][i];
            let (x, y) = ((ix + dx) as f32 * UNIT_SIZE, (iy + dy) as f32 * UNIT_SIZE);
            Vertex {
                pos: Vec3::new(x, y, x * 0.5 + y * 0.2),
                normal: Vec3::new(-0.5, -0.2, 1.0).normalize(),
            }
        }))
    }

    fn positions(mesh: &Mesh) -> &[[f32; 3]] {
        let Some(VertexAttributeValues::Float32x3(values)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("positions")
        };
        values
    }

    #[test]
    fn clips_exact_ground_triangles_and_preserves_surface_height() {
        let mesh = build_mesh(&road(), sloped_quad).unwrap();
        let positions = positions(&mesh);
        for &[x, height, minus_y] in positions {
            let y = -minus_y;
            assert!((0.29999..=8.30001).contains(&x), "{x}");
            assert!((0.39999..=4.40001).contains(&y), "{y}");
            assert!((height - (x * 0.5 + y * 0.2 + HEIGHT_OFFSET)).abs() < 1e-5);
        }
        let indices: Vec<_> = mesh.indices().unwrap().iter().collect();
        let area: f32 = indices
            .chunks_exact(3)
            .map(|tri| {
                let a = Vec3::from(positions[tri[0]]);
                let b = Vec3::from(positions[tri[1]]);
                let c = Vec3::from(positions[tri[2]]);
                let cross = (b - a).cross(c - a);
                assert!(cross.y > 0.0, "upward winding");
                cross.y * 0.5
            })
            .sum();
        assert!(
            (area - 32.0).abs() < 1e-4,
            "clipping must neither overlap nor leave gaps: {area}"
        );
    }

    #[test]
    fn only_global_spline_ends_fade_and_uv_keeps_cell_continuity() {
        let mut road = road();
        for (range, globals, start_alpha, end_alpha) in [
            ([2.0, 4.0], [0.0, 6.0], 1.0, 1.0),
            ([0.0, 2.0], [0.0, 6.0], 0.0, 1.0),
            ([4.0, 6.0], [0.0, 6.0], 1.0, 0.0),
            ([70.58527, 77.23568], [0.0, 82.43901], 1.0, 1.0),
        ] {
            road.tex_coords = range;
            road.global_tex_coords = globals;
            let mesh = build_mesh(&road, sloped_quad).unwrap();
            let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
                panic!("uv")
            };
            let Some(VertexAttributeValues::Float32x4(color)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else {
                panic!("color")
            };
            for ((&[x, _, _], uv), color) in positions(&mesh).iter().zip(uv).zip(color) {
                if (x - 0.3).abs() < 1e-5 {
                    assert!((uv[0] - range[0]).abs() < 1e-4);
                    assert!((color[3] - start_alpha).abs() < 1e-5);
                }
                if (x - 8.3).abs() < 1e-5 {
                    assert!((uv[0] - range[1]).abs() < 1e-4);
                    assert!((color[3] - end_alpha).abs() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn separate_opacity_keeps_its_own_tiling_rotation_and_channel_mask() {
        let xml = r#"<Material Shader="Illum"><Textures>
            <Texture Map="Diffuse" File="a.dds"><TexMod TileU="5" TileV="3"/></Texture>
            <Texture Map="Opacity" File="b.dds"><TexMod TileU="2" TileV="1" OffsetU="0.25" RotateW="90"/></Texture>
            </Textures><PublicParams opacityChannelMask="1,0,0,0"/></Material>"#;
        let source = Material::parse(xml).unwrap();
        let params = opacity_params(&source, true);
        let uv = Vec3::new(0.5, 0.75, 1.0);
        assert!((params.uv_x.truncate().dot(uv) + 0.5).abs() < 1e-5);
        assert!((params.uv_y.truncate().dot(uv) - 1.0).abs() < 1e-5);
        assert_eq!(params.mask, Vec4::X);
        assert_eq!(params.uv_x.w, 1.0);
        assert_eq!(opacity_params(&source, false).uv_x.w, 0.0);
    }

    #[test]
    fn only_type13_illum_qualifies_and_sort_key_does_not_change_opacity_uniforms() {
        for shader in ["Illum", "illum", "Decal", "Water", "Terrain.Layer"] {
            let material = Arc::new(OnceLock::new());
            material.set(Some(MaterialData {
                source: Arc::new(Material::parse(&format!(r#"<Material Shader="{shader}"/>"#)).unwrap()),
                images: Mutex::new(None), handles: Mutex::new(HashMap::new()), textures: Mutex::new(HashMap::new()),
            })).ok().unwrap();
            for source_road in [false, true] {
                let road = RoadMesh {
                    mesh: Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD),
                    material: material.clone(), priority: 17, radius: 1.0, distance: 100.0,
                    attenuation: Vec4::ZERO, deferred: None, source_road,
                };
                assert_eq!(road.early_surface(), source_road && shader.eq_ignore_ascii_case("Illum"));
            }
        }
        let source = Material::parse(r#"<Material Shader="Illum" Opacity="0.723"/>"#).unwrap();
        let params = opacity_params(&source, false);
        for early_surface in [false, true] {
            let extension = RoadExtension { params, opacity: None, early_surface };
            assert_eq!(RoadKey::from(&extension).0, early_surface);
            assert_eq!(extension.params.projection.center.w, 0.0);
            assert_eq!(extension.params.attenuation, Vec4::ZERO);
            assert_eq!(extension.params.mask, params.mask);
        }
        use bevy::core_pipeline::core_3d::TransparentSortingInfo3d;
        use bevy::render::render_phase::ViewRangefinder3d;
        for position in [Vec3::new(-38000.0, 500.0, -35000.0), Vec3::new(30000.0, 800.0, 10000.0)] {
            for forward in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
                let camera = Transform::from_translation(position).looking_to(forward, Vec3::Y);
                let range = ViewRangefinder3d::from_world_from_view(&camera.compute_affine());
                let key = |bias| TransparentSortingInfo3d::Sorted {
                    mesh_center: position + forward * 40.0, depth_bias: bias,
                }.sort_distance(&range);
                assert!(key(-600_000.0 + 255.0) < key(-500_000.0));
                assert!(key(-500_000.0 + 255.0) < key(-300_000.0));
                assert!(key(-300_000.0) < key(0.0));
            }
        }
    }

    #[test]
    fn original_lutesong_and_memoria_roads_project_across_cell_boundaries() {
        if !crate::pak_available() {
            return;
        }
        let terrain = TerrainQuery::new("main_world");
        for (cell, count) in [((14, 11), 11), ((10, 14), 35)] {
            let objects = crate::placement::load_cell_objects("main_world", cell.0, cell.1);
            assert_eq!(objects.roads.len(), count);
            let mut crosses_cell = false;
            for road in &objects.roads {
                assert!(road.material < objects.materials.len() as u32);
                assert!(objects.materials[road.material as usize].contains("aa_terrain/roads/"));
                let mesh = road_mesh(road, cell, &terrain).expect("real road has terrain and triangles");
                for &[x, height, minus_y] in positions(&mesh) {
                    crosses_cell |= !(0.0..=CELL_SIZE).contains(&x) || !(0.0..=CELL_SIZE).contains(&-minus_y);
                    // Original adjacent cells can disagree by centimetres on their shared edge
                    // (e.g. (10,14)/(10,15), x=514). Each side uses its own rendered triangle.
                    if [x, -minus_y]
                        .iter()
                        .any(|v| v.rem_euclid(CELL_SIZE).min((-v).rem_euclid(CELL_SIZE)) < 1e-4)
                    {
                        continue;
                    }
                    let (dx, dy) = ((x / CELL_SIZE).floor() as i32, (-minus_y / CELL_SIZE).floor() as i32);
                    let heights = terrain.cell(cell.0 + dx, cell.1 + dy).unwrap();
                    let z = heights.mesh_height(x - dx as f32 * CELL_SIZE, -minus_y - dy as f32 * CELL_SIZE);
                    assert!(
                        (height - z - HEIGHT_OFFSET).abs() < 0.03,
                        "road must match terrain triangles, cell={cell:?} xy=({x},{}) {height} vs {z}",
                        -minus_y
                    );
                }
            }
            assert!(crosses_cell, "spline sections continue outside the owning cell");
        }
    }
}
