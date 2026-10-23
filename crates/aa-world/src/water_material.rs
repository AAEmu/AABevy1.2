//! The water material (`water.wesl`): ponds, lakes, rivers and the sea.
//!
//! Every distinct [`WaterStyle`] (material path + the volume's own fog) gets one [`WaterMaterial`], built from the `.mtl`
//! of the volume (`Watersurface` shader: `BumpScale`, `BigWavesScale`, `SmallWavesScale`, `WavesSpeed`, `FresnelBias`,
//! `ReflectionAmount`, `TransparencyRatio`, `SunMultiplier`, `SunShinePow`, `SoftIntersectionFactor`, `FoamMultiplier`,
//! `FoamIntersectionFactor`, bump map `oceanwaves_ddn`) and the fog of the volume (colour and density, `object.dat`
//! type 11 header). The environment part of the uniform (sky colours for the reflection, sun, body colour, foam colour)
//! is refreshed by [`update_water`] from the sampled time of day.
//!
//! The material reads the frame's opaque colour (`reads_view_transmission_texture`) and the depth prepass of the camera
//! (`post::setup_post_cameras` adds it): without the prepass the water has a constant thickness and no screen space
//! reflections.

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::camera::Exposure;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureViewDimension,
};
use bevy::shader::ShaderRef;

use crate::coords::cry_to_bevy;
use crate::post::PostUniform;
use crate::sky::{AtmosphereTuning, Environment, view_exposure};
use crate::water::WaterStyle;
use crate::water::ocean::OceanParams;

/// `WaterParams.motion.x`: authored UV/raw-bump path with the existing reflection fallback.
pub const AUTHORED_WATER_UV: f32 = 1.0;
/// Authored UV plus source-qualified, successfully loaded static environment cube.
pub const AUTHORED_WATER_CUBE: f32 = 2.0;
/// Authored UV plus a source-qualified ordinary 2D environment texture (TT3_TCUBE absent).
pub const AUTHORED_WATER_ENVIRONMENT_2D: f32 = 3.0;

// Private encoding, not a new authored motion mode. Existing 0/1/2/3 consumers
// use positive comparisons; -1 retains the sea's old normals/UVs above water.
const OCEAN_UNDERSIDE: f32 = -1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct OceanUnderSurface {
    refraction: f32,
    soft: f32,
    bias: f32,
    power: f32,
}

impl OceanUnderSurface {
    fn parse(text: &str) -> Option<Self> {
        let m = aa_cry::material::Material::parse(text).ok()?;
        // Only the non-tessellated, non-volume sea branch with the same loaded
        // bump source. Other permutations retain their previous underside.
        if !m.shader.eq_ignore_ascii_case("Water") || m.gen_mask & 1 == 0
            || m.gen_mask & !(0x1 | 0x20 | 0x400) != 0
            || m.texture(&aa_cry::material::TextureMap::Bump)?.pak_path() != DEFAULT_NORMAL_MAP
        { return None; }
        let p = |name: &str, max: f32| {
            m.public_param(name)?.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0 && *v <= max)
        };
        Some(Self { refraction: p("RefractionBumpScale",1.0)?, soft: p("SoftIntersectionFactor",1000.0)?,
            bias: p("FresnelBias",1.0)?, power: p("FresnelPower",5.0)? })
    }

    fn load(world: &str) -> Option<Self> {
        let env = aa_pak::Pak::global().read_string(&format!("worlds/{world}/env.xml")).ok()?;
        let doc = roxmltree::Document::parse(&env).ok()?;
        let path = doc.root_element().children().find(|n| n.has_tag_name("Ocean"))?.attribute("Material")?;
        let path = format!("{}.mtl",path.trim().trim_start_matches("game/").trim_end_matches(".mtl"));
        Self::parse(&aa_pak::Pak::global().read_string(&path).ok()?)
    }
}

/// Uniform of `water.wesl` (see the shader for the meaning of the fields).
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct WaterParams {
    pub body: Vec4,
    pub refl: Vec4,
    pub waves: Vec4,
    pub sun: Vec4,
    pub sun_dir: Vec4,
    pub sky_h: Vec4,
    pub sky_z: Vec4,
    pub misc: Vec4,
    pub foam: Vec4,
    pub foam_col: Vec4,
    pub env_lit: Vec4,
    /// Authored mesh UVs, native NO_FLOW, optional diagnostic time (-1 live), diagnostic output (1 normal /2 alpha).
    /// Private qualified sea x=-1: y is source RefractionBumpScale, not NO_FLOW.
    pub motion: Vec4,
    /// Sea only (`water.cfx` FOAM): FoamSoftIntersectionFactor, FoamCoverFactor, FoamCoverOffset, FoamCoverBlendWeight.
    pub ocean_foam: Vec4,
    /// Sea only: foam repeats per metre, foam drift per second (xy), gloss map repeats per metre.
    pub ocean_uv: Vec4,
    /// Sea only: rgb sun colour times saturate(FoamAmount) (pre-exposed), a: 1 = ocean foam on.
    pub ocean_sun: Vec4,
    /// Sea only: rgb sky colour times saturate(FoamAmount) (pre-exposed).
    pub ocean_sky: Vec4,
    /// Sea only (deferred caustics): rgb sun colour x 0.5 x CausticIntensity (pre-exposed; zero = off), a: CausticDepth.
    pub caustic_col: Vec4,
    /// Sea only: x CausticsTilling, y caustics distance (r_watercausticsdistance).
    /// Private qualified sea: zw source FresnelBias/Power, guarded by motion.x=-1.
    pub caustics: Vec4,
    /// Sea only: the swell (vertex displacement), x peak height in metres (0 = flat), y repeat length of the base wave, z speed
    /// factor, w wind direction (radians, Cry XY).
    pub ocean_wave: Vec4,
}

