//! Native Terrain.Layer normal inputs. These textures retain their original GPU channels.

use aa_cry::material::{Material, Texture, TextureMap};
use aa_cry::texture::{Dds, DdsFormat, DdsOptions, dds_to_image};
use bevy::prelude::*;
use std::collections::HashMap;

use super::layers::{MAX_TEXTURE_BYTES, NormalLayer, projection};

/// Renderer 38205AE0 followed by TEMP_TERRAIN's EXPAND(IN.Normal.xyz).
pub(super) fn packed_normal(n: Vec3) -> Vec3 {
    Vec3::from_array(
        n.to_array()
            .map(|c| ((c * 127.5 + 128.0).clamp(0.0, 255.0) as u8 as f32) * (2.0 / 255.0) - 1.0),
    )
}

fn tile(texture: &Texture) -> Option<Vec2> {
    let Some(t) = &texture.tex_mod else {
        return Some(Vec2::ONE);
    };
    let tile = Vec2::new(t.tile_u, t.tile_v);
    (tile.is_finite()
        && tile.min_element() > 0.0
        && t.offset_u == 0.0
        && t.offset_v == 0.0
        && t.rotate_w == 0.0
        && t.unhandled.is_empty())
    .then_some(tile)
}

/// Raw ATI2 is intentionally *not* passed through aa-cry's semantic XY swap. The original
/// D3D10 GetNormalMap performs YX itself; detailMapSampler reads the raw XY without that swap.
fn image(bytes: &[u8], remaining: usize) -> Option<(Image, f32)> {
    let mut dds = Dds::parse(bytes).ok()?;
    let mode = match dds.format {
        DdsFormat::Bc1 => 1.0,
        DdsFormat::Bc5 if dds.swapped_rg => 2.0,
        _ => return None,
    };
    if dds.faces != 1
        || dds.depth != 1
        || dds.width > 4096
        || dds.height > 4096
        || dds.data.len() > remaining
    {
        return None;
    }
    dds.swapped_rg = false;
    let image = dds_to_image(&dds, DdsOptions::linear()).ok()?;
    (image.data.as_ref()?.len() <= remaining).then_some((image, mode))
}

fn texture(path: &str, textures: &mut HashMap<String, Image>, bytes: &mut usize) -> Option<f32> {
    if let Some(image) = textures.get(&format!("@raw:{path}")) {
        return match image.texture_descriptor.format {
            bevy::render::render_resource::TextureFormat::Bc1RgbaUnorm => Some(1.0),
            bevy::render::render_resource::TextureFormat::Bc5RgUnorm => Some(2.0),
            _ => None,
        };
    }
    let raw = aa_cry::read(path).ok()?;
    let (image, mode) = image(&raw, MAX_TEXTURE_BYTES.saturating_sub(*bytes))?;
    *bytes += image.data.as_ref()?.len();
    textures.insert(format!("@raw:{path}"), image);
    Some(mode)
}

