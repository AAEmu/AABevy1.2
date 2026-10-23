//! CryEngine material -> Bevy `StandardMaterial` (the shader mapping documented in `docs/aa-cry.md`).
//!
//! The shader sources of the client (`shaders/hwscripts/cryfx/*.cfx`, in the pak) define what the
//! `.mtl` numbers mean; the facts this mapping is built on:
//!
//! * `Diffuse` (`MatDifColor`) multiplies `texture * (ambient + sum(N.L * light))` in HDR light
//!   units (costumes: 0.216 = a lighting scale, not a dark albedo). The base colour stays
//!   `Diffuse * texture`; [`crate::stage`] provides lights in the same units.
//! * `Emissive` is not an additive glow: the shaders add it to the *ambient accumulation*,
//!   which is then multiplied by `texture * Diffuse`. Here: `emissive = Emissive * Diffuse` with
//!   the diffuse texture as emissive texture.
//! * `Specular` is the final specular colour (the editor stores `colour * SpecularLevel`; e.g.
//!   the face's `3.0, 2.82, 1.94` is `17, 16, 11 / 255 * 45`): it is *not* multiplied by
//!   `SpecularLevel` again. `Shininess` is the Phong exponent of the sun highlight. Environment
//!   reflections come from the cube map scaled by `ReflectAmount` and a Fresnel term
//!   (`FresnelBias`, `FresnelScale`) and the specular map (the "gloss map"), which also masks the
//!   sun highlight; `SpecularModifierInHDR` / `DiffuseModifierInHDR` scale the two terms.
//! * `HumanSkin`: the lit colour of skin is `BSkinColor^1.5 * 1.5` (bright) / `MSkinColor`
//!   (mid), times `Diffuse * texture`; the ambient term gets ~0.3 * a skin tinted fill.
//! * `Hair`: colour = `texture * HairColor * 2 * Diffuse` (Diffuse = 0.067), plus an anisotropic
//!   Kajiya-Kay highlight (`PrimarySpec*`, `SecondarySpec*`).
//! * `Eye`: texture times `lerp(1, IrisColor, specularMap.r)`.

use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::Face;

use crate::material::{Material as CryMaterial, TextureMap};
use crate::scene::Shading;

pub mod backlight;
mod eye;
pub mod hair;
pub mod illum;
pub mod skin;
mod ucc;

/// The conversion used by the loader: [`to_standard_material`] plus whatever needs pixel data.
pub(crate) fn convert_material(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, ucc_uv_square: bool) -> Option<StandardMaterial> {
    let mut out = to_standard_material(m, |path, srgb| prep.texture(path, srgb))?;
    // Remove a redundant test only when every authored mip proves full opacity. The smallest
    // mip's mean can be opaque while the original fur/lace texels still contain cutouts.
    if let AlphaMode::Mask(threshold) = out.alpha_mode
        && threshold.is_finite()
        && out.base_color.to_linear().alpha >= threshold
        && !m.shader.to_ascii_lowercase().starts_with("veg")
        && !m.shader.eq_ignore_ascii_case("grass")
    {
        let opaque = m
            .texture(&TextureMap::Diffuse)
            .map(|t| t.pak_path())
            .and_then(|p| prep.loader.alpha_is_opaque(&p))
            == Some(true);
        if opaque {
            out.alpha_mode = AlphaMode::Opaque;
        }
    }
    bake_hair_colour(prep, m, &mut out);
    eye::bake(prep, m, &mut out);
    if ucc_uv_square {
        ucc::bake(prep, m, &mut out);
    }
    bake_terrain_blend(prep, m, &mut out);
    if prep.shading == Shading::Full {
        // characters get the `Char BackLight` term when `backlight::CharLightPlugin` is on
        out.lightmap_exposure = if shader_is_skin(m) {
            backlight::skin_tag(luma(m.diffuse))
        } else if matches!(m.shader.to_ascii_lowercase().as_str(), "hair" | "eye") {
            backlight::NEUTRAL_TAG
        } else {
            backlight::TAG
        };
        // Native Illum composition replaces the metallic/roughness approximation.
        if !illum::prepare(prep, m, &mut out) {
            bake_specular(prep, m, &mut out);
        }
        // hair gets the strand shading of `hair::HairMaterial` (tagged here, swapped by `hair::HairPlugin`)
        hair::prepare(prep, m, &mut out);
        // skin gets the HumanSkin shading of `skin::SkinMaterial` (tagged here, swapped by `skin::SkinPlugin`)
        skin::prepare(prep, m, &mut out);
    } else {
        // World brushes use Basic shading but still need the native post-lighting vertex colour term.
        illum::prepare(prep, m, &mut out);
    }
    Some(out)
}

fn shader_is_skin(m: &CryMaterial) -> bool {
    m.shader.eq_ignore_ascii_case("humanskin")
}

/// `GenMask` bit of the `Vegetation` shader's terrain colour blend (`%BLEND_WITH_TERRAIN_COLOR`): set
/// on 276 of the 277 vegetation materials that have a `blendWithTerrainAmount` parameter (default
/// 0.5), and on no other.
const GEN_TERRAIN_BLEND: u64 = 0x2000;

