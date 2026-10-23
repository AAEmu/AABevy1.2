//! The `Illum` shader of the client (`shaders/hwscripts/cryfx/illum.cfx` with `fraglib.cfi`, extracted to
//! `data/extract/aa-cry/shaders/`) as a material, for the costumes, armour and weapons of characters.
//!
//! What the engine does (`frag_custom_per_light` / `_end`, `frag_unify`):
//!
//! * **Diffuse**: `texture * Diffuse * (ambient' + sum(saturate(N.L) * shadow * light))`, the ambient taken in the
//!   hemisphere (`N.z * 0.25 + 0.75`) and weighted by the sun: `ambient * 0.6 + ambient * 0.4 * (saturate(N.L * 0.5 +
//!   0.5) + 0.3)`.
//! * **Specular, per light**: unnormalised Phong `pow(saturate(R.L), Shininess)` times the light and
//!   `shadow * 0.8 + 0.2`, summed, times the Specular ("gloss") map (`gloss.r` with `SPLIT_GLOSS_MAP`, else
//!   `gloss.rgb`; white without `GLOSS_MAP`), times `Specular` (`MatSpecColor`) and, with `HDR_MODIFIER`,
//!   `SpecularModifierInHDR`. On armour that is `8 x 4.9` on a map of 0.2 - 0.6: white-hot highlights.
//! * **Reflection** (`ENVCMSPEC`): the material's cube map (`beach_blur2` on most costumes) in the reflected view
//!   direction, times `ReflectAmount`, times the Fresnel `FresnelBias + (1 - N.V)^5 * FresnelScale`, times **the light
//!   the surface receives** (`ambient + diffuse`), times `gloss.g` (split) or `gloss.rgb`. Not times `Specular`.
//!   The final colour takes `max(specular, reflection)`.
//!
//! That the reflection is multiplied by the received light is what makes the silver armour of the captures: a
//! bright mirror where it is lit, dark on the shadow side. `StandardMaterial`'s environment specular (a metal with
//! a raised albedo, the old `bake_specular` mapping) reflected the same everywhere and looked dull grey.
//!
//! [`IllumMaterial`] (`StandardMaterial` extended by `illum.wesl`) does this: Bevy's PBR lighting runs on a white
//! albedo with the specular off (ambient, environment diffuse, the stage's fill lights, SSAO), the shader takes the
//! shadow casting light as the engine's sun (diffuse, highlight) and adds the reflection and the character back
//! lights. In the world (the back lights are on) it carries the world's gain like skin and hair.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::backlight::{BackLightParams, CharBackLight, WORLD_DIFFUSE_GAIN};
use super::param_f32;
use crate::material::{Material as CryMaterial, TextureMap};

/// Native Illum composition for Full models, or the post-lighting vertex RGB term for Basic brushes.
pub type IllumMaterial = ExtendedMaterial<StandardMaterial, IllumExt>;

/// `lightmap_exposure` of an illum material is `TAG_BASE + index` into the registry (back light tags are below 3,
/// hair tags from 4 to 4 + 1e6, skin tags from 2e6).
const TAG_BASE: f32 = 3.0e6;

/// `GenMask` bits of `Illum` (`illum.ext`).
const GEN_HDR_MODIFIER: u64 = 0x4;
const GEN_SPLIT_GLOSS_MAP: u64 = 0x8;
const GEN_GLOSS_MAP: u64 = 0x10;
const GEN_ENVCMSPEC: u64 = 0x80;
const GEN_VERTCOLORS: u64 = 0x40_0000;
const GEN_GLOSSNESS_GLOSSZ: u64 = 0x8_0000_0000;

/// The colour stream is lighting data only with Illum `%VERTCOLORS`; alpha has a separate flag.
pub fn uses_vertex_color(m: &CryMaterial) -> bool {
    m.shader.eq_ignore_ascii_case("illum") && m.gen_mask & GEN_VERTCOLORS != 0
}