pub(super) fn load(
    m: &Material,
    axis: usize,
    scale: f32,
    diffuse_tile: Vec2,
    cell: (i32, i32),
    textures: &mut HashMap<String, Image>,
    bytes: &mut usize,
) -> Option<NormalLayer> {
    // terrain.ext: BUMP_MAP=4; OFFSETBUMPMAPPING=1000, PARALLAX=8000000.
    // Those coordinate-displacement branches need their own audited receiver implementation.
    if m.gen_mask & 4 == 0 || m.gen_mask & (0x1000 | 0x8000000) != 0 {
        return None;
    }
    let bump = m
        .texture(&TextureMap::Normal)
        .or_else(|| m.texture(&TextureMap::Bump))?;
    let bump_tile = tile(bump)?;
    let (u, v) = projection(axis, scale, bump_tile, Vec2::ZERO, cell);
    if !u.is_finite() || !v.is_finite() {
        return None;
    }
    let path = bump.pak_path();
    let mode = texture(&path, textures, bytes)?;
    let mut out = NormalLayer {
        texture: format!("@raw:{path}"),
        mode,
        u,
        v,
        detail: None,
        detail_u: Vec4::ZERO,
        detail_v: Vec4::ZERO,
        detail_scale: 0.0,
    };
    if m.gen_mask & 0x8000 != 0 {
        let param = |name: &str, default: f32| {
            m.public_param(name)
                .map_or(Some(default), |s| s.parse::<f32>().ok())
        };
        let detail_tile = Vec2::new(
            param("DetailBumpTillingU", 1.0)?,
            param("DetailBumpTillingV", 1.0)?,
        );
        let strength = param("DetailBumpScale", 1.0)?;
        if !detail_tile.is_finite() || detail_tile.min_element() <= 0.0 || !strength.is_finite() {
            return Some(out);
        }
        if let Some(detail) = m.texture(&TextureMap::Detail) {
            let path = detail.pak_path();
            // fraglib samples detail at diffuseUV * authored tiling, without TS_DETAIL TexMod.
            // Resolve the phase before wrapping so noninteger tiling remains cell-continuous.
            let (du, dv) = projection(axis, scale, diffuse_tile * detail_tile, Vec2::ZERO, cell);
            if du.is_finite() && dv.is_finite() && texture(&path, textures, bytes).is_some() {
                out.detail = Some(format!("@raw:{path}"));
                out.detail_u = du;
                out.detail_v = dv;
                out.detail_scale = strength;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_ati2_raw_channels_and_native_projected_normal_contract() {
        if !crate::pak_available() {
            return;
        }
        let bytes =
            aa_cry::read("textures/aa_terrain/canyon/tr_canyon_rock3d_406b01_ddn.dds").unwrap();
        let dds = Dds::parse(&bytes).unwrap();
        let (image, mode) = image(&bytes, MAX_TEXTURE_BYTES).unwrap();
        assert_eq!(mode, 2.0);
        assert_eq!(image.data.as_deref().unwrap(), dds.data);
        assert!(super::image(&bytes, dds.data.len() - 1).is_none());
        let rock = super::super::layers::Layers::load("main_world", (20, 8));
        let normal = rock.definitions[&(12, 0)].normal.as_ref().unwrap();
        assert_eq!((normal.mode, normal.detail_scale), (2.0, 1.0));
        assert!(
            normal
                .detail
                .as_ref()
                .unwrap()
                .ends_with("cliff_grey_blue_with_dry_bushes_ddn.dds")
        );
        assert!((normal.detail_u.y - 0.18).abs() < 1e-6 && (normal.detail_v.z + 0.18).abs() < 1e-6);
        assert!((normal.u.y - 0.03).abs() < 1e-6 && (normal.v.z + 0.03).abs() < 1e-6);

        let original =
            Material::load("materials/aa_terrain/canyon/tr_canyon_rock3d_406b.mtl").unwrap();
        let mut variant = original.sub_materials[0].clone();
        let mut textures = HashMap::new();
        let mut bytes = MAX_TEXTURE_BYTES;
        assert!(
            load(
                &variant,
                0,
                0.2,
                Vec2::splat(0.15),
                (20, 8),
                &mut textures,
                &mut bytes
            )
            .is_none()
        );
        assert!(textures.is_empty() && bytes == MAX_TEXTURE_BYTES);
        bytes = 0;
        variant.gen_mask |= 0x1000;
        assert!(
            load(
                &variant,
                0,
                0.2,
                Vec2::splat(0.15),
                (20, 8),
                &mut textures,
                &mut bytes
            )
            .is_none()
        );
        assert!(textures.is_empty());
        variant.gen_mask &= !0x1000;
        variant
            .textures
            .iter_mut()
            .find(|t| t.map == TextureMap::Bump)
            .unwrap()
            .tex_mod
            .as_mut()
            .unwrap()
            .offset_u = 0.25;
        assert!(
            load(
                &variant,
                0,
                0.2,
                Vec2::splat(0.15),
                (20, 8),
                &mut textures,
                &mut bytes
            )
            .is_none()
        );
        assert!(textures.is_empty());

        // A packed nonunit native normal must not be normalized before vertex projection.
        let n = packed_normal(Vec3::new(0.6, 0., 0.8));
        assert_eq!(n, Vec3::new(204., 128., 230.) * (2. / 255.) - Vec3::ONE);
        let t = (Vec3::X - n * Vec3::X.dot(n)).normalize();
        let b = (Vec3::NEG_Y - n * Vec3::NEG_Y.dot(n)).normalize();
        // common.cfi BC1 retains XYZ; ATI2 reconstructs Z before detail XY addition.
        let base = Vec2::new(0.2, -0.4);
        let z = (1. - base.length_squared()).sqrt();
        let detail = Vec2::new(0.4, 0.3);
        let bumped = Vec3::new(base.x + detail.x, base.y + detail.y, z);
        let native = (t * bumped.x + b * bumped.y + n * bumped.z).normalize();
        let wrong =
            (t * bumped.x + b * bumped.y + n * (1. - bumped.truncate().length_squared()).sqrt())
                .normalize();
        assert!(native.distance(wrong) > 0.03);
    }
}
