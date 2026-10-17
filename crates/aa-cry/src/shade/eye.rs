//! Static eye albedo from `eye.ext` and `eye.cfx:137–140`: with IRIS and GLOSS_MAP, above low
//! quality, diffuse *= lerp(1, IrisColor, gloss.r). Both shared samplers use sRGBLookup=true.
//! Lighting/reflections remain StandardMaterial; aa-anim can replace the final face texture.

use crate::material::{Material, TexMod, TextureMap};
use bevy::prelude::*;

const IRIS: u64 = 0x1;
const GLOSS_MAP: u64 = 0x4;

#[derive(Debug)]
struct Recipe {
    diffuse: String,
    gloss: String,
    iris: Vec3,
}

impl Recipe {
    fn from_material(m: &Material) -> Option<Self> {
        if !m.shader.eq_ignore_ascii_case("eye")
            || m.gen_mask & (IRIS | GLOSS_MAP) != (IRIS | GLOSS_MAP)
        {
            return None;
        }
        let iris = super::param_rgb(m, "IrisColor").unwrap_or(Vec3::ONE);
        if iris == Vec3::ONE
            || !iris.is_finite()
            || iris.cmplt(Vec3::ZERO).any()
            || iris.cmpgt(Vec3::ONE).any()
        {
            return None;
        }
        let diffuse = m.texture(&TextureMap::Diffuse)?;
        let gloss = m.texture(&TextureMap::Specular)?;
        // Decline animated, generated or modified mask UVs until their native coordinate
        // handling is implemented. The supported path is periodic for ordinary repeated UVs.
        if [diffuse, gloss]
            .iter()
            .any(|t| t.tex_mod.as_ref().is_some_and(|m| *m != TexMod::default()))
        {
            return None;
        }
        Some(Self {
            diffuse: diffuse.pak_path(),
            gloss: gloss.pak_path(),
            iris,
        })
    }
}

fn sample_red(mask: &[u8], width: u32, height: u32, uv: Vec2, linear: &[f32]) -> f32 {
    let p = uv * Vec2::new(width as f32, height as f32) - Vec2::splat(0.5);
    let lo = p.floor();
    let f = p - lo;
    let at = |x: i32, y: i32| {
        let i = (y.rem_euclid(height as i32) as u32 * width + x.rem_euclid(width as i32) as u32)
            as usize
            * 4;
        linear[mask[i] as usize]
    };
    let (x, y) = (lo.x as i32, lo.y as i32);
    let a = at(x, y) + (at(x + 1, y) - at(x, y)) * f.x;
    let b = at(x, y + 1) + (at(x + 1, y + 1) - at(x, y + 1)) * f.x;
    a + (b - a) * f.y
}

fn tint(
    diffuse: &mut [Vec<u8>],
    size: (u32, u32),
    gloss: &[Vec<u8>],
    mask_size: (u32, u32),
    iris: Vec3,
) -> bool {
    let (w, h) = size;
    let (mw, mh) = mask_size;
    if ![w, h, mw, mh].iter().all(|n| n.is_power_of_two()) || w * mh != h * mw || gloss.is_empty() {
        return false;
    }
    if diffuse
        .iter()
        .enumerate()
        .any(|(m, data)| data.len() != (w >> m).max(1) as usize * (h >> m).max(1) as usize * 4)
        || gloss.iter().enumerate().any(|(m, data)| {
            data.len() != (mw >> m).max(1) as usize * (mh >> m).max(1) as usize * 4
        })
    {
        return false;
    }
    let linear: Vec<f32> = (0..256)
        .map(|v| super::srgb_to_linear(v as f32 / 255.0))
        .collect();
    // Both native samples use baseTC.xy. The same UV derivatives therefore change the gloss
    // LOD by log2(gloss size / diffuse size). A uniform power-of-two ratio has an integer bias.
    let bias = mw.ilog2() as i32 - w.ilog2() as i32;
    for (mip, colour) in diffuse.iter_mut().enumerate() {
        let level = (mip as i32 + bias).clamp(0, gloss.len() as i32 - 1) as usize;
        let (dw, dh) = ((w >> mip).max(1), (h >> mip).max(1));
        let (gw, gh) = ((mw >> level).max(1), (mh >> level).max(1));
        for (i, p) in colour.chunks_exact_mut(4).enumerate() {
            let uv = Vec2::new((i as u32 % dw) as f32 + 0.5, (i as u32 / dw) as f32 + 0.5)
                / Vec2::new(dw as f32, dh as f32);
            let weight = sample_red(&gloss[level], gw, gh, uv, &linear);
            for c in 0..3 {
                let value = linear[p[c] as usize] * (1.0 + (iris[c] - 1.0) * weight);
                p[c] = (super::linear_to_srgb(value) * 255.0).round() as u8;
            }
            // Neither the mask's other channels nor its alpha affect eye albedo or opacity.
        }
    }
    true
}

