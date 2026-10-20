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
pub(crate) type Mirrors = HashMap<(AssetId<StandardMaterial>, u32, [u32;8]), AssetId<VegetationMaterial>>;
pub(crate) struct Mirror {
    pub material: Handle<VegetationMaterial>,
    pub variation: bool,
    pub brightness: f32,
    pub wind: Vec4,
}
pub(crate) type Materials = HashMap<(AssetId<StandardMaterial>, u32, u32), Mirror>;

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
    styles: &[crate::vegetation::Style],
    global_wind: Option<Vec3>,
    epoch: f32,
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
    let half_height=crate::vegetation_wind::model_half_height(loaded);
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
        for &style in styles {
            let brightness=style.brightness;
            // The original DETAIL_BENDING script explicitly enters VTM_BENDING for grass/leaves.
            // Only the main branch is supported here; packed vertex RGB detail remains separate.
            let wind=if style.bending > 0.0 && source.shader.eq_ignore_ascii_case("Vegetation")
                && source.gen_mask & 0x10000 != 0 && source.gen_mask & (0x100|0x2000) != 0
            {
                half_height.zip(global_wind).map(|(h,w)| crate::vegetation_wind::parameters(w,style.bending,h)).unwrap_or(Vec4::ZERO)
            } else { Vec4::ZERO };
            // Detail bending needs the main bending's wind strength, like the native `_VTBending`.
            let detail=if wind.z > 0.0 { crate::vegetation_wind::detail_params(source) } else { Vec4::ZERO };
            if !variation && !has_ao && !(brightness_supported && brightness != 1.0) && wind.z == 0.0 {
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
                .map(|m| wind_variant(base.id(), m, value, wind, detail, source.gen_mask & 0x2000 != 0, epoch, mirrors, materials));
            if let Some(handle) = handle {
                result.insert(
                    (base.id(), brightness.to_bits(), style.bending.to_bits()),
                    Mirror {
                        material: handle,
                        variation,
                        brightness: value,
                        wind,
                    },
                );
            }
        }
    }
    result
}

#[cfg(test)]
fn variant(
    base_id: AssetId<StandardMaterial>,
    base: &StandardMaterial,
    value: f32,
    mirrors: &mut Mirrors,
    materials: &mut Assets<VegetationMaterial>,
) -> Handle<VegetationMaterial> {
    wind_variant(base_id,base,value,Vec4::ZERO,Vec4::ZERO,false,0.0,mirrors,materials)
}

