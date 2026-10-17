//! The particle material: unlit, textured, vertex coloured, blended, no depth write (the transparent phase of Bevy).

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;

use crate::params::Blend;

/// Uniform of `fx.wesl`.
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct FxParams {
    /// x: colour gain, y: soft particle distance in metres (0 = off), z: blend mode (0 alpha, 1 premultiplied, 2 additive,
    /// 3 multiply, 4 opaque), w: alpha test threshold (0 = none).
    pub gain: Vec4,
    /// Weights of the opacity map's channels (all 0 = no opacity map).
    pub mask: Vec4,
    /// x: refraction strength (fraction of the viewport height a unit normal shifts the scene), y: 1 for a refraction material,
    /// z: 1 when the first component of the second texture coordinates is a per-vertex alpha test threshold (dissolving sprites).
    pub refr: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FxMaterial {
    #[uniform(0)]
    pub params: FxParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
    /// The separate opacity map of a mesh material (sampled with the mesh's second texture coordinates).
    #[texture(3)]
    pub mask: Option<Handle<Image>>,
    /// The normal map of a refraction material: it shifts the scene behind the particle instead of showing the texture.
    #[texture(4)]
    pub normal: Option<Handle<Image>>,
    /// Reads the scene drawn so far (`ScreenSpaceTransmission` on the camera).
    pub refract: bool,
    pub blend: Blend,
    /// Sort bias of `DrawLast`: larger draws later among equally distant emitters.
    pub depth_bias: f32,
}

impl FxParams {
    pub fn new(gain: f32, soft_distance: f32, blend: Blend) -> FxParams {
        let mode = match blend {
            Blend::ColorBased => 1.0,
            Blend::Additive => 2.0,
            Blend::Multiplicative => 3.0,
            Blend::Opaque => 4.0,
            _ => 0.0,
        };
        FxParams { gain: Vec4::new(gain, soft_distance, mode, 0.0), mask: Vec4::ZERO, refr: Vec4::ZERO }
    }
}

impl Material for FxMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_fx/fx.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            Blend::Additive => AlphaMode::Add,
            Blend::ColorBased => AlphaMode::Premultiplied,
            Blend::Multiplicative => AlphaMode::Multiply,
            Blend::Opaque => AlphaMode::Opaque,
            Blend::Alpha | Blend::None => AlphaMode::Blend,
        }
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
    }

    fn reads_view_transmission_texture(&self) -> bool {
        self.refract
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
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}
