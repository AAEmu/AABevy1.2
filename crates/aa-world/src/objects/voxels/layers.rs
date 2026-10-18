//! Bounded native Terrain.Layer diffuse/normal projection, separate from heightmap's 16-slot path.

use aa_cry::material::{Material, TextureMap};
use aa_cry::texture::{Dds, DdsOptions, load_dds};
use bevy::mesh::Indices;
use bevy::prelude::*;
use std::collections::HashMap;

use crate::placement::PlacedVoxel;

pub(super) const MAX_DRAWS: usize = 96;
pub(super) const MAX_TEXTURE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MESH_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Layer {
    pub texture: String,
    pub u: Vec4,
    pub v: Vec4,
    pub colour: Vec3,
    pub strength: f32,
    pub distance: f32,
    pub multi: bool,
    pub normal: Option<NormalLayer>,
}

#[derive(Clone)]
pub(super) struct NormalLayer {
    pub texture: String,
    /// 1: original BC1 XYZ, 2: raw ATI2 YX with reconstructed Z.
    pub mode: f32,
    pub u: Vec4,
    pub v: Vec4,
    pub detail: Option<String>,
    pub detail_u: Vec4,
    pub detail_v: Vec4,
    pub detail_scale: f32,
}

pub(super) struct Draw {
    pub mesh: Mesh,
    pub key: (u8, u8),
}

#[derive(Default)]
pub(super) struct Layers {
    pub definitions: HashMap<(u8, u8), Layer>,
    pub textures: HashMap<String, Image>,
    pub unsupported: usize,
    pub draws: usize,
    bytes: usize,
    texture_bytes: usize,
    names: Vec<String>,
}

/// Same truncation as renderer 38205AE0; 300DF410 decodes around 127.5, not 128.
pub(super) fn packed_axis(normal: Vec3) -> usize {
    let packed = normal
        .to_array()
        .map(|n| ((n * 127.5 + 128.0).clamp(0.0, 255.0) as u8 as f32 - 127.5).abs());
    if packed[0] >= packed[1] && packed[0] >= packed[2] {
        0
    } else if packed[1] >= packed[0] && packed[1] >= packed[2] {
        1
    } else {
        2
    }
}

#[cfg(test)]
fn fade(distance: f32, limit: f32) -> f32 {
    1.0 - (distance / limit.max(0.001)).min(1.0).powi(4)
}

fn named_palette(names: &[String], slots: &[String; 32]) -> [bool; 32] {
    let mut qualified = [false; 32];
    // Native resolves the first case-insensitive match, not every entry with that name.
    // This slice declines unknown-name fallback rather than choosing a different pointer.
    for name in slots.iter().filter(|name| !name.is_empty()) {
        if let Some(index) = names
            .iter()
            .take(32)
            .position(|entry| entry.eq_ignore_ascii_case(name))
        {
            qualified[index] = true;
        }
    }
    qualified
}

pub(super) fn projection(
    axis: usize,
    scale: f32,
    tile: Vec2,
    offset: Vec2,
    cell: (i32, i32),
) -> (Vec4, Vec4) {
    let su = scale * tile.x;
    let sv = scale * tile.y;
    let origin = [cell.0 as f64 * 1024.0, cell.1 as f64 * 1024.0];
    let fract = |v: f64| v.rem_euclid(1.0) as f32;
    // Native 300DC9B0: X -> (Y,-Z), Y -> (X,-Z), Z -> (X,-Y).
    match axis {
        0 => (
            Vec4::new(0.0, su, 0.0, fract(origin[1] * su as f64) + offset.x),
            Vec4::new(0.0, 0.0, -sv, offset.y),
        ),
        1 => (
            Vec4::new(su, 0.0, 0.0, fract(origin[0] * su as f64) + offset.x),
            Vec4::new(0.0, 0.0, -sv, offset.y),
        ),
        _ => (
            Vec4::new(su, 0.0, 0.0, fract(origin[0] * su as f64) + offset.x),
            Vec4::new(0.0, -sv, 0.0, fract(-origin[1] * sv as f64) + offset.y),
        ),
    }
}

