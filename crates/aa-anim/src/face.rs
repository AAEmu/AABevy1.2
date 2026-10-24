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
//!   0.5 grey and modulate the skin: `skin * colour * 2 * grey` with the character's `eyebrow_color`
//!   / `deco_color` (the brows and the nuian stubble darken the skin and keep its painted strands);
//!   other decals blend their own colours. Weight scales the alpha.
//! * lips: the orange part (red and green set) of `face/<pfx>_face_mask_new.dds` (Nuian male) or
//!   `face/<pfx>_face_mask.dds` marks the lips; `lip_color` (`0xAABBGGRR`) tints them.

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

/// Where the eye texture sits in the face atlas and the iris colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeTint {
    /// `[u_min, v_min, u_max, v_max]` of the eye mesh's UVs (the square holding the eyeball texture).
    pub uv: [f32; 4],
    pub color: [u8; 3],
}

/// `0xAABBGGRR` -> `[r, g, b]`.
pub fn abgr(c: u32) -> [u8; 3] {
    [(c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8]
}

/// Everything the composed face texture of a character depends on (the files are named by path, so
/// two keys that compare equal compose the same image). Build it with
/// [`crate::character::face_key`]; [`take_face`] / [`prewarm_face`] memoize by it.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceKey {
    /// Pak path of the base skin texture.
    pub base: String,
    pub layers: Vec<FaceLayer>,
    /// Pak path of the lip mask and the lip colour.
    pub lips: Option<(String, [u8; 3])>,
    /// Eyeball rectangles with the iris colour.
    pub eyes: Vec<EyeTint>,
}

impl FaceKey {
    /// A string that is the same for equal keys (floats are printed exactly).
    pub fn id(&self) -> String {
        format!("{self:?}")
    }

    /// The composition itself, not memoized: [`compose_face_eyes`].
    pub fn compose(&self) -> Result<Image> {
        compose_face_eyes(&self.base, &self.layers, self.lips.as_ref().map(|(p, c)| (p.as_str(), *c)), &self.eyes)
    }
}

type ComposedFaces = std::collections::HashMap<String, Image>;

/// Composed faces waiting for the main thread to turn them into an asset (`id` -> image). Composing
/// is 20-30 ms (BC decode, decals, lips, eyes, mip chain): worker threads fill this ahead of the
/// unit that needs it ([`prewarm_face`]).
static COMPOSED: std::sync::Mutex<Option<ComposedFaces>> = std::sync::Mutex::new(None);

fn composed() -> std::sync::MutexGuard<'static, Option<ComposedFaces>> {
    COMPOSED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Composes the face of `key` and keeps it for [`take_face`]; a no-op when it is already there.
/// Callable from any thread (CPU only, no Bevy resources). Returns whether it composed.
pub fn prewarm_face(key: &FaceKey) -> Result<bool> {
    let id = key.id();
    if composed().get_or_insert_with(Default::default).contains_key(&id) {
        return Ok(false);
    }
    let image = key.compose()?;
    composed().get_or_insert_with(Default::default).entry(id).or_insert(image);
    Ok(true)
}

/// The composed face of `key`: the one [`prewarm_face`] made (moved out of the store), else composed
/// now. The caller owns it and makes the asset (`images.add`); share the handle between equal keys.
pub fn take_face(key: &FaceKey) -> Result<Image> {
    if let Some(image) = composed().as_mut().and_then(|m| m.remove(&key.id())) {
        return Ok(image);
    }
    key.compose()
}

/// Drops a prewarmed face nobody needs any more (an equal one is already an asset).
pub fn discard_face(key: &FaceKey) {
    if let Some(m) = composed().as_mut() {
        m.remove(&key.id());
    }
}

/// Composites `layers` (and the optional lip tint with its mask) on the base texture; the result
/// is an sRGB image with a full mip chain.
pub fn compose_face(base: &str, layers: &[FaceLayer], lips: Option<(&str, [u8; 3])>, eye: Option<EyeTint>) -> Result<Image> {
    compose_face_eyes(base, layers, lips, eye.as_slice())
}

