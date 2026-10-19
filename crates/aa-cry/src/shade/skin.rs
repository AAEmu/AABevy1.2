//! The `HumanSkin` shader of the client (`shaders/hwscripts/cryfx/humanskin.cfx`, extracted to
//! `data/extract/aa-cry/skin/`) as a material.
//!
//! What the engine does (`frag_custom_per_light` / `_begin` / `_end`):
//!
//! * **Diffuse, per light**: `SkinfNdotL2`, a wrapped, coloured fall-off instead of Lambert. With the unclamped
//!   `N.L`, `T = smoothstep(-DiffusionAmount * dark, 1, N.L)` (`dark` = the `SubSurface` map, desaturated by
//!   `SSSMapSat`) and `lerp(dark, lerp(MSkinColor * (SubsurfaceStrenght + 1), BSkinColor, N.L^2), T) * T`, then
//!   `^1.5 * 1.5`, `x 0.8` towards a light behind the camera, times the light's colour (90 % of it grey). Shadows only
//!   take 80 % of it, softened by `L.V`; in the shadow of the sun a colourless rim `(1 - |N.V|) * RimPow` is added
//!   around the silhouette. Ambient light gets `0.3 x` a skin coloured fill (`reColorInShadow`).
//! * **Specular, per light**: plain, unnormalised Blinn-Phong `pow(N.H, Shininess)` times the light, masked by the
//!   Specular ("gloss") map: `x gloss.r x lerp(Specular, 1, gloss.b)` (`GLOSS_MAP_IGNORE_SPECULAR_COLOR`, the body)
//!   or `x gloss.rgb x Specular` (the faces). `Shininess` is 25 (bodies) to 65 (nuian face), the masks are dark
//!   (0.25): a small, faint highlight. No Fresnel, no GGX; an environment reflection only with the
//!   `ENVIRONMENT_MAP` bit, which the nuian faces and bodies do not have.
//! * The character back lights: `X2BackLight` (plain `N.L` and a Phong highlight).
//!
//! [`SkinMaterial`] (`StandardMaterial` extended by `skin.wesl`) does this: Bevy's PBR lighting runs on a white albedo
//! with every specular term off (the ambient and environment diffuse light, with SSAO), the shader replaces the
//! Lambert part of the directional lights with the skin fall-off and adds the Blinn-Phong highlight and the back
//! lights. Bevy's GGX lobe (a broad sheen over the whole face) and its environment specular (the sky mirrored at every
//! grazing angle) were what made the skin look oily.
//!
//! The skin colour stays where the calibration put it: the base colour (`shade::skin_base_color`, aa-anim's
//! `skin_colors` row and per model tune) is the colour of fully lit skin. The fall-off is applied relative to that:
//! `SkinfNdotL2(N.L)^1.5 / BSkinColor^1.5` with the material's own `BSkinColor` / `MSkinColor` / SSS map, so it is
//! 1 in full light and turns towards the sub-surface red in the terminator.
//!
//! In the world (the back lights are on) the material carries the world's gain `backlight::WORLD_DIFFUSE_GAIN` like
//! the other character materials (`backlight::world_look`).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::backlight::{BackLightParams, CharBackLight, WORLD_DIFFUSE_GAIN};
use super::{luma, param_f32, param_rgb};
use crate::material::{Material as CryMaterial, TextureMap};
use crate::stage::{LIGHT_UNIT_LUX, LOGIN_EXPOSURE};

/// A skin material: the `StandardMaterial` of the conversion plus the skin shading.
pub type SkinMaterial = ExtendedMaterial<StandardMaterial, SkinExt>;

/// `lightmap_exposure` of a skin material is `TAG_BASE + index` into the registry (back light tags are below 3, hair
/// tags from 4 to 4 + 1e6).
const TAG_BASE: f32 = 2.0e6;

/// `GenMask` bits of `HumanSkin` (`humanskin.ext`).
const GEN_GLOSS_MAP: u64 = 0x2;
const GEN_SPLIT_GLOSS_MAP: u64 = 0x8;
const GEN_FUZZYNESS: u64 = 0x10_0000_0000;
const GEN_GLOSS_MAP_IGNORE_SPECULAR_COLOR: u64 = 0x20_0000_0000;

