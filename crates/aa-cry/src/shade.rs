//! CryEngine material -> Bevy `StandardMaterial` (the shader mapping documented in `docs/aa-cry.md`).

use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::Face;

use crate::material::{Material as CryMaterial, TextureMap};

/// The conversion used by the loader: [`to_standard_material`] plus whatever needs pixel data.
pub(crate) fn convert_material(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial) -> Option<StandardMaterial> {
    to_standard_material(m, |path, srgb| prep.texture(path, srgb))
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
    let mut out = StandardMaterial {
        base_color: Color::linear_rgb(m.diffuse.x, m.diffuse.y, m.diffuse.z),
        base_color_texture: diffuse_tex.as_ref().and_then(|(f, _)| texture(f, true)),
        normal_map_texture: normal_tex.as_deref().and_then(|f| texture(f, false)),
        perceptual_roughness: shininess_to_roughness(m.shininess),
        metallic: 0.0,
        reflectance: ((m.specular.x * 0.2126 + m.specular.y * 0.7152 + m.specular.z * 0.0722) * m.specular_level * 0.5).clamp(0.0, 1.0),
        emissive: LinearRgba::rgb(m.emissive.x, m.emissive.y, m.emissive.z),
        double_sided: m.two_sided(),
        cull_mode: if m.two_sided() { None } else { Some(Face::Back) },
        ..default()
    };
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
            out.alpha_mode = AlphaMode::Mask(if m.alpha_test > 0.0 { m.alpha_test } else { 0.5 });
            // Hair textures are greyscale; the colour is the (customisable) HairColor param.
            if let Some(c) = m.public_param("HairColor").and_then(parse_rgb) {
                out.base_color = Color::linear_rgb(c.x, c.y, c.z);
            }
        }
        "humanskin" => {
            // The skin tone comes from the texture and the character's customisation; the
            // per-file Diffuse tints differ between face and body, so use one neutral value.
            out.base_color = Color::linear_rgb(SKIN_ALBEDO, SKIN_ALBEDO, SKIN_ALBEDO);
            // skin F0 ~0.028; the files' strong specular is for CryEngine's skin shader
            out.reflectance = 0.42;
            out.perceptual_roughness = out.perceptual_roughness.max(0.55);
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
        _ => {}
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

/// Diffuse multiplier used for every `HumanSkin` material.
pub const SKIN_ALBEDO: f32 = 0.22;

/// `"r,g,b"` -> Vec3.
fn parse_rgb(s: &str) -> Option<Vec3> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>());
    Some(Vec3::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
}

/// Blinn-Phong exponent -> perceptual roughness (`alpha = sqrt(2 / (n + 2))`, `r = sqrt(alpha)`).
pub fn shininess_to_roughness(n: f32) -> f32 {
    (2.0 / (n.max(0.0) + 2.0)).powf(0.25).clamp(0.05, 1.0)
}