impl Layers {
    pub fn load(world: &str, cell: (i32, i32)) -> Self {
        let mut out = Self::default();
        let mut normal_requests = Vec::new();
        let path = format!(
            "game/worlds/{world}/cells/{:03}_{:03}/client/leveldata.xml",
            cell.0, cell.1
        );
        let Ok(xml) = aa_pak::Pak::global().read_string(&path) else {
            return out;
        };
        let Ok(doc) = roxmltree::Document::parse(&xml) else {
            return out;
        };

        out.names = doc
            .descendants()
            .filter(|n| n.has_tag_name("SurfaceType"))
            .take(32)
            .map(|n| n.attribute("Name").unwrap_or("").to_string())
            .collect();
        for (surface, entry) in doc
            .descendants()
            .filter(|n| n.has_tag_name("SurfaceType"))
            .take(32)
            .enumerate()
        {
            let Some(path) = entry
                .attribute("DetailMaterial")
                .filter(|p| !p.trim().is_empty())
            else {
                continue;
            };
            let path = path.trim().replace('\\', "/").to_ascii_lowercase();
            let path = if path.ends_with(".mtl") {
                path
            } else {
                format!("{path}.mtl")
            };
            let Ok(material) = Material::load(&path) else {
                continue;
            };
            let multi = material.sub_materials.len() >= 3;
            if !multi && !material.sub_materials.is_empty() {
                continue;
            }
            let scale = entry
                .attribute("DetailScaleX")
                .unwrap_or("1")
                .parse::<f32>()
                .unwrap_or(f32::NAN);
            if !scale.is_finite() || scale <= 0.0 {
                continue;
            }
            let single_axis = match entry
                .attribute("ProjAxis")
                .and_then(|s| s.as_bytes().first())
            {
                Some(b'X') => Some(0),
                Some(b'Y') => Some(1),
                Some(b'Z') => Some(2),
                _ => None,
            };
            for axis in 0..3 {
                let m = if multi {
                    let Some(m) = material.sub_materials.iter().take(6).find(|m| match axis {
                        0 => m.name.eq_ignore_ascii_case("X") || m.name.eq_ignore_ascii_case("XP"),
                        1 => m.name.eq_ignore_ascii_case("Y") || m.name.eq_ignore_ascii_case("YP"),
                        _ => m.name.eq_ignore_ascii_case("Z") || m.name.eq_ignore_ascii_case("ZP"),
                    }) else {
                        continue;
                    };
                    m
                } else {
                    if single_axis != Some(axis) {
                        continue;
                    }
                    &material
                };
                if !m.shader.eq_ignore_ascii_case("Terrain.Layer")
                    || m.opacity != 1.0
                    || m.alpha_test != 0.0
                    || !m.diffuse.is_finite()
                {
                    continue;
                }
                let Some(texture) = m.texture(&TextureMap::Diffuse) else {
                    continue;
                };
                let mut tile = Vec2::ONE;
                let mut offset = Vec2::ZERO;
                if let Some(modifier) = &texture.tex_mod {
                    if modifier.rotate_w != 0.0
                        || modifier.offset_u != 0.0
                        || modifier.offset_v != 0.0
                        || !modifier.unhandled.is_empty()
                    {
                        continue;
                    }
                    tile = Vec2::new(modifier.tile_u, modifier.tile_v);
                    offset = Vec2::new(modifier.offset_u, modifier.offset_v);
                }
                if !tile.is_finite()
                    || !offset.is_finite()
                    || tile.min_element() <= 0.0
                    || !(tile * scale).is_finite()
                {
                    continue;
                }
                let strength = m
                    .public_param("DetailTextureStrength")
                    .unwrap_or("1")
                    .parse::<f32>()
                    .unwrap_or(f32::NAN);
                if !strength.is_finite() {
                    continue;
                }
                let path = texture.pak_path();
                if !out.textures.contains_key(&path) {
                    let Ok(bytes) = aa_cry::read(&path) else {
                        continue;
                    };
                    let Ok(dds) = Dds::parse(&bytes) else {
                        continue;
                    };
                    if dds.faces != 1
                        || dds.depth != 1
                        || dds.width > 4096
                        || dds.height > 4096
                        || dds.data.len() > MAX_TEXTURE_BYTES - out.texture_bytes
                    {
                        continue;
                    }
                    let Ok(image) = load_dds(&bytes, DdsOptions::linear()) else {
                        continue;
                    };
                    let size = image.data.as_ref().map_or(0, Vec::len);
                    if size > MAX_TEXTURE_BYTES - out.texture_bytes {
                        continue;
                    }
                    out.texture_bytes += size;
                    out.textures.insert(path.clone(), image);
                }
                normal_requests.push(((surface as u8, axis as u8), m.clone(), axis, scale, tile));
                let (u, v) = projection(axis, scale, tile, offset, cell);
                out.definitions.insert(
                    (surface as u8, axis as u8),
                    Layer {
                        texture: path,
                        u,
                        v,
                        colour: m.diffuse,
                        strength,
                        distance: if multi { 1024.0 } else { 128.0 },
                        multi,
                        normal: None,
                    },
                );
            }
        }
        // Existing diffuse receivers have priority within the same 32 MiB image budget.
        for (key, material, axis, scale, tile) in normal_requests {
            let normal = super::normals::load(
                &material,
                axis,
                scale,
                tile,
                cell,
                &mut out.textures,
                &mut out.texture_bytes,
            );
            out.definitions.get_mut(&key).unwrap().normal = normal;
        }
        out
    }