impl Default for WaterParams {
    fn default() -> Self {
        WaterParams {
            body: Vec4::new(0.02, 0.04, 0.05, 0.6),
            refl: Vec4::new(0.05, 1.0, 0.35, 100.0),
            waves: Vec4::new(1.875, 0.85, -1.5, 0.2),
            sun: Vec4::new(0.0, 0.0, 0.0, 0.5),
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            sky_h: Vec4::new(0.3, 0.35, 0.4, 1.0),
            sky_z: Vec4::new(0.1, 0.15, 0.3, 0.0),
            misc: Vec4::new(1.0, 0.012, 0.05, 1.0),
            foam: Vec4::new(0.25, 2.0, 0.0, 0.0),
            foam_col: Vec4::ZERO,
            env_lit: Vec4::ZERO,
            motion: Vec4::new(0.0,0.0,-1.0,0.0),
            ocean_foam: Vec4::ZERO,
            ocean_uv: Vec4::ZERO,
            ocean_sun: Vec4::ZERO,
            ocean_sky: Vec4::ZERO,
            caustic_col: Vec4::ZERO,
            caustics: Vec4::ZERO,
            ocean_wave: Vec4::ZERO,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct WaterMaterial {
    #[uniform(0)]
    pub params: WaterParams,
    #[texture(1)]
    #[sampler(2)]
    pub normal_map: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub foam_map: Option<Handle<Image>>,
    #[texture(5, dimension = "cube")]
    #[sampler(6)]
    pub env_map: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub environment_2d: Option<Handle<Image>>,
    /// Sea only: `textures/defaults/water_gloss.dds` (the ocean's foam threshold mask).
    #[texture(9)]
    #[sampler(10)]
    pub gloss_map: Option<Handle<Image>>,
    /// Sea only: `textures/defaults/caustics_sampler.dds`.
    #[texture(11)]
    #[sampler(12)]
    pub caustics_map: Option<Handle<Image>>,
}

impl Material for WaterMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/water.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/water.wesl".into()
    }

    fn reads_view_transmission_texture(&self) -> bool {
        true
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

/// The constants of a style (from its `.mtl` and the volume's fog); the environment part is added each frame.
#[derive(Clone, Debug, PartialEq)]
struct StyleBase {
    fog: Vec3,
    density: f32,
    sun_multiplier: f32,
    shine: f32,
    bump: f32,
    big: f32,
    small: f32,
    speed: f32,
    fresnel_bias: f32,
    reflection: f32,
    transparency: f32,
    soft: f32,
    foam: f32,
    foam_reach: f32,
    repeat: f32,
    /// Pak path of the environment cube map the surface reflects.
    env_map: String,
    /// The sea plane: its colour comes from the zone's ocean fog.
    sea: bool,
    native_uv: bool,
    no_flow: bool,
    native_cube: bool,
    native_environment_2d: bool,
    sun_shading: bool,
    /// The sea's `env.xml` / ocean material inputs (foam, caustics); `None` for volumes and a sea without them.
    ocean: Option<OceanParams>,
    under_surface: Option<OceanUnderSurface>,
}

fn static_cube_candidate(material: &aa_cry::material::Material) -> bool {
    material.shader.eq_ignore_ascii_case("WaterSurface")
        && material.gen_mask & 0x85 == 0x5 // BUMP_MAP + ENVIRONMENT_MAP; no REALTIME_REFLECTIONS.
        && material.texture(&aa_cry::material::TextureMap::Environment)
            .is_some_and(|t| !t.pak_path().is_empty())
}

impl StyleBase {
    fn of(style: &WaterStyle) -> (StyleBase, String) {
        let mut base = StyleBase {
            fog: Vec3::from(style.fog_color()),
            density: style.fog_density(),
            sun_multiplier: 0.1,
            shine: 100.0,
            bump: 0.35,
            big: 1.875,
            small: 0.85,
            speed: -1.5,
            fresnel_bias: 0.2,
            reflection: 1.0,
            transparency: 1.0,
            soft: 0.5,
            foam: 0.75,
            foam_reach: 0.25,
            repeat: 0.2,
            env_map: DEFAULT_ENV_MAP.to_string(),
            sea: false,
            native_uv: false,
            no_flow: false,
            native_cube: false,
            native_environment_2d: false,
            sun_shading: false,
            ocean: None,
            under_surface: None,
        };
        let mut normal_map = DEFAULT_NORMAL_MAP.to_string();
        if style.material == SEA {
            base.sea = true;
            base.repeat = 0.05;
            base.bump = 0.5;
            base.big = 2.0;
            base.small = 1.0;
            base.speed = -4.0;
            base.fresnel_bias = 0.1;
            base.soft = 0.25;
            base.sun_multiplier = 0.3;
            base.shine = 200.0;
            return (base, normal_map);
        }
        if style.material.is_empty() {
            return (base, normal_map);
        }
        let Ok(bytes) = aa_cry::read(&format!("{}.mtl", style.material.trim_end_matches(".mtl"))) else {
            return (base, normal_map);
        };
        let text = String::from_utf8_lossy(&bytes);
        let Ok(mtl) = aa_cry::material::Material::parse(&text) else { return (base, normal_map) };
        base.native_uv = mtl.shader.eq_ignore_ascii_case("WaterSurface") && mtl.gen_mask & 1 != 0;
        base.no_flow = base.native_uv && mtl.gen_mask & 0x200 != 0;
        base.sun_shading = mtl.gen_mask & 0x2000 != 0;
        base.native_cube = static_cube_candidate(&mtl);
        base.native_environment_2d = base.native_cube && plain_environment_sampler(&text);
        let param = |name: &str, default: f32| mtl.public_param(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        base.sun_multiplier = param("SunMultiplier", base.sun_multiplier);
        base.shine = param("SunShinePow", base.shine).max(1.0);
        base.bump = param("BumpScale", base.bump);
        base.big = param("BigWavesScale", base.big);
        base.small = param("SmallWavesScale", base.small);
        base.speed = param("WavesSpeed", base.speed);
        base.fresnel_bias = param("FresnelBias", base.fresnel_bias);
        base.reflection = param("ReflectionAmount", base.reflection);
        base.transparency = param("TransparencyRatio", base.transparency);
        base.soft = param("SoftIntersectionFactor", base.soft);
        base.foam = param("FoamMultiplier", base.foam);
        base.foam_reach = param("FoamIntersectionFactor", base.foam_reach);
        base.apply_surface_features(mtl.gen_mask);
        if let Some(t) = mtl.texture(&aa_cry::material::TextureMap::Bump) {
            let path = t.pak_path();
            if !path.is_empty() {
                normal_map = path;
            }
        }
        if let Some(t) = mtl.texture(&aa_cry::material::TextureMap::Environment) {
            let path = t.pak_path();
            if !path.is_empty() {
                base.env_map = path;
            }
        }
        (base, normal_map)
    }

    fn apply_surface_features(&mut self, mask: u64) {
        if !self.native_uv { return; }
        // WaterSurfacePS478/513: shader feature eligibility is independent of
        // authored multiplier sign/value. Preserve every other style parameter.
        if mask & (0x20 | 0x2000) == 0 { self.sun_multiplier = 0.0; }
        if mask & 0x1000 == 0 { self.foam = 0.0; }
    }

    fn surface_light(&self, sample: &crate::env::EnvSample) -> Vec3 {
        // WaterSurfacePS377 /438 /529. Restrict this slice to actually admitted
        // static images; realtime/missing/mode1 and sea keep their calibrated path.
        if self.native_cube || self.native_environment_2d {
            if self.sun_shading { sample.native_sun_sky } else { Vec3::ONE }
        } else { sample.sun + sample.sky }
    }

    fn motion_mode(&self) -> f32 {
        if self.sea && self.under_surface.is_some() { OCEAN_UNDERSIDE }
        else if self.native_cube { AUTHORED_WATER_CUBE }
        else if self.native_environment_2d { AUTHORED_WATER_ENVIRONMENT_2D }
        else if self.native_uv { AUTHORED_WATER_UV }
        else { 0.0 }
    }

    fn motion(&self) -> Vec4 {
        Vec4::new(self.motion_mode(),self.under_surface.map_or(if self.no_flow {1.0} else {0.0},|p| p.refraction),-1.0,0.0)
    }
}

const DEFAULT_NORMAL_MAP: &str = "textures/defaults/oceanwaves_ddn.dds";
const FOAM_MAP: &str = "textures/defaults/water_foam.dds";
/// `water.cfx` FoamSampler (`textures/defaults/WaterFoam.dds`, red channel) and WaterGlossMapSampler.
const OCEAN_FOAM_MAP: &str = "textures/defaults/waterfoam.dds";
const OCEAN_GLOSS_MAP: &str = "textures/defaults/water_gloss.dds";
/// `deferredcaustics.cfx` causticsSampler (LINEAR, no mips, CLAMP).
const CAUSTICS_MAP: &str = "textures/defaults/caustics_sampler.dds";
/// CryEngine `r_WaterCausticsDistance` default (CausticParams.x); the 1.2 value is not read from the client.
const CAUSTICS_DISTANCE: f32 = 100.0;
/// The cube map nearly every water material reflects, and the fallback for the others.
const DEFAULT_ENV_MAP: &str = "textures/cubemaps/camera_river_cubemap.dds";
/// Material name of the sea plane's style.
pub const SEA: &str = "<sea>";

/// All water materials of the world, by style.
#[derive(Resource, Default)]
pub struct WaterStyles {
    materials: HashMap<WaterStyle, Handle<WaterMaterial>>,
    bases: Vec<(Handle<WaterMaterial>, StyleBase)>,
    normal_maps: HashMap<String, Option<Handle<Image>>>,
    foam_map: Option<Option<Handle<Image>>>,
    // Image plus whether the authored source itself loaded as a cube (not the fallback).
    env_maps: HashMap<String, Option<(Handle<Image>, bool)>>,
    environment_2d: HashMap<String, Option<Handle<Image>>>,
}

// Native default profile: ordinary static 2D images, identity coordinates and no
// material sampler/view override. aa-cry does not expose those raw attributes.
fn plain_environment_sampler(text: &str) -> bool {
    let Ok(doc) = roxmltree::Document::parse(text) else { return false };
    let root = doc.root_element();
    if root.attributes().any(|a| a.name().eq_ignore_ascii_case("sRGB")) { return false; }
    let Some(texture) = root.children().find(|n| n.has_tag_name("Textures"))
        .and_then(|n| n.children().find(|n| n.has_tag_name("Texture") && n.attribute("Map").is_some_and(|s| s.eq_ignore_ascii_case("Environment"))))
    else { return false };
    texture.attributes().all(|a| matches!(a.name(), "Map" | "File"))
        && texture.children().filter(|n| n.is_element())
            .all(|n| n.has_tag_name("TexMod") && n.attributes().len() == 0 && !n.children().any(|c| c.is_element()))
}

const MAX_ENVIRONMENT_2D: usize = 16;

/// Raw BC blocks/mips and the native default SRV: CRYF 0x800 selects sRGB,
/// independently of sampler.sRGBLookup=0 (38100230 / 380FB9C0).
fn prepare_environment_2d(bytes: &[u8]) -> Option<Image> {
    use aa_cry::texture::{Dds, DdsFormat, DdsOptions, dds_to_image};
    let dds = Dds::parse(bytes).ok()?;
    if dds.faces != 1 || dds.depth != 1 || dds.mips_truncated
        || dds.width > 1024 || dds.height > 1024
        || u64::from(dds.width) * u64::from(dds.height) > 512 * 512
        || !matches!(dds.format, DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3)
    { return None; }
    let word = |at| u32::from_le_bytes(bytes[at..at+4].try_into().unwrap());
    if word(36) & 0x1000000 != 0 { return None; } // Native normal-map image variants differ.
    let srgb = word(4) >= 124 && word(124) == 0x43525946 && word(36) & 0x800 != 0;
    let mut image = dds_to_image(&dds, DdsOptions { srgb, bc_supported: true, repeat: true }).ok()?;
    // Cvar default TRILINEAR / anisotropy1. Do not inherit aa-cry's aniso4 preset.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 1,
        ..default()
    });
    Some(image)
}

/// A cube map from the pak (the default one when the file is missing or not a cube map).
fn load_cube(path: &str, images: &mut Assets<Image>) -> Option<(Handle<Image>, bool)> {
    let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: false };
    let is_cube = |i: &Image| i.texture_view_descriptor.as_ref().is_some_and(|d| d.dimension == Some(TextureViewDimension::Cube));
    match aa_cry::texture::load_dds_from_pak(path, opts) {
        Ok(image) if is_cube(&image) => return Some((images.add(image), true)),
        Ok(_) => warn!("aa-world: water environment map {path} is not a cube map"),
        Err(e) => warn!("aa-world: water environment map {path}: {e}"),
    }
    if path == DEFAULT_ENV_MAP {
        return None;
    }
    aa_cry::texture::load_dds_from_pak(DEFAULT_ENV_MAP, opts).ok().filter(is_cube).map(|i| (images.add(i), false))
}

impl WaterStyles {
    /// The material of a style, created on first use.
    pub fn material(
        &mut self,
        style: &WaterStyle,
        assets: &mut Assets<WaterMaterial>,
        images: &mut Assets<Image>,
    ) -> Handle<WaterMaterial> {
        if let Some(h) = self.materials.get(style) {
            return h.clone();
        }
        let (mut base, normal_path) = StyleBase::of(style);
        let normal_map = self
            .normal_maps
            .entry(if base.native_uv {format!("@raw:{normal_path}")} else {normal_path.clone()})
            .or_insert_with(|| {
                let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: true };
                let loaded = if base.native_uv {
                    aa_cry::read(&normal_path).and_then(|bytes| {
                        let mut dds=aa_cry::texture::Dds::parse(&bytes)?;
                        // WaterSurfacePS reads raw texture XY, not Common.cfi GetNormalMap.
                        dds.swapped_rg=false;
                        aa_cry::texture::dds_to_image(&dds,opts)
                    })
                } else { aa_cry::texture::load_dds_from_pak(&normal_path,opts) };
                match loaded {
                    Ok(image) => Some(images.add(image)),
                    Err(e) => {
                        warn!("aa-world: water normal map {normal_path}: {e}");
                        None
                    }
                }
            })
            .clone();
        let foam_map = self
            .foam_map
            .get_or_insert_with(|| {
                let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: true };
                match aa_cry::texture::load_dds_from_pak(FOAM_MAP, opts) {
                    Ok(image) => Some(images.add(image)),
                    Err(e) => {
                        warn!("aa-world: water foam map {FOAM_MAP}: {e}");
                        None
                    }
                }
            })
            .clone();
        let environment_2d = if base.native_environment_2d {
            let known_cube = self.env_maps.get(&base.env_map).and_then(|e| e.as_ref()).is_some_and(|(_, authored)| *authored);
            if !known_cube && !self.environment_2d.contains_key(&base.env_map) && self.environment_2d.len() < MAX_ENVIRONMENT_2D {
                let bytes = aa_cry::read(&base.env_map).ok();
                let cube = bytes.as_ref().and_then(|b| aa_cry::texture::Dds::parse(b).ok()).is_some_and(|dds| dds.faces == 6);
                // A successful cube belongs to the existing cube cache. Do not
                // let ordinary cubes exhaust the new 2D admission budget.
                if !cube {
                    let image = bytes.as_ref().and_then(|b| prepare_environment_2d(b)).map(|image| images.add(image));
                    self.environment_2d.insert(base.env_map.clone(), image);
                }
            }
            self.environment_2d.get(&base.env_map).cloned().flatten()
        } else { None };
        base.native_environment_2d &= environment_2d.is_some();
        // Keep the existing cube available to optional SSR/fallback paths. All
        // supported 2D maps share the same fallback instead of duplicating it.
        let cube_path = if base.native_environment_2d { DEFAULT_ENV_MAP } else { &base.env_map };
        let cube = self.env_maps.entry(cube_path.to_owned()).or_insert_with(|| load_cube(cube_path, images)).clone();
        base.native_cube &= !base.native_environment_2d && cube.as_ref().is_some_and(|(_, authored)| *authored);
        let env_map = cube.map(|(image, _)| image);
        let params = WaterParams { waves: Vec4::new(base.big, base.small, base.speed, base.repeat), motion: base.motion(), ..default() };
        let handle = assets.add(WaterMaterial { params, normal_map, foam_map, env_map, environment_2d, gloss_map: None, caustics_map: None });
        self.materials.insert(style.clone(), handle.clone());
        self.bases.push((handle.clone(), base));
        handle
    }

    /// The sea plane's material: the [`SEA`] style plus the world's ocean foam and caustics (see [`OceanParams`]).
    pub fn ocean(&mut self, world: &str, assets: &mut Assets<WaterMaterial>, images: &mut Assets<Image>) -> Handle<WaterMaterial> {
        let handle = self.material(&WaterStyle::new(SEA, 0, [0.0; 3], 0.05), assets, images);
        // Never retain source qualification from a previous world on load failure.
        if let Some((_,base)) = self.bases.iter_mut().find(|(h,_)| *h == handle) { base.under_surface = None; }
        let Some(params) = OceanParams::load(world) else { return handle };
        let under_surface = assets.get(&handle).filter(|m| m.normal_map.is_some())
            .and_then(|_| OceanUnderSurface::load(world));
        let load = |path: &str, repeat: bool, images: &mut Assets<Image>| {
            let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat };
            match aa_cry::texture::load_dds_from_pak(path, opts) {
                Ok(image) => Some(images.add(image)),
                Err(e) => { warn!("aa-world: ocean texture {path}: {e}"); None }
            }
        };
        let foam = load(OCEAN_FOAM_MAP, true, images);
        let gloss = load(OCEAN_GLOSS_MAP, true, images);
        let caustics = load(CAUSTICS_MAP, false, images);
        if let Some(mut m) = assets.get_mut(&handle) {
            if foam.is_some() { m.foam_map = foam; }
            m.gloss_map = gloss;
            m.caustics_map = caustics;
        }
        if let Some((_, base)) = self.bases.iter_mut().find(|(h, _)| *h == handle) {
            base.ocean = Some(params);
            base.under_surface = under_surface;
        }
        handle
    }

    pub(crate) fn ocean_underside_supported(&self) -> bool {
        self.bases.iter().any(|(_,base)| base.sea && base.under_surface.is_some())
    }

    pub fn len(&self) -> usize {
        self.bases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bases.is_empty()
    }

    /// Source style of a cached material, for bounded inspection/control fixtures.
    pub fn style_for(&self, material: bevy::asset::AssetId<WaterMaterial>) -> Option<&WaterStyle> {
        self.materials.iter().find_map(|(style, handle)| (handle.id() == material).then_some(style))
    }
}