/// The skin parameters of one `.mtl` sub-material, as the shader reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct SkinParams {
    /// rgb: `BSkinColor` (as r, g, b; the files store b, g, r), w: `DiffusionAmount`.
    pub bright: Vec4,
    /// rgb: `MSkinColor`, w: `SubsurfaceStrenght`.
    pub middle: Vec4,
    /// rgb: `Specular` (`MatSpecColor`) times [`SPEC_GAIN`], w: `Shininess`.
    pub spec: Vec4,
    /// x: `RimPow`, y: `SSSMapSat`, z: 1 when the gloss map is used (`GLOSS_MAP`), w: 1 when the SSS map is bound.
    pub shape: Vec4,
    /// x: 1 with `SPLIT_GLOSS_MAP` (gloss.g triples the exponent), y: 1 when only gloss.r masks the highlight
    /// (`SPLIT_GLOSS_MAP`, `FUZZYNESS`, `GLOSS_MAP_IGNORE_SPECULAR_COLOR`), z: 1 with `GLOSS_MAP_IGNORE_SPECULAR_COLOR`
    /// (gloss.b lerps `Specular` to white).
    pub mode: Vec4,
    /// x: gain on everything but the back lights (the world's, 1 on the stage), y: one engine light unit in lux / pi
    /// for the colourless terms (fill, rim), z: factor on the skin fall-off of the lights, w: factor on the ambient.
    pub gain: Vec4,
    /// rgb: factor on the base colour that gives the albedo of the back lights (`texture * Diffuse`, see
    /// `backlight::skin_tag`).
    pub back_albedo: Vec4,
}

impl SkinParams {
    /// From the material, with the shader's defaults for missing parameters.
    pub fn from_material(m: &CryMaterial, has_gloss: bool, has_sss: bool) -> Self {
        let f = |name: &str, default: f32| param_f32(m, name).unwrap_or(default);
        // stored b, g, r like `BSkinColor` (see `shade::to_standard_material`)
        let bgr = |name: &str, default: f32| param_rgb(m, name).map_or(Vec3::splat(default), |c| Vec3::new(c.z, c.y, c.x));
        let mask = m.gen_mask;
        let split = mask & GEN_SPLIT_GLOSS_MAP != 0;
        let ignore = mask & GEN_GLOSS_MAP_IGNORE_SPECULAR_COLOR != 0;
        let mask_r = split || ignore || mask & GEN_FUZZYNESS != 0;
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let t = tune();
        SkinParams {
            bright: bgr("BSkinColor", 0.9).extend(f("DiffusionAmount", 1.0)),
            middle: bgr("MSkinColor", 0.6).extend(f("SubsurfaceStrenght", 0.0)),
            spec: (m.specular * t.spec).extend(m.shininess.max(1.0)),
            shape: Vec4::new(f("RimPow", 0.5), f("SSSMapSat", 0.5).clamp(0.0, 1.0), flag(has_gloss && mask & GEN_GLOSS_MAP != 0), flag(has_sss)),
            mode: Vec4::new(flag(split), flag(mask_r), flag(ignore), 0.0),
            gain: Vec4::new(1.0, LOGIN_EXPOSURE * LIGHT_UNIT_LUX / std::f32::consts::PI * t.fill, t.diffuse, t.ambient),
            back_albedo: Vec4::ONE,
        }
    }
}

/// Factors fitted to the captures (1 = the shader).
#[derive(Clone, Copy, Debug)]
struct Tune {
    /// On `Specular`.
    spec: f32,
    /// On the skin fall-off of the lights.
    diffuse: f32,
    /// On the ambient light (which the calibrated base colour multiplies; the engine uses `texture * Diffuse`).
    ambient: f32,
    /// On the colourless fill and rim terms.
    fill: f32,
    /// Whether the SSS map is read as sRGB colour.
    sss_srgb: bool,
}

const TUNE: Tune = Tune { spec: 0.35, diffuse: 1.0, ambient: 0.5, fill: 1.0, sss_srgb: false };

