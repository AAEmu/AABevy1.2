//! Face-normal strength from the character's wrinkle slider / preset.
//!
//! Bevy's standard material has no normal-strength parameter. Bake the selected map towards
//! the neutral tangent-space normal instead, retaining the DDS mip chain and linear sampling.

use aa_cry::texture::{Dds, DdsFormat, rgba8_image};
use bevy::image::Image;
use glam::Vec3;

use crate::{Error, Result};

pub(crate) fn strength(weight: f32) -> f32 {
    if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 1.0 }
}

pub(crate) fn bake(path: &str, weight: f32) -> Result<Image> {
    let bytes = aa_cry::read(path)?;
    let dds = Dds::parse(&bytes)?;
    if dds.faces != 1 || dds.depth != 1 {
        return Err(Error::Format(format!("face normal must be a 2D texture: {path}")));
    }
    let weight = strength(weight);
    let mips: Vec<Vec<u8>> = (0..dds.mips)
        .map(|mip| {
            let mut pixels = dds.decode_rgba8(0, mip);
            weight_pixels(&mut pixels, weight, dds.format == DdsFormat::Bc5);
            pixels
        })
        .collect();
    Ok(rgba8_image(dds.width, dds.height, &mips, false))
}

fn weight_pixels(pixels: &mut [u8], weight: f32, two_component: bool) {
    for pixel in pixels.chunks_exact_mut(4) {
        let x = pixel[0] as f32 / 127.5 - 1.0;
        let y = pixel[1] as f32 / 127.5 - 1.0;
        // ATI2 stores only XY (aa-cry already swaps its YX block order). Reconstruct Z before
        // blending; its decoder's blue byte is zero, not a backwards-pointing normal.
        let z = if two_component {
            (1.0 - x * x - y * y).max(0.0).sqrt()
        } else {
            pixel[2] as f32 / 127.5 - 1.0
        };
        let normal = Vec3::new(x, y, z).try_normalize().unwrap_or(Vec3::Z);
        let blended = Vec3::Z.lerp(normal, weight).try_normalize().unwrap_or(Vec3::Z);
        for (channel, value) in pixel[..3].iter_mut().zip(blended.to_array()) {
            *channel = ((value * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(pixel: &[u8]) -> Vec3 {
        Vec3::new(pixel[0] as f32, pixel[1] as f32, pixel[2] as f32) / 127.5 - Vec3::ONE
    }

    #[test]
    fn strength_blends_directions_and_keeps_alpha() {
        let original = [230, 128, 204, 91]; // approximately (0.8, 0, 0.6)
        let mut zero = original;
        weight_pixels(&mut zero, 0.0, false);
        assert_eq!(zero, [128, 128, 255, 91]);

        let mut half = original;
        weight_pixels(&mut half, 0.5, false);
        // Half of the original ~53 degree tilt is ~26.6 degrees, rather than scaling the
        // encoded RGB bytes (which would shorten / skew the normal).
        assert!((normal(&half).x - 0.448).abs() < 0.01);
        assert!((normal(&half).z - 0.894).abs() < 0.01);
        assert_eq!(half[3], 91);

        let mut full = original;
        weight_pixels(&mut full, 1.0, false);
        assert!((normal(&full).normalize() - normal(&original).normalize()).length() < 0.01);
    }

    #[test]
    fn ati2_reconstructs_forward_z_and_handles_outside_unit_disc() {
        let mut pixels = [230, 128, 0, 255, 255, 255, 0, 255];
        weight_pixels(&mut pixels, 0.5, true);
        assert!(normal(&pixels[..4]).z > 0.8, "ATI2 blue is reconstructed");
        for p in pixels.chunks_exact(4) {
            assert!((normal(p).length() - 1.0).abs() < 0.01);
            assert!(normal(p).z > 0.0);
        }
    }

    #[test]
    fn malformed_strength_stays_in_slider_range() {
        assert_eq!(strength(-2.0), 0.0);
        assert_eq!(strength(3.0), 1.0);
        assert_eq!(strength(f32::NAN), 1.0);
        assert_eq!(strength(f32::INFINITY), 1.0);
    }

    #[test]
    fn real_face_normal_keeps_linear_mips() {
        if !aa_cry::pak_available() {
            return;
        }
        let path = "objects/characters/nuian/male/face/normal_custom/nu_m_face_normal001_ddn.dds";
        let image = bake(path, 0.62).unwrap();
        let bytes = aa_cry::read(path).unwrap();
        let source = Dds::parse(&bytes).unwrap();
        assert_eq!(image.texture_descriptor.format, bevy::render::render_resource::TextureFormat::Rgba8Unorm);
        assert_eq!(image.texture_descriptor.mip_level_count, source.mips);
        assert_eq!(
            image.data.as_ref().unwrap().len(),
            (0..source.mips)
                .map(|m| (source.width >> m).max(1) as usize * (source.height >> m).max(1) as usize * 4)
                .sum::<usize>()
        );
        assert!(image.data.as_ref().unwrap().chunks_exact(4).all(|p| normal(p).z > 0.0));
    }
}