/// The hue (linear, only its direction counts: the blend keeps the texel's brightness) that blended
/// vegetation turns towards: the terrain colour map under it. In the captures the ground is a
/// yellow-green with no blue (the cover texture carries the evening light); the engine reads it per
/// pixel from the cell's cover, here one field hue stands for it.
const TERRAIN_BLEND_HUE: Vec3 = Vec3::new(0.2, 0.2, 0.0);

/// Vegetation with the terrain blend (`blendWithTerrainAmount`, grass, flowers, bushes) takes on the
/// hue of the ground it grows on: `lerp(diffuse, terrain, amount)`. Without it the orange marigolds
/// of the login fields stay orange where the captures have them yellow (their G / R ratio is 0.73,
/// the map's 0.62). The blend keeps the brightness of every texel (the grass is dimmed per field to
/// match the captures; blending towards a bright ground lit it up): the colour map is baked again with
/// the hue moved `amount` of the way, in linear light, alpha and mips kept.
fn bake_terrain_blend(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("vegetation") || m.gen_mask & GEN_TERRAIN_BLEND == 0 || out.base_color_texture.is_none() {
        return;
    }
    let amount = param_f32(m, "blendWithTerrainAmount").unwrap_or(0.0).clamp(0.0, 1.0);
    let Some(df) = m.texture(&TextureMap::Diffuse).map(|t| t.pak_path()) else { return };
    if amount <= 0.0 {
        return;
    }
    let to = TERRAIN_BLEND_HUE;
    let key = format!("terrainblend:{df}:{amount:.3}:{:.3},{:.3},{:.3}", to.x, to.y, to.z);
    let image = prep.baked_image(&key, |loader| {
        let (w, h, mut mips) = loader.decode_mips_rgba8(&df)?;
        blend_towards(&mut mips, amount, to);
        Some(crate::texture::rgba8_image(w, h, &mips, true))
    });
    if let Some(image) = image {
        out.base_color_texture = Some(image);
    }
}

/// Moves the colour of every texel of the RGBA8 `mips` (sRGB) `amount` of the way to the hue `to`
/// (linear) at the same brightness, computed in linear light; the alpha stays.
fn blend_towards(mips: &mut [Vec<u8>], amount: f32, to: Vec3) {
    let decode: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let hue = to / luma(to).max(1e-4);
    for level in mips {
        for px in level.as_chunks_mut::<4>().0.iter_mut() {
            let lin = Vec3::new(decode[px[0] as usize], decode[px[1] as usize], decode[px[2] as usize]);
            let out = lin.lerp(hue * luma(lin), amount);
            for c in 0..3 {
                px[c] = (linear_to_srgb(out[c]) * 255.0 + 0.5) as u8;
            }
        }
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Hair textures have white (or any) colour in the fully transparent texels: with the mask at the
/// strand ends, bilinear and mip filtering pull it into the visible edge pixels (a white halo,
/// worst on dark hair). The colour map is baked again with the colour of the visible hair spread
/// into the transparent texels and the mips averaged weighted by alpha.
fn bake_hair_colour(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("hair") || out.base_color_texture.is_none() {
        return;
    }
    let Some(df) = m.texture(&TextureMap::Diffuse).map(|t| t.pak_path()) else { return };
    if prep.loader.mean_alpha(&df).is_none_or(|a| a >= 0.98) {
        return;
    }
    let key = format!("haircolor:{df}");
    let image = prep.baked_image(&key, |loader| {
        let (w, h, mips) = loader.decode_mips_rgba8(&df)?;
        let mut base = mips.into_iter().next()?;
        spread_colour(&mut base, w as usize, h as usize, HAIR_VISIBLE_ALPHA);
        Some(crate::texture::rgba8_image(w, h, &alpha_weighted_mips(base, w as usize, h as usize), true))
    });
    if let Some(image) = image {
        out.base_color_texture = Some(image);
    }
}

/// Alpha (0..255) below which a hair texel is cut by the mask (`Mask(0.5)` on the doubled alpha).
const HAIR_VISIBLE_ALPHA: u8 = 64;

/// Gives every texel with alpha below `visible` the average colour of the visible texels around it
/// (up to 8 texels away, growing one ring per pass); the alpha stays.
fn spread_colour(px: &mut [u8], w: usize, h: usize, visible: u8) {
    let mut known: Vec<bool> = px.chunks_exact(4).map(|p| p[3] >= visible).collect();
    for _ in 0..8 {
        let mut next: Vec<(usize, [u8; 3])> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if known[y * w + x] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let nx = (x as i32 + dx).rem_euclid(w as i32) as usize;
                    let ny = (y as i32 + dy).rem_euclid(h as i32) as usize;
                    if known[ny * w + nx] {
                        let p = &px[(ny * w + nx) * 4..];
                        sum[0] += p[0] as u32;
                        sum[1] += p[1] as u32;
                        sum[2] += p[2] as u32;
                        n += 1;
                    }
                }
                if n > 0 {
                    next.push((y * w + x, [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]));
                }
            }
        }
        if next.is_empty() {
            break;
        }
        for (i, c) in next {
            px[i * 4..i * 4 + 3].copy_from_slice(&c);
            known[i] = true;
        }
    }
}

