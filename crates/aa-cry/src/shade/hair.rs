//! The `Hair` shader of the client (`shaders/hwscripts/cryfx/hair.cfx`, `docs/research/hair.md`) as a material.
//!
//! What the engine does per light: a wrapped diffuse `saturate(N.L * DiffuseWrap + 1 - DiffuseWrap)` plus a back
//! rim `(1 - N.V)^3 * RimMultiplier * (1 - L.V)`, and two Kajiya-Kay strand highlights
//! `sqrt(max(1 - (T.H)^2, 0.01))^Pow` along the binormal (the texture's V axis, along the strands) shifted along the
//! normal: `T = B + N * (noise + Shift)`, the noise from the Specular map's alpha times `BumpScale`. The primary lobe
//! has `PrimarySpec{Color,Pow,Shift}`, the secondary `SecondarySpec*`. The ambient adds the same two lobes with the
//! view direction as half vector, times `AmbientSpecMult` and the light received. The highlights are masked by the
//! Specular map's rgb and scaled by the material's `Specular`; the colour is `texture * HairColor * 2 * Diffuse`.
//! There is no environment reflection and no GGX term on the hair path.
//!
//! [`HairMaterial`] (`StandardMaterial` extended by `hair.wesl`) does this: Bevy's PBR lighting runs on a white albedo
//! with every specular term switched off (it gives the light received: sun, fill, ambient, shadows), the shader adds
//! the wrap, the rim, the strand highlights of the directional lights and of the character back lights.
//!
//! The loader marks hair materials of `Shading::Full` models ([`prepare`]): the parameters go to a registry, its
//! index into `StandardMaterial::lightmap_exposure` (like the back light tag, the field is only read with a
//! lightmap). [`HairPlugin`] (added by `CryPlugin` when rendering) swaps those entities to a [`HairMaterial`].
//! Without the plugin the `StandardMaterial` stays: matte hair with a faint highlight.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::light::NotShadowCaster;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::backlight::{BackLightParams, CharBackLight, WORLD_DIFFUSE_GAIN};
use super::{hair_colour, param_f32, param_rgb};
use crate::material::{Material as CryMaterial, TextureMap};

/// A hair material: the `StandardMaterial` of the conversion plus the strand shading.
pub type HairMaterial = ExtendedMaterial<StandardMaterial, HairExt>;

/// `lightmap_exposure` of a hair material is `TAG_BASE + index` into the registry (the back light tags are below 3).
const TAG_BASE: f32 = 4.0;

/// The hair parameters of one `.mtl` sub-material, as the shader reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct HairParams {
    /// rgb: `PrimarySpecColor`, w: `PrimarySpecPow`.
    pub primary: Vec4,
    /// rgb: `SecondarySpecColor`, w: `SecondarySpecPow`.
    pub secondary: Vec4,
    /// x: `PrimarySpecShift`, y: `SecondarySpecShift`, z: `BumpScale` (amplitude of the shift noise), w: `DetailTilling`.
    pub shape: Vec4,
    /// x: `RimMultiplier`, y: `DiffuseWrap`, z: `AmbientSpecMult`, w: 1 when the Specular map is bound.
    pub light: Vec4,
    /// rgb: `Specular` (the engine's `MatSpecColor`) times [`SPEC_GAIN`], w: factor on the diffuse colour.
    pub spec: Vec4,
    /// x: 1 for the blended fringe pass ([`HairFringe`]), y: the opaque pass's cut on the doubled alpha, z: the world's
    /// gain on everything but the back lights (`backlight::WORLD_DIFFUSE_GAIN` while the back lights are on, else 1).
    pub mode: Vec4,
}

impl HairParams {
    /// From the material, with the shader's defaults for missing parameters.
    pub fn from_material(m: &CryMaterial, has_map: bool) -> Self {
        let f = |name: &str, default: f32| param_f32(m, name).unwrap_or(default);
        // the colours are picker values like `HairColor` (see `hair_colour`)
        let colour = |name: &str| hair_colour(param_rgb(m, name).unwrap_or(Vec3::ONE));
        HairParams {
            primary: colour("PrimarySpecColor").extend(f("PrimarySpecPow", 200.0).max(0.1)),
            secondary: colour("SecondarySpecColor").extend(f("SecondarySpecPow", 16.0).max(0.1)),
            shape: Vec4::new(f("PrimarySpecShift", 0.08), f("SecondarySpecShift", 0.54), f("BumpScale", 0.5), f("DetailTilling", 1.0)),
            light: Vec4::new(f("RimMultiplier", 0.2), f("DiffuseWrap", 0.75).clamp(0.0, 1.0), f("AmbientSpecMult", 1.0), if has_map { 1.0 } else { 0.0 }),
            spec: (m.specular * SPEC_GAIN).extend(DIFFUSE_GAIN),
            mode: Vec4::ZERO,
        }
    }
}