/// The parameters of one `.mtl` sub-material, as the shader reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct IllumParams {
    /// rgb: `Specular` (x `SpecularModifierInHDR` with `HDR_MODIFIER`) x the highlight tune, w: `Shininess`.
    pub spec: Vec4,
    /// x: `ReflectAmount` x the reflection tune (0: no reflection), y: `FresnelBias`, z: `FresnelScale`.
    pub env: Vec4,
    /// x: 1 with `SPLIT_GLOSS_MAP`, y: 1 when the gloss map is bound (`GLOSS_MAP`), z: 1 with `GLOSSNESS_GLOSSZ`
    /// (gloss.b scales the exponent), w: 1 when the cube map is bound.
    pub mode: Vec4,
    /// x: gain on everything but the back lights (the world's, 1 on the stage), y: factor on the ambient,
    /// z: debug output (1 = the reflection alone, 2 = the highlight alone).
    pub gain: Vec4,
    /// x: post-lighting RGB squared, y: native Illum composition (Full), z: character lighting (Full).
    pub vertex: Vec4,
    /// `MatEmissiveColor` in Bevy light units (engine units x `LIGHT_UNIT_LUX / pi`), added to ambient.
    pub emission: Vec4,
}

impl IllumParams {
    /// From the material, with the shader's defaults for missing parameters.
    pub fn from_material(m: &CryMaterial, has_gloss: bool, has_env: bool) -> Self {
        let f = |name: &str, default: f32| param_f32(m, name).unwrap_or(default);
        let mask = m.gen_mask;
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let t = tune();
        let modifier = if mask & GEN_HDR_MODIFIER != 0 {
            f("SpecularModifierInHDR", 1.0)
        } else {
            1.0
        };
        let reflect = if mask & GEN_ENVCMSPEC != 0 && has_env {
            f("ReflectAmount", 0.25) * t.env
        } else {
            0.0
        };
        IllumParams {
            spec: (m.specular * modifier * t.spec).extend(m.shininess.max(0.0)),
            env: Vec4::new(reflect, f("FresnelBias", 0.0), f("FresnelScale", 1.0), 0.0),
            mode: Vec4::new(
                flag(mask & GEN_SPLIT_GLOSS_MAP != 0),
                flag(has_gloss && mask & GEN_GLOSS_MAP != 0),
                flag(
                    has_gloss
                        && mask & GEN_GLOSS_MAP != 0
                        && mask & GEN_GLOSSNESS_GLOSSZ != 0
                        && mask & GEN_SPLIT_GLOSS_MAP != 0,
                ),
                flag(has_env),
            ),
            gain: Vec4::new(1.0, t.ambient, t.debug, 0.0),
            vertex: Vec4::new(flag(uses_vertex_color(m)), 1.0, 1.0, 0.0),
            emission: (m.emissive * (crate::stage::LIGHT_UNIT_LUX / std::f32::consts::PI))
                .extend(0.0),
        }
    }
}

/// Optional experimental factors (1 = the shader).
#[derive(Clone, Copy, Debug)]
struct Tune {
    /// On `Specular`.
    spec: f32,
    /// On `ReflectAmount`.
    env: f32,
    /// On the ambient light.
    ambient: f32,
    /// Debug output: 1 the reflection alone, 2 the highlight alone.
    debug: f32,
}

const TUNE: Tune = Tune {
    spec: 1.0,
    env: 1.0,
    ambient: 1.0,
    debug: 0.0,
};

/// [`TUNE`], overridden by `AA_ILLUM_TUNE="spec=1,env=1,ambient=1,debug=0"` for experiments.
fn tune() -> Tune {
    let mut t = TUNE;
    for kv in std::env::var("AA_ILLUM_TUNE")
        .unwrap_or_default()
        .split(',')
    {
        let Some((k, v)) = kv.split_once('=') else {
            continue;
        };
        let Ok(v) = v.trim().parse::<f32>() else {
            continue;
        };
        match k.trim() {
            "spec" => t.spec = v,
            "env" => t.env = v,
            "ambient" => t.ambient = v,
            "debug" => t.debug = v,
            _ => {}
        }
    }
    t
}

