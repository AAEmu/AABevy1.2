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
    /// x: colour gain, y: soft particle distance in metres (0 = off), z: blend mode (0 alpha, 1 premultiplied, 2 additive, 3 multiply).
    pub gain: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FxMaterial {
    #[uniform(0)]
    pub params: FxParams,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Option<Handle<Image>>,
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
            _ => 0.0,
        };
        FxParams { gain: Vec4::new(gain, soft_distance, mode, 0.0) }
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
            Blend::Alpha | Blend::Opaque | Blend::None => AlphaMode::Blend,
        }
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
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
