//! Costume dyeing: the CPU bake of a dyed diffuse texture.
//!
//! A dyeable costume material (`Name="dyeing"`, `Shader="Illum"`) has the public parameters
//! `DyeingChannelMask="0,1,0,0"` (which channel of the mask texture selects the dyed region) and
//! `DyeingColor`, plus a mask texture in the `[1] Custom` slot (`.../costume/mask/*_mask.dds`).
//! The diffuse texture is grey where the dye goes. The item's dye colour comes from
//! `dyeable_items.default_dyeing_item_id` -> `item_dyeings` -> `dyeing_colors.color_rgb`
//! (`0x00BBGGRR`) for the login costumes.
//!
//! The original shader multiplies in the pixel shader; here the result is baked once per
//! character into a copy of the diffuse texture (all mips): inside the mask the pixel becomes the
//! dye colour shaded by the grey diffuse, outside it stays untouched.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::{Error, Result};

/// Grey level of the undyed diffuse that maps to the pure dye colour.
const GREY_REFERENCE: f32 = 0.3;

/// `0x00BBGGRR` (the `dyeing_colors.color_rgb` encoding) to sRGB bytes.
pub fn dye_rgb(color: u32) -> [u8; 3] {
    [(color & 0xFF) as u8, ((color >> 8) & 0xFF) as u8, ((color >> 16) & 0xFF) as u8]
}

/// Linear intensity -> sRGB-encoded value (both 0..1).
fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Which mask channel (0..4 = r, g, b, a) a `DyeingChannelMask` value (`"0,1,0,0"`) selects.
pub fn mask_channel(param: &str) -> usize {
    param.split(',').position(|v| v.trim().parse::<f32>().is_ok_and(|v| v > 0.5)).unwrap_or(0)
}

/// Dyes `diffuse` (a pak DDS path) with `dye` inside the `channel` of `mask`; returns an sRGB
/// RGBA8 image with the full mip chain of the diffuse texture.
pub fn bake_dyed_diffuse(diffuse: &str, mask: &str, channel: usize, dye: [u8; 3]) -> Result<Image> {
    let diffuse_bytes = aa_cry::read(diffuse)?;
    let mask_bytes = aa_cry::read(mask)?;
    let dds = aa_cry::texture::Dds::parse(&diffuse_bytes)?;
    let mask_dds = aa_cry::texture::Dds::parse(&mask_bytes)?;
    if dds.faces != 1 || dds.depth > 1 {
        return Err(Error::Format(format!("{diffuse}: not a plain 2D texture")));
    }
    // the dye colours (`0x00BBGGRR` of `dyeing_colors`) are linear intensities: the vivid orange
    // (244, 37, 0) renders as (250, 105, 0) in the client
    let dye = dye.map(|c| linear_to_srgb(c as f32 / 255.0));
    let (mw, mh) = (mask_dds.width, mask_dds.height);

    let mut data = Vec::new();
    for mip in 0..dds.mips {
        let (w, h) = ((dds.width >> mip).max(1), (dds.height >> mip).max(1));
        let mut px = dds.decode_rgba8(0, mip);
        // the mask mip closest in size to this diffuse mip
        let mut mask_mip = 0;
        while mask_mip + 1 < mask_dds.mips && (mw >> (mask_mip + 1)) >= w && (mh >> (mask_mip + 1)) >= h {
            mask_mip += 1;
        }
        let m = mask_dds.decode_rgba8(0, mask_mip);
        let (mww, mhh) = ((mw >> mask_mip).max(1) as usize, (mh >> mask_mip).max(1) as usize);
        for y in 0..h as usize {
            let my = (y * mhh / h as usize).min(mhh - 1);
            for x in 0..w as usize {
                let mx = (x * mww / w as usize).min(mww - 1);
                let weight = m[(my * mww + mx) * 4 + channel.min(3)] as f32 / 255.0;
                if weight <= 0.0 {
                    continue;
                }
                let p = &mut px[(y * w as usize + x) * 4..][..4];
                let c = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
                let grey = (c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11) / GREY_REFERENCE;
                let shade = grey.clamp(0.0, 1.5);
                for i in 0..3 {
                    let dyed = (dye[i] * shade).min(1.0);
                    let out = c[i] + (dyed - c[i]) * weight;
                    p[i] = (out.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
        data.extend_from_slice(&px);
    }

    let format = TextureFormat::Rgba8UnormSrgb;
    let size = Extent3d { width: dds.width, height: dds.height, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.mip_level_count = dds.mips;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(aa_cry::texture::repeat_sampler());
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dye_colour_encoding() {
        // "Hot red" 181 = 0x0000B5 -> red; orange 9716 = 0x0025F4
        assert_eq!(dye_rgb(181), [181, 0, 0]);
        assert_eq!(dye_rgb(9716), [0xF4, 0x25, 0]);
        assert_eq!(dye_rgb(0x00CA_DEFB), [0xFB, 0xDE, 0xCA]);
    }

    #[test]
    fn dye_is_linear() {
        assert!((linear_to_srgb(0.0)).abs() < 1e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-4);
        // 37/255 linear is about 105/255 encoded
        assert!((linear_to_srgb(37.0 / 255.0) * 255.0 - 105.0).abs() < 3.0);
    }

    #[test]
    fn channel_mask() {
        assert_eq!(mask_channel("0,1,0,0"), 1);
        assert_eq!(mask_channel("1,0,0,0"), 0);
        assert_eq!(mask_channel("0,0,0,1"), 3);
        assert_eq!(mask_channel(""), 0);
    }

    #[test]
    fn bakes_the_real_costume_texture() {
        if !aa_cry::pak_available() {
            return;
        }
        let img = bake_dyed_diffuse(
            "game/objects/characters/nuian/female/costume/nu_f_sk_hariharan001/nu_f_sk_hariharan001_df.dds",
            "game/objects/characters/nuian/male/costume/mask/nu_sk_m_hariharan001_f_hariharan001_mask.dds",
            1,
            [181, 0, 0],
        )
        .unwrap();
        assert_eq!(img.texture_descriptor.mip_level_count, 9);
        let want: usize = (0..9).map(|m| ((2048usize >> m).max(1)) * ((1024usize >> m).max(1)) * 4).sum();
        assert_eq!(img.data.as_ref().unwrap().len(), want);
    }
}
