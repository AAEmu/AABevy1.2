//! Bounded authored sampler overrides for the existing moving Illum opacity input.
//! Native producer/descriptor evidence and the inherited-filter limit are in docs/aa-cry.md.
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSamplerDescriptor};

use crate::material::Texture;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Filter {
    Existing,
    Point,
    Linear,
    Bilinear,
    Trilinear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OpacitySampler {
    tile_u: bool,
    tile_v: bool,
    filter: Filter,
}

impl OpacitySampler {
    /// `None` keeps the original image/cache path, including unsupported authoring.
    /// Native accepts more XML spellings; this slice only interprets strict integers.
    pub(crate) fn from_texture(texture: &Texture) -> Option<Self> {
        let mut sampler = Self {
            tile_u: true,
            tile_v: true,
            filter: Filter::Existing,
        };
        let mut seen = 0u8;
        for (name, text) in &texture.attributes {
            let bit = match name.to_ascii_lowercase().as_str() {
                "istileu" => 1,
                "istilev" => 2,
                "filter" => 4,
                "textype" => 8,
                _ => return None,
            };
            if seen & bit != 0 {
                return None;
            }
            seen |= bit;
            let value: i32 = text.trim().parse().ok()?;
            match name.to_ascii_lowercase().as_str() {
                "istileu" if matches!(value, 0 | 1) => sampler.tile_u = value != 0,
                "istilev" if matches!(value, 0 | 1) => sampler.tile_v = value != 0,
                "filter" => {
                    sampler.filter = match value {
                        // Native -1 copies the configured global filter. Keep our existing
                        // filtering here; there is no authoritative native live profile bridge.
                        -1 => Filter::Existing,
                        0 => Filter::Point,
                        1 => Filter::Linear,
                        2 => Filter::Bilinear,
                        3 => Filter::Trilinear,
                        _ => return None,
                    }
                }
                // For ordinary named static textures these use the same native loader;
                // the DDS must still prove flat. Auto/cube/volume sources are outside this slice.
                "textype" if matches!(value, 0 | 1) => {}
                _ => return None,
            }
        }
        (!sampler.tile_u || !sampler.tile_v || sampler.filter != Filter::Existing)
            .then_some(sampler)
    }

    pub(crate) fn descriptor(self) -> ImageSamplerDescriptor {
        let address = |tile| {
            if tile {
                ImageAddressMode::Repeat
            } else {
                ImageAddressMode::ClampToEdge
            }
        };
        let mut descriptor = super::repeat_sampler();
        descriptor.address_mode_u = address(self.tile_u);
        descriptor.address_mode_v = address(self.tile_v);
        descriptor.address_mode_w = address(self.tile_u);
        if self.filter != Filter::Existing {
            descriptor.anisotropy_clamp = 1;
            descriptor.mag_filter = if self.filter == Filter::Point {
                ImageFilterMode::Nearest
            } else {
                ImageFilterMode::Linear
            };
            descriptor.min_filter = descriptor.mag_filter;
            descriptor.mipmap_filter = if self.filter == Filter::Trilinear {
                ImageFilterMode::Linear
            } else {
                ImageFilterMode::Nearest
            };
            // Native descriptors disable mips for 0/1 (MaxLOD=0), including linear 1.
            descriptor.lod_max_clamp = if matches!(self.filter, Filter::Point | Filter::Linear) {
                0.0
            } else {
                100.0
            };
        }
        descriptor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::Material;

    fn texture(attributes: &str) -> Texture {
        Material::parse(&format!(r#"<Material><Textures><Texture Map="Opacity" File="a.dds" {attributes}/></Textures></Material>"#))
            .unwrap().textures.remove(0)
    }

    #[test]
    fn defaults_and_unsupported_authoring_keep_the_existing_path() {
        for attributes in [
            "",
            r#"Filter="-1" IsTileU="1" IsTileV="1" TexType="0""#,
            r#"Filter="4""#,
            r#"Filter="255""#,
            r#"Filter="true""#,
            r#"Filter="""#,
            r#"IsTileU="2""#,
            r#"IsTileV="false""#,
            r#"TexType="3""#,
            r#"TexType="4" IsTileV="0""#,
            r#"Other="1""#,
        ] {
            assert_eq!(
                OpacitySampler::from_texture(&texture(attributes)),
                None,
                "{attributes}"
            );
        }
    }

    #[test]
    fn native_address_axes_and_inherited_filter_are_independent() {
        for (u, v) in [(0, 0), (0, 1), (1, 0)] {
            let s = OpacitySampler::from_texture(&texture(&format!(
                r#"iStIlEu="{u}" IsTileV="{v}" Filter="-1""#
            )))
            .unwrap();
            let d = s.descriptor();
            let address = |v| {
                if v == 0 {
                    ImageAddressMode::ClampToEdge
                } else {
                    ImageAddressMode::Repeat
                }
            };
            assert_eq!(d.address_mode_u, address(u));
            assert_eq!(d.address_mode_v, address(v));
            assert_eq!(d.address_mode_w, address(u));
            assert_eq!(d.mag_filter, ImageFilterMode::Linear);
            assert_eq!(d.mipmap_filter, ImageFilterMode::Linear);
            assert_eq!(
                d.anisotropy_clamp, 4,
                "preserve existing filtering, not a native-global claim"
            );
            assert_eq!(
                OpacitySampler::from_texture(&texture(&format!(r#"IsTileU="{u}" IsTileV="{v}""#))),
                Some(s)
            );
        }
    }

    #[test]
    fn explicit_native_filters_include_the_no_mip_lod_rule() {
        for code in 0..4 {
            let d = OpacitySampler::from_texture(&texture(&format!(r#"Filter="{code}""#)))
                .unwrap()
                .descriptor();
            assert_eq!(
                d.mag_filter,
                if code == 0 {
                    ImageFilterMode::Nearest
                } else {
                    ImageFilterMode::Linear
                }
            );
            assert_eq!(d.min_filter, d.mag_filter);
            assert_eq!(
                d.mipmap_filter,
                if code == 3 {
                    ImageFilterMode::Linear
                } else {
                    ImageFilterMode::Nearest
                }
            );
            assert_eq!(d.lod_max_clamp, if code < 2 { 0.0 } else { 100.0 });
            assert_eq!(d.lod_min_clamp, 0.0);
            assert_eq!(d.anisotropy_clamp, 1);
        }
    }
}
