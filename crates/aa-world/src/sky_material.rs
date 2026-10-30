//! Sky background, late atmosphere composition (`sky.wesl`) and the zone's sky box.
//! `SkyMaterial` retains the environment inputs; [`composite`] draws them after the celestial layers.
//!
//! A zone's `env.xml` names a `SkyBoxMaterial` (`Materials/sky/sky_solzreed_cbt1`): a `Sky` shader material whose
//! diffuse map is `box_12.dds`; its siblings `box_34.dds` and `box_5.dds` complete a five sided box. Layout (found
//! by matching the face edges): the side faces are the upper halves of cube faces, 1024 x 512 each, going
//! clockwise north, east, south, west; `box_12` stacks north (top half) and east (bottom half, flipped vertically),
//! `box_34` south and west; `box_5` is the top face, north at its bottom edge. The shader blends them over the
//! procedural sky the way `sky.cfx` does (`a = tex.a * Opacity * tex.r ^ AlphaSaturation`, colour from the cloud
//! shading colours).

use bevy::asset::embedded_asset;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use bevy::mesh::MeshVertexBufferLayoutRef;

#[path = "sky_composite.rs"]
pub mod composite;

/// Uniform of `sky.wesl`.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct SkyParams {
    pub horizon: Vec4,
    pub zenith: Vec4,
    pub sun_dir: Vec4,
    pub sun_color: Vec4,
    pub box_sky: Vec4,
    pub box_sun: Vec4,
    pub box_params: Vec4,
    /// x: e-fold height of the gradient in degrees of elevation (0 = `horizon.w` as an exponent of the blend), y: how much
    /// brighter the sky is towards the sun than away from it (Rayleigh phase, 0 = not at all), z: weight of the engine's
    /// night sky gradient (1 with the sun below the horizon), w: its `Night sky: Zenith shift`
    pub shape: Vec4,
}

