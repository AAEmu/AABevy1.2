//! The customisable face texture: the base skin texture of the head with the character-create
//! decals composited on it (eyebrows, makeup, tattoo, cheek / beard "deco", scar) and the lips
//! tinted.
//!
//! The client builds this texture at runtime (`SetCustomizingEyebrow`, `...Deco`, `...MakeUp`,
//! `...Tattoo`, `...Scar`, `...LipColor` ...). Data used here:
//!
//! * base: `face/face01/<pfx>_face01_df.dds` (512x512; the `face00` texture named by the face
//!   material is a placeholder, its file `nu_m_face00_df copy.dds` does not exist),
//! * decals: `face_decal_assets` (`category_id` 1 scar (movable), 2 tattoo, 3 makeup, 4 eyebrow,
//!   5 deco), each a small BC3 image whose alpha is the shape: pixels are drawn at
//!   `(defaultX, defaultY)` (top-left, in base texture pixels), plus `move_x / move_y`, scale and
//!   rotation about their centre for the movable scar. Eyebrow and deco decals carry a constant
//!   grey; their colour is the character's `eyebrow_color` / `deco_color`, other decals use their
//!   own colours. Weight scales the alpha.
//! * lips: the green channel of `face/<pfx>_face_mask_new.dds` marks the lips; `lip_color`
//!   (`0xAABBGGRR`) tints them.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::Result;

/// One decal drawn on the face texture.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceLayer {
    /// Pak path of the decal DDS.
    pub path: String,
    /// Top-left corner in base texture pixels.
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    /// Degrees, about the decal centre.
    pub rotate: f32,
    /// 0..1, multiplies the decal alpha.
    pub weight: f32,
    /// Replaces the decal colour (eyebrow / deco colours), sRGB.
    pub tint: Option<[u8; 3]>,
}