pub(super) fn bake(prep: &mut crate::loader::Prep<'_>, m: &Material, out: &mut StandardMaterial) {
    let Some(recipe) = Recipe::from_material(m) else {
        return;
    };
    let key = format!("eye-iris-v1:{recipe:?}");
    let image = prep.baked_image(&key, |loader| {
        let (w, h, mut diffuse) = loader.decode_mips_rgba8(&recipe.diffuse)?;
        let (mw, mh, mask) = loader.decode_mips_rgba8(&recipe.gloss)?;
        if !tint(&mut diffuse, (w, h), &mask, (mw, mh), recipe.iris) {
            return None;
        }
        Some(crate::texture::rgba8_image(w, h, &diffuse, true))
    });
    if let Some(image) = image {
        if out.emissive_texture.is_some() {
            out.emissive_texture = Some(image.clone());
        }
        out.base_color_texture = Some(image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(flags: u64) -> Material {
        Material::parse(&format!(
            r#"<Material Shader="Eye" MtlFlags="524416" GenMask="{flags:x}"><Textures>
            <Texture Map="Diffuse" File="colour.dds"/><Texture Map="Specular" File="mask.dds"/>
            </Textures><PublicParams IrisColor="0,0,1"/></Material>"#
        ))
        .unwrap()
    }

    #[test]
    fn both_shader_flags_and_authored_unmodified_inputs_are_required() {
        for flags in [0, IRIS, GLOSS_MAP] {
            assert!(Recipe::from_material(&material(flags)).is_none());
        }
        let mut m = material(IRIS | GLOSS_MAP);
        assert!(Recipe::from_material(&m).is_some());
        m.textures[1].tex_mod = Some(TexMod {
            offset_u: 0.25,
            ..default()
        });
        assert!(Recipe::from_material(&m).is_none());
        m.textures[1].tex_mod = Some(TexMod {
            unhandled: vec![("TexMod_UOscillatorType".into(), "2".into())],
            ..default()
        });
        assert!(Recipe::from_material(&m).is_none());
        m.textures[1].tex_mod = None;
        m.public_params.clear();
        assert!(
            Recipe::from_material(&m).is_none(),
            "white shader default is an identity"
        );
        m.public_params.push(("IrisColor".into(), "0,0,1".into()));
        m.shader = "Illum".into();
        assert!(Recipe::from_material(&m).is_none());
    }

    #[test]
    fn gloss_red_is_decoded_from_srgb_and_diffuse_alpha_survives_each_mip() {
        let mut diffuse = vec![
            vec![128, 128, 128, 17, 128, 128, 128, 221],
            vec![255, 255, 255, 83],
        ];
        let gloss = vec![vec![0, 255, 255, 0, 255, 0, 0, 255], vec![128, 255, 0, 0]];
        assert!(tint(&mut diffuse, (2, 1), &gloss, (2, 1), Vec3::Z));
        assert_eq!(diffuse[0], [128, 128, 128, 17, 0, 0, 128, 221]);
        assert_eq!(
            diffuse[1],
            [229, 229, 255, 83],
            "midgrey gloss is a 0.216 linear weight, not 0.502"
        );
    }

    #[test]
    fn half_resolution_gloss_uses_linear_bilinear_samples_and_one_lower_lod() {
        let mut diffuse = vec![vec![255; 4 * 2 * 4], vec![255; 2 * 4], vec![255; 4]];
        let gloss = vec![vec![0, 255, 0, 0, 255, 0, 255, 255], vec![128, 0, 0, 255]];
        assert!(tint(&mut diffuse, (4, 2), &gloss, (2, 1), Vec3::Z));
        assert_eq!(&diffuse[1][..4], &[255, 255, 255, 255]);
        assert_eq!(
            &diffuse[1][4..8],
            &[0, 0, 255, 255],
            "diffuse mip1 still samples gloss mip0"
        );
        assert_eq!(
            diffuse[2],
            [229, 229, 255, 255],
            "diffuse mip2 samples authored gloss mip1"
        );
        assert_eq!(
            &diffuse[0][4..8],
            &[225, 225, 255, 255],
            "decode each mask texel before bilinear filtering"
        );
        assert!(
            !tint(&mut diffuse, (4, 2), &gloss, (2, 2), Vec3::Z),
            "nonuniform mask domains are declined"
        );
    }

    #[test]
    fn real_ferre_material_bakes_once_and_preserves_shading_and_emissive_albedo() {
        if !crate::pak_available() {
            return;
        }
        use crate::loader::{CryLoader, Pending, Prep, Shared};
        use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
        use bevy::render::render_resource::TextureFormat;
        let images = Assets::<Image>::default();
        let meshes = Assets::<Mesh>::default();
        let materials = Assets::<StandardMaterial>::default();
        let binds = Assets::<SkinnedMeshInverseBindposes>::default();
        let loader = CryLoader::new(
            images.get_handle_provider(),
            meshes.get_handle_provider(),
            materials.get_handle_provider(),
            binds.get_handle_provider(),
            false,
        );
        let mtl =
            Material::load("objects/characters/ferre/female/face/face01/fe_f_face01.mtl").unwrap();
        let m = mtl.sub(3).unwrap();
        let mut prep = Prep::new(&loader);
        let out = prep.convert(m).unwrap();
        let again = prep.convert(m).unwrap();
        assert_eq!(out.base_color_texture, again.base_color_texture);
        let mut old = m.clone();
        old.gen_mask &= !IRIS;
        let control = prep.convert(&old).unwrap();
        assert_ne!(out.base_color_texture, control.base_color_texture);
        assert_eq!(out.base_color, control.base_color);
        assert_eq!(out.alpha_mode, control.alpha_mode);
        assert_eq!(out.normal_map_texture, control.normal_map_texture);
        assert_eq!(out.emissive, control.emissive);
        assert_eq!(out.base_color_texture, out.emissive_texture);
        let image = prep
            .pending
            .iter()
            .find_map(|p| match p {
                Pending::Shared(Shared::Image(h, slot))
                    if Some(h) == out.base_color_texture.as_ref() =>
                {
                    Some(crate::loader::lock(slot).as_ref().unwrap().clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            image.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );
        let (w, h, source) = loader
            .decode_mips_rgba8(&m.texture(&TextureMap::Diffuse).unwrap().pak_path())
            .unwrap();
        let (mw, mh, _) = loader
            .decode_mips_rgba8(&m.texture(&TextureMap::Specular).unwrap().pak_path())
            .unwrap();
        assert_eq!((w, h), (512, 512));
        assert_eq!((mw, mh), (256, 256));
        let original: Vec<u8> = source.concat();
        let result = image.data.unwrap();
        assert_eq!(original.len(), result.len());
        assert_eq!(
            original.iter().skip(3).step_by(4).collect::<Vec<_>>(),
            result.iter().skip(3).step_by(4).collect::<Vec<_>>()
        );
        assert!(original.iter().zip(&result).any(|(a, b)| a != b));
        let tinted: Vec<_> = original
            .chunks_exact(4)
            .zip(result.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .collect();
        assert!(
            tinted
                .iter()
                .all(|(a, b)| b[0] <= a[0] && b[1] <= a[1] && b[2] <= a[2]),
            "the authored factors multiply albedo"
        );
        let mut blue = m.clone();
        blue.public_params
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case("IrisColor"))
            .unwrap()
            .1 = "0,0,1".into();
        assert_ne!(
            out.base_color_texture,
            prep.convert(&blue).unwrap().base_color_texture,
            "the palette belongs to the cache key"
        );
    }
}