/// Factors on the strand highlights (`Specular`) and on the diffuse colour of hair, fitted to the hair of the
/// `step2_*` captures (`compare`, mean and 10 / 50 / 90 % brightness of the lit top of the hair; 1 = the shader).
/// The stage lights are brighter than the engine's for hair (the skins carry per model tunes for the same reason):
/// at 1 / 1 the blond hair came out 1.7 times too bright, with too little contrast between strands and grooves.
const SPEC_GAIN: f32 = 1.3;
const DIFFUSE_GAIN: f32 = 0.55;

/// What [`prepare`] registered: the parameters and the Specular map of each distinct hair material.
#[derive(Clone)]
struct Entry {
    params: HairParams,
    spec_map: Option<Handle<Image>>,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    by_key: HashMap<String, usize>,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

fn registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    let mut guard = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(Registry::default))
}

/// Marks a converted `Hair` material for the [`HairMaterial`] (models with `Shading::Full`).
pub(crate) fn prepare(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("hair") {
        return;
    }
    let spec_path = m.texture(&TextureMap::Specular).map(|t| t.pak_path());
    let spec_map = spec_path.as_deref().and_then(|p| prep.texture(p, false));
    let params = HairParams::from_material(m, spec_map.is_some());
    let key = format!("{}|{params:?}", spec_path.unwrap_or_default());
    let index = registry(|r| {
        if let Some(i) = r.by_key.get(&key) {
            return *i;
        }
        r.entries.push(Entry { params, spec_map });
        let i = r.entries.len() - 1;
        r.by_key.insert(key, i);
        i
    });
    out.lightmap_exposure = TAG_BASE + index as f32;
}

/// The registry entry of a material tagged by [`prepare`].
fn entry_of(base: &StandardMaterial) -> Option<Entry> {
    let e = base.lightmap_exposure;
    if !(TAG_BASE..TAG_BASE + 1.0e6).contains(&e) {
        return None;
    }
    let index = (e - TAG_BASE).round() as usize;
    registry(|r| r.entries.get(index).cloned())
}

/// The GPU form of a [`HairExt`].
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct HairData {
    hair: HairParams,
    back: BackLightParams,
}

/// The material extension (bindless like `StandardMaterial` and the back light material, see
/// `backlight::CharBackExt`: data array at binding 101, index table at 100 with the indices 50..53).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[data(50, HairData, binding_array(101))]
#[bindless(index_table(range(50..53), binding(100)))]
pub struct HairExt {
    pub params: HairParams,
    /// The character back lights (`Char BackLight`), from [`CharBackLight`] when that resource exists.
    pub back: BackLightParams,
    /// The Specular map: rgb = highlight mask, a = strand shift noise.
    #[texture(51)]
    #[sampler(52)]
    pub spec_map: Option<Handle<Image>>,
}

impl From<&HairExt> for HairData {
    fn from(e: &HairExt) -> Self {
        HairData { hair: e.params, back: e.back }
    }
}

impl MaterialExtension for HairExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_cry/shade/hair.wesl".into()
    }
}

/// Registers [`HairMaterial`] and the system that gives hair its material.
pub struct HairPlugin;

impl Plugin for HairPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "hair.wesl");
        app.add_plugins(MaterialPlugin::<HairMaterial>::default())
            .init_resource::<Mirrors>()
            .add_systems(PostUpdate, (swap_materials, follow_meshes, upload_back_lights).chain());
    }
}

/// The `HairMaterial`s made for each tagged `StandardMaterial` (ids only: the entities own the handles).
#[derive(Resource, Default)]
struct Mirrors(HashMap<AssetId<StandardMaterial>, Mirror>);

/// The two materials of a hair material: the opaque (masked) one and the blended fringe.
#[derive(Clone, Copy)]
struct Mirror {
    opaque: AssetId<HairMaterial>,
    fringe: AssetId<HairMaterial>,
}