/// `0xAABBGGRR` -> `[r, g, b]`.
pub fn abgr(c: u32) -> [u8; 3] {
    [(c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8]
}

/// Composites `layers` (and the optional lip tint with its mask) on the base texture; the result
/// is an sRGB image with a full mip chain.
pub fn compose_face(base: &str, layers: &[FaceLayer], lips: Option<(&str, [u8; 3])>) -> Result<Image> {
    let bytes = aa_cry::read(base)?;
    let dds = aa_cry::texture::Dds::parse(&bytes)?;
    let (w, h) = (dds.width as usize, dds.height as usize);
    let mut px = dds.decode_rgba8(0, 0);

    if let Some((mask_path, color)) = lips {
        if let Ok(mask_bytes) = aa_cry::read(mask_path) {
            if let Ok(mask) = aa_cry::texture::Dds::parse(&mask_bytes) {
                if mask.width as usize == w && mask.height as usize == h {
                    let m = mask.decode_rgba8(0, 0);
                    for (p, m) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
                        // lips are the yellow-orange part of the mask (red and green both set)
                        let a = (m[1] as f32 / 255.0) * 0.55;
                        if a > 0.0 {
                            let luma = (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / 255.0;
                            for i in 0..3 {
                                let tinted = (color[i] as f32 / 255.0 * (0.35 + luma * 0.9)).min(1.0);
                                let v = p[i] as f32 / 255.0;
                                p[i] = ((v + (tinted - v) * a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                            }
                        }
                    }
                }
            }
        }
    }

    for layer in layers {
        if layer.weight <= 0.0 {
            continue;
        }
        let Ok(bytes) = aa_cry::read(&layer.path) else { continue };
        let Ok(decal) = aa_cry::texture::Dds::parse(&bytes) else { continue };
        draw_layer(&mut px, w, h, &decal.decode_rgba8(0, 0), decal.width as usize, decal.height as usize, layer);
    }

    let mut data = Vec::with_capacity(px.len() * 4 / 3);
    let (mut mw, mut mh) = (w, h);
    let mut level = px;
    let mut mips = 1;
    loop {
        data.extend_from_slice(&level);
        if mw == 1 && mh == 1 {
            break;
        }
        let (nw, nh) = ((mw / 2).max(1), (mh / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(mw - 1);
                        let sy = (y * 2 + dy).min(mh - 1);
                        sum += level[(sy * mw + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        level = next;
        (mw, mh) = (nw, nh);
        mips += 1;
    }

    let size = Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.mip_level_count = mips;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(aa_cry::texture::repeat_sampler());
    Ok(image)
}

/// Alpha-blends one decal (`src`, RGBA8 `sw x sh`) into `dst` (RGBA8 `w x h`).
fn draw_layer(dst: &mut [u8], w: usize, h: usize, src: &[u8], sw: usize, sh: usize, layer: &FaceLayer) {
    let scale = if layer.scale > 0.0 { layer.scale } else { 1.0 };
    let (sw_f, sh_f) = (sw as f32, sh as f32);
    let (cx, cy) = (layer.x + sw_f * 0.5, layer.y + sh_f * 0.5);
    let (sin, cos) = layer.rotate.to_radians().sin_cos();
    // destination bounding box of the transformed decal
    let half = (sw_f.hypot(sh_f) * 0.5 * scale).ceil();
    let (x0, x1) = (((cx - half).floor().max(0.0)) as usize, ((cx + half).ceil().min(w as f32)) as usize);
    let (y0, y1) = (((cy - half).floor().max(0.0)) as usize, ((cy + half).ceil().min(h as f32)) as usize);
    let texel = |x: i32, y: i32| -> [f32; 4] {
        if x < 0 || y < 0 || x >= sw as i32 || y >= sh as i32 {
            return [0.0; 4];
        }
        let i = (y as usize * sw + x as usize) * 4;
        [src[i] as f32, src[i + 1] as f32, src[i + 2] as f32, src[i + 3] as f32]
    };
    for y in y0..y1 {
        for x in x0..x1 {
            // destination pixel centre -> decal pixel coordinates (inverse rotate and scale)
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let (u, v) = ((dx * cos + dy * sin) / scale + sw_f * 0.5, (-dx * sin + dy * cos) / scale + sh_f * 0.5);
            let (fu, fv) = (u - 0.5, v - 0.5);
            let (iu, iv) = (fu.floor() as i32, fv.floor() as i32);
            let (tu, tv) = (fu - iu as f32, fv - iv as f32);
            let (a, b, c, d) = (texel(iu, iv), texel(iu + 1, iv), texel(iu, iv + 1), texel(iu + 1, iv + 1));
            let mut s = [0f32; 4];
            for k in 0..4 {
                s[k] = (a[k] * (1.0 - tu) + b[k] * tu) * (1.0 - tv) + (c[k] * (1.0 - tu) + d[k] * tu) * tv;
            }
            let alpha = (s[3] / 255.0 * layer.weight).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let color = match layer.tint {
                Some(t) => [t[0] as f32, t[1] as f32, t[2] as f32],
                None => [s[0], s[1], s[2]],
            };
            let p = &mut dst[(y * w + x) * 4..][..4];
            for i in 0..3 {
                p[i] = (p[i] as f32 + (color[i] - p[i] as f32) * alpha).clamp(0.0, 255.0).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_byte_order() {
        // eyebrow colour of the first Nuian male preset: ABGR 0xFF00233C = rgb(60, 35, 0)
        assert_eq!(abgr(0xFF00_233C), [0x3C, 0x23, 0x00]);
        assert_eq!(abgr(4294489434), [0x5A, 0xB5, 0xF8]);
    }

    #[test]
    fn decal_blends_with_weight_and_offset() {
        let mut dst = vec![0u8; 8 * 8 * 4];
        let src = vec![255u8; 2 * 2 * 4];
        let layer = FaceLayer { path: String::new(), x: 3.0, y: 2.0, scale: 1.0, rotate: 0.0, weight: 0.5, tint: Some([200, 100, 0]) };
        draw_layer(&mut dst, 8, 8, &src, 2, 2, &layer);
        let at = |x: usize, y: usize| dst[(y * 8 + x) * 4..][..3].to_vec();
        assert_eq!(at(3, 2), vec![100, 50, 0]);
        assert_eq!(at(4, 3), vec![100, 50, 0]);
        assert_eq!(at(2, 2), vec![0, 0, 0]);
        assert_eq!(at(5, 2), vec![0, 0, 0]);
    }

    #[test]
    fn composes_the_real_face() {
        if !aa_cry::pak_available() {
            return;
        }
        let layer = FaceLayer {
            path: "game/objects/characters/nuian/male/face/decal_eyebrow/nu_m_decal_eyebrow001.dds".into(),
            x: 133.0,
            y: 92.0,
            scale: 1.0,
            rotate: 0.0,
            weight: 1.0,
            tint: Some([60, 35, 0]),
        };
        let img = compose_face("game/objects/characters/nuian/male/face/face01/nu_m_face01_df.dds", &[layer], None).unwrap();
        assert_eq!(img.texture_descriptor.mip_level_count, 10);
        assert_eq!(img.data.as_ref().unwrap().len(), (0..10).map(|m| ((512usize >> m).max(1)).pow(2) * 4).sum::<usize>());
    }
}