/// A full mip chain of RGBA8 `base`: colour averaged weighted by alpha (transparent texels do not
/// dilute the visible ones), alpha averaged plainly.
fn alpha_weighted_mips(base: Vec<u8>, w: usize, h: usize) -> Vec<Vec<u8>> {
    let mut chain = vec![base];
    let (mut mw, mut mh) = (w, h);
    while mw > 1 || mh > 1 {
        let (nw, nh) = ((mw / 2).max(1), (mh / 2).max(1));
        let prev = chain.last().expect("a level");
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let (mut rgb, mut wsum, mut asum, mut plain) = ([0f32; 3], 0f32, 0f32, [0f32; 3]);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((x * 2 + dx).min(mw - 1), (y * 2 + dy).min(mh - 1));
                    let p = &prev[(sy * mw + sx) * 4..][..4];
                    let a = p[3] as f32;
                    for c in 0..3 {
                        rgb[c] += p[c] as f32 * a;
                        plain[c] += p[c] as f32;
                    }
                    wsum += a;
                    asum += a;
                }
                let o = &mut next[(y * nw + x) * 4..][..4];
                for c in 0..3 {
                    o[c] = (if wsum > 0.0 { rgb[c] / wsum } else { plain[c] / 4.0 }).round() as u8;
                }
                o[3] = (asum / 4.0).round() as u8;
            }
        }
        chain.push(next);
        (mw, mh) = (nw, nh);
    }
    chain
}

/// `PrimarySpecColor` of a hair material: its brightness, and its hue (damped) as a tint.
fn hair_spec_colour(m: &CryMaterial) -> (f32, Vec3) {
    let c = param_rgb(m, "PrimarySpecColor").unwrap_or(Vec3::splat(0.2));
    (luma(c).clamp(0.02, 1.0), Vec3::ONE.lerp(c / c.max_element().max(1e-3), 0.5))
}