/// The back lights and the world gain: in the world (the back lights are on) hair gets the world's gain like the skin
/// and the costumes (its colours are calibrated under the login stage's lights, three times the engine's units), on
/// the login stage it does not.
fn back_params(light: Option<&CharBackLight>) -> (BackLightParams, f32) {
    match light {
        Some(l) if l.active() => (BackLightParams { dirs: l.dirs.map(|d| d.extend(1.0)), lights: l.lights }, WORLD_DIFFUSE_GAIN),
        _ => (BackLightParams::default(), 1.0),
    }
}

/// The base of a hair material: the conversion with every specular term off (the strands do the highlight).
fn base_of(standard: &StandardMaterial) -> StandardMaterial {
    let mut base = standard.clone();
    base.reflectance = 0.0;
    base.specular_tint = Color::WHITE;
    base.specular_texture = None;
    base.specular_tint_texture = None;
    base.metallic = 0.0;
    base.metallic_roughness_texture = None;
    base.perceptual_roughness = 1.0;
    base.anisotropy_strength = 0.0;
    base.lightmap_exposure = 1.0;
    base
}

/// On a hair mesh: the child entity that draws its blended fringe.
///
/// The engine draws hair in three passes: an opaque one (alpha tested), then back and front faces blended with
/// `alpha * 2` where the texture alpha is above 0.05, which gives the soft strand ends. Here the hair mesh is the
/// opaque pass (alpha mask on the doubled alpha: safe in the depth / normal prepasses) and this child, the same mesh
/// with a blended [`HairMaterial`] (`mode.x` = 1), draws only the texels the mask cuts away, after everything opaque.
#[derive(Component, Clone, Copy, Debug)]
pub struct HairFringe(pub Entity);

/// Lowest texture alpha the blended passes draw (`clip(alpha - 0.05)`).
const FRINGE_MIN_ALPHA: f32 = 0.05;

#[allow(clippy::type_complexity)]
fn swap_materials(
    mut commands: Commands,
    light: Option<Res<CharBackLight>>,
    standard: Res<Assets<StandardMaterial>>,
    mut hair: ResMut<Assets<HairMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    fresh: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, &Mesh3d, Option<&SkinnedMesh>, Option<&HairFringe>),
        Changed<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    for (entity, handle, mesh, skin, fringe) in &fresh {
        let Some(material) = standard.get(&handle.0) else { continue };
        let Some(entry) = entry_of(material) else { continue };
        let id = handle.0.id();
        let shared = mirrors.0.get(&id).and_then(|m| hair.get_strong_handle(m.opaque)).zip(mirrors.0.get(&id).and_then(|m| hair.get_strong_handle(m.fringe)));
        let (opaque, blended) = shared.unwrap_or_else(|| {
            let (back, gain) = back_params(light.as_deref());
            let base = base_of(material);
            let cut = match base.alpha_mode {
                AlphaMode::Mask(c) => c,
                _ => 0.0,
            };
            let mut fringe_base = base.clone();
            fringe_base.alpha_mode = AlphaMode::Blend;
            let mut params = entry.params;
            params.mode.z = gain;
            let mut fringe_params = params;
            fringe_params.mode.x = 1.0;
            fringe_params.mode.y = cut;
            let opaque = hair.add(HairMaterial { base, extension: HairExt { params, back, spec_map: entry.spec_map.clone() } });
            let fringe = hair.add(HairMaterial { base: fringe_base, extension: HairExt { params: fringe_params, back, spec_map: entry.spec_map.clone() } });
            mirrors.0.insert(id, Mirror { opaque: opaque.id(), fringe: fringe.id() });
            (opaque, fringe)
        });
        commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(opaque));
        // the fringe only where the mask cuts something away (a mask of 0 is a fully opaque material)
        if !matches!(material.alpha_mode, AlphaMode::Mask(c) if c > FRINGE_MIN_ALPHA * 2.0) {
            continue;
        }
        let parts = (Mesh3d(mesh.0.clone()), MeshMaterial3d(blended), Transform::IDENTITY, NotShadowCaster, Name::new("hair fringe"));
        let child = match fringe {
            Some(f) => {
                commands.entity(f.0).insert(parts);
                f.0
            }
            None => commands.spawn(parts).id(),
        };
        if let Some(skin) = skin {
            commands.entity(child).insert(skin.clone());
        }
        commands.entity(entity).add_child(child).insert(HairFringe(child));
    }
}

