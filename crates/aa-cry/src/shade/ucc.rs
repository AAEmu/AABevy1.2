//! Static Cloth UCC composition from the client's `cloth.ext`, `cloth.cfx:384–467,532–539,735–753`
//! and `FXSamplerDefs.cfi`. Both the RGB mask and the crest are sampled without sRGB decoding.
//! The native `$CustomMap` is a runtime binding; for static materials without a Custom texture we
//! explicitly use the authored Diffuse pattern. Dynamic player crests need a separate binding.
//!
//! This bake covers one UV square (the galleon sail's UCC subsets use 0..1). Animated TexMods,
//! rotated TexMods and nonidentity Diffuse TexMods are left to a future shader implementation.

use bevy::prelude::*;

use crate::material::{Material, TexMod, TextureMap};

const UCC: u64 = 0x10_0000;
const BACKGROUND: u64 = 0x20_0000;
const IGNORE_BACKGROUND: u64 = 0x40_0000;

#[derive(Debug)]
struct Recipe {
    diffuse: String,
    mask: Option<String>,
    logo: String,
    colours: [Vec3; 3],
    scale: Vec2,
    offset: Vec2,
    add_offset: Vec2,
    tex_mod: TexMod,
}

impl Recipe {
    fn from_material(m: &Material) -> Option<Self> {
        if !m.shader.eq_ignore_ascii_case("cloth") || m.gen_mask & UCC == 0 {
            return None;
        }
        let diffuse = m.texture(&TextureMap::Diffuse)?;
        // A single StandardMaterial UV transform also changes normals. Bake in the unmodified
        // mesh UV domain, retaining the existing diffuse/normal behaviour for other materials.
        if diffuse
            .tex_mod
            .as_ref()
            .is_some_and(|t| *t != TexMod::default())
        {
            return None;
        }
        let logo = m.texture(&TextureMap::SubSurface)?;
        let tex_mod = logo.tex_mod.clone().unwrap_or_default();
        if tex_mod.rotate_w != 0.0 {
            return None;
        }
        let param = |name, fallback| {
            super::param_f32(m, name)
                .filter(|x| x.is_finite())
                .unwrap_or(fallback)
        };
        let add_offset = Vec2::new(param("uccAddXOffset", 0.0), param("uccAddYOffset", 0.0));
        if add_offset.x == 1.0 || add_offset.y == 1.0 {
            return None; // the source shader divides by 1-offset
        }
        Some(Self {
            diffuse: diffuse.pak_path(),
            mask: (m.gen_mask & BACKGROUND != 0 && m.gen_mask & IGNORE_BACKGROUND == 0)
                .then(|| m.texture(&TextureMap::Custom).unwrap_or(diffuse).pak_path()),
            logo: logo.pak_path(),
            colours: [
                super::param_rgb(m, "uccRedMask").unwrap_or(Vec3::X),
                super::param_rgb(m, "uccGreenMask").unwrap_or(Vec3::Y),
                super::param_rgb(m, "uccBlueMask").unwrap_or(Vec3::Z),
            ],
            scale: Vec2::new(param("uccScaleX", 1.0), param("uccScaleY", 1.0)),
            offset: Vec2::new(param("uccOffsetX", 0.0), param("uccOffsetY", 0.0)),
            add_offset,
            tex_mod,
        })
    }

    fn custom_uv(&self, uv: Vec2) -> Vec2 {
        uv * Vec2::new(self.tex_mod.tile_u, self.tex_mod.tile_v)
            + Vec2::new(self.tex_mod.offset_u, self.tex_mod.offset_v)
    }