#[allow(clippy::too_many_arguments)]
fn wind_variant(
    base_id: AssetId<StandardMaterial>, base: &StandardMaterial, value: f32, wind: Vec4, detail: Vec4,
    grass: bool, epoch: f32, mirrors: &mut Mirrors, materials: &mut Assets<VegetationMaterial>,
) -> Handle<VegetationMaterial> {
    let [w0, w1, w2, w3] = wind.to_array().map(f32::to_bits);
    let [d0, d1, d2, d3] = detail.to_array().map(f32::to_bits);
    let key = (base_id, value.to_bits(), [w0, w1, w2, w3, d0, d1, d2, d3]);
    if let Some(handle) = mirrors
        .get(&key)
        .and_then(|id| materials.get_strong_handle(*id))
    {
        return handle;
    }
    let handle = materials.add(VegetationMaterial {
        base: base.clone(),
        extension: VegetationColor { brightness: value, wind, detail, grass, clock_epoch: epoch, seconds: None, high_quality: wind.z > 0.0 },
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
    /// Steady native base XY, group bending*0.1, native model vertical half-height.
    pub wind: Vec4,
    /// Detail bending from the material's PublicParams: frequency, leaf amplitude, branch amplitude, phase.
    /// Zero frequency disables it; the mesh must carry the vertex data of `vegetation_color::ao`.
    pub detail: Vec4,
    pub grass: bool,
    /// Unwrapped game-time epoch; shader globals supply the within-period seconds.
    pub clock_epoch: f32,
    /// Diagnostic frozen-time control; ordinary streamed materials use the app's game clock.
    pub seconds: Option<f32>,
    /// Diagnostic known-frame HQ variation. Streamed scenes await the native sliding-origin bridge.
    pub high_quality: bool,
}
impl Default for VegetationColor {
    fn default() -> Self {
        Self { brightness: 1.0, wind: Vec4::ZERO, detail: Vec4::ZERO, grass: false, clock_epoch: 0.0, seconds: None, high_quality: false }
    }
}
#[derive(Clone, ShaderType)]
pub struct VegetationUniform {
    pub brightness: Vec4,
    pub wind: Vec4,
    pub clock: Vec4,
    pub detail: Vec4,
}
impl From<&VegetationColor> for VegetationUniform {
    fn from(v: &VegetationColor) -> Self {
        Self {
            brightness: Vec4::splat(v.brightness),
            wind: v.wind,
            detail: v.detail,
            clock: Vec4::new(v.clock_epoch,u8::from(v.grass) as f32 + 2.0*u8::from(v.high_quality) as f32,v.seconds.unwrap_or(0.0),if v.seconds.is_some() { 1.0 } else { 0.0 }),
        }
    }
}
impl MaterialExtension for VegetationColor {
    fn vertex_shader() -> ShaderRef { "embedded://aa_world/vegetation_vertex.wesl".into() }
    fn prepass_vertex_shader() -> ShaderRef { Self::vertex_shader() }
    fn deferred_vertex_shader() -> ShaderRef { Self::vertex_shader() }
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
        bevy::shader::load_shader_library!(app, "vegetation_bindings.wesl");
        bevy::shader::load_shader_library!(app, "vegetation_wind.wesl");
        embedded_asset!(app, "vegetation_vertex.wesl");
        app.add_plugins(MaterialPlugin::<VegetationMaterial>::default())
            .add_systems(Update, clock_epoch);
    }
}

/// Ordinary frames use the shared shader clock and require no material uploads or instance walks.
/// On wrap/reset only, refresh the small shared model/group variants. Pausing follows Bevy's game clock.
pub(crate) fn epoch(time:&Time)->f32 {
    let periods=time.elapsed().as_nanos()/time.wrap_period().as_nanos();
    (periods as f64*time.wrap_period().as_secs_f64()) as f32
}
fn clock_epoch(time: Res<Time>, mut previous: Local<Option<f32>>, mut materials: ResMut<Assets<VegetationMaterial>>) {
    let epoch=epoch(&time);
    if *previous == Some(epoch) { return; }
    *previous=Some(epoch);
    for (_,material) in materials.iter_mut() { material.extension.clock_epoch=epoch; }
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
    fn game_clock_wrap_pause_and_reset_keep_native_seconds() {
        use std::time::Duration;
        let mut app=App::new();
        let mut time=Time::<()>::default();
        time.set_wrap_period(Duration::from_secs(1));
        time.advance_by(Duration::from_secs_f32(3600.125));
        app.insert_resource(time).init_resource::<Assets<VegetationMaterial>>().add_systems(Update,clock_epoch);
        let h=app.world_mut().resource_mut::<Assets<VegetationMaterial>>().add(VegetationMaterial {base:StandardMaterial::default(),extension:VegetationColor::default()});
        app.update();
        let epoch=app.world().resource::<Assets<VegetationMaterial>>().get(&h).unwrap().extension.clock_epoch;
        assert_eq!(epoch,3600.0);
        assert_eq!(epoch+app.world().resource::<Time>().elapsed_secs_wrapped(),3600.125);
        app.update(); // paused/no advancement does not move the native clock.
        assert_eq!(app.world().resource::<Assets<VegetationMaterial>>().get(&h).unwrap().extension.clock_epoch,epoch);
        let mut reset=Time::<()>::default();reset.set_wrap_period(Duration::from_secs(1));
        reset.advance_by(Duration::from_secs_f32(0.25));app.insert_resource(reset);app.update();
        assert_eq!(app.world().resource::<Assets<VegetationMaterial>>().get(&h).unwrap().extension.clock_epoch,0.0);
    }
    #[test]
    fn wind_clock_follows_real_app_virtual_pause_and_reset() {
        use std::time::Duration;
        let mut app=App::new();
        app.add_plugins(bevy::time::TimePlugin).init_resource::<Assets<VegetationMaterial>>().add_systems(Update,clock_epoch);
        let h=app.world_mut().resource_mut::<Assets<VegetationMaterial>>().add(VegetationMaterial {base:StandardMaterial::default(),extension:VegetationColor::default()});
        {
            let mut game=app.world_mut().resource_mut::<Time<Virtual>>();
            game.set_wrap_period(Duration::from_secs(1));game.advance_by(Duration::from_secs_f32(3.125));game.pause();
        }
        for _ in 0..2 {
            app.update();
            assert_eq!(app.world().resource::<Time>().elapsed_secs(),3.125);
            assert_eq!(app.world().resource::<Assets<VegetationMaterial>>().get(&h).unwrap().extension.clock_epoch,3.0);
        }
        let mut reset=Time::<Virtual>::default();reset.set_wrap_period(Duration::from_secs(1));reset.pause();app.insert_resource(reset);
        app.update();
        assert_eq!(app.world().resource::<Time>().elapsed_secs(),0.0);
        assert_eq!(app.world().resource::<Assets<VegetationMaterial>>().get(&h).unwrap().extension.clock_epoch,0.0);
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
