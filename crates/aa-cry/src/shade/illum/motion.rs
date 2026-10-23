//! Narrow native type-2 TexMod path. See the executable x86 oracle in tools/.
//! Rotation, coordinate generation and other oscillator types deliberately decline this path.
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

use crate::material::{Material, TexMod, TextureMap};

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub(super) struct MotionData {
    /// rate U/V, amplitude U/V. Native multiplies rate * seconds before amplitude.
    pub oscillator: Vec4,
    pub opacity_mask: Vec4,
    /// x: type-2 motion, y: opacity map present. Zero is the unchanged material path.
    pub mode: Vec4,
}

#[derive(Clone, Debug)]
pub(super) struct Motion {
    pub data: MotionData,
    pub opacity_map: Option<Handle<Image>>,
}

fn oscillator(t: &TexMod) -> Option<Vec4> {
    if t.rotate_w != 0.0 {
        return None;
    }
    let mut types = [0.0; 2];
    let mut rate = [0.0; 2];
    let mut amplitude = [0.0; 2];
    for (name, value) in &t.unhandled {
        let value: f32 = value.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        match name.to_ascii_lowercase().as_str() {
            "texmod_uoscillatortype" => types[0] = value,
            "texmod_voscillatortype" => types[1] = value,
            "texmod_uoscillatorrate" => rate[0] = value,
            "texmod_voscillatorrate" => rate[1] = value,
            "texmod_uoscillatoramplitude" => amplitude[0] = value,
            "texmod_voscillatoramplitude" => amplitude[1] = value,
            // Original type 2 ignores these fields (only periodic/jitter types read phase).
            "texmod_uoscillatorphase" | "texmod_voscillatorphase" => {}
            "rotateu"
            | "rotatev"
            | "texmod_rotatetype"
            | "texmod_texgentype"
            | "texmod_btexgenprojected"
                if value == 0.0 => {}
            _ => return None,
        }
    }
    for axis in 0..2 {
        match types[axis] {
            0.0 => amplitude[axis] = 0.0,
            2.0 => {}
            _ => return None,
        }
        if !(rate[axis] * amplitude[axis]).is_finite() {
            return None;
        }
    }
    (rate[0] * amplitude[0] != 0.0 || rate[1] * amplitude[1] != 0.0)
        .then(|| Vec4::new(rate[0], rate[1], amplitude[0], amplitude[1]))
}

fn parameters(m: &Material) -> Option<MotionData> {
    // These permutations change coordinate ownership/composition; keep the old path.
    const CONFLICT: u64 = 0x20_0000 | 0x80_0000 | 0x100_0000_0000;
    if !m.shader.eq_ignore_ascii_case("illum") || m.gen_mask & CONFLICT != 0 {
        return None;
    }
    let oscillator = oscillator(m.texture(&TextureMap::Diffuse)?.tex_mod.as_ref()?)?;
    let mask = m
        .public_param("opacityChannelMask")
        .and_then(|s| {
            let values: Vec<f32> = s
                .split(',')
                .map(str::trim)
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            (values.len() == 4 && values.iter().all(|v| v.is_finite()))
                .then(|| Vec4::from_slice(&values))
        })
        .unwrap_or(Vec4::W);
    Some(MotionData {
        oscillator,
        opacity_mask: mask,
        mode: Vec4::X,
    })
}

pub(super) fn prepare(
    prep: &mut crate::loader::Prep<'_>,
    m: &Material,
    out: &StandardMaterial,
) -> Option<Motion> {
    if out.alpha_mode != AlphaMode::Blend || std::env::var_os("AA_ILLUM_MOTION_LEGACY").is_some() {
        return None;
    }
    let mut data = parameters(m)?;
    // fxsamplerdefs.cfi: OPACITYMAP has no sRGB lookup. VertexLib sets opacity
    // coordinates from raw IN.baseTC, even if the Opacity texture authors a TexMod.
    let opacity_map = (m.gen_mask & 0x8000_0000 != 0)
        .then(|| m.texture(&TextureMap::Opacity))
        .flatten()
        .and_then(|t| prep.texture(&t.pak_path(), false));
    data.mode.y = if opacity_map.is_some() { 1.0 } else { 0.0 };
    Some(Motion { data, opacity_map })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Material {
        Material::parse(
            r#"<Material Shader="Illum" MtlFlags="524288" GenMask="80000000" Opacity="0.7">
          <Textures><Texture Map="Diffuse" File="a.dds"><TexMod TileV="0.3"
           TexMod_VOscillatorType="2" TexMod_UOscillatorRate="0.1" TexMod_VOscillatorRate="0.25"
           TexMod_VOscillatorAmplitude="-3" TexMod_VOscillatorPhase="7"/></Texture></Textures>
        </Material>"#,
        )
        .unwrap()
    }

    #[test]
    fn native_type_two_ignores_phase_and_disabled_axis() {
        let m = source();
        let p = parameters(&m).unwrap();
        assert_eq!(p.oscillator, Vec4::new(0.1, 0.25, 0.0, -3.0));
        assert_eq!(p.opacity_mask, Vec4::W);
        // Values independently measured by executing the original x86 update routine.
        let tile = m.textures[0].tex_mod.as_ref().unwrap().tile_v;
        for (seconds, native) in [
            (0., 0.),
            (1., -0.225),
            (2., -0.45),
            (4., -0.9),
            (3601., -810.225),
        ] {
            let offset = p.oscillator.w * (p.oscillator.y * seconds) * tile;
            assert!((offset - native).abs() < 0.0001);
        }
    }

    #[test]
    fn unknown_types_rotation_generation_and_separate_uvs_decline() {
        for attribute in [
            "TexMod_VOscillatorType",
            "RotateU",
            "TexMod_TexGenType",
            "Other",
        ] {
            let mut m = source();
            m.textures[0]
                .tex_mod
                .as_mut()
                .unwrap()
                .unhandled
                .push((attribute.into(), "3".into()));
            assert!(parameters(&m).is_none(), "{attribute}");
        }
        let mut m = source();
        m.textures[0].tex_mod.as_mut().unwrap().rotate_w = 1.0;
        assert!(parameters(&m).is_none());
        m = source();
        m.gen_mask |= 0x20_0000;
        assert!(parameters(&m).is_none());
        m = source();
        m.textures[0]
            .tex_mod
            .as_mut()
            .unwrap()
            .unhandled
            .push(("TexMod_VOscillatorAmplitude".into(), "0".into()));
        assert!(parameters(&m).is_none(), "no effective motion");
    }

    #[test]
    fn original_waterfall_materials_keep_their_authored_rates_and_masks() {
        if !crate::pak_available() {
            return;
        }
        for (path, sub, amplitude) in [
            ("objects/effects/waterfall/waterfall_size_15.mtl", 0, -3.),
            ("objects/env/02_harihara/01_ma/ma_waterfall_a.mtl", 1, -0.7),
        ] {
            let source = Material::load(path).unwrap();
            let p = parameters(source.sub(sub).unwrap()).unwrap();
            assert_eq!(p.oscillator.y, 0.25);
            assert_eq!(p.oscillator.w, amplitude);
            assert_eq!(p.oscillator.z, 0.0, "disabled U oscillator");
            assert_eq!(p.opacity_mask, Vec4::W);
        }
    }
}
