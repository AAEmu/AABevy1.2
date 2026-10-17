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

/// The conversion used by the loader: [`to_standard_material`] plus whatever needs pixel data.
pub(crate) fn convert_material(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial) -> Option<StandardMaterial> {
    let mut out = to_standard_material(m, |path, srgb| prep.texture(path, srgb))?;
    // `AlphaTest` is set on nearly every costume `Illum` material, whose diffuse maps have no
    // alpha at all (BC1): testing them only costs a prepass and risks holes.
    if matches!(out.alpha_mode, AlphaMode::Mask(_)) && !m.shader.to_ascii_lowercase().starts_with("veg") && !m.shader.eq_ignore_ascii_case("grass") {
        let opaque = m
            .texture(&TextureMap::Diffuse)
            .map(|t| t.pak_path())
            .and_then(|p| prep.loader.mean_alpha(&p))
            .is_some_and(|a| a >= 0.98);
        if opaque {
            out.alpha_mode = AlphaMode::Opaque;
        }
    }
    if prep.shading == Shading::Full {
        bake_specular(prep, m, &mut out);
        bake_hair_specular(prep, m, &mut out);
    }
    Some(out)
}

/// Strength of the hair highlight on the brightest strands (`reflectance` of the material, scaled
/// by the baked strand map: `F0 = 0.16 * (reflectance * 0.5 * map)^2`).
const HAIR_SPEC_REFLECTANCE: f32 = 1.5;

/// `Hair` materials: the highlight follows the strand pattern of the hair map. The shader's
/// Kajiya-Kay term shines on the strands that face the light; a strand map (bright strands, dark
/// gaps) as specular strength makes the same streaks with Bevy's isotropic GGX.
fn bake_hair_specular(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("hair") {
        return;
    }
    let Some(spec) = m.texture(&TextureMap::Specular).or_else(|| m.texture(&TextureMap::Diffuse)).map(|t| t.pak_path()) else { return };
    let key = format!("hairspec:{spec}");
    let image = prep.baked_image(&key, |loader| {
        let (w, h, mips) = loader.decode_mips_rgba8(&spec)?;
        let mips: Vec<Vec<u8>> = mips
            .iter()
            .map(|mip| {
                let mut data = Vec::with_capacity(mip.len());
                for p in mip.chunks_exact(4) {
                    let l = luma(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0);
                    let strength = 0.15 + 0.85 * smoothstep(0.12, 0.7, l);
                    data.extend_from_slice(&[255, 255, 255, (strength * 255.0).round() as u8]);
                }
                data
            })
            .collect();
        Some(crate::texture::rgba8_image(w, h, &mips, false))
    });
    if let Some(image) = image {
        out.specular_texture = Some(image);
        out.reflectance = HAIR_SPEC_REFLECTANCE;
    }
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
    let gain = (luma(m.specular) * param_f32(m, "SpecularModifierInHDR").unwrap_or(1.0)).clamp(0.0, 1.5);
    // `ReflectAmount` scales the environment reflection: 15 on costumes, 0.25 on props and weapons
    let reflect = 0.35 + 0.65 * (param_f32(m, "ReflectAmount").unwrap_or(1.0) / 2.0).clamp(0.0, 1.0);
    let matte = shininess_to_roughness(m.shininess);
    let shiny = (matte * 0.6).clamp(0.3, 0.55);
    let key = format!("gloss:{spec}:{gain:.3}:{matte:.3}:{reflect:.3}:{SPEC_STRENGTH}:{SPEC_TINT_AMOUNT}");
    // per texel: g = how glossy the map is, metal = how much of it reflects like a metal
    let texel = move |p: &[u8]| {
        let s = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0;
        let g = (luma(s) * gain).clamp(0.0, 1.2);
        let metal = smoothstep(0.2, 0.75, g) * reflect;
        // glossier texels have sharper highlights (leather streaks, polished trims)
        let rough_dielectric = matte + (matte.min(SHARP_ROUGHNESS) - matte) * smoothstep(0.1, 0.5, g);
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
                    let hue = Vec3::ONE.lerp(norm, SPEC_TINT_AMOUNT * warm);
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

/// Roughness of the sharpest dielectric highlights (glossy map texels).
const SHARP_ROUGHNESS: f32 = 0.42;
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

/// Exponent turning the `HairColor` picker value into linear colour.
const HAIR_GAMMA: f32 = 1.5;

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
        // a big multiplier (bows 13) scales a map that is dark for wood: less sheen
        reflectance: if spec_gain > 3.0 { 0.35 } else { (spec_gain * 0.5).clamp(0.0, 0.5) },
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
            // the colour picker values are gamma encoded: ^1.5 matches the captures (golden hair
            // stays golden instead of turning cream)
            let c = hair.powf(HAIR_GAMMA) * 2.0 * m.diffuse;
            out.base_color = Color::linear_rgba(c.x, c.y, c.z, 2.0);
            // Hair is lit by the Kajiya-Kay highlight, not the generic one: streaks along the
            // strands. Bevy's anisotropic GGX would give them, but it breaks with the normal
            // prepass (SSAO, `Msaa::Off`): the PBR fragment shader then skips setting the tangent
            // frame (`anisotropy_T` / `_B` stay zero) while the lighting still takes the
            // anisotropic path, whose GGX term blows up and turns the hair white. So no
            // `anisotropy_strength`: `Shading::Full` masks an isotropic highlight with the
            // strand pattern of the hair map instead ([`bake_hair_specular`]).
            out.perceptual_roughness = 0.45;
            out.reflectance = 0.22;
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
            out.perceptual_roughness = 0.12;
            out.reflectance = 0.2;
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
        if m.flags & crate::material::flags::ADDITIVE != 0 {
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
