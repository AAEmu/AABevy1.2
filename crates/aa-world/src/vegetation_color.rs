//! Native vegetation final-light group brightness, vertex AO and per-instance COLOR_VARIATION.
//! The original B/G/R bytes are six-bit packed in MeshTag; all instances share their material.

use aa_cry::{CryAssets, material::Material as CryMaterial, scene::LoadedModel};
use bevy::{
    asset::{AssetId, embedded_asset},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{
        ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
        MaterialPlugin,
    },
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use std::collections::HashMap;

pub(crate) mod ao;
pub use ao::VegetationAoPart;

/// Instance flag: AO-only submaterials must not receive COLOR_VARIATION.
pub const NO_COLOR_VARIATION: u32 = 1 << 18;
/// Diagnostic instance control; the world streamer never sets this bit.
pub const IGNORE_VERTEX_AO: u32 = 1 << 19;

pub type VegetationMaterial = ExtendedMaterial<StandardMaterial, VegetationColor>;
pub(crate) type Mirrors = HashMap<(AssetId<StandardMaterial>, u32), AssetId<VegetationMaterial>>;
pub(crate) struct Mirror {
    pub material: Handle<VegetationMaterial>,
    pub variation: bool,
    pub brightness: f32,
}
pub(crate) type Materials = HashMap<(AssetId<StandardMaterial>, u32), Mirror>;

/// A nonunity authored scalar part, for scene diagnostics and cell-lifetime checks.
#[derive(Component, Clone, Copy, Debug)]
pub struct VegetationBrightnessPart {
    pub cell: (i32, i32),
    pub group: u16,
    pub brightness: f32,
}

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
    ao: &ao::Meshes,
    brightnesses: &[f32],
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
        let source = mtl.sub(i).unwrap_or(mtl.as_ref());
        let variation = enabled(source);
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
        let has_ao = loaded
            .parts
            .iter()
            .flat_map(|(_, p)| p)
            .any(|p| p.material == base && ao.contains_key(&p.mesh.id()));
        // Alpha-glow is added after AO in the native shader; that distinct pass is unsupported.
        let brightness_supported =
            source.shader.eq_ignore_ascii_case("Vegetation") && source.gen_mask & 0x4000_0000 == 0;
        for &brightness in brightnesses {
            if !variation && !has_ao && !(brightness_supported && brightness != 1.0) {
                continue;
            }
            let value = if brightness_supported {
                brightness
            } else {
                1.0
            };
            let handle = cry
                .materials
                .get(&base)
                .map(|m| variant(base.id(), m, value, mirrors, materials));
            if let Some(handle) = handle {
                result.insert(
                    (base.id(), brightness.to_bits()),
                    Mirror {
                        material: handle,
                        variation,
                        brightness: value,
                    },
                );
            }
        }
    }
    result
}

fn variant(
    base_id: AssetId<StandardMaterial>,
    base: &StandardMaterial,
    value: f32,
    mirrors: &mut Mirrors,
    materials: &mut Assets<VegetationMaterial>,
) -> Handle<VegetationMaterial> {
    let key = (base_id, value.to_bits());
    if let Some(handle) = mirrors
        .get(&key)
        .and_then(|id| materials.get_strong_handle(*id))
    {
        return handle;
    }
    let handle = materials.add(VegetationMaterial {
        base: base.clone(),
        extension: VegetationColor { brightness: value },
    });
    mirrors.insert(key, handle.id());
    handle
}

/// One shared exact group scalar; geometry and per-instance colour tags stay unchanged.
#[derive(Asset, Reflect, Debug, Clone, AsBindGroup)]
#[data(50, VegetationUniform, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct VegetationColor {
    pub brightness: f32,
}
impl Default for VegetationColor {
    fn default() -> Self {
        Self { brightness: 1.0 }
    }
}
#[derive(Clone, ShaderType)]
pub struct VegetationUniform {
    pub brightness: Vec4,
}
impl From<&VegetationColor> for VegetationUniform {
    fn from(v: &VegetationColor) -> Self {
        Self {
            brightness: Vec4::splat(v.brightness),
        }
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
        assert_eq!(d.index_tables.len(), 1);
        assert_eq!(VegetationColor::default().brightness, 1.0);
        assert!(VegetationMaterial::bindless_slot_count().is_some());
    }
    #[test]
    fn scalar_variants_share_exact_values_without_mutating_base_or_unity() {
        let mut bases: Assets<StandardMaterial> = default();
        let base = StandardMaterial {
            base_color: Color::linear_rgba(0.2, 0.4, 0.6, 0.3),
            ..default()
        };
        let id = bases.add(base.clone()).id();
        let mut materials = Assets::default();
        let mut mirrors = Mirrors::new();
        let dark = variant(id, &base, 0.4, &mut mirrors, &mut materials);
        let same = variant(id, &base, 0.4, &mut mirrors, &mut materials);
        let unity = variant(id, &base, 1.0, &mut mirrors, &mut materials);
        let adjacent = variant(
            id,
            &base,
            f32::from_bits(0.4f32.to_bits() + 1),
            &mut mirrors,
            &mut materials,
        );
        assert_eq!(dark, same);
        assert_ne!(dark, unity);
        assert_ne!(dark, adjacent); // exact f32 cache key, no scalar quantization.
        assert_eq!(materials.len(), 3);
        for h in [&dark, &unity, &adjacent] {
            assert_eq!(materials.get(h).unwrap().base.base_color, base.base_color);
        }
        assert_eq!(materials.get(&dark).unwrap().extension.brightness, 0.4);
        assert_eq!(materials.get(&unity).unwrap().extension.brightness, 1.0);
        assert_eq!(base.base_color.alpha(), 0.3); // no albedo, opacity or geometry bake.
        materials.remove(dark.id());
        let replacement = variant(id, &base, 0.4, &mut mirrors, &mut materials);
        assert_ne!(replacement, dark); // stale weak cache id does not retain a material.
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
