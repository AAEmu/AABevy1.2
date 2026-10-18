//! Native CVegetation per-instance COLOR_VARIATION (300F1740 / vegetation.cfx).
//! The original B/G/R bytes are six-bit packed in MeshTag; all instances share their material.

use aa_cry::{CryAssets, material::Material as CryMaterial, scene::LoadedModel};
use bevy::{
    asset::{AssetId, embedded_asset},
    ecs::system::SystemParamItem,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{
        ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
        MaterialPlugin,
    },
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, AsBindGroupError, BindGroupBuilder, BindGroupLayout, BindGroupLayoutEntry,
            BindlessDescriptor, BindlessSlabResourceLimit, RenderPipelineDescriptor,
            SpecializedMeshPipelineError,
        },
        renderer::RenderDevice,
    },
    shader::ShaderRef,
};
use std::{borrow::Cow, collections::HashMap};

pub type VegetationMaterial = ExtendedMaterial<StandardMaterial, VegetationColor>;
pub(crate) type Mirrors = HashMap<AssetId<StandardMaterial>, AssetId<VegetationMaterial>>;
pub(crate) type Materials = HashMap<AssetId<StandardMaterial>, Handle<VegetationMaterial>>;

/// A material-qualified vegetation part, for scene diagnostics and cell-lifetime checks.
#[derive(Component, Clone, Copy, Debug)]
pub struct VegetationColorPart {
    pub cell: (i32, i32),
    /// Serialized bytes 61/62/63, in B/G/R order. Byte 59 is not used by the native loader.
    pub bgr: [u8; 3],
}

/// Native truncation and 1..63 clamp, B high / G middle / R low (not byte59 brightness).
pub fn pack(bgr: [u8; 3]) -> u32 {
    let q = bgr.map(|v| ((v as f32 * (1.0 / 255.0) * 63.0) as u32).clamp(1, 63));
    q[0] * 4096 + q[1] * 64 + q[2]
}

fn enabled(material: &CryMaterial) -> bool {
    material.shader.eq_ignore_ascii_case("Vegetation") && material.gen_mask & 0x40000 != 0
}

/// Called once when a worker-prepared model arrives. CryAssets uses the same Basic/domain=true
/// cache key for these non-UCC materials, so only matching original sub-material handles qualify.
/// The mirror ids are weak; model/part handles own their actual asset lifetime, like the base.
pub(crate) fn prepare(
    cry: &mut CryAssets,
    loaded: &LoadedModel,
    mirrors: &mut Mirrors,
    materials: &mut Assets<VegetationMaterial>,
) -> Materials {
    let mut result = Materials::new();
    let Some(path) = loaded.material_path.as_deref() else {
        return result;
    };
    let Some(mtl) = cry.mtl(path) else {
        return result;
    };
    // Follow the exact subset ids of drawn nodes, including the loader's root fallback
    // for non-multi materials; source sub-material order need not equal Part order.
    let mut subsets = Vec::new();
    for (node, _) in &loaded.parts {
        if let Some(mesh) = loaded.model.nodes[*node].mesh {
            subsets.extend(
                loaded.model.meshes[mesh]
                    .subsets
                    .iter()
                    .map(|s| s.material_id as usize),
            );
        }
    }
    subsets.sort_unstable();
    subsets.dedup();
    for i in subsets {
        if !enabled(mtl.sub(i).unwrap_or(mtl.as_ref())) {
            continue;
        }
        // NoDraw/proxy filtering remains the original loader's responsibility.
        let Some(base) = cry.material(Some(path), i) else {
            continue;
        };
        if !loaded
            .parts
            .iter()
            .flat_map(|(_, p)| p)
            .any(|p| p.material == base)
        {
            continue;
        }
        let handle = mirrors
            .get(&base.id())
            .and_then(|id| materials.get_strong_handle(*id))
            .or_else(|| {
                cry.materials.get(&base).map(|m| {
                    materials.add(VegetationMaterial {
                        base: m.clone(),
                        extension: VegetationColor,
                    })
                })
            });
        if let Some(handle) = handle {
            mirrors.insert(base.id(), handle.id());
            result.insert(base.id(), handle);
        }
    }
    result
}