/// [`TUNE`], overridden by `AA_SKIN_TUNE="spec=1,diffuse=1,ambient=0.5,fill=1"` for experiments.
fn tune() -> Tune {
    let mut t = TUNE;
    for kv in std::env::var("AA_SKIN_TUNE").unwrap_or_default().split(',') {
        let Some((k, v)) = kv.split_once('=') else { continue };
        let Ok(v) = v.trim().parse::<f32>() else { continue };
        match k.trim() {
            "spec" => t.spec = v,
            "diffuse" => t.diffuse = v,
            "ambient" => t.ambient = v,
            "fill" => t.fill = v,
            "sss_srgb" => t.sss_srgb = v > 0.5,
            _ => {}
        }
    }
    t
}

/// What [`prepare`] registered: the parameters and the maps of each distinct skin material.
#[derive(Clone)]
struct Entry {
    params: SkinParams,
    diffuse: f32,
    gloss_map: Option<Handle<Image>>,
    sss_map: Option<Handle<Image>>,
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

/// Marks a converted `HumanSkin` material for the [`SkinMaterial`] (models with `Shading::Full`).
pub(crate) fn prepare(prep: &mut crate::loader::Prep<'_>, m: &CryMaterial, out: &mut StandardMaterial) {
    if !m.shader.eq_ignore_ascii_case("humanskin") {
        return;
    }
    let gloss_path = m.texture(&TextureMap::Specular).map(|t| t.pak_path());
    let sss_path = m.texture(&TextureMap::SubSurface).map(|t| t.pak_path());
    let gloss_map = gloss_path.as_deref().and_then(|p| prep.texture(p, false));
    let sss_map = sss_path.as_deref().and_then(|p| prep.texture(p, tune().sss_srgb));
    let params = SkinParams::from_material(m, gloss_map.is_some(), sss_map.is_some());
    let diffuse = luma(m.diffuse);
    debug!("skin {}: GenMask {:x}, Specular {:?}, Shininess {}, {params:?}", m.name, m.gen_mask, m.specular, m.shininess);
    let key = format!("{}|{}|{diffuse}|{params:?}", gloss_path.unwrap_or_default(), sss_path.unwrap_or_default());
    let index = registry(|r| {
        if let Some(i) = r.by_key.get(&key) {
            return *i;
        }
        r.entries.push(Entry { params, diffuse, gloss_map, sss_map });
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

/// The GPU form of a [`SkinExt`].
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SkinData {
    skin: SkinParams,
    back: BackLightParams,
}

/// The material extension (bindless like `StandardMaterial`, see `backlight::CharBackExt`: data array at binding
/// 101, index table at 100 with the indices 50..55).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[data(50, SkinData, binding_array(101))]
#[bindless(index_table(range(50..55), binding(100)))]
pub struct SkinExt {
    pub params: SkinParams,
    /// The character back lights (`Char BackLight`), from [`CharBackLight`] when that resource exists.
    pub back: BackLightParams,
    /// The Specular ("gloss") map: r = highlight mask, g = exponent x 3 (split), b = white highlight, a = exponent.
    #[texture(51)]
    #[sampler(52)]
    pub gloss_map: Option<Handle<Image>>,
    /// The `SubSurface` map: the colour of the terminator.
    #[texture(53)]
    #[sampler(54)]
    pub sss_map: Option<Handle<Image>>,
}

impl From<&SkinExt> for SkinData {
    fn from(e: &SkinExt) -> Self {
        SkinData { skin: e.params, back: e.back }
    }
}

impl MaterialExtension for SkinExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_cry/shade/skin.wesl".into()
    }
}

/// Registers [`SkinMaterial`] and the systems that give skin its material.
pub struct SkinPlugin;

impl Plugin for SkinPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "skin.wesl");
        app.add_plugins(MaterialPlugin::<SkinMaterial>::default())
            .init_resource::<Mirrors>()
            .add_systems(PostUpdate, (swap_materials, upload_lights).chain());
    }
}

/// The `SkinMaterial` made for each tagged `StandardMaterial` (ids only: the entities own the handles).
#[derive(Resource, Default)]
struct Mirrors(HashMap<AssetId<StandardMaterial>, AssetId<SkinMaterial>>);