    fn colour(&self, uv: Vec2, diffuse: Vec3, mask: Option<&Pixels>, logo: &Pixels) -> Vec3 {
        let tc = self.custom_uv(uv);
        let mut colour = match mask {
            Some(mask) => {
                let weights = mask
                    .sample((tc - self.add_offset) / (Vec2::ONE - self.add_offset))
                    .truncate();
                (self.colours[0] * weights.x
                    + self.colours[1] * weights.y
                    + self.colours[2] * weights.z)
                    .clamp(Vec3::ZERO, Vec3::ONE)
            }
            None => diffuse,
        };
        let clip_uv = tc * self.scale - self.offset;
        // The source condition is strict: the logo never wraps across its placement rectangle.
        if clip_uv.cmpgt(Vec2::ZERO).all() && clip_uv.cmplt(Vec2::ONE).all() {
            let mark = logo.sample(clip_uv);
            colour = colour.lerp(mark.truncate(), mark.w);
        }
        colour
    }
}

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    /// Bilinear wrap, at texel centres, in the samplers' linear numeric space.
    fn sample(&self, uv: Vec2) -> Vec4 {
        let p = uv * Vec2::new(self.width as f32, self.height as f32) - Vec2::splat(0.5);
        let lo = p.floor();
        let f = p - lo;
        let at = |x: i32, y: i32| {
            let x = x.rem_euclid(self.width as i32) as usize;
            let y = y.rem_euclid(self.height as i32) as usize;
            let i = (y * self.width as usize + x) * 4;
            let p = &self.rgba[i..i + 4];
            Vec4::new(p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32) / 255.0
        };
        let x = lo.x as i32;
        let y = lo.y as i32;
        at(x, y)
            .lerp(at(x + 1, y), f.x)
            .lerp(at(x, y + 1).lerp(at(x + 1, y + 1), f.x), f.y)
    }
}

fn compose(
    recipe: &Recipe,
    width: u32,
    height: u32,
    source: &[Vec<u8>],
    mask: Option<&Pixels>,
    logo: &Pixels,
) -> Vec<Vec<u8>> {
    let mut top = Vec::with_capacity(source[0].len());
    for (i, p) in source[0].chunks_exact(4).enumerate() {
        let uv = Vec2::new(
            (i as u32 % width) as f32 + 0.5,
            (i as u32 / width) as f32 + 0.5,
        ) / Vec2::new(width as f32, height as f32);
        // Only the ordinary Diffuse sampler has sRGBLookup=true. The background mask replaces
        // this RGB, while its alpha survives both background remapping and crest blending.
        let diffuse = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
            .map(|v| super::srgb_to_linear(v / 255.0));
        let rgb = recipe.colour(uv, diffuse, mask, logo);
        top.extend_from_slice(&[
            (rgb.x * 255.0).round() as u8,
            (rgb.y * 255.0).round() as u8,
            (rgb.z * 255.0).round() as u8,
            p[3],
        ]);
    }
    let mut levels = vec![top];
    // Filter the composed colour in linear space. Keep each original Diffuse alpha mip, rather
    // than filtering crest alpha into opacity or changing the authored silhouette.
    for mip in 1..source.len() {
        let pw = (width >> (mip - 1)).max(1) as usize;
        let ph = (height >> (mip - 1)).max(1) as usize;
        let w = (width >> mip).max(1) as usize;
        let h = (height >> mip).max(1) as usize;
        let prev = levels.last().unwrap();
        let mut level = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    let mut sum = 0u32;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            sum += prev[(((y * 2 + dy).min(ph - 1)) * pw
                                + (x * 2 + dx).min(pw - 1))
                                * 4
                                + c] as u32;
                        }
                    }
                    level.push(((sum + 2) / 4) as u8);
                }
                level.push(source[mip][(y * w + x) * 4 + 3]);
            }
        }
        levels.push(level);
    }
    levels
}

