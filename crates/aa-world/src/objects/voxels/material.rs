//! Original TerrainPS cover decoding and interpolation with the packed voxel colour.

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MaterialPlugin,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

pub type VoxelMaterial = ExtendedMaterial<StandardMaterial, VoxelExtension>;

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct VoxelParams {
    /// cover present, projected-layer pass, original detail strength, native fade distance.
    pub flags: Vec4,
    pub u: Vec4,
    pub v: Vec4,
    pub origin: Vec4,
    pub colour: Vec4,
    /// base encoding (0/BC1/ATI2), detail present, authored detail strength, diagnostic output.
    pub normals: Vec4,
    pub bump_u: Vec4,
    pub bump_v: Vec4,
    pub detail_u: Vec4,
    pub detail_v: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(VoxelKey)]
pub struct VoxelExtension {
    #[uniform(100)]
    pub params: VoxelParams,
    #[texture(101)]
    #[sampler(102)]
    pub cover: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub diffuse: Option<Handle<Image>>,
    #[texture(105)]
    #[sampler(106)]
    pub normal: Option<Handle<Image>>,
    #[texture(107)]
    #[sampler(108)]
    pub detail_normal: Option<Handle<Image>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelKey(bool);
impl From<&VoxelExtension> for VoxelKey {
    fn from(value: &VoxelExtension) -> Self {
        Self(value.params.flags.y > 0.5)
    }
}

impl VoxelExtension {
    pub fn new(cover: Option<Handle<Image>>) -> Self {
        Self {
            params: VoxelParams {
                flags: Vec4::new(if cover.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
                u: Vec4::ZERO,
                v: Vec4::ZERO,
                origin: Vec4::ZERO,
                colour: Vec4::ONE,
                normals: Vec4::ZERO,
                bump_u: Vec4::ZERO,
                bump_v: Vec4::ZERO,
                detail_u: Vec4::ZERO,
                detail_v: Vec4::ZERO,
            },
            cover,
            diffuse: None,
            normal: None,
            detail_normal: None,
        }
    }
}

impl MaterialExtension for VoxelExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/objects/voxels/vertex.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/objects/voxels/material.wesl".into()
    }
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.0
            && let Some(depth) = descriptor.depth_stencil.as_mut()
        {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::Equal);
            // Native layer order is CPU transparency order; it must not shift receiver depth.
            depth.bias.constant = 0;
        }
        Ok(())
    }
}

/// Registers the native compiled-voxel material, also usable by isolated numeric fixtures.
pub struct VoxelMaterialPlugin;

impl Plugin for VoxelMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "material.wesl");
        embedded_asset!(app, "vertex.wesl");
        bevy::shader::load_shader_library!(app, "io.wesl");
        app.add_plugins(MaterialPlugin::<VoxelMaterial>::default());
    }
}