/// What [`prepare`] registered: the parameters and the maps of each distinct material.
#[derive(Clone)]
struct Entry {
    params: IllumParams,
    gloss_map: Option<Handle<Image>>,
    env_map: Option<Handle<Image>>,
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

/// Whether Full shading can replace the StandardMaterial approximation with native Illum lighting.
pub(crate) fn applies(m: &CryMaterial, out: &StandardMaterial) -> bool {
    m.shader.eq_ignore_ascii_case("illum") && out.alpha_mode != AlphaMode::Blend && !out.unlit
}

/// Tags native Full materials and Basic Illum brushes with vertex colours. Basic preserves its PBR lighting.
pub(crate) fn prepare(
    prep: &mut crate::loader::Prep<'_>,
    m: &CryMaterial,
    out: &mut StandardMaterial,
) -> bool {
    // Retain the previous Full conversion as an explicit visual comparison control.
    let native = prep.shading == crate::scene::Shading::Full
        && applies(m, out)
        && std::env::var_os("AA_ILLUM_LEGACY").is_none();
    if !native && !uses_vertex_color(m) {
        return false;
    }
    let gloss_path = (native && m.gen_mask & GEN_GLOSS_MAP != 0)
        .then(|| m.texture(&TextureMap::Specular).map(|t| t.pak_path()))
        .flatten();
    let env_path = (native && m.gen_mask & GEN_ENVCMSPEC != 0).then(|| {
        crate::stage::material_environment(m)
            .unwrap_or_else(|| crate::stage::DEFAULT_ENVIRONMENT.to_string())
    });
    // `GLOSSMAP` in fxsamplerdefs.cfi explicitly uses sRGBLookup=true.
    let gloss_map = gloss_path.as_deref().and_then(|p| prep.texture(p, true));
    // ENVIRONMENTCUBEMAP has no sRGBLookup, and GetEnvironmentCMap returns the raw sample.
    let env_map = env_path
        .as_deref()
        .and_then(|p| prep.cube_texture_with_encoding(p, false));
    let mut params = IllumParams::from_material(m, gloss_map.is_some(), env_map.is_some());
    params.vertex.y = if native { 1.0 } else { 0.0 };
    params.vertex.z = if prep.shading == crate::scene::Shading::Full {
        1.0
    } else {
        0.0
    };
    if native && m.gen_mask & GEN_HDR_MODIFIER == 0 {
        // The generic StandardMaterial approximation always applies this parameter. Native Illum
        // reads it only with HDR_MODIFIER. Material hooks run after this conversion.
        out.base_color = Color::linear_rgba(
            m.diffuse.x,
            m.diffuse.y,
            m.diffuse.z,
            out.base_color.alpha(),
        );
    }
    debug!(
        "illum {}: GenMask {:x}, Specular {:?}, Shininess {}, {params:?}",
        m.name, m.gen_mask, m.specular, m.shininess
    );
    let key = format!(
        "{}|{}|{params:?}",
        gloss_path.unwrap_or_default(),
        env_path.unwrap_or_default()
    );
    let index = registry(|r| {
        if let Some(i) = r.by_key.get(&key) {
            return *i;
        }
        r.entries.push(Entry {
            params,
            gloss_map,
            env_map,
        });
        let i = r.entries.len() - 1;
        r.by_key.insert(key, i);
        i
    });
    out.lightmap_exposure = TAG_BASE + index as f32;
    true
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

/// The GPU form of an [`IllumExt`].
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct IllumData {
    illum: IllumParams,
    back: BackLightParams,
}

/// The material extension (bindless like `StandardMaterial`, see `backlight::CharBackExt`: data array at binding
/// 101, index table at 100 with the indices 50..55).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[data(50, IllumData, binding_array(101))]
#[bindless(index_table(range(50..55), binding(100)))]
pub struct IllumExt {
    pub params: IllumParams,
    /// The character back lights (`Char BackLight`), from [`CharBackLight`] when that resource exists.
    pub back: BackLightParams,
    /// The Specular ("gloss") map: r = highlight mask, g = reflection mask (split), b = exponent (`GLOSSNESS_GLOSSZ`).
    #[texture(51)]
    #[sampler(52)]
    pub gloss_map: Option<Handle<Image>>,
    /// The environment cube map.
    #[texture(53, dimension = "cube")]
    #[sampler(54)]
    pub env_map: Option<Handle<Image>>,
}

impl From<&IllumExt> for IllumData {
    fn from(e: &IllumExt) -> Self {
        IllumData {
            illum: e.params,
            back: e.back,
        }
    }
}

impl MaterialExtension for IllumExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_cry/shade/illum.wesl".into()
    }
}

/// Registers [`IllumMaterial`] and the systems that specialize tagged StandardMaterials.
pub struct IllumPlugin;

impl Plugin for IllumPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "illum.wesl");
        app.add_plugins(MaterialPlugin::<IllumMaterial>::default())
            .init_resource::<Mirrors>()
            .add_systems(PostUpdate, (swap_materials, upload_lights).chain());
    }
}