/// `Illum` materials: the specular (gloss) map becomes a metallic / roughness texture. In the
/// shader the map masks both the sun highlight and the cube-map reflection (leather has a black
/// map, trims and armour a bright one, often tinted: gold); here bright texels are metal that
/// reflects the environment in the colour of the diffuse texture, the rest is a matte dielectric.
/// Without this every pixel of a costume gets the same sheen and dark leather turns grey.
fn bake_specular(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("illum") || out.alpha_mode == AlphaMode::Blend {
        return;
    }
    let Some(spec) = m.texture(&TextureMap::Specular).map(|t| t.pak_path()) else { return };
    // overall strength: the file's own specular colour and modifier scale the map, but are not a
    // measure of how metallic the object is (bows: 7.6 x 1.7 on a map that is dark for the wood),
    // so only a modest boost is taken from them
    let gain = (luma(m.specular) * param_f32(m, "SpecularModifierInHDR").unwrap_or(1.0)).clamp(0.0, METAL_GAIN_MAX);
    // `ReflectAmount` scales the environment reflection: 15 on costumes, 0.25 on props and weapons
    let reflect = 0.35 + 0.65 * (param_f32(m, "ReflectAmount").unwrap_or(1.0) / 2.0).clamp(0.0, 1.0);
    let matte = shininess_to_roughness(m.shininess);
    let shiny = (matte * 0.6).clamp(0.3, 0.55);
    // Costumes (`ReflectAmount` 7 - 15, props 0.25) are far glossier in the captures than their
    // maps alone give: the shader multiplies the dim specular map (purple on leather, 0.05 luma) by
    // the reflect amount. A material counts as costume by `ReflectAmount` (0 props .. 1 costumes),
    // and the costume side of every constant below is blended in by that factor.
    let costume = (param_f32(m, "ReflectAmount").unwrap_or(1.0) / COSTUME_REFLECT_AMOUNT).clamp(0.0, 1.0);
    let mix = |prop: f32, costume_value: f32| prop + (costume_value - prop) * costume;
    let boost = mix(1.0, COSTUME_GLOSS_BOOST);
    let metal_boost = mix(1.0, COSTUME_METAL_BOOST);
    let sharp = mix(SHARP_ROUGHNESS, COSTUME_SHARP_ROUGHNESS);
    let (ramp_lo, ramp_hi) = (mix(0.1, 0.05), mix(0.5, 0.3));
    let tint_amount = mix(SPEC_TINT_AMOUNT, COSTUME_TINT_AMOUNT);
    // The costume lighting: the gloss gives the highlights, the diffuse base is darker (the captures'
    // costumes have deeper midtones than a Lambert surface under the stage rig).
    let diffuse_k = mix(1.0, COSTUME_DIFFUSE);
    if diffuse_k != 1.0 {
        let c = out.base_color.to_linear();
        out.base_color = Color::linear_rgba(c.red * diffuse_k, c.green * diffuse_k, c.blue * diffuse_k, c.alpha);
    }
    let key = format!("gloss:{spec}:{gain:.3}:{matte:.3}:{reflect:.3}:{boost:.2}:{metal_boost:.2}:{sharp:.2}:{ramp_lo:.2}:{ramp_hi:.2}:{tint_amount:.2}");
    // per texel: g = how glossy the map is, metal = how much of it reflects like a metal
    let texel = move |p: &[u8]| {
        let s = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0;
        let g = (luma(s) * gain * boost).clamp(0.0, 1.2);
        // what turns into a metal is the unboosted-ish map: leather and cloth stay dielectrics (a boosted dim map
        // would make them mirrors of the sky in the world's light)
        let metal = smoothstep(0.2, 0.75, (luma(s) * gain * metal_boost).clamp(0.0, 1.2)) * reflect;
        // glossier texels have sharper highlights (leather streaks, polished trims)
        let rough_dielectric = matte + (matte.min(sharp) - matte) * smoothstep(ramp_lo, ramp_hi, g);
        let rough = rough_dielectric + (shiny - rough_dielectric) * metal;
        (s, g, metal, rough)
    };
    let image = prep.baked_image(&key, |loader| {
        let (w, h, mips) = loader.decode_mips_rgba8(&spec)?;
        let mips: Vec<Vec<u8>> = mips
            .iter()
            .map(|mip| {
                let mut data = Vec::with_capacity(mip.len());
                for p in mip.chunks_exact(4) {
                    let (_, _, metal, rough) = texel(p);
                    data.extend_from_slice(&[0, (rough * 255.0).round() as u8, (metal * 255.0).round() as u8, 255]);
                }
                data
            })
            .collect();
        Some(crate::texture::rgba8_image(w, h, &mips, false))
    });
    let Some(image) = image else { return };
    // Metal reflects the environment in the colour of its albedo (F0), which is dark in the diffuse
    // maps (the engine multiplies the reflection by `Specular` x `ReflectAmount` x Fresnel, far above
    // 1 on armour): the albedo of the metal texels is raised so that plate armour shines.
    if let Some(df) = m.texture(&TextureMap::Diffuse).map(|t| t.pak_path()) {
        let albedo_key = format!("metalalbedo:{df}:{spec}:{key}:{METAL_ALBEDO_GAIN}");
        let baked = prep.baked_image(&albedo_key, |loader| {
            let (w, h, dmips) = loader.decode_mips_rgba8(&df)?;
            let (sw, sh, smips) = loader.decode_mips_rgba8(&spec)?;
            let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
            let mips: Vec<Vec<u8>> = dmips
                .iter()
                .enumerate()
                .map(|(level, d)| {
                    // the maps may differ in size and mips: nearest texel of the specular map
                    let sl = level.min(smips.len() - 1);
                    let sp = &smips[sl];
                    let (dw, dh) = ((w as usize >> level).max(1), (h as usize >> level).max(1));
                    let (sw, sh) = ((sw as usize >> sl).max(1), (sh as usize >> sl).max(1));
                    let mut data = d.clone();
                    for (i, px) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        let (x, y) = (i % dw, i / dw);
                        let o = ((y * sh / dh) * sw + x * sw / dw) * 4;
                        let (_, _, metal, _) = texel(&sp[o..o + 4]);
                        if metal > 0.0 {
                            let k = 1.0 + (METAL_ALBEDO_GAIN - 1.0) * metal;
                            for c in 0..3 {
                                px[c] = (linear_to_srgb(lut[px[c] as usize] * k) * 255.0 + 0.5) as u8;
                            }
                        }
                    }
                    data
                })
                .collect();
            Some(crate::texture::rgba8_image(w, h, &mips, true))
        });
        if let Some(b) = baked {
            out.base_color_texture = Some(b);
        }
    }
    out.metallic_roughness_texture = Some(image);
    out.metallic = 1.0;
    out.perceptual_roughness = 1.0;
    // The dielectric part (everything that is not metal): the map's colour is the specular tint
    // (rgb) and its brightness the strength (alpha): F0 = 0.16 * strength * tint^2, a coloured,
    // masked highlight as in the shader (`Specular` map x `Specular` colour x Phong).
    out.reflectance = 2.0;
    let tint_key = format!("spectint:{key}");
    let tint = prep.baked_image(&tint_key, |loader| {
        let (w, h, mips) = loader.decode_mips_rgba8(&spec)?;
        let mips: Vec<Vec<u8>> = mips
            .iter()
            .map(|mip| {
                let mut data = Vec::with_capacity(mip.len());
                for p in mip.chunks_exact(4) {
                    let (s, g, _, _) = texel(p);
                    // only warm maps (gold, bronze, leather) tint: the greenish maps of plate armour are
                    // silver in the captures
                    let norm = s / s.max_element().max(1e-3);
                    let warm = smoothstep(0.0, 0.2, norm.x - norm.y);
                    let hue = Vec3::ONE.lerp(norm, tint_amount * warm);
                    let strength = (SPEC_STRENGTH * g).sqrt().clamp(0.0, 1.0);
                    data.extend_from_slice(&[
                        (hue.x * 255.0).round() as u8,
                        (hue.y * 255.0).round() as u8,
                        (hue.z * 255.0).round() as u8,
                        (strength * 255.0).round() as u8,
                    ]);
                }
                data
            })
            .collect();
        Some(crate::texture::rgba8_image(w, h, &mips, false))
    });
    if let Some(tint) = tint {
        // one image: alpha is the strength (`specular_texture`), rgb the tint (`specular_tint_texture`)
        out.specular_texture = Some(tint.clone());
        out.specular_tint_texture = Some(tint);
    } else {
        out.reflectance = 0.25;
    }
}