impl Default for SkyParams {
    fn default() -> Self {
        SkyParams {
            horizon: Vec4::new(0.5, 0.55, 0.6, 0.6),
            zenith: Vec4::new(0.2, 0.3, 0.5, 0.0),
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            sun_color: Vec4::ZERO,
            box_sky: Vec4::ZERO,
            box_sun: Vec4::ZERO,
            box_params: Vec4::new(1.0, 1.0, 0.0, 0.0),
            shape: Vec4::ZERO,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkyMaterial {
    #[uniform(0)]
    pub params: SkyParams,
    #[texture(1)]
    #[sampler(2)]
    pub box_12: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub box_34: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub box_5: Option<Handle<Image>>,
}

impl Material for SkyMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/sky_background.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/sky_background.wesl".into()
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // seen from inside
        descriptor.primitive.cull_mode = None;
        // A sky background must not fill depth: the catalogue stars are drawn at the far plane.
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(bevy::render::render_resource::CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

pub(crate) fn build(app: &mut App) {
    embedded_asset!(app, "sky.wesl");
    embedded_asset!(app, "sky_background.wesl");
    app.add_plugins(MaterialPlugin::<SkyMaterial>::default());
    composite::build(app);
}

/// Sky background/composition materials, also usable by bounded renderer fixtures.
pub struct SkyMaterialPlugin;
impl Plugin for SkyMaterialPlugin {
    fn build(&self, app: &mut App) { build(app); }
}

/// A zone's sky box: the three textures and the `Sky` material parameters.
#[derive(Clone, Debug)]
pub struct SkyBox {
    pub box_12: Handle<Image>,
    pub box_34: Handle<Image>,
    pub box_5: Handle<Image>,
    /// Material `Opacity`.
    pub opacity: f32,
    pub attenuation: f32,
    pub alpha_saturation: f32,
    pub sky_multiplier: f32,
    pub sun_multiplier: f32,
}

/// What a zone's `env.xml` says about the sky.
#[derive(Clone, Debug, Default)]
pub struct ZoneSky {
    /// `SkyBoxMaterial` path, when it has textures.
    pub material: Option<String>,
    /// `Angle`: rotation of the box about the vertical (degrees).
    pub angle: f32,
    /// `<Ocean FogColor FogDensity>` of the zone (colour 0..255).
    pub ocean_fog: Option<([f32; 3], f32)>,
}

impl ZoneSky {
    /// `variant` is the element name to read (`Zone` is the normal one, `Zone_Dominion_Normal` another).
    pub fn parse(xml: &str, variant: &str) -> ZoneSky {
        let Ok(doc) = roxmltree::Document::parse(xml) else { return ZoneSky::default() };
        // 365F3830 loads a standalone Zone into both native layers; a list loads
        // exact element names and leaves an absent layer at its initialized defaults.
        let root = doc.root_element();
        let Some(zone) = (if root.has_tag_name("Zone") { Some(root) } else { root.children().find(|n| n.has_tag_name(variant)) })
        else {
            return ZoneSky::default();
        };
        let material = zone.attribute("SkyBoxMaterial").filter(|m| !m.trim().is_empty()).map(|m| m.trim().to_string());
        let angle = zone.attribute("Angle").and_then(|a| a.parse().ok()).unwrap_or(0.0);
        let ocean_fog = zone.children().find(|n| n.has_tag_name("Ocean")).and_then(|o| {
            let c: Vec<f32> = o.attribute("FogColor")?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            (c.len() == 3).then(|| ([c[0], c[1], c[2]], o.attribute("FogDensity").and_then(|d| d.parse().ok()).unwrap_or(0.05)))
        });
        ZoneSky { material, angle, ocean_fog }
    }
}

/// Loads the sky box of a `Sky` material (`None` for materials without a box texture, like the procedural default).
pub fn load_sky_box(material: &str, images: &mut Assets<Image>) -> Option<SkyBox> {
    use aa_cry::material::{Material as CryMaterial, TextureMap};
    let mtl = CryMaterial::load(&format!("{}.mtl", material.trim_end_matches(".mtl"))).ok()?;
    if !mtl.shader.eq_ignore_ascii_case("sky") {
        return None;
    }
    let diffuse = mtl.texture(&TextureMap::Diffuse)?.pak_path();
    let stem = diffuse.strip_suffix("box_12.dds")?;
    let mut load = |name: &str| -> Option<Handle<Image>> {
        let opts = aa_cry::texture::DdsOptions { srgb: true, bc_supported: true, repeat: false };
        let mut image = aa_cry::texture::load_dds_from_pak(&format!("{stem}{name}.dds"), opts).ok()?;
        image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
            address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
            address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
            mag_filter: bevy::image::ImageFilterMode::Linear,
            min_filter: bevy::image::ImageFilterMode::Linear,
            mipmap_filter: bevy::image::ImageFilterMode::Linear,
            ..default()
        });
        Some(images.add(image))
    };
    let param = |name: &str, default: f32| mtl.public_param(name).and_then(|v| v.parse().ok()).unwrap_or(default);
    Some(SkyBox {
        box_12: load("box_12")?,
        box_34: load("box_34")?,
        box_5: load("box_5")?,
        opacity: mtl.opacity,
        attenuation: param("Attenuation", 0.6),
        alpha_saturation: param("AlphaSaturation", 2.0),
        sky_multiplier: param("SkyColorMultiplier", 1.5),
        sun_multiplier: param("SunColorMultiplier", 4.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_sky_parse() {
        let xml = r#"<Env><Zone SkyBoxMaterial="Materials/sky/x" Angle="130"><Ocean FogColor="21,74,155" FogDensity="0.025"/></Zone>
            <Zone_Dominion_Normal SkyBoxMaterial="Materials/sky/y"/></Env>"#;
        let z = ZoneSky::parse(xml, "Zone");
        assert_eq!(z.material.as_deref(), Some("Materials/sky/x"));
        assert_eq!(z.angle, 130.0);
        assert_eq!(z.ocean_fog, Some(([21.0, 74.0, 155.0], 0.025)));
        assert_eq!(ZoneSky::parse(xml, "Zone_Dominion_Normal").material.as_deref(), Some("Materials/sky/y"));
        assert!(ZoneSky::parse(r#"<Env><Zone SkyBoxMaterial="x"/></Env>"#, "Zone_Dominion_Normal").material.is_none());
        assert_eq!(ZoneSky::parse(r#"<Zone SkyBoxMaterial="x"/>"#, "Zone_Dominion_Normal").material.as_deref(), Some("x"));
        assert!(ZoneSky::parse("not xml", "Zone").material.is_none());
    }

    #[test]
    fn solzreed_sky_box() {
        if !crate::pak_available() {
            return;
        }
        let mut images = Assets::<Image>::default();
        let b = load_sky_box("Materials/sky/sky_solzreed_cbt1", &mut images).expect("sky box");
        assert!((b.opacity - 0.5).abs() < 1e-4 && (b.alpha_saturation - 1.2).abs() < 1e-4);
        assert!(images.get(&b.box_12).is_some() && images.get(&b.box_5).is_some());
        // a material that is not a sky box
        assert!(load_sky_box("game/materials/ocean/lake_tank", &mut images).is_none());
    }
}