/// The `IllumMaterial` made for each tagged `StandardMaterial` (ids only: the entities own the handles).
#[derive(Resource, Default)]
struct Mirrors(HashMap<AssetId<StandardMaterial>, AssetId<IllumMaterial>>);

/// The back lights and the world gain: in the world (the back lights are on) costumes get the world's gain like the
/// other character materials, on the login stage (no back lights) they do not.
fn lighting(light: Option<&CharBackLight>) -> (BackLightParams, f32) {
    match light {
        Some(l) if l.active() => (
            BackLightParams {
                dirs: l.dirs.map(|d| d.extend(1.0)),
                lights: l.lights,
            },
            WORLD_DIFFUSE_GAIN,
        ),
        _ => (BackLightParams::default(), 1.0),
    }
}

/// Native composition handles its own specular; the Basic vertex-colour path keeps PBR unchanged.
fn base_of(standard: &StandardMaterial, native: bool) -> StandardMaterial {
    let mut base = standard.clone();
    if native {
        base.reflectance = 0.0;
        base.specular_tint = Color::WHITE;
        base.specular_texture = None;
        base.specular_tint_texture = None;
        base.metallic = 0.0;
        base.metallic_roughness_texture = None;
        base.perceptual_roughness = 1.0;
    }
    base.lightmap_exposure = 1.0;
    base
}

fn swap_materials(
    mut commands: Commands,
    light: Option<Res<CharBackLight>>,
    standard: Res<Assets<StandardMaterial>>,
    mut illum: ResMut<Assets<IllumMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    fresh: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        Changed<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    for (entity, handle) in &fresh {
        let Some(material) = standard.get(&handle.0) else {
            continue;
        };
        let Some(entry) = entry_of(material) else {
            continue;
        };
        let id = handle.0.id();
        let shared = mirrors.0.get(&id).and_then(|m| illum.get_strong_handle(*m));
        let mirror = shared.unwrap_or_else(|| {
            let (back, gain) = lighting(light.as_deref());
            let mut params = entry.params;
            let character = params.vertex.z > 0.5;
            params.gain.x = if character { gain } else { 1.0 };
            let strong = illum.add(IllumMaterial {
                base: base_of(material, params.vertex.y > 0.5),
                extension: IllumExt {
                    params,
                    back: if character {
                        back
                    } else {
                        BackLightParams::default()
                    },
                    gloss_map: entry.gloss_map.clone(),
                    env_map: entry.env_map.clone(),
                },
            });
            mirrors.0.insert(id, strong.id());
            strong
        });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(mirror));
    }
}

/// The least time between two uploads of the back lights, in seconds (the time of day moves slowly).
const UPLOAD_INTERVAL: f32 = 0.5;

/// How many materials an upload rewrites per frame (every changed material is re-prepared by the render world,
/// about 10 us each; see `backlight::UPLOAD_CHUNK`).
const UPLOAD_CHUNK: usize = 64;

/// What [`upload_lights`] remembers.
#[derive(Default)]
struct Upload {
    /// The lighting of the last upload that was started, and when.
    sent: Option<(BackLightParams, f32, f32)>,
    /// The materials that still have to get it.
    queue: Vec<AssetId<IllumMaterial>>,
}