/// Upper limit of the specular gain (`Specular` x `SpecularModifierInHDR`) that scales the map into
/// "how metallic": saint armour has 7.4 x 4.9 on a map that never exceeds 0.19, the bows 7.6 x 1.7 on
/// a dark one (they stay wood, the sword blades gain a little).
const METAL_GAIN_MAX: f32 = 5.0;
/// Factor on the albedo of fully metallic texels (see [`bake_specular`]).
const METAL_ALBEDO_GAIN: f32 = 8.0;
/// Roughness of the sharpest dielectric highlights (glossy map texels).
const SHARP_ROUGHNESS: f32 = 0.42;
/// `ReflectAmount` from which a material counts fully as a costume (see [`bake_specular`]).
const COSTUME_REFLECT_AMOUNT: f32 = 8.0;
/// Factor on the glossiness of costume specular maps.
const COSTUME_GLOSS_BOOST: f32 = 8.0;
/// Factor on the glossiness that decides which texels are metal (plate armour, trims), less than the boost on the
/// highlights: leather and cloth are lacquered dielectrics, not metals.
const COSTUME_METAL_BOOST: f32 = 2.0;
/// Roughness of the sharpest costume highlights (leather streaks).
const COSTUME_SHARP_ROUGHNESS: f32 = 0.3;
/// Factor on the diffuse colour of costume materials.
const COSTUME_DIFFUSE: f32 = 0.85;
/// Warm tint of costume highlights: the trims of the captures shine white-silver.
const COSTUME_TINT_AMOUNT: f32 = 0.1;
/// Dielectric specular strength per unit of map glossiness: `F0 = 0.16 * strength * tint^2`
/// (strength 1 at glossiness 1 = the full 16% a dielectric can show).
const SPEC_STRENGTH: f32 = 1.0;
/// How much of the specular map's colour tints the dielectric highlight of warm maps (gold trims,
/// bronze, leather; 0 = white). The greenish maps of plate armour do not tint: in the captures
/// the plates are silver.
const SPEC_TINT_AMOUNT: f32 = 0.3;

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn luma(c: Vec3) -> f32 {
    c.x * 0.2126 + c.y * 0.7152 + c.z * 0.0722
}

/// `"r,g,b"` -> Vec3.
pub fn parse_rgb(s: &str) -> Option<Vec3> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>());
    Some(Vec3::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}

fn param_rgb(m: &CryMaterial, name: &str) -> Option<Vec3> {
    m.public_param(name).and_then(parse_rgb)
}

fn param_f32(m: &CryMaterial, name: &str) -> Option<f32> {
    m.public_param(name).and_then(|v| v.trim().parse().ok())
}

/// The lit colour of a skin (the `HumanSkin` shader's `pow(bright, 1.5) * 1.5`): multiplied into
/// the albedo it gives the hue of sunlit skin. `bright` is `BSkinColor` or a `skin_colors`
/// row's bright colour (0..1), **as r, g, b** (the files store them b, g, r).
pub fn skin_lit_color(bright: Vec3) -> Vec3 {
    bright.clamp(Vec3::ZERO, Vec3::ONE).powf(1.5) * 1.5
}

/// What the skin shader adds on top of the bright colour that a single-colour conversion cannot
/// show: the pink subsurface colour (`SubSurface` map, `SSSMapSat`) and the fill light
/// (`0.3 * skin colour` into the ambient). Calibrated against the character-create captures:
/// every light skin shows as pale pink, whatever its `BSkinColor` (pale elf: tan-grey, nuian:
/// pink). The hue of the lit colour is pulled `SKIN_PINK_WEIGHT` of the way to [`SKIN_PINK`] and
/// the brightness raised by `SKIN_GAIN`.
pub const SKIN_PINK: Vec3 = Vec3::new(1.28, 0.92, 0.90);
pub const SKIN_PINK_WEIGHT: f32 = 0.6;
pub const SKIN_GAIN: f32 = 1.8;

/// Exponent on the brightness of the `HairColor` picker values (see [`hair_colour`]).
const HAIR_GAMMA: f32 = 1.5;

/// A hair colour parameter (`HairColor`, `PrimarySpecColor`, `SecondarySpecColor`) as linear colour: the hue as
/// written, the brightness (luma) raised to [`HAIR_GAMMA`]. Fitted to the `step2_*` captures: dark hair (luma 0.04)
/// is near black there with a faint warm sheen, which `^1.5` gives, while blond keeps the hue of the plain values
/// (`^1.5` per channel turned it a saturated gold where the captures are ash gold).
pub(crate) fn hair_colour(c: Vec3) -> Vec3 {
    let c = c.max(Vec3::ZERO);
    c * luma(c).max(1e-4).powf(HAIR_GAMMA - 1.0)
}

/// Base colour of a skin material from its `Diffuse` and bright skin colour.
pub fn skin_base_color(diffuse: Vec3, bright: Vec3) -> Color {
    let lit = skin_lit_color(bright);
    let y = luma(lit).max(1e-4);
    let chroma = (lit / y).lerp(SKIN_PINK, SKIN_PINK_WEIGHT);
    // the tint of `Diffuse` (nuian skins: yellowish) is not seen in the captures, only its level
    let c = Vec3::splat(luma(diffuse)) * chroma * (y * SKIN_GAIN);
    Color::linear_rgb(c.x, c.y, c.z)
}