/// [`compose_face`] with any number of eyeball rectangles (the ferre atlas has one per eye).
pub fn compose_face_eyes(base: &str, layers: &[FaceLayer], lips: Option<(&str, [u8; 3])>, eyes: &[EyeTint]) -> Result<Image> {
    let bytes = aa_cry::read(base)?;
    let dds = aa_cry::texture::Dds::parse(&bytes)?;
    let (w, h) = (dds.width as usize, dds.height as usize);
    let mut px = dds.decode_rgba8(0, 0);

    if let Some((mask_path, color)) = lips {
        let mask_bytes = aa_cry::read(mask_path)?;
        let mask = aa_cry::texture::Dds::parse(&mask_bytes)?;
        if mask.width as usize != w || mask.height as usize != h {
            return Err(crate::Error::Format(format!("lip mask size differs: {mask_path}")));
        }
        let m = mask.decode_rgba8(0, 0);
        for (p, m) in px.chunks_exact_mut(4).zip(m.chunks_exact(4)) {
            // lips are the yellow-orange part of the mask (red and green both set); the
            // red disc around them and the green patches of some masks are not lips
            let mask = m[0].min(m[1]) as f32 / 255.0;
            if mask > 0.0 {
                let luma = (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / 255.0;
                // skin-coloured lips are tinted part way; the ferre lips are painted grey
                // for the lip colour to replace (a pink-red mouth, not a lilac one)
                let sat = (p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2])) as f32 / 255.0;
                let grey = 1.0 - (sat / 0.15).clamp(0.0, 1.0);
                let a = mask * 0.55 + ((mask * 2.2).min(0.7) - mask * 0.55) * grey;
                for i in 0..3 {
                    let tinted = (color[i] as f32 / 255.0 * (0.35 + luma * 0.9)).min(1.0);
                    let v = p[i] as f32 / 255.0;
                    p[i] = ((v + (tinted - v) * a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }

    for eye in eyes {
        tint_iris(&mut px, w, h, *eye);
    }

    for layer in layers {
        if layer.weight <= 0.0 {
            continue;
        }
        let bytes = aa_cry::read(&layer.path)?;
        let decal = aa_cry::texture::Dds::parse(&bytes)?;
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
                    let mut sum = 0f32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(mw - 1);
                        let sy = (y * 2 + dy).min(mh - 1);
                        let v = level[(sy * mw + sx) * 4 + c];
                        // colour is averaged as light, not as sRGB codes (which darken contrasty
                        // textures: tattoos on skin lose a quarter of their brightness in the mips
                        // a face uses at game distance); alpha is plain
                        sum += if c < 3 { srgb_to_linear(v) } else { v as f32 / 255.0 };
                    }
                    let avg = sum / 4.0;
                    next[(y * nw + x) * 4 + c] = if c < 3 { linear_to_srgb(avg) } else { (avg * 255.0).round() as u8 };
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

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(l: f32) -> u8 {
    let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Recolours the grey iris disc (about 21% of the eye square's width in radius, around its
/// centre; the engine's mask is the eye material's Specular map red) by multiplying it with the
/// colour in linear light; the pupil, sclera and veins are left alone.
fn tint_iris(px: &mut [u8], w: usize, h: usize, eye: EyeTint) {
    let [u0, v0, u1, v1] = eye.uv;
    let (cx, cy) = ((u0 + u1) * 0.5 * w as f32, (v0 + v1) * 0.5 * h as f32);
    let radius = 0.215 * (u1 - u0).min(v1 - v0) * w as f32;
    if radius < 1.0 {
        return;
    }
    let (x0, x1) = ((cx - radius).floor().max(0.0) as usize, ((cx + radius).ceil() as usize).min(w));
    let (y0, y1) = ((cy - radius).floor().max(0.0) as usize, ((cy + radius).ceil() as usize).min(h));
    for y in y0..y1 {
        for x in x0..x1 {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt() / radius;
            if d > 1.0 {
                continue;
            }
            let p = &mut px[(y * w + x) * 4..][..4];
            let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
            let luma = r * 0.3 + g * 0.59 + b * 0.11;
            // not the black pupil, not the red veins of the sclera
            let vein = ((r - g.max(b) - 0.1) / 0.15).clamp(0.0, 1.0);
            let lit = ((luma - 0.08) / 0.12).clamp(0.0, 1.0);
            let edge = ((1.0 - d) / 0.12).clamp(0.0, 1.0);
            let a = (1.0 - vein) * lit * edge;
            if a <= 0.0 {
                continue;
            }
            // `eye.cfx`: `diffuse *= lerp(1, IrisColor, gloss.r)` in linear light (both samplers decode sRGB) on the
            // eye material's own atlas, the `face00` one, whose iris is grey. The `face01` atlases the face is composed
            // on have a grey iris too, except the female elf's (painted teal): the iris is taken as its grey, and the
            // colour multiplies it and its painted darker rim (replacing the texel by the picker colour, or keeping
            // the teal, gave a bright saturated cyan where the captures have a muted grey-blue)
            let grey = srgb_to_linear(p[0]) * 0.2126 + srgb_to_linear(p[1]) * 0.7152 + srgb_to_linear(p[2]) * 0.0722;
            for (c, iris) in p.iter_mut().zip(eye.color) {
                let (iris, own) = (srgb_to_linear(iris), srgb_to_linear(*c));
                *c = linear_to_srgb((own + (grey - own) * a) * (1.0 + (iris - 1.0) * a));
            }
        }
    }
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
            // bilinear with colour weighted by alpha: the colour of transparent texels (black or
            // white in the decal files) must not bleed into the edge of the shape
            let weights = [(1.0 - tu) * (1.0 - tv), tu * (1.0 - tv), (1.0 - tu) * tv, tu * tv];
            let texels = [a, b, c, d];
            let mut s = [0f32; 4];
            let mut wa = 0.0;
            for (t, w) in texels.iter().zip(weights) {
                s[3] += t[3] * w;
                wa += t[3] * w;
                for k in 0..3 {
                    s[k] += t[k] * t[3] * w;
                }
            }
            for v in s.iter_mut().take(3) {
                *v = if wa > 0.0 { *v / wa } else { 0.0 };
            }
            let alpha = (s[3] / 255.0 * layer.weight).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let p = &mut dst[(y * w + x) * 4..][..4];
            match layer.tint {
                // eyebrow / deco: a mid-grey (0.5) decal that modulates the skin, `skin * colour * 2 * grey`:
                // the brows and the stubble darken the skin and keep the strands painted into it (a blend
                // towards the flat colour turned blond characters' brows ginger and washed the stubble out)
                Some(t) => {
                    for i in 0..3 {
                        let m = 1.0 + (t[i] as f32 / 255.0 * (s[i] / 127.5) - 1.0) * alpha;
                        p[i] = (p[i] as f32 * m).clamp(0.0, 255.0).round() as u8;
                    }
                }
                None => {
                    for i in 0..3 {
                        p[i] = (p[i] as f32 + (s[i] - p[i] as f32) * alpha).clamp(0.0, 255.0).round() as u8;
                    }
                }
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
    fn mips_average_light_and_decal_edges_keep_their_colour() {
        // half black, half white averages to sRGB 188 (linear 0.5), not to the code 128
        assert_eq!(linear_to_srgb((srgb_to_linear(0) + srgb_to_linear(255)) / 2.0), 188);
        // a decal texel of colour (200, 100, 0) next to a transparent black one: the edge keeps its colour
        let mut dst = vec![0u8; 4 * 4 * 4];
        let src = [200, 100, 0, 255, 0, 0, 0, 0];
        let layer = FaceLayer { path: String::new(), x: 1.0, y: 1.0, scale: 1.0, rotate: 0.0, weight: 1.0, tint: None };
        draw_layer(&mut dst, 4, 4, &src, 2, 1, &layer);
        let r = dst[(1 * 4 + 1) * 4];
        assert_eq!(r, 200, "opaque texel unchanged: {r}");
        let edge = dst[(1 * 4 + 2) * 4..][..3].to_vec();
        assert!(edge == vec![0, 0, 0], "fully transparent texel draws nothing: {edge:?}");
    }

    #[test]
    fn prewarmed_faces_are_made_once_and_taken_once() {
        if !aa_cry::pak_available() {
            return;
        }
        let key = FaceKey {
            base: "game/objects/characters/nuian/male/face/face01/nu_m_face01_df.dds".into(),
            layers: Vec::new(),
            lips: None,
            eyes: vec![EyeTint { uv: [0.0, 0.0, 0.1, 0.1], color: [0, 93, 188] }],
        };
        assert_eq!(key.id(), key.clone().id());
        assert!(prewarm_face(&key).unwrap(), "first call composes");
        assert!(!prewarm_face(&key).unwrap(), "second call finds it");
        let image = take_face(&key).unwrap();
        assert_eq!(image.texture_descriptor.mip_level_count, 10);
        assert!(prewarm_face(&key).unwrap(), "taken: composed again");
        discard_face(&key);
        assert!(prewarm_face(&key).unwrap(), "discarded: composed again");
        discard_face(&key);
    }

    #[test]
    fn decal_blends_with_weight_and_offset() {
        // an untinted decal (tattoo, makeup) blends its own colour
        let mut dst = vec![0u8; 8 * 8 * 4];
        let src = vec![255u8; 2 * 2 * 4];
        let layer = FaceLayer { path: String::new(), x: 3.0, y: 2.0, scale: 1.0, rotate: 0.0, weight: 0.5, tint: None };
        draw_layer(&mut dst, 8, 8, &src, 2, 2, &layer);
        let at = |x: usize, y: usize| dst[(y * 8 + x) * 4..][..3].to_vec();
        assert_eq!(at(3, 2), vec![128, 128, 128]);
        assert_eq!(at(4, 3), vec![128, 128, 128]);
        assert_eq!(at(2, 2), vec![0, 0, 0]);
        assert_eq!(at(5, 2), vec![0, 0, 0]);
    }

    #[test]
    fn tinted_decals_multiply_the_skin() {
        // eyebrow / deco: a 0.5 grey decal, the tint modulates the skin (`skin * tint * 2 * grey`)
        let mut dst = vec![200u8; 4 * 4 * 4];
        let mut src = vec![128u8; 2 * 2 * 4];
        src.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
        let layer = FaceLayer { path: String::new(), x: 1.0, y: 1.0, scale: 1.0, rotate: 0.0, weight: 0.5, tint: Some([200, 100, 0]) };
        draw_layer(&mut dst, 4, 4, &src, 2, 2, &layer);
        let p = &dst[(4 + 1) * 4..][..3];
        // 200 * (1 + (tint / 255 * 128 / 127.5 - 1) * 0.5)
        assert!((p[0] as i32 - 179).abs() <= 1 && (p[1] as i32 - 139).abs() <= 1 && (p[2] as i32 - 100).abs() <= 1, "{p:?}");
        // outside the decal nothing changes
        assert_eq!(&dst[..3], &[200, 200, 200]);
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
        let img = compose_face("game/objects/characters/nuian/male/face/face01/nu_m_face01_df.dds", &[layer], None, None).unwrap();
        assert_eq!(img.texture_descriptor.mip_level_count, 10);
        assert_eq!(img.data.as_ref().unwrap().len(), (0..10).map(|m| ((512usize >> m).max(1)).pow(2) * 4).sum::<usize>());
    }
}