/// The water shader, also available to isolated native sampling fixtures.
pub struct WaterMaterialPlugin;
impl Plugin for WaterMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app,"water.wesl");
        bevy::shader::load_shader_library!(app,"water_motion.wesl");
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_plugins(WaterMaterialPlugin).init_resource::<WaterStyles>();
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The colour of the deep water of the sea (pre-exposed): the zone's ocean fog lit by sky and sun.
pub(crate) fn sea_color(env: &Environment, cur: &crate::env::EnvSample, tuning: &AtmosphereTuning, rs: f32) -> Vec3 {
    let light = (luminance(cur.sky) + 0.25 * luminance(cur.sun)) * 0.35;
    env.ocean_fog * tuning.sea_body * light * rs
}

/// The colour of the deep water of a pond, lake or river (pre-exposed): the volume's fog colour lit by sky and sun.
pub(crate) fn volume_color(fog: Vec3, cur: &crate::env::EnvSample, tuning: &AtmosphereTuning, rs: f32) -> Vec3 {
    fog * (cur.sky + cur.sun * tuning.water_lit) * tuning.water_body * rs
}

/// Refreshes the environment dependent part of every water material: sky for the reflections, the sun, the body colour.
pub fn update_water(
    env: Res<Environment>,
    tuning: Res<AtmosphereTuning>,
    styles: Res<WaterStyles>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    let Some(cur) = &env.sample else { return };
    let rs = tuning.radiance * view_exposure(&exposure);
    let sun_bevy = cry_to_bevy(cur.sun_dir).normalize();
    let sun_up = ((cur.sun_dir.z + 0.04) / 0.1).clamp(0.0, 1.0);
    let sky_h = (cur.horizon * rs * tuning.sky_gain).extend(tuning.sky_exp);
    let sky_z = (cur.zenith * rs * tuning.sky_gain).extend(0.0);
    for (handle, base) in &styles.bases {
        let foam_light = base.surface_light(cur) * rs;
        let density = if base.density > 0.0 { base.density } else { 0.2 };
        let (body, f0, weight, amount, foam, sun_gain, ssr, cube) = if base.sea {
            // WaterPS: exp2(-cFogColorDensity.w * depth), w = the zone's `<Ocean FogDensity>`.
            let density = if env.ocean_density > 0.0 { env.ocean_density } else { density };
            let body = sea_color(&env, cur, &tuning, rs).extend(density * tuning.sea_absorb);
            (body, (base.fresnel_bias * 0.25).clamp(0.02, 0.08), tuning.sea_refl, 1.0, Vec3::ZERO, tuning.sea_sun, tuning.ssr, 0.0)
        } else {
            // the volume fog: constant colour, `exp2(-density * thickness)`
            let body = volume_color(base.fog, cur, &tuning, rs).extend(density * tuning.water_absorb);
            let foam = foam_light * base.foam * tuning.water_foam;
            let amount = base.reflection * tuning.water_env;
            (body, base.fresnel_bias, base.transparency * tuning.water_refl, amount, foam, tuning.water_sun, tuning.water_ssr, 1.0)
        };
        let mut ocean = [Vec4::ZERO; 7];
        if base.sea && let Some(o) = &base.ocean {
            let [dx, dy] = o.foam_drift();
            let foam = o.foam_amount * tuning.sea_foam * rs;
            ocean = [
                Vec4::new(o.foam_soft, o.foam_cover_factor, o.foam_cover_offset, o.foam_cover_blend),
                Vec4::new(o.foam_scale(), dx, dy, 0.005 * o.tilling * 0.05),
                (cur.sun * foam).extend(if foam > 0.0 { 1.0 } else { 0.0 }),
                (cur.sky * foam).extend(0.0),
                // CausticDepth 0 switches the caustics (light and darkening) off.
                (cur.sun * rs * 0.5 * o.caustic_intensity * tuning.sea_caustics)
                    .extend(if o.caustic_intensity > 0.0 && tuning.sea_caustics > 0.0 { o.caustic_depth } else { 0.0 }),
                Vec4::new(o.caustic_tilling, CAUSTICS_DISTANCE, 0.0, 0.0),
                Vec4::from(o.swell()) * Vec4::new(tuning.sea_waves, 1.0, 1.0, 1.0),
            ];
        }
        // Existing sea-unused lanes; read only behind the exact negative tag
        // AND the depth/transmission guard. Public uniform layout stays fixed.
        if let Some(p) = base.under_surface { ocean[5].z = p.bias; ocean[5].w = p.power; }
        let params = WaterParams {
            body,
            refl: Vec4::new(f0, weight, base.bump, base.shine),
            waves: Vec4::new(base.big, base.small, base.speed, base.repeat),
            sun: (cur.sun * rs * base.sun_multiplier * sun_gain).extend(base.soft),
            sun_dir: sun_bevy.extend(sun_up * cur.day.max(0.05)),
            sky_h,
            sky_z,
            misc: Vec4::new(ssr, tuning.refraction, tuning.water_rough, amount),
            foam: Vec4::new(base.foam_reach, base.speed, tuning.water_lod, base.under_surface.map_or(0.0,|p| p.soft)),
            foam_col: foam.extend(0.0),
            env_lit: foam_light.extend(cube),
            motion: base.motion(),
            ocean_foam: ocean[0],
            ocean_uv: ocean[1],
            ocean_sun: ocean[2],
            ocean_sky: ocean[3],
            caustic_col: ocean[4],
            caustics: ocean[5],
            ocean_wave: ocean[6],
        };
        let unchanged = materials.get(handle).is_none_or(|m| m.params == params);
        if !unchanged && let Some(mut m) = materials.get_mut(handle) {
            m.params = params;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_underside_requires_the_original_branch_and_bounded_source_bundle() {
        let xml = |mask: &str, params: &str| format!(r#"<Material MtlFlags="526336" Shader="Water" GenMask="{mask}">
            <Textures><Texture Map="Bumpmap" File="textures/defaults/oceanwaves_ddn.dds"/></Textures>
            <PublicParams {params}/></Material>"#);
        let params = r#"RefractionBumpScale="0.05" SoftIntersectionFactor="0.2" FresnelBias="0.05" FresnelPower="4""#;
        let supported = xml("421",params);
        assert_eq!(OceanUnderSurface::parse(&supported),Some(OceanUnderSurface {refraction:0.05,soft:0.2,bias:0.05,power:4.0}));
        for mask in ["0","420","425","431","c21","521","429"] {
            assert_eq!(OceanUnderSurface::parse(&xml(mask,params)),None,"unsupported source permutation {mask}");
        }
        for bad in ["NaN","inf","-0.01","1.01"] {
            assert_eq!(OceanUnderSurface::parse(&supported.replace("RefractionBumpScale=\"0.05\"",&format!("RefractionBumpScale=\"{bad}\""))),None);
        }
        for (a,b) in [("FresnelPower=\"4\"","FresnelPower=\"6\""),("Shader=\"Water\"","Shader=\"WaterSurface\""),(DEFAULT_NORMAL_MAP,"missing.dds"),("FresnelBias=\"0.05\"","")] {
            assert_eq!(OceanUnderSurface::parse(&supported.replace(a,b)),None);
        }
        // Reservation cannot collide with authored modes or change NO_FLOW.
        let (mut base,_) = StyleBase::of(&WaterStyle::new(SEA,0,[0.0;3],0.2));
        for (uv,cube,flat,mode) in [(false,false,false,0.0),(true,false,false,1.0),(true,true,false,2.0),(true,false,true,3.0)] {
            base.native_uv=uv; base.native_cube=cube; base.native_environment_2d=flat;
            for no_flow in [false,true] { base.no_flow=no_flow; assert_eq!(base.motion(),Vec4::new(mode,if no_flow {1.0}else{0.0},-1.0,0.0)); }
        }
    }

    #[test]
    fn original_ocean_underside_qualifies_only_after_source_and_bump_load() {
        if !crate::pak_available() {return}
        let source = OceanUnderSurface::load("main_world").unwrap();
        assert_eq!(source,OceanUnderSurface {refraction:0.05,soft:0.2,bias:0.05,power:4.0});
        let mut styles=WaterStyles::default(); let mut materials=Assets::<WaterMaterial>::default(); let mut images=Assets::<Image>::default();
        let handle=styles.ocean("main_world",&mut materials,&mut images);
        assert!(styles.ocean_underside_supported());
        let base=&styles.bases.iter().find(|(h,_)| *h==handle).unwrap().1;
        assert_eq!(base.motion(),Vec4::new(-1.0,0.05,-1.0,0.0));
        assert!(materials.get(&handle).unwrap().normal_map.is_some());
        styles.ocean("missing_ocean_underside_world",&mut materials,&mut images);
        assert!(!styles.ocean_underside_supported(),"failed world source must clear the previous qualification");
        assert_eq!(styles.bases.iter().find(|(h,_)| *h==handle).unwrap().1.motion().x,0.0);
    }

    #[test]
    fn source_ocean_preserves_every_existing_uniform_bit_outside_private_lanes() {
        if !crate::pak_available() {return}
        let mut styles=WaterStyles::default(); let mut materials=Assets::<WaterMaterial>::default(); let mut images=Assets::<Image>::default();
        let handle=styles.ocean("main_world",&mut materials,&mut images);
        let source=styles.bases[0].1.under_surface.take().unwrap();
        let mut app=App::new();
        app.insert_resource(styles).insert_resource(materials)
            .insert_resource(AtmosphereTuning::default())
            .insert_resource(Environment {sample:Some(crate::env::TimeOfDay::default().sample(10.0,crate::env::Lighting::default())),zone:None,hour:10.0,ocean_fog:Vec3::new(0.1,0.2,0.3),ocean_density:0.05})
            .add_systems(Update,update_water);
        app.world_mut().spawn((PostUniform::default(),Exposure::default()));
        app.update();
        let old=app.world().resource::<Assets<WaterMaterial>>().get(&handle).unwrap().params;
        app.world_mut().resource_mut::<WaterStyles>().bases[0].1.under_surface=Some(source);
        app.update();
        let mut new=app.world().resource::<Assets<WaterMaterial>>().get(&handle).unwrap().params;
        assert_eq!((new.motion.x,new.motion.y,new.foam.w,new.caustics.z,new.caustics.w),(-1.0,0.05,0.2,0.05,4.0));
        new.motion=old.motion; new.foam.w=old.foam.w; new.caustics.z=old.caustics.z;new.caustics.w=old.caustics.w;
        let bits=|p:WaterParams| [p.body,p.refl,p.waves,p.sun,p.sun_dir,p.sky_h,p.sky_z,p.misc,p.foam,p.foam_col,p.env_lit,p.motion,
            p.ocean_foam,p.ocean_uv,p.ocean_sun,p.ocean_sky,p.caustic_col,p.caustics,p.ocean_wave].map(|v| v.to_array().map(f32::to_bits));
        assert_eq!(bits(new),bits(old),"all old arithmetic inputs must stay bit-identical");
    }

    #[test]
    fn environment_2d_declines_unproven_sampler_and_coordinate_overrides() {
        let xml = |root: &str, attrs: &str, modifier: &str| format!(
            "<Material {root}><Textures><Texture Map=\"Environment\" File=\"x.dds\" {attrs}>{modifier}</Texture></Textures></Material>"
        );
        assert!(plain_environment_sampler(&xml("", "", "<TexMod/>")));
        for (root, attrs, modifier) in [
            ("sRGB=\"0\"", "", ""),
            ("", "Filter=\"4\"", ""),
            ("", "TexType=\"3\"", ""),
            ("", "IsTileU=\"0\"", ""),
            ("", "", "<TexMod TileU=\"2\"/>"),
            ("", "", "<Unknown/>"),
        ] { assert!(!plain_environment_sampler(&xml(root, attrs, modifier))); }
        assert!(!plain_environment_sampler("invalid"));
    }

    #[test]
    fn original_environment_2d_preserves_blocks_mips_default_view_and_shared_images() {
        use bevy::render::render_resource::TextureFormat;
        if !crate::pak_available() { return }
        let mut styles=WaterStyles::default();
        let mut materials=Assets::<WaterMaterial>::default();
        let mut images=Assets::<Image>::default();
        for (name, srgb) in [("swamp_rescue",false),("rescue_watertank",false),("river",true),("fleet_watervolume",true)] {
            let source=WaterStyle::new(&format!("materials/ocean/{name}"),2,[0.0;3],0.2);
            let handle=styles.material(&source,&mut materials,&mut images);
            let material=materials.get(&handle).unwrap();
            assert_eq!(material.params.motion.x,AUTHORED_WATER_ENVIRONMENT_2D,"{name}");
            let image_id=material.environment_2d.as_ref().unwrap().id();
            let image=images.get(image_id).unwrap();
            let cry=aa_cry::material::Material::load(&format!("{}.mtl",source.material)).unwrap();
            let bytes=aa_cry::read(&cry.texture(&aa_cry::material::TextureMap::Environment).unwrap().pak_path()).unwrap();
            let dds=aa_cry::texture::Dds::parse(&bytes).unwrap();
            assert_eq!(image.data.as_deref().unwrap(),dds.data,"unaltered original BC blocks/mips");
            assert_eq!(image.texture_descriptor.mip_level_count,dds.mips);
            assert_eq!(image.texture_descriptor.format.is_srgb(),srgb);
            assert!(matches!(image.texture_descriptor.format,TextureFormat::Bc1RgbaUnorm|TextureFormat::Bc1RgbaUnormSrgb|TextureFormat::Bc2RgbaUnormSrgb));
            let ImageSampler::Descriptor(sampler)=&image.sampler else {panic!("native sampler")};
            assert_eq!(sampler.address_mode_u,ImageAddressMode::Repeat);
            assert_eq!(sampler.address_mode_v,ImageAddressMode::Repeat);
            assert_eq!(sampler.min_filter,ImageFilterMode::Linear);
            assert_eq!(sampler.mipmap_filter,ImageFilterMode::Linear);
            assert_eq!(sampler.anisotropy_clamp,1);
            let another=WaterStyle::new(&source.material,2,[0.1,0.2,0.3],0.4);
            let another=styles.material(&another,&mut materials,&mut images);
            assert_eq!(materials.get(&another).unwrap().environment_2d.as_ref().unwrap().id(),image_id);
        }
        assert_eq!(styles.environment_2d.len(),4);
        assert_eq!(styles.env_maps.len(),1,"one shared fallback cube");
        let cube_style=WaterStyle::new("materials/ocean/lake_tank",2,[0.0;3],0.2);
        let cube=styles.material(&cube_style,&mut materials,&mut images);
        assert_eq!(materials.get(&cube).unwrap().params.motion.x,AUTHORED_WATER_CUBE);
        assert_eq!(styles.environment_2d.len(),4,"successful cubes cannot consume 2D admissions");
        let bytes=aa_cry::read(DEFAULT_ENV_MAP).unwrap();
        assert!(prepare_environment_2d(&bytes).is_none(),"cubes use the existing path");
        let mut bounded=WaterStyles::default();
        for i in 0..MAX_ENVIRONMENT_2D {bounded.environment_2d.insert(format!("reserved{i}"),None);}
        let h=bounded.material(&WaterStyle::new("materials/ocean/swamp_rescue",2,[0.0;3],0.2),&mut materials,&mut images);
        assert!(materials.get(&h).unwrap().environment_2d.is_none());
        assert_eq!(materials.get(&h).unwrap().params.motion.x,AUTHORED_WATER_UV);
        assert_eq!(bounded.environment_2d.len(),MAX_ENVIRONMENT_2D);
    }

    #[test]
    fn static_native_light_uses_sun_shading_or_unity_without_changing_fallbacks() {
        let (template, _) = StyleBase::of(&WaterStyle::new(SEA, 0, [0.0; 3], 0.0));
        let mut sample = crate::env::TimeOfDay::default().sample(10.0, crate::env::Lighting::default());
        sample.sun = Vec3::new(0.1, 0.2, 0.3);
        sample.sky = Vec3::new(0.4, 0.3, 0.2);
        sample.native_sun_sky = Vec3::new(1.25, 0.75, 0.25);
        for mode in 0..4 {
            for sun_shading in [false, true] {
                let mut base = template.clone();
                base.native_uv = mode != 0;
                base.native_cube = mode == 2;
                base.native_environment_2d = mode == 3;
                base.sun_shading = sun_shading;
                let expected = if mode >= 2 {
                    if sun_shading { sample.native_sun_sky } else { Vec3::ONE }
                } else { sample.sun + sample.sky };
                assert_eq!(base.surface_light(&sample), expected);
                for multiplier in [-2.0, 0.0, 3.0] {
                    assert_eq!(base.surface_light(&sample) * multiplier, expected * multiplier);
                }
            }
        }
    }

    #[test]
    fn native_surface_feature_flags_preserve_signed_gains_and_other_style_fields() {
        let (template, _) = StyleBase::of(&WaterStyle::new(SEA,0,[0.1,0.2,0.3],0.4));
        for native in [false, true] {
            for sun in [-0.5_f32, 0.0, 1.5] {
                for foam in [-2.0_f32, 0.0, 3.0] {
                    for mask in [0,0x20,0x2000,0x1000,0x1020,0x3000] {
                        let mut before=template.clone();
                        before.native_uv=native; before.sun_multiplier=sun; before.foam=foam;
                        let mut actual=before.clone(); actual.apply_surface_features(mask);
                        let mut expected=before;
                        if native && mask & 0x2020 == 0 {expected.sun_multiplier=0.0;}
                        if native && mask & 0x1000 == 0 {expected.foam=0.0;}
                        assert_eq!(actual,expected,"native{native} mask{mask:x} sun{sun} foam{foam}");
                    }
                }
            }
        }
    }

    #[test]
    fn original_authored_water_feature_flags_override_nonzero_public_multipliers() {
        if !crate::pak_available() {return}
        for (path,sun,foam) in [
            ("lake_tank",0.5,0.5),
            ("valley_river_watervolume2",0.30000001,0.2), // Legacy decimal12405 == hex3075.
            ("swamp_rescue",0.5,0.0),
            ("water_plantation_cave",0.0,0.0),
        ] {
            let style=WaterStyle::new(&format!("materials/ocean/{path}"),3,[0.0;3],0.2);
            let (base,_)=StyleBase::of(&style);
            assert!(base.native_uv);
            assert_eq!(base.sun_multiplier,sun); assert_eq!(base.foam,foam);
        }
        for path in [SEA,"materials/ocean/missing_water_feature_test"] {
            let (before,_)=StyleBase::of(&WaterStyle::new(path,0,[0.0;3],0.0));
            assert!(!before.native_uv); let mut after=before.clone();after.apply_surface_features(0);
            assert_eq!(before,after,"unqualified/unknown material retains its original defaults");
        }
    }

    #[test]
    fn static_authored_cubes_require_flags_and_successful_source_load() {
        if !crate::pak_available() { return }
        let mut m = aa_cry::material::Material::load("materials/ocean/lake_tank.mtl").unwrap();
        assert!(static_cube_candidate(&m));
        m.gen_mask |= 0x80;
        assert!(!static_cube_candidate(&m), "realtime reflection is a different source branch");
        m.gen_mask &= !0x84;
        assert!(!static_cube_candidate(&m), "no authored environment map feature");
        let mut images = Assets::<Image>::default();
        let (original, authored) = load_cube(DEFAULT_ENV_MAP, &mut images).unwrap();
        assert!(authored);
        let image = images.get(&original).unwrap();
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        assert_eq!(image.texture_descriptor.mip_level_count, 9);
        let (_, authored) = load_cube("textures/cubemaps/missing_world_test.dds", &mut images).unwrap();
        assert!(!authored, "fallback cubes retain their previous direction path");
        let (mut base, _) = StyleBase::of(&WaterStyle::new("materials/ocean/lake_tank",2,[0.0;3],0.2));
        assert_eq!(base.motion_mode(), AUTHORED_WATER_CUBE);
        base.native_cube = false;
        assert_eq!(base.motion_mode(), AUTHORED_WATER_UV);
    }

    #[test]
    fn original_watersurface_no_flow_is_a_material_flag_not_the_volume_kind() {
        if !crate::pak_available() { return }
        let pond=WaterStyle::new("game/materials/ocean/lake_tank",2,[0.0;3],0.2);
        let river=WaterStyle::new("game/materials/ocean/valley_river_watervolume1",3,[0.0;3],0.2);
        let (p,_)=StyleBase::of(&pond); let (r,_)=StyleBase::of(&river);
        assert!(p.native_uv && p.no_flow && r.native_uv && !r.no_flow);
        assert_eq!((p.speed,r.speed),(1.5,1.0));
        let (sea,_)=StyleBase::of(&WaterStyle::new(SEA,0,[0.0;3],0.2));
        assert!(!sea.native_uv && !sea.no_flow);
    }

    #[test]
    fn style_reads_the_material() {
        if !crate::pak_available() {
            return;
        }
        let style = WaterStyle::new("game/materials/ocean/lilyut_lake01", 3, [0.003, 0.0047, 0.0063], 0.1);
        let (base, normal_map) = StyleBase::of(&style);
        assert!((base.bump - 0.35).abs() < 1e-4 && (base.big - 1.875).abs() < 1e-4 && (base.shine - 100.0).abs() < 1e-3);
        assert_eq!(normal_map, "textures/defaults/oceanwaves_ddn.dds");
        assert!((base.reflection - 0.25).abs() < 1e-4 && (base.transparency - 0.248).abs() < 1e-4);
        assert!((base.foam - 0.5).abs() < 1e-4 && (base.foam_reach - 0.75).abs() < 1e-4);
        assert!(base.env_map.ends_with("camera_river_cubemap.dds"), "{}", base.env_map);
        assert!((base.density - 0.1).abs() < 1e-6);
        // an unknown material keeps the defaults
        let (base, _) = StyleBase::of(&WaterStyle::new("game/materials/ocean/does_not_exist", 2, [0.0; 3], 0.0));
        assert!((base.big - 1.875).abs() < 1e-4);
        // the normal map, the foam map and the environment cube map load
        let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: true };
        assert!(aa_cry::texture::load_dds_from_pak(&normal_map, opts).is_ok());
        assert!(aa_cry::texture::load_dds_from_pak(FOAM_MAP, opts).is_ok());
        let cube = aa_cry::texture::load_dds_from_pak(DEFAULT_ENV_MAP, opts).unwrap();
        assert_eq!(cube.texture_view_descriptor.and_then(|d| d.dimension), Some(TextureViewDimension::Cube));
    }
}