/// CryEngine material -> `StandardMaterial`; `None` for materials that are not drawn.
/// `texture(path, srgb)` resolves texture files. The mapping is documented in `docs/aa-cry.md`.
pub fn to_standard_material(
    m: &CryMaterial,
    mut texture: impl FnMut(&str, bool) -> Option<Handle<Image>>,
) -> Option<StandardMaterial> {
    let shader = m.shader.to_ascii_lowercase();
    if m.no_draw() || shader == "cut" {
        return None;
    }
    let diffuse_tex = m.texture(&TextureMap::Diffuse).map(|t| (t.file.clone(), t.tex_mod.clone()));
    let normal_tex = m
        .texture(&TextureMap::Normal)
        .or_else(|| m.texture(&TextureMap::Bump).filter(|t| t.file.to_ascii_lowercase().contains("_ddn")))
        .map(|t| t.file.clone());
    let diffuse_modifier = param_f32(m, "DiffuseModifierInHDR").unwrap_or(1.0);
    let specular_modifier = param_f32(m, "SpecularModifierInHDR").unwrap_or(1.0);
    let base = m.diffuse * diffuse_modifier;
    // `Specular` already includes `SpecularLevel` (see the module docs); 1.0 (the editor's default
    // grey 0.5 x level 2) is a plain dielectric (reflectance 0.5, F0 4%).
    let spec_gain = luma(m.specular) * specular_modifier;
    let mut out = StandardMaterial {
        base_color: Color::linear_rgb(base.x, base.y, base.z),
        base_color_texture: diffuse_tex.as_ref().and_then(|(f, _)| texture(f, true)),
        normal_map_texture: normal_tex.as_deref().and_then(|f| texture(f, false)),
        perceptual_roughness: shininess_to_roughness(m.shininess),
        metallic: 0.0,
        // a big multiplier (bows 13) scales a map that is dark for wood: less sheen; `ReflectAmount`
        // (0.25 on props, 10-15 on costumes) scales what the surface reflects of the environment: at
        // a grazing view a smooth prop (the Hariharan detect machine, Specular 3.4) mirrored the
        // whole bright sky and turned pale grey where the captures have dark rust
        reflectance: (if spec_gain > 3.0 { 0.35 } else { (spec_gain * 0.5).clamp(0.0, 0.5) }) * reflect_scale(m),
        double_sided: m.two_sided(),
        cull_mode: if m.two_sided() { None } else { Some(Face::Back) },
        ..default()
    };
    // Emissive = extra ambient light, so it carries the albedo (see the module docs).
    if m.emissive != Vec3::ZERO {
        let e = m.emissive * base;
        out.emissive = LinearRgba::rgb(e.x, e.y, e.z);
        out.emissive_texture = out.base_color_texture.clone();
    }
    if let Some((_, Some(tm))) = &diffuse_tex {
        out.uv_transform = Affine2::from_scale_angle_translation(
            Vec2::new(tm.tile_u, tm.tile_v),
            tm.rotate_w.to_radians(),
            Vec2::new(tm.offset_u, tm.offset_v),
        );
    }
    let two_sided = |o: &mut StandardMaterial| {
        o.double_sided = true;
        o.cull_mode = None;
    };
    match shader.as_str() {
        "vegetation" | "vegetationleaves" | "grass" => {
            two_sided(&mut out);
            out.alpha_mode = AlphaMode::Mask(if m.alpha_test > 0.0 { m.alpha_test } else { 0.5 });
        }
        "hair" => {
            two_sided(&mut out);
            // The shader draws hair twice (an opaque alpha-tested pass and a blended pass with
            // alpha * 2). Alpha-to-coverage would give soft strand ends without sorting, but the
            // depth / normal prepasses (always on in the world, SSAO) treat it as opaque: the
            // transparent fringe of the hair mesh then hides the head behind it and renders black.
            // A mask on the doubled alpha keeps the strand ends and works with the prepasses.
            out.alpha_mode = AlphaMode::Mask(0.5);
            // Hair has no glow (Bevy multiplies emissive by the sharpened alpha of alpha-to-coverage
            // materials, thousands in flat areas): the shader's `Emissive` only brightens the ambient.
            out.emissive = LinearRgba::BLACK;
            out.emissive_texture = None;
            // Hair textures are greyscale; the colour is the (customisable) HairColor param.
            // `texture * HairColor * 2 * Diffuse`: Diffuse is 0.067 for every hair.
            let hair = param_rgb(m, "HairColor").unwrap_or(Vec3::splat(0.4));
            // the colour picker values: hue as written, brightness ^1.5 (`hair_colour`)
            let c = hair_colour(hair) * 2.0 * m.diffuse;
            out.base_color = Color::linear_rgba(c.x, c.y, c.z, 2.0);
            // Hair is lit by the Kajiya-Kay strand highlights of `hair::HairMaterial` (`Shading::Full`).
            // Bevy's anisotropic GGX is no substitute: it breaks with the normal prepass (SSAO,
            // `Msaa::Off`; the tangent frame is never set up, the GGX term turns the hair white), and an
            // isotropic GGX lobe on hair reads as plastic. This `StandardMaterial` alone (`Shading::Basic`,
            // no `HairPlugin`) keeps only a faint, broad sheen of the `PrimarySpecColor` strength.
            out.perceptual_roughness = 0.65;
            let (strength, tint) = hair_spec_colour(m);
            out.reflectance = (0.3 * strength.powf(0.5)).clamp(0.05, 0.3);
            out.specular_tint = Color::linear_rgb(tint.x, tint.y, tint.z);
        }
        "humanskin" => {
            // lit skin hue from the bright skin colour, Diffuse = the lighting scale
            // BSkinColor / MSkinColor are stored blue first (the elf's `0.52, 0.61, 0.71` is a warm
            // skin tone written as b, g, r; read as r, g, b it would be a blue one)
            let bright = param_rgb(m, "BSkinColor").map_or(Vec3::splat(0.9), |c| Vec3::new(c.z, c.y, c.x));
            out.base_color = skin_base_color(base, bright);
            // skin F0 ~0.028; the files' strong specular (up to 3.0 x a dark grey map) is for
            // CryEngine's skin shader
            out.reflectance = 0.4;
            out.perceptual_roughness = out.perceptual_roughness.max(0.5);
            // translucency of the lit side's edge (the shader's `Translucence` term)
            out.diffuse_transmission = 0.1;
        }
        "eye" => {
            // `eye.cfx` has no GGX and no Fresnel: a small Phong glint inside the iris (`gloss.r`), none on the
            // sclera, and an environment term of about 0.04 (`ReflectionAmount` 0.05); a glossy eyeball
            // (roughness 0.12, reflectance 0.2) put a wet sheen and the sky over the whole eye
            out.perceptual_roughness = 0.45;
            out.reflectance = 0.08;
        }
        "glass" | "liquid" | "ice" => {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color.set_alpha(m.opacity.min(0.6));
            out.perceptual_roughness = 0.1;
        }
        "metal" => {
            out.metallic = 0.6;
        }
        s if s.starts_with("water") || s.starts_with("ocean") => {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color = Color::linear_rgba(0.05, 0.12, 0.15, 0.7);
            out.perceptual_roughness = 0.05;
        }
        s if s.contains("particle") || s.starts_with("fx_") || s.starts_with("light") || s.contains("beam")
            || s.starts_with("sky") || s.contains("cloud") || s == "specialeffects" || s == "hologram" =>
        {
            out.unlit = true;
            out.alpha_mode = AlphaMode::Add;
            two_sided(&mut out);
        }
        _ => {
            // Illum, Cloth, ...: `Specular` above 1 (bows: 7.6, armour: 6.5 x 4.9) is a multiplier
            // of the specular map, which is dark for wood and leather and bright for metal. A
            // uniform factor cannot tell them apart, and a uniform metal turns wood silver: stay
            // a dielectric here; `Shading::Full` bakes the map into per-pixel metalness.
        }
    }
    if matches!(out.alpha_mode, AlphaMode::Opaque) {
        if m.flags & crate::material::flags::ADDITIVE != 0 && additive_applies(m) {
            out.alpha_mode = AlphaMode::Add;
        } else if m.opacity < 0.999 {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color.set_alpha(m.opacity);
        } else if m.alpha_test > 0.0 {
            out.alpha_mode = AlphaMode::Mask(m.alpha_test);
        }
    }
    Some(out)
}