/// Pushes the back lights and the gain to every [`IllumMaterial`] when they changed, a few materials per frame.
fn upload_lights(
    light: Option<Res<CharBackLight>>,
    time: Res<Time<Real>>,
    mut illum: ResMut<Assets<IllumMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    mut up: Local<Upload>,
) {
    let (back, gain) = lighting(light.as_deref());
    let now = time.elapsed_secs();
    if up.queue.is_empty() {
        if up.sent.as_ref().is_some_and(|(b, g, at)| {
            (*b == back && *g == gain) || (*g == gain && now - at < UPLOAD_INTERVAL)
        }) {
            return;
        }
        up.sent = Some((back, gain, now));
        mirrors.0.retain(|_, m| illum.contains(*m));
        up.queue.extend(illum.ids());
    }
    let Some((back, gain, _)) = up.sent else {
        return;
    };
    for _ in 0..UPLOAD_CHUNK {
        let Some(id) = up.queue.pop() else { break };
        let Some(m) = illum.get(id) else { continue };
        if m.extension.params.vertex.z < 0.5 {
            continue;
        }
        if m.extension.back == back && m.extension.params.gain.x == gain {
            continue;
        }
        if let Some(mut m) = illum.get_mut(id) {
            m.extension.back = back;
            m.extension.params.gain.x = gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the plate armour of `nu_f_sk_saint001.mtl`
    const ARMOUR: &str = r#"<Material Shader="Illum" GenMask="809c" Diffuse="0.216,0.216,0.216" Specular="6.464,8.04,4.94" Shininess="13.3">
 <PublicParams SpecularModifierInHDR="4.94" FresnelScale="24.55" FresnelBias="0.426" DiffuseModifierInHDR="1" ReflectAmount="10.05"/></Material>"#;
    // a prop: no gloss map, no reflection
    const PROP: &str = r#"<Material Shader="Illum" GenMask="0" Diffuse="0.5,0.5,0.5" Specular="0.2,0.2,0.2" Shininess="10">
 <PublicParams ReflectAmount="0.25" SpecularModifierInHDR="3"/></Material>"#;

    #[test]
    fn the_illum_material_is_bindless_like_the_standard_material() {
        assert!(<IllumMaterial as AsBindGroup>::bindless_slot_count().is_some());
        let d =
            <IllumMaterial as AsBindGroup>::bindless_descriptor().expect("a bindless descriptor");
        assert_eq!(d.index_tables.len(), 2);
    }

    #[test]
    fn parameters_and_modes_come_from_the_mtl() {
        let a = IllumParams::from_material(&CryMaterial::parse(ARMOUR).expect("mtl"), true, true);
        // HDR_MODIFIER scales the highlight, not the reflection
        assert!((a.spec.y - 8.04 * 4.94 * TUNE.spec).abs() < 1e-3, "{a:?}");
        assert_eq!(a.spec.w, 13.3);
        assert!((a.env.x - 10.05 * TUNE.env).abs() < 1e-4 && a.env.y == 0.426 && a.env.z == 24.55);
        // SPLIT_GLOSS_MAP + GLOSS_MAP, a cube map
        assert_eq!(
            (a.mode.x, a.mode.y, a.mode.z, a.mode.w),
            (1.0, 1.0, 0.0, 1.0)
        );
        // without the cube map there is no reflection
        assert_eq!(
            IllumParams::from_material(&CryMaterial::parse(ARMOUR).expect("mtl"), true, false)
                .env
                .x,
            0.0
        );
        let p = IllumParams::from_material(&CryMaterial::parse(PROP).expect("mtl"), false, false);
        // no HDR_MODIFIER: the modifier is not used; no ENVCMSPEC: no reflection; no gloss map: white
        assert!((p.spec.x - 0.2 * TUNE.spec).abs() < 1e-5 && p.env.x == 0.0 && p.mode.y == 0.0);
    }

    #[test]
    fn tags_are_outside_the_other_tags_and_round_trip() {
        let m = CryMaterial::parse(ARMOUR).expect("mtl");
        let params = IllumParams::from_material(&m, false, false);
        let index = registry(|r| {
            r.entries.push(Entry {
                params,
                gloss_map: None,
                env_map: None,
            });
            r.entries.len() - 1
        });
        let tagged = StandardMaterial {
            lightmap_exposure: TAG_BASE + index as f32,
            metallic: 1.0,
            reflectance: 2.0,
            ..default()
        };
        assert_eq!(entry_of(&tagged).expect("entry").params, params);
        assert!(entry_of(&StandardMaterial::default()).is_none());
        assert!(
            entry_of(&StandardMaterial {
                lightmap_exposure: super::super::backlight::TAG,
                ..default()
            })
            .is_none()
        );
        assert!(
            entry_of(&StandardMaterial {
                lightmap_exposure: 2.0e6 + 3.0,
                ..default()
            })
            .is_none()
        );
        let b = base_of(&tagged, true);
        assert_eq!(
            (b.reflectance, b.metallic, b.lightmap_exposure),
            (0.0, 0.0, 1.0)
        );
        let basic = base_of(&tagged, false);
        assert_eq!(
            (basic.reflectance, basic.metallic, basic.lightmap_exposure),
            (2.0, 1.0, 1.0)
        );
    }

    #[test]
    fn gloss_blue_requires_every_feature_and_a_bound_gloss_texture() {
        let mut m = CryMaterial::parse(PROP).unwrap();
        m.gen_mask = GEN_GLOSSNESS_GLOSSZ | GEN_SPLIT_GLOSS_MAP | GEN_GLOSS_MAP;
        assert_eq!(IllumParams::from_material(&m, true, false).mode.z, 1.0);
        assert_eq!(IllumParams::from_material(&m, false, false).mode.z, 0.0);
        for missing in [GEN_GLOSSNESS_GLOSSZ, GEN_SPLIT_GLOSS_MAP, GEN_GLOSS_MAP] {
            m.gen_mask = (GEN_GLOSSNESS_GLOSSZ | GEN_SPLIT_GLOSS_MAP | GEN_GLOSS_MAP) & !missing;
            assert_eq!(IllumParams::from_material(&m, true, false).mode.z, 0.0);
        }
    }

    #[test]
    fn vertex_rgb_does_not_follow_the_vertex_alpha_flag_or_another_shaders_mask() {
        let mut m = CryMaterial::parse(PROP).unwrap();
        m.gen_mask = 0x400; // VERTALPHA, not VERTCOLORS
        assert!(!uses_vertex_color(&m));
        m.gen_mask = GEN_VERTCOLORS;
        assert!(uses_vertex_color(&m));
        assert_eq!(IllumParams::from_material(&m, false, false).vertex.x, 1.0);
        m.shader = "Vegetation".into();
        assert!(!uses_vertex_color(&m));
    }

    #[test]
    fn missing_reflection_parameters_use_the_source_defaults() {
        let m = CryMaterial::parse(r#"<Material Shader="Illum" GenMask="0x80"/>"#).unwrap();
        let p = IllumParams::from_material(&m, false, true);
        assert_eq!(p.env, Vec4::new(0.25 * TUNE.env, 0.0, 1.0, 0.0));
    }

    #[test]
    fn specialization_shares_handles_and_keeps_basic_brushes_out_of_character_lighting() {
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(Assets::<IllumMaterial>::default())
            .init_resource::<Mirrors>()
            .insert_resource(CharBackLight::from_positions(
                super::super::backlight::CDF_POSITIONS,
                [Vec4::ONE; 2],
            ))
            .add_systems(Update, swap_materials);
        for full in [false, true] {
            let flag = if full { 1.0 } else { 0.0 };
            let index = registry(|r| {
                r.entries.push(Entry {
                    params: IllumParams {
                        vertex: Vec4::new(1.0, flag, flag, 0.0),
                        gain: Vec4::ONE,
                        ..default()
                    },
                    gloss_map: None,
                    env_map: None,
                });
                r.entries.len() - 1
            });
            let base = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    lightmap_exposure: TAG_BASE + index as f32,
                    reflectance: 0.7,
                    metallic: 0.3,
                    ..default()
                });
            let a = app.world_mut().spawn(MeshMaterial3d(base.clone())).id();
            let b = app.world_mut().spawn(MeshMaterial3d(base)).id();
            app.update();
            assert!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(a)
                    .is_none()
            );
            let a = app.world().get::<MeshMaterial3d<IllumMaterial>>(a).unwrap();
            let b = app.world().get::<MeshMaterial3d<IllumMaterial>>(b).unwrap();
            assert_eq!(a.0, b.0, "instances must share the specialized asset");
            let material = app
                .world()
                .resource::<Assets<IllumMaterial>>()
                .get(&a.0)
                .unwrap();
            assert_eq!(material.base.lightmap_exposure, 1.0);
            if full {
                assert_eq!(material.base.reflectance, 0.0);
                assert_eq!(material.extension.params.gain.x, WORLD_DIFFUSE_GAIN);
                assert_eq!(material.extension.back.lights, [Vec4::ONE; 2]);
            } else {
                assert_eq!(
                    (material.base.reflectance, material.base.metallic),
                    (0.7, 0.3)
                );
                assert_eq!(material.extension.params.gain.x, 1.0);
                assert_eq!(material.extension.back, BackLightParams::default());
            }
        }
    }

    #[test]
    fn blended_and_other_shaders_keep_the_standard_material() {
        let m = CryMaterial::parse(ARMOUR).expect("mtl");
        assert!(applies(&m, &StandardMaterial::default()));
        assert!(!applies(
            &m,
            &StandardMaterial {
                alpha_mode: AlphaMode::Blend,
                ..default()
            }
        ));
        let skin = CryMaterial::parse(r#"<Material Shader="HumanSkin" Diffuse="0.2,0.2,0.2"/>"#)
            .expect("mtl");
        assert!(!applies(&skin, &StandardMaterial::default()));
    }
}
