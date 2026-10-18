//! Original TerrainPS cover decoding and interpolation with the packed voxel colour.

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub type VoxelMaterial = ExtendedMaterial<StandardMaterial, VoxelExtension>;

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct VoxelParams {
    /// x: original cover tile present; other lanes reserved/zero.
    pub flags: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct VoxelExtension {
    #[uniform(100)]
    pub params: VoxelParams,
    #[texture(101)]
    #[sampler(102)]
    pub cover: Option<Handle<Image>>,
}

impl VoxelExtension {
    pub fn new(cover: Option<Handle<Image>>) -> Self {
        Self {
            params: VoxelParams {
                flags: Vec4::new(if cover.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0),
            },
            cover,
        }
    }
}

impl MaterialExtension for VoxelExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/objects/voxels/material.wesl".into()
    }
}

/// Registers the native compiled-voxel material, also usable by isolated numeric fixtures.
pub struct VoxelMaterialPlugin;

impl Plugin for VoxelMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "material.wesl");
        app.add_plugins(MaterialPlugin::<VoxelMaterial>::default());
    }
}