/// No extra uniforms/textures: the native constant varies by instance, not by material.
/// An empty bindless descriptor keeps StandardMaterial's batching without adding an empty
/// index-table binding (which the derive macro would otherwise generate).
#[derive(Asset, Reflect, Debug, Clone, Default)]
pub struct VegetationColor;
impl AsBindGroup for VegetationColor {
    type Data = ();
    type Param = ();
    fn label() -> &'static str {
        "vegetation instance colour"
    }
    fn bind_group_data(&self) {}
    fn bindless_slot_count() -> Option<BindlessSlabResourceLimit> {
        Some(BindlessSlabResourceLimit::Auto)
    }
    fn bindless_descriptor() -> Option<BindlessDescriptor> {
        Some(BindlessDescriptor {
            resources: Cow::Borrowed(&[]),
            buffers: Cow::Borrowed(&[]),
            index_tables: Cow::Borrowed(&[]),
        })
    }
    fn build_bind_group(
        &self,
        _: &BindGroupLayout,
        _: &RenderDevice,
        _: &mut SystemParamItem<'_, '_, Self::Param>,
        _: bool,
        _: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        Ok(())
    }
    fn bind_group_layout_entries(_: &RenderDevice, _: bool) -> Vec<BindGroupLayoutEntry> {
        Vec::new()
    }
}
impl MaterialExtension for VegetationColor {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/vegetation_color.wesl".into()
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor
            .vertex
            .shader_defs
            .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment
                .shader_defs
                .push("VERTEX_OUTPUT_INSTANCE_INDEX".into());
        }
        Ok(())
    }
}

pub struct VegetationColorPlugin;
impl Plugin for VegetationColorPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "vegetation_color.wesl");
        app.add_plugins(MaterialPlugin::<VegetationMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_quantization_channels_and_neutral_are_not_rounded() {
        assert_eq!(pack([0, 0, 0]), 4161);
        assert_eq!(pack([255, 255, 255]), 262143);
        assert_eq!(pack([127, 127, 127]), 128991);
        assert_eq!(pack([0, 128, 255]), 1 * 4096 + 31 * 64 + 63);
        for v in 0..=255u8 {
            let q = ((f64::from(v) * 63.0 / 255.0).floor() as u32).max(1);
            assert_eq!(pack([v, 0, 0]) >> 12, q);
        }
    }
    #[test]
    fn exact_material_generation_flag_not_grass_or_terrain_hue() {
        let mut m = CryMaterial {
            shader: "Vegetation".into(),
            gen_mask: 0x2000,
            ..default()
        };
        assert!(!enabled(&m));
        m.gen_mask |= 0x40000;
        assert!(enabled(&m));
        m.shader = "Illum".into();
        assert!(!enabled(&m));
        let d = VegetationColor::bindless_descriptor().unwrap();
        assert!(d.resources.is_empty() && d.buffers.is_empty() && d.index_tables.is_empty());
        assert!(VegetationMaterial::bindless_slot_count().is_some());
    }
    #[test]
    fn original_forest_variation_uses_the_authored_shader_flag() {
        if !crate::pak_available() {
            return;
        }
        let groups = crate::vegetation::VegGroups::load("main_world").unwrap();
        assert_eq!(groups.groups[&799].material_layers, 0);
        assert_eq!(
            groups
                .groups
                .values()
                .filter(|g| g.material_layers != 0)
                .count(),
            20
        );
        assert_eq!(
            groups.groups[&799].model,
            "objects/env/05_nature/00_common/waterweed_b.cgf"
        );
        let source = CryMaterial::load("objects/env/05_nature/00_common/waterweed_b.mtl").unwrap();
        assert_eq!(source.sub(0).unwrap().gen_mask, 0x80052000);
        assert!(enabled(source.sub(0).unwrap()));
        assert!(!enabled(source.sub(1).unwrap()));
        let unrelated =
            CryMaterial::load("objects/env/05_nature/00_common/common_plants_shine_fa04.mtl")
                .unwrap();
        assert!(!unrelated.sub_materials.iter().any(enabled));
        let bytes = aa_pak::Pak::global()
            .read("worlds/main_world/cells/012_014/client/vegetation.dat")
            .unwrap();
        let plants = crate::placement::parse_vegetation_dat(&bytes).unwrap();
        assert_eq!(plants.len(), 28368);
        assert_eq!(plants.iter().filter(|p| p.group == 799).count(), 7260);
        assert_eq!(
            plants
                .iter()
                .filter(|p| p.group == 799 && p.color_variation != [127; 3])
                .count(),
            6847
        );
        let target = plants
            .iter()
            .find(|p| {
                (p.pos.x - 524.47).abs() < 0.01 && (p.pos.y - 239.70).abs() < 0.01 && p.group == 799
            })
            .unwrap();
        assert_eq!(target.color_variation, [69, 208, 233]);
    }
}