/// The back lights and the world gain: in the world (the back lights are on) the skin gets the world's gain like
/// the other character materials, on the login stage (no back lights) it does not.
fn lighting(light: Option<&CharBackLight>) -> (BackLightParams, f32) {
    match light {
        Some(l) if l.active() => (BackLightParams { dirs: l.dirs.map(|d| d.extend(1.0)), lights: l.lights }, WORLD_DIFFUSE_GAIN),
        _ => (BackLightParams::default(), 1.0),
    }
}

/// The base of a skin material: the conversion with every specular term off (the shader does the highlight).
fn base_of(standard: &StandardMaterial) -> StandardMaterial {
    let mut base = standard.clone();
    base.reflectance = 0.0;
    base.specular_tint = Color::WHITE;
    base.specular_texture = None;
    base.specular_tint_texture = None;
    base.metallic = 0.0;
    base.metallic_roughness_texture = None;
    base.perceptual_roughness = 1.0;
    base.diffuse_transmission = 0.0;
    base.lightmap_exposure = 1.0;
    base
}

/// The parameters of a material for the current lighting.
fn params_for(entry: &Entry, base: &StandardMaterial, back: BackLightParams, gain: f32) -> (SkinParams, BackLightParams) {
    let mut p = entry.params;
    p.gain.x = gain;
    // the engine's unit lights the colourless terms; in the world the gain does the stage's exposure
    p.gain.y /= gain;
    let c = base.base_color.to_linear();
    p.back_albedo = (Vec3::splat(entry.diffuse) / Vec3::new(c.red, c.green, c.blue).max(Vec3::splat(1e-4))).extend(1.0);
    (p, back)
}

fn swap_materials(
    mut commands: Commands,
    light: Option<Res<CharBackLight>>,
    standard: Res<Assets<StandardMaterial>>,
    mut skin: ResMut<Assets<SkinMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    fresh: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Changed<MeshMaterial3d<StandardMaterial>>>,
) {
    for (entity, handle) in &fresh {
        let Some(material) = standard.get(&handle.0) else { continue };
        let Some(entry) = entry_of(material) else { continue };
        let id = handle.0.id();
        let shared = mirrors.0.get(&id).and_then(|m| skin.get_strong_handle(*m));
        let mirror = shared.unwrap_or_else(|| {
            let (back, gain) = lighting(light.as_deref());
            let base = base_of(material);
            let (params, back) = params_for(&entry, &base, back, gain);
            let strong = skin.add(SkinMaterial {
                base,
                extension: SkinExt { params, back, gloss_map: entry.gloss_map.clone(), sss_map: entry.sss_map.clone() },
            });
            mirrors.0.insert(id, strong.id());
            strong
        });
        commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(mirror));
    }
}

/// The least time between two uploads of the back lights, in seconds (the time of day moves slowly).
const UPLOAD_INTERVAL: f32 = 0.5;