pub(super) fn bake(prep: &mut crate::loader::Prep<'_>, m: &Material, out: &mut StandardMaterial) {
    let Some(recipe) = Recipe::from_material(m) else {
        return;
    };
    let key = format!("cloth-ucc-v1:{recipe:?}");
    let image = prep.baked_image(&key, |loader| {
        let (width, height, source) = loader.decode_mips_rgba8(&recipe.diffuse)?;
        let pixels = |path: &str| {
            let (width, height, rgba) = loader.decode_rgba8(path, 0)?;
            Some(Pixels {
                width,
                height,
                rgba,
            })
        };
        let mask = match recipe.mask.as_deref() {
            Some(path) => Some(pixels(path)?),
            None => None,
        };
        let logo = pixels(&recipe.logo)?;
        let mips = compose(&recipe, width, height, &source, mask.as_ref(), &logo);
        Some(crate::texture::rgba8_image(width, height, &mips, false))
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
        Material::parse(&format!(r#"<Material Shader="Cloth" MtlFlags="524416" GenMask="{flags:x}"><Textures>
            <Texture Map="Diffuse" File="pattern.dds"/><Texture Map="SubSurface" File="logo.dds"/>
            </Textures><PublicParams uccRedMask="0.21568629,0.21568629,0.21568629" uccGreenMask="1,1,1" uccBlueMask="1,1,1"
            uccScaleX="3" uccScaleY="3" uccOffsetX="1" uccOffsetY="1"/></Material>"#)).unwrap()
    }

    fn pixel(rgba: [u8; 4]) -> Pixels {
        Pixels {
            width: 1,
            height: 1,
            rgba: rgba.to_vec(),
        }
    }

    #[test]
    fn background_uses_linear_weights_and_authored_palette() {
        let r = Recipe::from_material(&material(UCC | BACKGROUND)).unwrap();
        let transparent = pixel([255, 0, 0, 0]);
        for (mask, expected) in [
            ([255, 0, 0, 255], 55.0 / 255.0),
            ([0, 128, 0, 255], 128.0 / 255.0),
            ([255, 255, 255, 255], 1.0),
        ] {
            let c = r.colour(
                Vec2::splat(0.1),
                Vec3::ZERO,
                Some(&pixel(mask)),
                &transparent,
            );
            assert!((c - Vec3::splat(expected)).length() < 1e-6, "{c:?}");
        }
    }

    #[test]
    fn logo_uses_authored_uv_strict_clip_and_alpha_blend() {
        let r = Recipe::from_material(&material(UCC)).unwrap();
        let blue = Vec3::Z;
        let mark = pixel([255, 0, 0, 128]);
        for uv in [
            Vec2::splat(0.1),
            Vec2::new(1.0 / 3.0, 0.5),
            Vec2::new(2.0 / 3.0, 0.5),
        ] {
            assert_eq!(r.colour(uv, blue, None, &mark), blue);
        }
        let centre = r.colour(Vec2::splat(0.5), blue, None, &mark);
        assert!((centre - blue.lerp(Vec3::X, 128.0 / 255.0)).length() < 1e-6);
    }

    #[test]
    fn flags_defaults_and_unsupported_transforms_are_explicit() {
        assert!(Recipe::from_material(&material(BACKGROUND)).is_none());
        let ignored =
            Recipe::from_material(&material(UCC | BACKGROUND | IGNORE_BACKGROUND)).unwrap();
        assert!(ignored.mask.is_none());
        let mut m = material(UCC | BACKGROUND);
        m.public_params.clear();
        let r = Recipe::from_material(&m).unwrap();
        assert_eq!(r.colours, [Vec3::X, Vec3::Y, Vec3::Z]);
        assert_eq!(r.scale, Vec2::ONE);
        assert_eq!(r.offset, Vec2::ZERO);
        m.shader = "Illum".into();
        assert!(Recipe::from_material(&m).is_none());
        m.shader = "Cloth".into();
        m.textures[0].tex_mod = Some(TexMod {
            tile_u: 2.0,
            ..default()
        });
        assert!(Recipe::from_material(&m).is_none());
    }

    #[test]
    fn composition_preserves_every_diffuse_alpha_mip() {
        let r = Recipe::from_material(&material(UCC | BACKGROUND)).unwrap();
        let source = vec![vec![255, 0, 0, 7, 0, 255, 0, 199], vec![128, 128, 0, 103]];
        let mask = Pixels {
            width: 2,
            height: 1,
            rgba: source[0].clone(),
        };
        let mips = compose(&r, 2, 1, &source, Some(&mask), &pixel([0, 0, 0, 0]));
        assert_eq!(mips[0], [55, 55, 55, 7, 255, 255, 255, 199]);
        assert_eq!(mips[1], [155, 155, 155, 103]);
    }

    #[test]
    fn ignored_background_keeps_the_ordinary_diffuse_srgb_lookup() {
        let r = Recipe::from_material(&material(UCC | BACKGROUND | IGNORE_BACKGROUND)).unwrap();
        let source = vec![vec![128, 128, 128, 91]];
        let result = compose(&r, 1, 1, &source, None, &pixel([255, 0, 0, 0]));
        assert_eq!(result[0], [55, 55, 55, 91]);
    }

    #[test]
    fn background_and_crest_share_subsurface_uv_before_their_own_controls() {
        let mut r = Recipe::from_material(&material(UCC | BACKGROUND)).unwrap();
        r.tex_mod = TexMod {
            tile_u: 0.5,
            offset_u: 0.25,
            ..default()
        };
        r.add_offset = Vec2::new(0.25, 0.0);
        let mask = Pixels {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        };
        let transparent = pixel([0, 0, 0, 0]);
        // Mesh U=.375 -> SubSurface U=.4375 -> background U=.25 (first texel centre).
        let c = r.colour(Vec2::new(0.375, 0.1), Vec3::ZERO, Some(&mask), &transparent);
        assert!((c - Vec3::splat(55.0 / 255.0)).length() < 1e-6);
        // The crest uses SubSurface U=.5, then scale3-offset1: its centre.
        assert_eq!(
            r.colour(Vec2::splat(0.5), Vec3::ZERO, None, &pixel([0, 0, 255, 255])),
            Vec3::Z
        );
    }

    #[test]
    fn real_galleon_conversion_caches_linear_composition_and_retains_material() {
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
        let m = Material::load("objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl").unwrap();
        let m = m.sub(3).unwrap();
        let mut prep = Prep::new(&loader);
        let out = prep.convert(m).unwrap();
        let mut control = m.clone();
        control.gen_mask = 0;
        let control = prep.convert(&control).unwrap();
        assert_eq!(out.base_color, control.base_color);
        assert_eq!(out.alpha_mode, control.alpha_mode);
        assert_eq!(out.cull_mode, control.cull_mode);
        assert_eq!(out.normal_map_texture, control.normal_map_texture);
        assert_ne!(out.base_color_texture, control.base_color_texture);
        assert_eq!(
            out.base_color_texture,
            prep.convert(m).unwrap().base_color_texture
        );
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
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        let (w, h, original) = loader
            .decode_mips_rgba8("objects/env/07_ucc/ucc_pattern04.dds")
            .unwrap();
        let data = image.data.unwrap();
        let mut offset = 0;
        for mip in original {
            assert_eq!(
                data[offset..offset + mip.len()]
                    .iter()
                    .skip(3)
                    .step_by(4)
                    .copied()
                    .collect::<Vec<_>>(),
                mip.iter().skip(3).step_by(4).copied().collect::<Vec<_>>()
            );
            offset += mip.len();
        }
        // Outside the authored crest rectangle, every texel is neutral cloth, never raw RGB.
        for y in 0..h / 3 {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                assert_eq!(data[i], data[i + 1]);
                assert_eq!(data[i + 1], data[i + 2]);
            }
        }
    }

    #[test]
    fn real_galleon_recipe_and_uv_domain() {
        if !crate::pak_available() {
            return;
        }
        let m = Material::load("objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl").unwrap();
        let r = Recipe::from_material(m.sub(3).unwrap()).unwrap();
        assert_eq!(
            r.mask.as_deref(),
            Some("objects/env/07_ucc/ucc_pattern04.dds")
        );
        assert_eq!(r.logo, "objects/env/07_ucc/ucc_logo32.dds");
        assert_eq!(r.scale, Vec2::splat(3.0));
        assert_eq!(r.offset, Vec2::ONE);
        let model =
            crate::Model::load("objects/env/06_unit/01_ship/galleon/galleon_deco_sail01.cgf")
                .unwrap();
        let mut used = 0;
        for mesh in &model.meshes {
            for sub in mesh.subsets.iter().filter(|s| s.material_id == 3) {
                for &i in &mesh.indices
                    [sub.first_index as usize..(sub.first_index + sub.index_count) as usize]
                {
                    let uv = mesh.uvs[i as usize];
                    assert!(
                        uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all(),
                        "{uv:?}"
                    );
                    used += 1;
                }
            }
        }
        assert!(used > 0);
    }
}
