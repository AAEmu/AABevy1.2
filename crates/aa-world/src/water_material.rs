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
    /// Authored mesh UVs, native NO_FLOW, optional diagnostic time (-1 live), diagnostic normal output.
    pub motion: Vec4,
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
}

impl Material for WaterMaterial {
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
#[derive(Clone, Debug)]
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
        let Ok(mtl) = aa_cry::material::Material::load(&format!("{}.mtl", style.material.trim_end_matches(".mtl"))) else {
            return (base, normal_map);
        };
        base.native_uv = mtl.shader.eq_ignore_ascii_case("WaterSurface") && mtl.gen_mask & 1 != 0;
        base.no_flow = base.native_uv && mtl.gen_mask & 0x200 != 0;
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
}

const DEFAULT_NORMAL_MAP: &str = "textures/defaults/oceanwaves_ddn.dds";
const FOAM_MAP: &str = "textures/defaults/water_foam.dds";
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
    env_maps: HashMap<String, Option<Handle<Image>>>,
}

/// A cube map from the pak (the default one when the file is missing or not a cube map).
fn load_cube(path: &str, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    let opts = aa_cry::texture::DdsOptions { srgb: false, bc_supported: true, repeat: false };
    let is_cube = |i: &Image| i.texture_view_descriptor.as_ref().is_some_and(|d| d.dimension == Some(TextureViewDimension::Cube));
    match aa_cry::texture::load_dds_from_pak(path, opts) {
        Ok(image) if is_cube(&image) => return Some(images.add(image)),
        Ok(_) => warn!("aa-world: water environment map {path} is not a cube map"),
        Err(e) => warn!("aa-world: water environment map {path}: {e}"),
    }
    if path == DEFAULT_ENV_MAP {
        return None;
    }
    aa_cry::texture::load_dds_from_pak(DEFAULT_ENV_MAP, opts).ok().filter(is_cube).map(|i| images.add(i))
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
        let (base, normal_path) = StyleBase::of(style);
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
        let env_map = self.env_maps.entry(base.env_map.clone()).or_insert_with(|| load_cube(&base.env_map, images)).clone();
        let params = WaterParams { waves: Vec4::new(base.big, base.small, base.speed, base.repeat), motion: Vec4::new(if base.native_uv {1.0}else{0.0},if base.no_flow {1.0}else{0.0},-1.0,0.0), ..default() };
        let handle = assets.add(WaterMaterial { params, normal_map, foam_map, env_map });
        self.materials.insert(style.clone(), handle.clone());
        self.bases.push((handle.clone(), base));
        handle
    }

    pub fn len(&self) -> usize {
        self.bases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bases.is_empty()
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
    // foam and the surface shading use the engine's `sun + sky` colour
    let foam_light = (cur.sun + cur.sky) * rs;
    for (handle, base) in &styles.bases {
        let density = if base.density > 0.0 { base.density } else { 0.2 };
        let (body, f0, weight, amount, foam, sun_gain, ssr, cube) = if base.sea {
            let body = sea_color(&env, cur, &tuning, rs).extend(density * tuning.sea_absorb);
            (body, (base.fresnel_bias * 0.25).clamp(0.02, 0.08), tuning.sea_refl, 1.0, Vec3::ZERO, tuning.sea_sun, tuning.ssr, 0.0)
        } else {
            // the volume fog: constant colour, `exp2(-density * thickness)`
            let body = volume_color(base.fog, cur, &tuning, rs).extend(density * tuning.water_absorb);
            let foam = foam_light * base.foam * tuning.water_foam;
            let amount = base.reflection * tuning.water_env;
            (body, base.fresnel_bias, base.transparency * tuning.water_refl, amount, foam, tuning.water_sun, tuning.water_ssr, 1.0)
        };
        let params = WaterParams {
            body,
            refl: Vec4::new(f0, weight, base.bump, base.shine),
            waves: Vec4::new(base.big, base.small, base.speed, base.repeat),
            sun: (cur.sun * rs * base.sun_multiplier * sun_gain).extend(base.soft),
            sun_dir: sun_bevy.extend(sun_up * cur.day.max(0.05)),
            sky_h,
            sky_z,
            misc: Vec4::new(ssr, tuning.refraction, tuning.water_rough, amount),
            foam: Vec4::new(base.foam_reach, base.speed, 0.0, 0.0),
            foam_col: foam.extend(0.0),
            env_lit: foam_light.extend(cube),
            motion: Vec4::new(if base.native_uv {1.0}else{0.0},if base.no_flow {1.0}else{0.0},-1.0,0.0),
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