/// Whether a material's `ADDITIVE` flag blends it additively. Hundreds of ordinary opaque `Illum`
/// materials carry the flag (costume skins, the Hariharan rail sets and detect machines, Nuian
/// tents, ...) and the real client draws them opaque: additive blending turned the Hariharan field's
/// carcass into a white ghost. The flag counts for materials that are not fully opaque and for
/// effect textures (`p_effects`, `sprites`, or no texture at all: pointers and the like).
fn additive_applies(m: &CryMaterial) -> bool {
    if m.opacity < 0.999 {
        return true;
    }
    match m.texture(&TextureMap::Diffuse) {
        None => true,
        Some(t) => {
            let f = t.file.to_ascii_lowercase();
            f.contains("p_effects") || f.contains("sprites")
        }
    }
}

/// Factor on the environment reflection of a material from its `ReflectAmount` (absent or 2 and more: 1).
fn reflect_scale(m: &CryMaterial) -> f32 {
    param_f32(m, "ReflectAmount").map_or(1.0, |r| 0.35 + 0.65 * (r / 2.0).clamp(0.0, 1.0))
}

/// The old flat skin multiplier (kept for source compatibility; skin colour now comes from the
/// material, see [`skin_base_color`]).
pub const SKIN_ALBEDO: f32 = 0.22;

