//! Ordinary Illum output-alpha and ambient alpha-glow composition.
//! OPACITY_MAP and every coordinate/composition-changing permutation decline this slice.
use bevy::prelude::*;

use crate::material::{Material, TextureMap};

pub(super) fn parameters(m: &Material) -> Option<Vec4> {
    const VERTALPHA: u64 = 0x400;
    const ALPHAGLOW: u64 = 0x2000;
    // ALLOW_POM alone only exposes editor parameters. Active POM, blend, opacity,
    // dark/UCC/dye/decal maps, outline alpha, unlight and unknown bits are excluded.
    const ORDINARY: u64 = 0x4
        | 0x8
        | 0x10
        | 0x80
        | VERTALPHA
        | 0x1000
        | ALPHAGLOW
        | 0x40_0000
        | 0x100_0000
        | 0x8_0000_0000;
    if !m.shader.eq_ignore_ascii_case("illum")
        || m.gen_mask & (VERTALPHA | ALPHAGLOW) == 0
        || m.gen_mask & !ORDINARY != 0
    {
        return None;
    }
    // Static scale/translation share StandardMaterial's UV transform. Motion and
    // native rotation/texgen have separate ownership and are deliberately excluded.
    if let Some(t) = m.texture(&TextureMap::Diffuse)?.tex_mod.as_ref()
        && (t.rotate_w != 0.0 || !t.unhandled.is_empty())
    {
        return None;
    }
    let multiplier = super::param_f32(m, "AmbientMultiplier").unwrap_or(1.0);
    if !multiplier.is_finite() {
        return None;
    }
    Some(Vec4::new(
        f32::from(m.gen_mask & VERTALPHA != 0),
        f32::from(m.gen_mask & ALPHAGLOW != 0),
        multiplier * (crate::stage::LIGHT_UNIT_LUX / std::f32::consts::PI),
        0.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Material {
        Material::parse(
            r#"<Material Shader="Illum" GenMask="2400" MtlFlags="524288">
          <Textures><Texture Map="Diffuse" File="a.dds"/></Textures></Material>"#,
        )
        .unwrap()
    }

    #[test]
    fn flags_defaults_and_authored_multiplier_are_independent_of_material_colour() {
        let mut m = source();
        let unit = crate::stage::LIGHT_UNIT_LUX / std::f32::consts::PI;
        assert_eq!(parameters(&m), Some(Vec4::new(1., 1., unit, 0.)));
        m.diffuse = Vec3::ZERO;
        m.public_params
            .push(("AmbientMultiplier".into(), "2.5".into()));
        assert_eq!(parameters(&m).unwrap().z, unit * 2.5);
        m.gen_mask = 0x400;
        assert_eq!(parameters(&m).unwrap().truncate().truncate(), Vec2::X);
        m.gen_mask = 0x2000;
        assert_eq!(parameters(&m).unwrap().truncate().truncate(), Vec2::Y);
    }

    #[test]
    fn unsupported_opacity_coordinates_and_composition_decline() {
        let mut m = source();
        for bit in [
            0x8000_0000,
            0x80_0000,
            0x100,
            0x40,
            0x2_0000,
            0x100_0000_0000,
            0x800_0000_0000,
        ] {
            m.gen_mask = 0x2400 | bit;
            assert!(parameters(&m).is_none(), "{bit:x}");
        }
        m = source();
        m.textures[0].tex_mod = Some(crate::material::TexMod {
            unhandled: vec![("TexMod_VOscillatorType".into(), "2".into())],
            ..default()
        });
        assert!(parameters(&m).is_none());
        m = source();
        m.shader = "Vegetation".into();
        assert!(parameters(&m).is_none());
        m = source();
        m.alpha_test = 0.059;
        assert!(
            parameters(&m).is_some(),
            "private prepass shares the native alpha law"
        );
    }
}