/// Keeps the fringe on the mesh of its hair (morphs and LOD swaps replace `Mesh3d`).
fn follow_meshes(mut commands: Commands, hair: Query<(&Mesh3d, &HairFringe), Changed<Mesh3d>>) {
    for (mesh, fringe) in &hair {
        if let Ok(mut e) = commands.get_entity(fringe.0) {
            e.insert(Mesh3d(mesh.0.clone()));
        }
    }
}

/// The least time between two uploads of the back lights, in seconds (the time of day moves slowly).
const UPLOAD_INTERVAL: f32 = 0.5;

/// Pushes the back lights to every [`HairMaterial`] when they changed.
fn upload_back_lights(
    light: Option<Res<CharBackLight>>,
    time: Res<Time<Real>>,
    mut hair: ResMut<Assets<HairMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    mut sent: Local<Option<(BackLightParams, f32, f32)>>,
) {
    let (params, gain) = back_params(light.as_deref());
    let now = time.elapsed_secs();
    if sent.as_ref().is_some_and(|(p, g, at)| *g == gain && (*p == params || now - at < UPLOAD_INTERVAL)) {
        return;
    }
    *sent = Some((params, gain, now));
    mirrors.0.retain(|_, m| hair.contains(m.opaque) || hair.contains(m.fringe));
    let ids: Vec<_> = hair.ids().collect();
    for id in ids {
        if hair.get(id).is_some_and(|m| m.extension.back != params || m.extension.params.mode.z != gain)
            && let Some(mut m) = hair.get_mut(id)
        {
            m.extension.back = params;
            m.extension.params.mode.z = gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAIR: &str = r#"<Material Diffuse="0.067,0.067,0.067" Shader="Hair" Specular="0.31,0.31,0.31" AlphaTest="0.3">
 <PublicParams HairColor="0.07,0.055,0.035" PrimarySpecColor="0.106,0.075,0.047" PrimarySpecPow="180" PrimarySpecShift="-1"
  SecondarySpecColor="0.035,0.027,0.016" SecondarySpecPow="110" SecondarySpecShift="-0.4" BumpScale="0.19" DiffuseWrap="0.968"
  RimMultiplier="0.299" AmbientSpecMult="0.2"/></Material>"#;

    #[test]
    fn the_hair_material_is_bindless_like_the_standard_material() {
        assert!(<HairMaterial as AsBindGroup>::bindless_slot_count().is_some());
        let d = <HairMaterial as AsBindGroup>::bindless_descriptor().expect("a bindless descriptor");
        assert_eq!(d.index_tables.len(), 2);
    }

    #[test]
    fn parameters_come_from_the_mtl() {
        let m = CryMaterial::parse(HAIR).expect("mtl");
        let p = HairParams::from_material(&m, true);
        assert_eq!(p.primary.w, 180.0);
        assert_eq!(p.secondary.w, 110.0);
        assert_eq!(p.shape.truncate(), Vec3::new(-1.0, -0.4, 0.19));
        assert!((p.light.y - 0.968).abs() < 1e-6 && (p.light.z - 0.2).abs() < 1e-6 && p.light.w == 1.0);
        // dark hair has a dark highlight
        assert!(p.primary.x < 0.05 && p.primary.x > p.primary.z, "{:?}", p.primary);
        assert!((p.spec.x - 0.31 * SPEC_GAIN).abs() < 1e-6);
    }

    #[test]
    fn tags_are_not_back_light_tags_and_round_trip() {
        let m = CryMaterial::parse(HAIR).expect("mtl");
        let params = HairParams::from_material(&m, false);
        let index = registry(|r| {
            r.entries.push(Entry { params, spec_map: None });
            r.entries.len() - 1
        });
        let tagged = StandardMaterial { lightmap_exposure: TAG_BASE + index as f32, ..default() };
        assert_eq!(entry_of(&tagged).map(|e| e.params), Some(params));
        assert!(entry_of(&StandardMaterial::default()).is_none());
        assert!(entry_of(&StandardMaterial { lightmap_exposure: super::super::backlight::TAG, ..default() }).is_none());
        let b = base_of(&tagged);
        assert_eq!((b.reflectance, b.lightmap_exposure), (0.0, 1.0));
    }
}