/// Blinn-Phong exponent -> perceptual roughness (`alpha = sqrt(2 / (n + 2))`, `r = sqrt(alpha)`).
pub fn shininess_to_roughness(n: f32) -> f32 {
    (2.0 / (n.max(0.0) + 2.0)).powf(0.25).clamp(0.05, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(xml: &str) -> StandardMaterial {
        let m = CryMaterial::parse(xml).expect("mtl");
        to_standard_material(&m, |_, _| None).expect("drawn")
    }

    const HAIR: &str = r#"<Material Diffuse="0.067,0.067,0.067" Emissive="0.353,0.353,0.353" Shader="Hair" Shininess="10" Specular="0.39,0.39,0.39" AlphaTest="0.3" Opacity="0.99" MtlFlags="526464">
 <PublicParams HairColor="1,0.5,0.25"/></Material>"#;

    #[test]
    fn hair_is_safe_in_the_depth_and_normal_prepasses() {
        let m = convert(HAIR);
        // alpha-to-coverage is opaque in the prepasses: the transparent fringe of the hair mesh
        // would hide the head behind it and render black
        assert!(matches!(m.alpha_mode, AlphaMode::Mask(_)), "{:?}", m.alpha_mode);
        // Bevy's anisotropic GGX turns white when the normal prepass is loaded (the tangent
        // frame is never set up in that path)
        assert_eq!(m.anisotropy_strength, 0.0);
        assert!(m.anisotropy_texture.is_none());
        assert_eq!(m.emissive, LinearRgba::BLACK);
        assert!(m.double_sided);
        let c = m.base_color.to_linear();
        assert!(c.red > c.green && c.green > c.blue, "hair colour from HairColor: {c:?}");
        assert!(c.red < 0.2, "Diffuse 0.067 keeps the hair dark: {c:?}");
    }

    #[test]
    fn hair_colour_map_has_no_white_in_the_cut_texels() {
        // 4x1: dark visible texels next to white transparent ones (the usual hair map)
        let mut px = vec![30, 20, 10, 255, 30, 20, 10, 255, 255, 255, 255, 0, 255, 255, 255, 0];
        spread_colour(&mut px, 4, 1, HAIR_VISIBLE_ALPHA);
        for t in px.chunks_exact(4) {
            assert_eq!(&t[..3], &[30, 20, 10], "{px:?}");
        }
        assert_eq!(px[11], 0, "the alpha stays");
        // mips weigh the colour by alpha: a half transparent white texel does not lighten the strand
        let chain = alpha_weighted_mips(vec![30, 20, 10, 255, 255, 255, 255, 0], 2, 1);
        assert_eq!(chain.len(), 2);
        assert_eq!(&chain[1][..3], &[30, 20, 10]);
        assert_eq!(chain[1][3], 128);
    }

    #[test]
    fn the_additive_flag_does_not_make_opaque_props_glow() {
        // ti_detectmachine_rust: an opaque rusty prop with MtlFlags 0x80086 (two sided + additive)
        let prop = convert(
            r#"<Material MtlFlags="524422" Shader="Illum" Diffuse="0.22,0.22,0.22" Specular="3.4,3.4,3.4" Opacity="1" AlphaTest="0.495">
 <Textures><Texture Map="Diffuse" File="game/objects/characters/monster/detectmachine_rust/detectmachine_rust_df.dds"/></Textures></Material>"#,
        );
        assert!(!matches!(prop.alpha_mode, AlphaMode::Add | AlphaMode::Blend), "{:?}", prop.alpha_mode);
        // an effect texture with the same flag is still additive, and so is a translucent one
        let glow = convert(
            r#"<Material MtlFlags="4" Shader="Illum" Opacity="1">
 <Textures><Texture Map="Diffuse" File="game/textures/p_effects/o_effect/glow_com_01a.dds"/></Textures></Material>"#,
        );
        assert!(matches!(glow.alpha_mode, AlphaMode::Add));
        let decal = convert(r#"<Material MtlFlags="4" Shader="Illum" Opacity="0.5"><Textures><Texture Map="Diffuse" File="a/b.dds"/></Textures></Material>"#);
        assert!(matches!(decal.alpha_mode, AlphaMode::Add));
    }

    #[test]
    fn props_with_a_low_reflect_amount_do_not_mirror_the_sky() {
        // ti_detectmachine_rust: Specular 3.4, ReflectAmount 0.25
        let prop = convert(
            r#"<Material Shader="Illum" Diffuse="0.22,0.22,0.22" Specular="3.4,3.4,3.4" Shininess="7">
 <PublicParams ReflectAmount="0.25"/></Material>"#,
        );
        // costumes (ReflectAmount 10) keep their sheen
        let costume = convert(
            r#"<Material Shader="Illum" Diffuse="0.22,0.22,0.22" Specular="3.4,3.4,3.4" Shininess="7">
 <PublicParams ReflectAmount="10"/></Material>"#,
        );
        assert!(prop.reflectance < 0.2, "{}", prop.reflectance);
        assert!(costume.reflectance >= 0.34, "{}", costume.reflectance);
    }

    #[test]
    fn strong_specular_is_not_a_metal_without_the_map() {
        // bows: Specular 7.6 x modifier 1.7 on a map that is dark for the wood
        let m = convert(
            r#"<Material Shader="Illum" Diffuse="0.22,0.22,0.22" Specular="7.6,7.7,8.0" Shininess="19.4" SpecularLevel="8.8">
 <PublicParams SpecularModifierInHDR="1.7" ReflectAmount="0.25"/></Material>"#,
        );
        assert_eq!(m.metallic, 0.0);
        assert!(m.reflectance <= 0.5);
    }

    #[test]
    fn skin_colour_is_read_blue_first_and_pale_skins_stay_pink() {
        let bright = Vec3::new(0.71, 0.61, 0.52); // BSkinColor "0.52,0.61,0.71" is b, g, r
        let c = skin_base_color(Vec3::splat(0.2), bright).to_linear();
        assert!(c.red > c.green && c.green >= c.blue * 0.95, "pink, not blue: {c:?}");
        let tan = skin_lit_color(bright);
        assert!(tan.x > tan.z);
    }
}