    pub fn prepare(
        &mut self,
        voxel: &PlacedVoxel,
        source: &aa_cry::mesh::MeshData,
        ids: &[u8],
        base: &Mesh,
    ) -> Vec<Draw> {
        // Original fixtures use translation-only voxel matrices. Avoid inventing how rotated
        // authored volumes combine native local index-axis and world vertex-axis selection.
        let (lo, hi) = source.positions.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        // Compiled native surfaces overlap the serialized voxel box (about .46 m in this
        // original fixture). Bound actual geometry; do not clip/reject that authored overlap.
        if !Mat3::from_mat4(voxel.mat4()).abs_diff_eq(Mat3::IDENTITY, 1e-6)
            || !(hi - lo).is_finite()
            || (hi - lo).max_element() > 1024.0
        {
            self.unsupported += 1;
            return Vec::new();
        }
        let axes: Vec<_> = source.normals.iter().map(|n| packed_axis(*n)).collect();
        let qualified = named_palette(&self.names, &voxel.surfaces);
        let mut out = Vec::new();
        for surface in 0..32 {
            if !ids.contains(&(surface as u8)) {
                continue;
            }
            if !qualified[surface] {
                self.unsupported += 1;
                continue;
            }
            if !(0..3).any(|axis| self.definitions.contains_key(&(surface as u8, axis))) {
                self.unsupported += 1;
                continue;
            }
            for axis in 0..3 {
                let Some(layer) = self.definitions.get(&(surface as u8, axis as u8)) else {
                    continue;
                };
                let weights: Vec<_> = ids
                    .iter()
                    .enumerate()
                    .map(|(i, &id)| {
                        if id as usize == surface && (!layer.multi || axes[i] == axis) {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let indices: Vec<u32> = base
                    .indices()
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>()
                    .chunks_exact(3)
                    .filter(|tri| tri.iter().any(|&i| weights[i] != 0.0))
                    .flat_map(|tri| tri.iter().map(|&i| i as u32))
                    .collect();
                if indices.is_empty() {
                    continue;
                }
                let bytes = base.count_vertices() * 56 + indices.len() * 4;
                if self.draws >= MAX_DRAWS || bytes > MAX_MESH_BYTES.saturating_sub(self.bytes) {
                    self.unsupported += 1;
                    continue;
                }
                self.draws += 1;
                self.bytes += bytes;
                let mut mesh = base.clone();
                mesh.insert_indices(Indices::U32(indices));
                // Native TEMP_TERRAIN expands the renderer's packed bytes without normalization.
                // Preserve that vector for the independent projected vertex frame.
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_NORMAL,
                    source
                        .normals
                        .iter()
                        .map(|n| crate::cry_to_bevy(super::normals::packed_normal(*n)))
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_1,
                    weights
                        .into_iter()
                        .map(|w| [w, layer.distance])
                        .collect::<Vec<_>>(),
                );
                out.push(Draw {
                    mesh,
                    key: (surface as u8, axis as u8),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_signed_projection_and_fade_order_are_numeric_contracts() {
        let p = Vec3::new(2.0, 3.0, 4.0).extend(1.0);
        for (axis, expected) in [
            (0, Vec2::new(6.0, -24.0)),
            (1, Vec2::new(4.0, -24.0)),
            (2, Vec2::new(4.0, -18.0)),
        ] {
            let (u, v) = projection(axis, 2.0, Vec2::new(1.0, 3.0), Vec2::ZERO, (0, 0));
            assert_eq!(Vec2::new(u.dot(p), v.dot(p)), expected);
        }
        assert_eq!(fade(0.0, 128.0), 1.0);
        assert_eq!(fade(64.0, 128.0), 0.9375);
        assert_eq!(fade(128.0, 128.0), 0.0);
        assert_eq!(fade(2048.0, 1024.0), 0.0);
        // Vertex fade is multiplied before interpolation and sqrt in the fragment.
        let native = (0.5 * fade(64.0, 128.0)).sqrt();
        assert!((native - 0.6846532).abs() < 1e-6);
        assert!((native - 0.5_f32.sqrt() * fade(64.0, 128.0)).abs() > 0.02);
        assert_eq!(packed_axis(Vec3::new(-0.4, 0.4, 0.0)), 1);
        assert_eq!(packed_axis(Vec3::new(0.5, 0.5, 0.0)), 0);
        let names = [
            "rock".to_string(),
            "default".to_string(),
            "default".to_string(),
        ];
        let mut slots: [String; 32] = Default::default();
        slots[0] = "DEFAULT".to_string();
        slots[1] = "missing".to_string();
        let palette = named_palette(&names, &slots);
        assert!(palette[1] && !palette[0] && !palette[2]);
        assert_eq!(palette.iter().filter(|v| **v).count(), 1);
    }

    #[test]
    fn original_voxel_layer_palette_projection_texture_and_budget_contracts() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 20, 8);
        let (source, ids) = objects.voxels[0].compiled_surface_mesh().unwrap();
        assert_eq!(ids[0], 12);
        let mut layers = Layers::load("main_world", (20, 8));
        let rock = &layers.definitions[&(12, 0)];
        assert!(rock.texture.ends_with("tr_canyon_rock3d_406b01_df.dds"));
        assert!(rock.multi);
        assert_eq!(rock.distance, 1024.0);
        assert!((rock.u.y - 0.03).abs() < 1e-6 && (rock.v.z + 0.03).abs() < 1e-6);
        assert_eq!(rock.colour, Vec3::new(0.32503697, 0.26735809, 0.25084025));
        assert!(
            layers.textures[&rock.texture].texture_descriptor.format
                == bevy::render::render_resource::TextureFormat::Bc1RgbaUnorm
        );
        assert!(
            layers
                .definitions
                .values()
                .any(|layer| !layer.multi && layer.distance == 128.0)
        );
        let mesh = super::super::mesh(&objects.voxels[0], source.clone(), None).unwrap();
        let draws = layers.prepare(&objects.voxels[0], &source, &ids, &mesh);
        assert!(!draws.is_empty());
        assert!(draws.iter().any(|d| d.key == (12, 0)));
        for draw in &draws {
            let Some(bevy::mesh::VertexAttributeValues::Float32x2(weights)) =
                draw.mesh.attribute(Mesh::ATTRIBUTE_UV_1)
            else {
                panic!()
            };
            let layer = &layers.definitions[&draw.key];
            for (i, w) in weights.iter().enumerate() {
                assert_eq!(w[1], layer.distance);
                assert_eq!(
                    w[0],
                    if ids[i] == draw.key.0
                        && (!layer.multi || packed_axis(source.normals[i]) == draw.key.1 as usize)
                    {
                        1.0
                    } else {
                        0.0
                    }
                );
            }
            assert!(
                draw.mesh
                    .indices()
                    .unwrap()
                    .iter()
                    .collect::<Vec<_>>()
                    .chunks_exact(3)
                    .all(|tri| tri.iter().any(|&i| weights[i][0] > 0.0))
            );
        }
        for _ in 0..100 {
            let _ = layers.prepare(&objects.voxels[0], &source, &ids, &mesh);
        }
        assert!(
            layers.draws <= MAX_DRAWS && layers.bytes <= MAX_MESH_BYTES && layers.unsupported > 0
        );
        let mut rotated = objects.voxels[0].clone();
        rotated.matrix[0] = 0.0;
        rotated.matrix[1] = -1.0;
        rotated.matrix[4] = 1.0;
        rotated.matrix[5] = 0.0;
        assert!(layers.prepare(&rotated, &source, &ids, &mesh).is_empty());
        info!(
            "native voxel layers: definitions={} textures={} first_draws={} bounded_draws={} bounded_bytes={}",
            layers.definitions.len(),
            layers.textures.len(),
            draws.len(),
            layers.draws,
            layers.bytes
        );
    }
}