/// Pushes the back lights and the gain to every [`SkinMaterial`] when they changed.
fn upload_lights(
    light: Option<Res<CharBackLight>>,
    time: Res<Time<Real>>,
    mut skin: ResMut<Assets<SkinMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    mut sent: Local<Option<(BackLightParams, f32, f32)>>,
) {
    let (back, gain) = lighting(light.as_deref());
    let now = time.elapsed_secs();
    if sent.as_ref().is_some_and(|(b, g, at)| (*b == back && *g == gain) || (*g == gain && now - at < UPLOAD_INTERVAL)) {
        return;
    }
    *sent = Some((back, gain, now));
    mirrors.0.retain(|_, m| skin.contains(*m));
    let ids: Vec<_> = skin.ids().collect();
    for id in ids {
        let Some(m) = skin.get(id) else { continue };
        let old = m.extension.params.gain.x;
        if m.extension.back == back && old == gain {
            continue;
        }
        if let Some(mut m) = skin.get_mut(id) {
            m.extension.back = back;
            if old != gain {
                m.extension.params.gain.y *= old / gain;
                m.extension.params.gain.x = gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `nu_m_nude.mtl` and the face of `nu_m_face00.mtl`
    const BODY: &str = r#"<Material Shader="HumanSkin" GenMask="200000a002" Diffuse="0.086,0.086,0.063" Specular="0.153,0.286,0.471" Shininess="25">
 <PublicParams MSkinColor="0.592,0.561,0.435" BSkinColor="0.976,0.8,0.741" RimPow="1.3" DiffusionAmount="1.5" SubsurfaceStrenght="1" SSSMapSat="0.5"/></Material>"#;
    const FACE: &str = r#"<Material Shader="HumanSkin" GenMask="2002" Diffuse="0.204,0.204,0.204" Specular="0.459,0.357,0.306" Shininess="65">
 <PublicParams MSkinColor="0.894,0.894,0.894" DiffusionAmount="1.2" SubsurfaceStrenght="0" SSSMapSat="0.2" RimPow="0.4" BSkinColor="0.878,0.878,0.878"/></Material>"#;

    #[test]
    fn the_skin_material_is_bindless_like_the_standard_material() {
        assert!(<SkinMaterial as AsBindGroup>::bindless_slot_count().is_some());
        let d = <SkinMaterial as AsBindGroup>::bindless_descriptor().expect("a bindless descriptor");
        assert_eq!(d.index_tables.len(), 2);
    }

    #[test]
    fn parameters_and_gloss_modes_come_from_the_mtl() {
        let body = SkinParams::from_material(&CryMaterial::parse(BODY).expect("mtl"), true, true);
        // stored b, g, r
        assert!((body.bright.x - 0.741).abs() < 1e-6 && (body.bright.z - 0.976).abs() < 1e-6);
        assert_eq!((body.bright.w, body.middle.w, body.spec.w, body.shape.x), (1.5, 1.0, 25.0, 1.3));
        // GLOSS_MAP_IGNORE_SPECULAR_COLOR: gloss.r masks, gloss.b whitens
        assert_eq!((body.shape.z, body.mode.x, body.mode.y, body.mode.z), (1.0, 0.0, 1.0, 1.0));
        let face = SkinParams::from_material(&CryMaterial::parse(FACE).expect("mtl"), true, false);
        assert_eq!((face.spec.w, face.shape.w, face.mode.y, face.mode.z), (65.0, 0.0, 0.0, 0.0));
        // no gloss map: the diffuse alpha (shader default), mode off
        assert_eq!(SkinParams::from_material(&CryMaterial::parse(FACE).expect("mtl"), false, false).shape.z, 0.0);
    }

    #[test]
    fn tags_are_neither_back_light_nor_hair_tags_and_round_trip() {
        let m = CryMaterial::parse(FACE).expect("mtl");
        let params = SkinParams::from_material(&m, false, false);
        let index = registry(|r| {
            r.entries.push(Entry { params, diffuse: 0.204, gloss_map: None, sss_map: None });
            r.entries.len() - 1
        });
        let tagged = StandardMaterial { lightmap_exposure: TAG_BASE + index as f32, base_color: Color::linear_rgb(0.4, 0.3, 0.3), ..default() };
        let e = entry_of(&tagged).expect("entry");
        assert_eq!(e.params, params);
        assert!(entry_of(&StandardMaterial::default()).is_none());
        assert!(entry_of(&StandardMaterial { lightmap_exposure: super::super::backlight::skin_tag(0.2), ..default() }).is_none());
        assert!(entry_of(&StandardMaterial { lightmap_exposure: 4.0 + 7.0, ..default() }).is_none());
        let b = base_of(&tagged);
        assert_eq!((b.reflectance, b.lightmap_exposure, b.diffuse_transmission), (0.0, 1.0, 0.0));
        // the world gain scales everything but the back lights, the fill keeps the stage's light unit
        let (stage, _) = params_for(&e, &b, BackLightParams::default(), 1.0);
        let (world, _) = params_for(&e, &b, BackLightParams::default(), WORLD_DIFFUSE_GAIN);
        assert!((stage.gain.y - world.gain.y * world.gain.x).abs() < 1e-3 * stage.gain.y);
        assert!((stage.back_albedo.x - 0.204 / 0.4).abs() < 1e-5);
    }
}
