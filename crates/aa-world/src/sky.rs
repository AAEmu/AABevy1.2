//! Sky, sun, ambient light, fog parameters and ocean from the time of day data.
//!
//! * [`WorldClock`] holds the hour (0..24); `speed` advances it (game hours per real second).
//! * Nearby sectors select weighted `zone/<id>/time_of_day.xml` inputs (cached). Their raw curve
//!   values blend at the clock's warped hour, then drive:
//!   * a directional sun / moon light (colour and strength from `Sun color * multiplier`, direction from
//!     the sun path of `lighting.xml`; at night the light comes from the moon side),
//!   * ambient light (`Sky color` / `Ambient ground color`),
//!   * the sky dome ([`SkyMaterial`](crate::sky_material::SkyMaterial): gradient, sun, the zone's sky box),
//!   * the post pass parameters (fog, film curve, see `post.rs`),
//!   * a large ocean plane at the world's ocean level, centred on the camera.
//!
//! All colours are scaled by [`AtmosphereTuning::radiance`] and the camera exposure into the HDR range the lit
//! scene uses (sun illuminance = `sun_lux_scale x` the time of day's sun value).

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::light::{CascadeShadowConfigBuilder, EnvironmentMapLight, NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

use crate::coords::{bevy_to_cry, cry_to_bevy};
use crate::env::{EnvSample, HdrSetup, Lighting, TimeOfDay};
use crate::post::{Post, PostUniform};
use crate::sky_material::{SkyBox, SkyMaterial, ZoneSky, load_sky_box};
use crate::sky_transition::SkySelection;
use crate::water_material::{WaterMaterial, WaterStyles};
use crate::water_query::{WaterKind, WaterQuery};
use crate::stream::{WorldConfig, WorldFocus, WorldInfo};
use crate::terrain_query::TerrainQuery;
use crate::world::World;

/// The time of day. `hour` in `0..24`.
#[derive(Resource, Clone, Debug)]
pub struct WorldClock {
    pub hour: f32,
    /// Game hours per real second (0 = frozen).
    pub speed: f32,
    /// Force a zone's time of day instead of the one under the camera.
    pub zone: Option<u16>,
    /// Which `<TimeOfDay relic_layer>` block of the zone files to use (0 or 1).
    pub layer: u32,
}

impl Default for WorldClock {
    fn default() -> Self {
        WorldClock { hour: 10.0, speed: 0.0, zone: None, layer: 0 }
    }
}

/// Calibration constants of the atmosphere (the data's multipliers are engine units, the real client's exposure and
/// scattering are not reproduced). `AA_TUNE="radiance=500,fog_ext=0.004"` overrides them for experiments.
#[derive(Resource, Clone, Debug)]
pub struct AtmosphereTuning {
    /// Scene radiance (cd/m2) of a time of day colour value of 1.
    pub radiance: f32,
    /// Multiplier on the fog colour.
    pub fog_gain: f32,
    /// Fog extinction per metre at layer density 1 and global density 1.
    pub fog_ext: f32,
    /// Sun glow added to the fog towards the sun.
    pub glow: f32,
    /// Multiplier on the sky gradient.
    pub sky_gain: f32,
    /// Exponent of the horizon -> zenith blend (larger keeps the horizon colour higher up); used when `sky_tau` is 0.
    pub sky_exp: f32,
    /// Sky gradient: e-fold height in degrees of elevation of the horizon colour's share (0 = the old exponent blend).
    pub sky_tau: f32,
    /// Sky gradient: how much brighter the sky is towards the sun (Rayleigh phase; 0 = even).
    pub sky_phase: f32,
    /// Sky colour: 1 = the hue of the zone's sky light (Rayleigh / Mie scattering and wavelengths), 0 = the gradient colours.
    pub sky_hue: f32,
    /// Sky colour: weight of the Mie scattering (white haze) against the Rayleigh scattering in the hue.
    pub sky_mie: f32,
    /// Sky colour: multiplier of the green of the scattering hue (the real sky is greener than single scattering of 650 nm).
    pub sky_green: f32,
    /// Radiance of the sun's glow and disc in the sky.
    pub sun_sky: f32,
    /// Sun disc multiplier.
    pub disc: f32,
    /// Extra opacity of the sky box.
    pub box_gain: f32,
    /// Extra rotation of the sky box (degrees).
    pub box_yaw: f32,
    /// How much of the sky horizon colour the fog takes (0 = the time of day's fog colour).
    pub fog_sky_mix: f32,
    /// Colour grading: multiplier of the time of day's saturation.
    pub saturation: f32,
    /// Colour grading: exposure multiplier before the film curve.
    pub exposure: f32,
    /// Screen space ambient occlusion on the post camera (1) or not (0, keeps MSAA 4x); read when the camera is set up.
    pub ssao: f32,
    /// Cloud sprites: brightness multiplier on top of `sky_gain` (0 = no clouds visible).
    pub cloud_gain: f32,
    /// Ambient light: 1 = hemisphere environment light (sky above, ground below), 0 = Bevy's flat `GlobalAmbientLight`.
    pub ambient_env: f32,
    /// Ambient light: brightness multiplier on `ambient_scale x luminance`.
    pub ambient_gain: f32,
    /// Ambient light: saturation of the sky / ground colours (1 = the data's colours).
    pub ambient_sat: f32,
    /// Ambient light: brightness of the environment light's specular reflections relative to its diffuse light.
    pub ambient_spec: f32,
    /// Ambient light: how much the ground colour gives way to the sky colour with the camera's height (1 = as the zone's data
    /// says: `Ambient min height` to `Ambient max height`, 0 = never).
    pub ambient_height: f32,
    /// Ambient light: 1 measures that height above the terrain under the camera (the ground bounce of the ambient light dies out with
    /// the distance to the ground), 0 above sea level.
    pub ambient_rel: f32,
    /// Ambient light: how much of the sky dome's colour (horizon / zenith, blue grey) replaces `Sky color` (green yellow in the data).
    pub ambient_dome: f32,
    /// Water volumes: multiplier of the volume's fog colour (the engine's fog colour is constant and unlit).
    pub water_body: f32,
    /// Water volumes: how much of the sun's colour (next to the sky's) lights the fog colour.
    pub water_lit: f32,
    /// Water volumes: absorption per metre = volume fog density x this (the engine's `exp2` is ln 2 = 0.693).
    pub water_absorb: f32,
    /// Water volumes: gain on the material's `TransparencyRatio` (the weight of the reflection in the surface colour).
    pub water_refl: f32,
    /// Water volumes: sun glint multiplier (on the material's `SunMultiplier`).
    pub water_sun: f32,
    /// Water volumes: multiplier of the material's `FoamMultiplier` (the white flecks).
    pub water_foam: f32,
    /// Water volumes: brightness of the reflected image (1 = the engine's: the material's cube map times `sun + sky`).
    pub water_env: f32,
    /// Water volumes: screen space reflections of the scene on top of the cube map (the engine has none): on (1) or off (0).
    pub water_ssr: f32,
    /// Water volumes: mip level the environment cube map is sampled at (0 = the sharp base level, as the engine's `texCUBE` of a
    /// map without a mip filter; the seams between the faces show as the steps of the real pond's reflection).
    pub water_lod: f32,
    /// Sea: screen space reflections on (1) or off (0).
    pub ssr: f32,
    /// Water: refraction offset (fraction of the screen per unit of bump).
    pub refraction: f32,
    /// Water: spread of the reflection rays (roughness of the surface).
    pub water_rough: f32,
    /// Sea: multiplier of the zone's ocean fog colour.
    pub sea_body: f32,
    /// Sea: absorption per metre = the zone's `<Ocean FogDensity>` x this (ln 2: the engine's `exp2(-density x depth)`).
    pub sea_absorb: f32,
    /// Sea: gain of the ocean material's foam (`water.cfx` FOAM, 1 = the formula as is).
    pub sea_foam: f32,
    /// Sea: gain of the ocean caustics on the sea bed (`deferredcaustics.cfx`, 1 = the formula as is).
    pub sea_caustics: f32,
    /// Under water (camera below the surface): multiplier of the extinction per metre (the zone's `Ocean fog density under water`
    /// for the sea, the volume's density for ponds and lakes).
    pub under_absorb: f32,
    /// Under water: gain of the light scattered towards the eye (the colour of the water itself).
    pub under_scatter: f32,
    /// Sea: reflection gain on the Fresnel term.
    pub sea_refl: f32,
    /// Sea: sun glint multiplier.
    pub sea_sun: f32,
    /// Bloom (the engine's `HDRBrightPassFilterPS` + `FilmicMapping`): `r_HDRBrightLevel` (1.25) / `r_HDRBrightThreshold`
    /// (6) / `r_HDRBrightOffset` (5) of the 1.2 client's renderer. A pixel of exposed brightness `c` brightens the bloom by
    /// `v / (offset + v)`, `v = max(c x level / 0.18 - threshold, 0)`.
    pub bloom_level: f32,
    pub bloom_threshold: f32,
    pub bloom_offset: f32,
    /// Strength of the bloom added to the exposed scene on top of the world's `HDRSetup` (colour x multiplier): `r_HDRBloomMul` 0.2
    /// x 3 x 0.5 of `FilmicMapping`.
    pub bloom_gain: f32,
    /// Sun shafts: multiplier of the zone's `Sun shafts visibility` (0 = off).
    pub shafts: f32,
    /// Eye adaptation: strength of the exposure change against the measured brightness (0 = off, 1 = half the stops the scene is
    /// brighter or darker than the calibrated look).
    pub adapt: f32,
    /// Eye adaptation: speed (1 / seconds) of the exposure going down in a brighter scene; it comes up 2.5 times slower.
    pub adapt_speed: f32,
    /// Eye adaptation: log2 of the exposed scene luminance the calibration views have (no exposure change there).
    pub adapt_ref: f32,
    /// Eye adaptation: darkest and brightest exposure change in stops.
    pub adapt_min: f32,
    /// Eye adaptation: 1 paints the state (linear grey 0.5 + stops / 8) instead of the scene, to calibrate `adapt_ref`.
    pub adapt_debug: f32,
    pub adapt_max: f32,
    /// Experiments: the sun's bearing (degrees clockwise from north) and elevation, instead of the sun path (NaN = off).
    pub sun_az: f32,
    pub sun_el: f32,
}

impl Default for AtmosphereTuning {
    fn default() -> Self {
        AtmosphereTuning {
            radiance: 540.0,
            fog_gain: 1.0,
            fog_ext: 0.04,
            glow: 0.05,
            sky_gain: 2.4,
            sky_exp: 1.0,
            sky_tau: 16.5,
            sky_phase: 2.0,
            sky_hue: 1.0,
            sky_mie: 0.5,
            sky_green: 1.2,
            sun_sky: 0.6,
            disc: 40.0,
            box_gain: 2.0,
            box_yaw: 0.0,
            fog_sky_mix: 0.6,
            saturation: 0.85,
            exposure: 1.15,
            ssao: 0.0,
            cloud_gain: 1.0,
            ambient_env: 1.0,
            ambient_gain: 1.3,
            ambient_sat: 0.75,
            ambient_dome: 1.0,
            ambient_spec: 0.2,
            ambient_height: 1.0,
            ambient_rel: 1.0,
            water_body: 7.0,
            water_lit: 0.35,
            water_absorb: std::f32::consts::LN_2,
            water_refl: 1.0,
            water_sun: 1.0,
            water_foam: 0.7,
            water_env: 0.08,
            water_ssr: 0.0,
            water_lod: 0.0,
            ssr: 1.0,
            refraction: 0.012,
            water_rough: 0.06,
            sea_body: 2.5,
            sea_absorb: std::f32::consts::LN_2,
            sea_foam: 1.0,
            sea_caustics: 1.0,
            under_absorb: 1.0,
            under_scatter: 1.0,
            sea_refl: 0.5,
            sea_sun: 3.0,
            bloom_level: 1.25,
            bloom_threshold: 6.0,
            bloom_offset: 5.0,
            bloom_gain: 0.3,
            shafts: 1.0,
            adapt: 1.0,
            adapt_speed: 1.2,
            adapt_ref: -4.6,
            adapt_min: -0.6,
            adapt_debug: 0.0,
            adapt_max: 0.75,
            sun_az: f32::NAN,
            sun_el: f32::NAN,
        }
    }
}

impl AtmosphereTuning {
    pub fn from_env() -> Self {
        let mut t = AtmosphereTuning::default();
        for kv in std::env::var("AA_TUNE").unwrap_or_default().split(',') {
            let Some((k, v)) = kv.split_once('=') else { continue };
            let Ok(v) = v.trim().parse::<f32>() else { continue };
            match k.trim() {
                "radiance" => t.radiance = v,
                "fog_gain" => t.fog_gain = v,
                "fog_ext" => t.fog_ext = v,
                "glow" => t.glow = v,
                "sky_gain" => t.sky_gain = v,
                "sky_exp" => t.sky_exp = v,
                "sky_tau" => t.sky_tau = v,
                "sky_phase" => t.sky_phase = v,
                "sky_hue" => t.sky_hue = v,
                "sky_mie" => t.sky_mie = v,
                "sky_green" => t.sky_green = v,
                "sun_sky" => t.sun_sky = v,
                "disc" => t.disc = v,
                "box_gain" => t.box_gain = v,
                "box_yaw" => t.box_yaw = v,
                "fog_sky_mix" => t.fog_sky_mix = v,
                "saturation" => t.saturation = v,
                "exposure" => t.exposure = v,
                "ssao" => t.ssao = v,
                "cloud_gain" => t.cloud_gain = v,
                "ambient_env" => t.ambient_env = v,
                "ambient_gain" => t.ambient_gain = v,
                "ambient_sat" => t.ambient_sat = v,
                "ambient_dome" => t.ambient_dome = v,
                "ambient_spec" => t.ambient_spec = v,
                "ambient_height" => t.ambient_height = v,
                "ambient_rel" => t.ambient_rel = v,
                "water_body" => t.water_body = v,
                "water_lit" => t.water_lit = v,
                "water_absorb" => t.water_absorb = v,
                "water_refl" => t.water_refl = v,
                "water_sun" => t.water_sun = v,
                "water_foam" => t.water_foam = v,
                "water_env" => t.water_env = v,
                "water_ssr" => t.water_ssr = v,
                "water_lod" => t.water_lod = v,
                "ssr" => t.ssr = v,
                "refraction" => t.refraction = v,
                "water_rough" => t.water_rough = v,
                "sea_body" => t.sea_body = v,
                "sea_absorb" => t.sea_absorb = v,
                "sea_foam" => t.sea_foam = v,
                "sea_caustics" => t.sea_caustics = v,
                "under_absorb" => t.under_absorb = v,
                "under_scatter" => t.under_scatter = v,
                "sea_refl" => t.sea_refl = v,
                "sea_sun" => t.sea_sun = v,
                "bloom_level" => t.bloom_level = v,
                "bloom_threshold" => t.bloom_threshold = v,
                "bloom_offset" => t.bloom_offset = v,
                "bloom_gain" => t.bloom_gain = v,
                "shafts" => t.shafts = v,
                "adapt" => t.adapt = v,
                "adapt_speed" => t.adapt_speed = v,
                "adapt_ref" => t.adapt_ref = v,
                "adapt_min" => t.adapt_min = v,
                "adapt_debug" => t.adapt_debug = v,
                "adapt_max" => t.adapt_max = v,
                "sun_az" => t.sun_az = v,
                "sun_el" => t.sun_el = v,
                _ => {}
            }
        }
        t
    }
}

/// The environment currently applied from native spatially weighted time-of-day inputs.
#[derive(Resource, Clone, Debug)]
pub struct Environment {
    pub sample: Option<EnvSample>,
    pub zone: Option<u16>,
    pub hour: f32,
    /// The zone's `<Ocean FogColor>` (linear): the colour of the sea.
    pub ocean_fog: Vec3,
    /// The zone's `<Ocean FogDensity>` (0: unknown, the sea keeps its style density).
    pub ocean_density: f32,
}

#[derive(Component)]
pub struct SunLight;
#[derive(Component)]
pub struct SkyDome;
#[derive(Component)]
pub struct OceanPlane;

#[derive(Resource)]
pub struct SkyAssets {
    pub dome_material: Handle<SkyMaterial>,
    pub ocean_material: Option<Handle<WaterMaterial>>,
}

/// Cached zone data: time of day files, the sky of each zone and its sky box textures.
#[derive(Resource, Default)]
pub struct TodCache {
    world: Option<String>,
    zones: HashMap<(u16, u32), Option<Arc<TimeOfDay>>>,
    world_default: HashMap<u32, Option<Arc<TimeOfDay>>>,
    lighting: Option<Lighting>,
    hdr: HdrSetup,
    sector_rx: Option<Mutex<Receiver<World>>>,
    zone_sky: HashMap<(Option<u16>, u32), ZoneSky>,
    sky_boxes: HashMap<String, Option<SkyBox>>,
}

impl TodCache {
    fn sync_world(&mut self, world: &str) {
        if self.world.as_deref() == Some(world) {
            return;
        }
        *self = Self { world: Some(world.to_string()), ..default() };
        self.lighting = aa_pak::Pak::global()
            .read_string(&format!("worlds/{world}/lighting.xml"))
            .ok().and_then(|x| Lighting::parse(&x));
        self.hdr = aa_pak::Pak::global()
            .read_string(&format!("worlds/{world}/env.xml"))
            .ok().and_then(|x| HdrSetup::parse(&x)).unwrap_or_default();
    }

    fn sample_zones(&mut self, world: &str, weights: &[(u16, f32)], layer: u32, hour: f32, areas: &crate::area_tod::AreaToDState, current_zone: Option<u16>) -> Option<EnvSample> {
        self.sync_world(world);
        // Only nearby inputs stay resident; revisiting another world/zone cannot reuse stale curves.
        self.zones.retain(|(zone, l), _| *l == layer && weights.iter().take(10).any(|(z, _)| z == zone));
        let mut layers: Vec<_> = weights.iter().take(10).filter_map(|(zone, weight)| {
            // Seamless365EEC20 subtracts area total from each spatial zone.
            let weight = (*weight - areas.total).clamp(0.0, 1.0);
            if weight == 0.0 { return None; }
            self.zones.entry((*zone, layer)).or_insert_with(|| load_tod(world, Some(*zone), layer))
                .clone().map(|tod| (tod, weight))
        }).collect();
        // Native fallback when no sector contributed: current zone at weight 1,
        // even with a partial area. The manager normalizes the combined inputs.
        if layers.is_empty() && areas.total < 1.0 && let Some(zone) = current_zone {
            if let Some(tod) = self.zones.entry((zone, layer)).or_insert_with(|| load_tod(world, Some(zone), layer)).clone() {
                layers.push((tod, 1.0));
            }
        }
        let lighting = self.lighting.unwrap_or_default();
        if layers.is_empty() && areas.inputs.is_empty() {
            return self.world_default.entry(layer).or_insert_with(|| load_tod(world, None, layer))
                .as_ref().map(|tod| tod.sample(hour, lighting));
        }
        let mut inputs: Vec<(&TimeOfDay, f32)> = Vec::with_capacity(10);
        for (tod, weight) in areas.inputs.iter().chain(layers.iter()) {
            if let Some((_, previous)) = inputs.iter_mut().find(|(existing, _)| std::ptr::eq(*existing, tod.as_ref())) {
                *previous = *weight;
            } else if inputs.len() < 10 {
                inputs.push((tod.as_ref(), *weight));
            }
        }
        Some(TimeOfDay::sample_blended(&inputs, hour, lighting))
    }
}

/// Registers the sky systems.
pub fn build(app: &mut App) {
    crate::post::build(app);
    crate::sky_material::build(app);
    crate::water_material::build(app);
    crate::sky_clouds::build(app);
    crate::char_light::build(app);
    app.init_resource::<WorldClock>()
        .insert_resource(Environment { sample: None, zone: None, hour: 13.0, ocean_fog: Vec3::new(0.01, 0.07, 0.3), ocean_density: 0.0 })
        .init_resource::<TodCache>()
        .init_resource::<SkySelection>()
        .init_resource::<crate::area_tod::AreaToDState>()
        .insert_resource(AtmosphereTuning::from_env())
        .add_systems(Startup, setup_sky)
        .add_systems(Update, (receive_zones, update_clock, crate::area_tod::update_areas, update_environment, update_ambient, update_sky, crate::sky_clouds::update_clouds, update_post, crate::water_material::update_water).chain())
        .add_systems(PostUpdate, follow_camera.before(bevy::transform::TransformSystems::Propagate));
}

#[allow(clippy::too_many_arguments)]
fn setup_sky(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    info: Res<WorldInfo>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sky_materials: ResMut<Assets<SkyMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut water_styles: ResMut<WaterStyles>,
    mut images: ResMut<Assets<Image>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut cache: ResMut<TodCache>,
) {
    if !cfg.environment {
        return;
    }
    cache.sync_world(&cfg.world);
    // sector -> zone ownership comes from the big world.xml: parse it off the main thread
    let (tx, rx) = channel();
    let mut world = (*info.0).clone();
    std::thread::spawn(move || {
        if world.load_sector_zones().is_ok() {
            let _ = tx.send(world);
        }
    });
    cache.sector_rx = Some(Mutex::new(rx));

    ambient.brightness = 300.0;

    let dome_material = sky_materials.add(SkyMaterial { params: Default::default(), box_12: None, box_34: None, box_5: None });
    commands.spawn((
        SkyDome,
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(48, 24))),
        MeshMaterial3d(dome_material.clone()),
        Transform::from_scale(Vec3::splat(cfg.sky_radius)),
        NotShadowCaster,
        NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    ));
    let mut ocean_material = None;
    if info.0.ocean_level > 0.0 {
        let material = water_styles.ocean(&cfg.world, &mut water_materials, &mut images);
        commands.spawn((
            OceanPlane,
            Mesh3d(meshes.add(Plane3d::default().mesh().size(cfg.sky_radius * 2.4, cfg.sky_radius * 2.4))),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, info.0.ocean_level, 0.0),
            NotShadowCaster,
        ));
        ocean_material = Some(material);
    }
    let mut sun = DirectionalLight { illuminance: 10000.0, shadow_maps_enabled: cfg.shadows, ..default() };
    sun.color = Color::WHITE;
    commands.spawn((
        SunLight,
        sun,
        Transform::from_rotation(Quat::from_rotation_x(-1.0)),
        CascadeShadowConfigBuilder {
            num_cascades: 3,
            minimum_distance: 0.3,
            maximum_distance: cfg.shadow_distance,
            first_cascade_far_bound: cfg.shadow_distance * 0.08,
            ..default()
        }
        .build(),
    ));
    commands.insert_resource(SkyAssets { dome_material, ocean_material });
}

fn receive_zones(cfg: Res<WorldConfig>, mut cache: ResMut<TodCache>, mut info: ResMut<WorldInfo>) {
    let received = match &cache.sector_rx {
        Some(rx) => rx.lock().unwrap().try_recv().ok(),
        None => None,
    };
    if let Some(world) = received {
        if world.name == cfg.world {
            info.0 = Arc::new(world);
        }
        cache.sector_rx = None;
    }
}

fn update_clock(time: Res<Time>, mut clock: ResMut<WorldClock>) {
    if clock.speed != 0.0 {
        clock.hour = (clock.hour + clock.speed * time.delta_secs()).rem_euclid(24.0);
    }
}

fn load_tod(world: &str, zone: Option<u16>, layer: u32) -> Option<Arc<TimeOfDay>> {
    let pak = aa_pak::Pak::global();
    let candidates = zone
        .map(|z| format!("worlds/{world}/zone/{z}/time_of_day.xml"))
        .into_iter()
        .chain([format!("worlds/{world}/time_of_day.xml")]);
    for path in candidates {
        if let Ok(xml) = pak.read_string(&path)
            && let Some(t) = TimeOfDay::parse_layer(&xml, layer)
        {
            return Some(Arc::new(t));
        }
    }
    None
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

// Client 300A5DD0 selects the moon vector for SetSunDir when the sun is at/below zero.
fn directional_light_dir(sample: &EnvSample) -> Vec3 {
    if sample.sun_dir.z > 0.0 { sample.sun_dir } else { sample.moon_dir }
}

/// Exposure of the (first) post processed camera, else Bevy's default.
pub(crate) fn view_exposure(cameras: &Query<Option<&Exposure>, With<PostUniform>>) -> f32 {
    cameras.iter().next().flatten().map_or(Exposure::BLENDER.exposure(), |e| e.exposure())
}

#[allow(clippy::too_many_arguments)]
pub fn update_environment(
    mut commands: Commands,
    time: Res<Time>,
    cfg: Res<WorldConfig>,
    info: Res<WorldInfo>,
    clock: Res<WorldClock>,
    tuning: Res<AtmosphereTuning>,
    mut env: ResMut<Environment>,
    mut cache: ResMut<TodCache>,
    mut selection: ResMut<SkySelection>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<SunLight>>,
    cameras: Query<(Entity, &GlobalTransform, Has<PostUniform>), With<Camera3d>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
    focus: Query<Entity, With<WorldFocus>>,
    areas: Res<crate::area_tod::AreaToDState>,
) {
    if !cfg.environment {
        env.sample = None;
        env.zone = None;
        cache.zones.clear();
        cache.world_default.clear();
        selection.clear();
        return;
    }
    let focus_entity = focus.iter().next();
    let Some((cam_entity, cam_tf, post_camera)) = cameras
        .iter()
        .find(|(e, ..)| Some(*e) == focus_entity)
        .or_else(|| cameras.iter().next())
    else {
        return;
    };
    let cry_pos = bevy_to_cry(cam_tf.translation());
    let same_world = info.0.name == cfg.world;
    let zone = clock.zone.or_else(|| same_world.then(|| info.0.zone_at(cry_pos)).flatten());

    let weights = clock.zone.map(|z| vec![(z, 1.0)]).unwrap_or_else(|| {
        if same_world { info.0.environment_weights(cry_pos) } else { Vec::new() }
    });
    let Some(mut cur) = cache.sample_zones(&cfg.world, &weights, clock.layer, clock.hour, &areas, zone) else {
        selection.clear();
        env.sample = None;
        env.zone = zone;
        return;
    };
    selection.update(&cfg.world, &weights, clock.layer, time.delta_secs());
    // the clouds are shaded by the sky colour, which some zones keep green (see `update_ambient`): same hue as the ambient light
    cur.cloud_sky = ambient_hue(&cur, &tuning, cur.cloud_sky);
    if tuning.sun_az.is_finite() && tuning.sun_el.is_finite() {
        let (az, el) = (tuning.sun_az.to_radians(), tuning.sun_el.to_radians());
        cur.sun_dir = Vec3::new(az.sin() * el.cos(), az.cos() * el.cos(), el.sin());
    }
    // Native 300A5010 evaluates weighted curves each frame; there is no temporal filter.
    env.zone = zone;
    env.hour = clock.hour;
    if let Some((c, density)) = zone_sky_of(&mut cache, &cfg.world, zone, 0).ocean_fog {
        env.ocean_fog = crate::env::srgb_to_linear(Vec3::from(c) / 255.0);
        env.ocean_density = density;
    }

    // --- sun / moon ---
    let elev = cur.sun_dir.z;
    let light_dir_cry = directional_light_dir(&cur);
    let to_light = cry_to_bevy(light_dir_cry).normalize();
    // fade through the horizon so the light does not flip abruptly
    let horizon_fade = ((elev.abs() + 0.02) / 0.12).clamp(0.12, 1.0);
    let sun_lum = luminance(cur.sun);
    for (mut light, mut tf) in &mut sun {
        let col = cur.sun / sun_lum.max(1e-4);
        light.color = Color::linear_rgb(col.x.min(1.5), col.y.min(1.5), col.z.min(1.5));
        light.illuminance = cfg.sun_lux_scale * sun_lum * horizon_fade * (0.3 + 0.7 * cur.day);
        *tf = Transform::IDENTITY.looking_to(-to_light, Vec3::Y);
    }
    // --- ambient: a flat light, or (default) the hemisphere environment light of `update_ambient` ---
    if tuning.ambient_env > 0.5 {
        ambient.brightness = 0.0;
    } else {
        let hemi = (cur.sky + cur.ambient_ground) * 0.5;
        let hemi_lum = luminance(hemi).max(1e-4);
        ambient.color = Color::linear_rgb((hemi.x / hemi_lum).min(2.0), (hemi.y / hemi_lum).min(2.0), (hemi.z / hemi_lum).min(2.0));
        ambient.brightness = cfg.ambient_scale * hemi_lum;
    }

    // --- clear colour (what no geometry covers) and, without the post pass, the camera's distance fog ---
    let expo = view_exposure(&exposure);
    let horizon = cur.horizon * tuning.radiance * expo * tuning.sky_gain;
    clear.0 = Color::linear_rgb(horizon.x, horizon.y, horizon.z);
    if post_camera {
        commands.entity(cam_entity).remove::<DistanceFog>();
    } else {
        let day = cur.day;
        let horizon_day = lerp3(cur.fog_srgb * 1.1, Vec3::new(0.7, 0.76, 0.82), 0.55 * day).min(Vec3::splat(1.0));
        let night = cur.night_horizon * 0.18;
        let c = lerp3(night, horizon_day, day);
        let visibility = cfg.fog_visibility / (0.5 + cur.fog_density * 2.0);
        commands.entity(cam_entity).insert(DistanceFog {
            color: Color::srgb(c.x, c.y, c.z),
            directional_light_color: Color::BLACK,
            directional_light_exponent: 30.0,
            falloff: FogFalloff::from_visibility_squared(visibility),
        });
        clear.0 = Color::srgb(c.x, c.y, c.z);
    }
    env.sample = Some(cur);
}

/// `colour` with the hue of the sky dome mixed in (`ambient_dome`) and desaturated (`ambient_sat`), at the same luminance.
fn ambient_hue(sample: &EnvSample, tuning: &AtmosphereTuning, colour: Vec3) -> Vec3 {
    let dome = unit_luminance(sample.horizon * 0.6 + sample.zenith * 0.4);
    desaturate(lerp3(unit_luminance(colour), dome, tuning.ambient_dome), tuning.ambient_sat) * luminance(colour)
}

fn desaturate(c: Vec3, s: f32) -> Vec3 {
    let l = luminance(c);
    Vec3::splat(l) + (c - Vec3::splat(l)) * s
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How visible the sun's glow and disc are: by day the full sun, and at the dusk and dawn the warm (red, orange) sun colours of
/// the data too (a sunset sun has a low sRGB luminance, `day` alone would hide it); the blue colours of the night are the moon.
fn sun_shine(cur: &EnvSample) -> f32 {
    let warm = ((cur.sun.x - cur.sun.z) / (cur.sun.x + cur.sun.z).max(1e-3) * 2.0).clamp(0.0, 1.0);
    cur.day.max(warm)
}

fn unit_luminance(c: Vec3) -> Vec3 {
    c / luminance(c).max(1e-4)
}

/// The ambient light: an environment light on the camera with a hemisphere gradient, sky colour above, ground colour below.
///
/// The data's `Sky color` is a yellow green and its ambient flat, the real picture has blue grey shade (sky brightening): the
/// colour is mixed towards the dome's horizon / zenith colour and desaturated, the brightness keeps the old
/// `ambient_scale x luminance`. The cube maps are 1 x 1 pixels per face and rebuilt only when a colour changes by 1.5 %.
#[allow(clippy::too_many_arguments)]
fn update_ambient(
    mut commands: Commands,
    env: Res<Environment>,
    cfg: Res<WorldConfig>,
    tuning: Res<AtmosphereTuning>,
    mut images: ResMut<Assets<Image>>,
    mut last: Local<[f32; 9]>,
    mut lit: Local<Option<Entity>>,
    mut cubes: Local<Option<AmbientCubes>>,
    terrain: Option<Res<TerrainQuery>>,
    focus: Query<Entity, With<WorldFocus>>,
    cameras: Query<(Entity, &GlobalTransform), With<Camera3d>>,
) {
    if !cfg.environment || tuning.ambient_env < 0.5 {
        return;
    }
    let Some(cur) = &env.sample else { return };
    let focus = focus.iter().next();
    let Some((camera, camera_tf)) = cameras.iter().find(|(e, _)| Some(*e) == focus).or_else(|| cameras.iter().next()) else { return };

    let dome = unit_luminance(cur.horizon * 0.6 + cur.zenith * 0.4);
    let sky_dir = lerp3(unit_luminance(cur.sky), dome, tuning.ambient_dome);
    let ground_dir = lerp3(unit_luminance(cur.ambient_ground), dome, tuning.ambient_dome * 0.5);
    let scale = cfg.ambient_scale * tuning.ambient_gain;
    let top = desaturate(sky_dir, tuning.ambient_sat) * luminance(cur.sky) * scale;
    let ground = desaturate(ground_dir, tuning.ambient_sat) * luminance(cur.ambient_ground) * scale;
    // the ground colour fades into the sky colour with the height above the ground (`Ambient min / max height`: in the
    // court yard of the palace, 240 m up, it is the sky colour already)
    let at = bevy_to_cry(camera_tf.translation());
    let mut height = at.z;
    if tuning.ambient_rel > 0.5 {
        height -= terrain.and_then(|t| t.height_at(at.x, at.y)).unwrap_or(0.0).min(height);
    }
    let span = (cur.ambient_heights.y - cur.ambient_heights.x).max(1e-3);
    let up = ((height - cur.ambient_heights.x) / span).clamp(0.0, 1.0);
    let bottom = lerp3(ground, top, up * tuning.ambient_height);
    let mid = (top + bottom) * 0.5;
    let now = [top.x, top.y, top.z, mid.x, mid.y, mid.z, bottom.x, bottom.y, bottom.z];
    let peak = now.iter().cloned().fold(1e-3, f32::max);
    let change = now.iter().zip(last.iter()).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    // a colour that did not move still has to reach a camera that just became the focus (the login flow's anchor camera
    // holds the focus first; with a frozen clock the game's camera never got the ambient light)
    let moved = *lit != Some(camera);
    if change < 0.015 * peak && !moved {
        return;
    }
    *lit = Some(camera);
    *last = now;
    // the specular half is much dimmer: shiny things (hair, metal, wet rock) would blow out under the full sky radiance
    let k = tuning.ambient_spec;
    let (diffuse, specular) = (cube_pixels(top, mid, bottom), cube_pixels(top * k, mid * k, bottom * k));
    // The maps are written in place: a new image is not on the GPU for a frame or two, and a camera whose environment map is
    // not ready is a different pipeline key (`environment_map` off), so every change of the light compiled the whole
    // material set again (20 pipelines in a loading screen) and drew without ambient meanwhile.
    let maps = match cubes.clone() {
        Some(maps) => {
            for (handle, data) in [(&maps.diffuse, diffuse), (&maps.specular, specular)] {
                if let Some(mut image) = images.get_mut(handle) {
                    image.data = Some(data);
                }
            }
            maps
        }
        None => {
            let maps = AmbientCubes { diffuse: images.add(cube_image(diffuse)), specular: images.add(cube_image(specular)) };
            commands.insert_resource(maps.clone());
            *cubes = Some(maps.clone());
            maps
        }
    };
    if !moved {
        return;
    }
    commands.entity(camera).insert(EnvironmentMapLight {
        diffuse_map: maps.diffuse,
        specular_map: maps.specular,
        intensity: 1.0,
        ..default()
    });
}

/// The two cube maps of the ambient environment light (1 x 1 pixels per face); [`update_ambient`] rewrites their pixels in
/// place. Cameras that must render with the game's pipeline keys without having the world focus (the character list's
/// prewarm) take their [`EnvironmentMapLight`] from these.
#[derive(Resource, Clone)]
pub struct AmbientCubes {
    pub diffuse: Handle<Image>,
    pub specular: Handle<Image>,
}

impl AmbientCubes {
    /// An environment light of these maps.
    pub fn light(&self) -> EnvironmentMapLight {
        EnvironmentMapLight { diffuse_map: self.diffuse.clone(), specular_map: self.specular.clone(), intensity: 1.0, ..default() }
    }
}

/// The six faces (+X, -X, +Y, -Y, +Z, -Z of the cube: sky on top, ground below) as `Rgba16Float` pixels.
fn cube_pixels(top: Vec3, mid: Vec3, bottom: Vec3) -> Vec<u8> {
    [mid, mid, top, bottom, mid, mid]
        .into_iter()
        .flat_map(|c| [c.x, c.y, c.z, 1.0].map(half::f16::from_f32))
        .flat_map(half::f16::to_le_bytes)
        .collect()
}

fn cube_image(data: Vec<u8>) -> Image {
    Image {
        texture_view_descriptor: Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() }),
        ..Image::new(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::default(),
        )
    }
}

/// The zone's `env.xml` (sky box, angle, ocean colour), read once.
fn zone_sky_of(cache: &mut TodCache, world: &str, zone: Option<u16>, layer: u32) -> ZoneSky {
    cache
        .zone_sky
        .entry((zone, layer))
        .or_insert_with(|| {
            zone.and_then(|z| aa_pak::Pak::global().read_string(&format!("worlds/{world}/zone/{z}/env.xml")).ok())
                .map(|xml| ZoneSky::parse(&xml, if layer == 1 { "Zone_Dominion_Normal" } else { "Zone" }))
                .unwrap_or_default()
        })
        .clone()
}

/// The sky dome's gradient, sun and sky box.
#[allow(clippy::too_many_arguments)]
fn update_sky(
    env: Res<Environment>,
    selection: Res<SkySelection>,
    cfg: Res<WorldConfig>,
    tuning: Res<AtmosphereTuning>,
    assets: Option<Res<SkyAssets>>,
    mut cache: ResMut<TodCache>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut images: ResMut<Assets<Image>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    let Some(assets) = assets else { return };
    if !cfg.environment || env.sample.is_none() {
        cache.zone_sky.clear();
        cache.sky_boxes.clear();
        if let Some(mut m) = materials.get_mut(&assets.dome_material) {
            m.params.box_params.w = 0.0;
            m.box_12 = None;
            m.box_34 = None;
            m.box_5 = None;
        }
        return;
    }
    let cur = env.sample.as_ref().unwrap();
    // the zone's sky box (loaded once)
    let zone_sky = zone_sky_of(&mut cache, &cfg.world, selection.zone, selection.layer);
    cache.zone_sky.retain(|key, _| *key == (selection.zone, selection.layer) || *key == (env.zone, 0));
    cache.sky_boxes.retain(|material, _| Some(material) == zone_sky.material.as_ref());
    let sky_box = zone_sky.material.as_ref().and_then(|m| {
        cache
            .sky_boxes
            .entry(m.clone())
            .or_insert_with(|| {
                let b = load_sky_box(m, &mut images);
                debug!("aa-world: sky box {m}: {:?}", b);
                b
            })
            .clone()
    });
    let expo = view_exposure(&exposure);
    let rs = tuning.radiance * expo;
    let Some(mut m) = materials.get_mut(&assets.dome_material) else { return };
    let sun_bevy = cry_to_bevy(cur.sun_dir).normalize();
    let sun_up = ((cur.sun_dir.z + 0.04) / 0.1).clamp(0.0, 1.0);
    let sun_col = cur.sun / luminance(cur.sun).max(1e-3);
    // the hue of the sky light (Rayleigh scattering is blue, Mie scattering white), the brightness of the zone's gradient colours
    // (the scatter hue is the noon sky of the real captures: towards the dusk the zone's own gradient colours take over, a
    // sun low in the west must not paint the whole dome with its colour)
    let scatter_hue = unit_luminance((cur.sky_rayleigh + cur.sky_mie * tuning.sky_mie) * Vec3::new(1.0, tuning.sky_green, 1.0));
    let sun_elevation = cur.sun_dir.z.clamp(-1.0, 1.0).asin().to_degrees();
    let hue_weight = tuning.sky_hue * smoothstep(20.0, 40.0, sun_elevation);
    let hue = |c: Vec3| lerp3(unit_luminance(c), scatter_hue, hue_weight) * luminance(c);
    m.params.horizon = (hue(cur.horizon) * rs * tuning.sky_gain).extend(tuning.sky_exp);
    m.params.zenith = (hue(cur.zenith) * rs * tuning.sky_gain).extend(0.0);
    m.params.shape = Vec4::new(tuning.sky_tau, tuning.sky_phase, 0.0, 0.0);
    m.params.sun_dir = sun_bevy.extend(sun_up * sun_shine(cur));
    m.params.sun_color = (sun_col * rs * tuning.sun_sky).extend(tuning.disc);
    match &sky_box {
        Some(b) => {
            m.params.box_sky = (cur.cloud_sky * b.sky_multiplier * rs * tuning.sky_gain).extend(b.opacity * tuning.box_gain * selection.opacity);
            m.params.box_sun = (cur.cloud_sun * b.sun_multiplier * rs * tuning.sky_gain).extend(b.attenuation);
            let a = (zone_sky.angle + tuning.box_yaw).to_radians();
            m.params.box_params = Vec4::new(b.alpha_saturation, a.cos(), a.sin(), 1.0);
            m.box_12 = Some(b.box_12.clone());
            m.box_34 = Some(b.box_34.clone());
            m.box_5 = Some(b.box_5.clone());
        }
        None => {
            m.params.box_params.w = 0.0;
            m.box_12 = None;
            m.box_34 = None;
            m.box_5 = None;
        }
    }
}

/// Strength of the sun shafts at `Sun shafts visibility` 1 and `AtmosphereTuning::shafts` 1 (what reads as the engine's rays).
const SHAFT_GAIN: f32 = 1.4;

/// The post pass parameters of the applied environment (fog, film curve, grading).
fn update_post(
    env: Res<Environment>,
    cfg: Res<WorldConfig>,
    tuning: Res<AtmosphereTuning>,
    water: Option<Res<WaterQuery>>,
    cache: Res<TodCache>,
    time: Res<Time>,
    mut cameras: Query<(&Post, &mut PostUniform, Option<&Exposure>, &GlobalTransform)>,
) {
    let Some(cur) = &env.sample else { return };
    for (post, mut u, exposure, camera) in &mut cameras {
        let expo = exposure.map_or(Exposure::BLENDER.exposure(), |e| e.exposure());
        let rs = tuning.radiance * expo;
        let (film, grade, balance) = PostUniform::film_from(cur, 2f32.powf(post.exposure_ev));
        let sun_bevy = cry_to_bevy(cur.sun_dir).normalize();
        let fog_low = lerp3(cur.fog, cur.horizon, tuning.fog_sky_mix);
        u.fog_color = (fog_low * rs * tuning.fog_gain).extend(cur.fog_density);
        u.fog_top = (cur.fog_top * rs * tuning.fog_gain).extend(0.0);
        u.fog_layer = cur.fog_layer;
        u.fog_params = Vec4::new(cur.fog_ramp.x, tuning.fog_ext, 1.0, cfg.sky_radius);
        u.sun_dir = sun_bevy.extend(12.0);
        u.sun_glow = (cur.sun * rs * tuning.glow * sun_shine(cur)).extend(0.0);
        u.film = film;
        u.grade = grade * Vec4::new(tuning.saturation, 1.0, 1.0, tuning.exposure);
        u.balance = balance.truncate().extend(if post.fog { 1.0 } else { 0.0 });
        // the world's `HDRSetup` colour and multiplier times `r_HDRBloomMul x 3 x 0.5`
        u.bloom = Vec4::new(tuning.bloom_level / 0.18, tuning.bloom_threshold, tuning.bloom_offset.max(1e-3), 0.0);
        u.bloom_color = (cache.hdr.bloom_color * cache.hdr.bloom_mul * tuning.bloom_gain * post.bloom).extend(0.0);
        // sun shafts: the zone's visibility, fading out as the sun sets; the rays get longer with `Sun rays visibility`
        let sun_up = ((sun_bevy.y + 0.03) / 0.15).clamp(0.0, 1.0);
        u.shafts = Vec4::new(cur.sun_shafts.max(0.0) * tuning.shafts * SHAFT_GAIN * sun_up, (0.35 + 0.3 * cur.sun_rays).clamp(0.2, 0.95), cur.sun_rays_atten.max(0.0), 0.0);
        u.shafts_color = lerp3(Vec3::ONE, cur.sun / cur.sun.max_element().max(1e-4), 0.6).extend(0.0);
        // eye adaptation: blends per frame from the speed (a darker scene dark adapts slower)
        let dt = time.delta_secs().min(0.25);
        u.adapt = Vec4::new(1.0 - (-dt * tuning.adapt_speed).exp(), 0.5 * tuning.adapt, tuning.adapt_min, tuning.adapt_max);
        u.adapt2 = Vec4::new(tuning.adapt_ref, 4.0, 1.0 - (-dt * tuning.adapt_speed / 2.5).exp(), tuning.adapt_debug);
        // under water: the camera below the surface of the sea or of a volume (ponds, lakes, rivers)
        u.under = Vec4::ZERO;
        let pos = camera.translation();
        let body = water.as_ref().and_then(|w| w.body_at(bevy_to_cry(pos))).filter(|b| pos.y < b.height - 0.02);
        if let Some(body) = body {
            let (colour, extinction) = match body.kind {
                WaterKind::Sea => {
                    let density = if cur.ocean_under > 0.0 { cur.ocean_under } else { 0.04 };
                    (crate::water_material::sea_color(&env, cur, &tuning, rs), density * std::f32::consts::LN_2)
                }
                _ => {
                    let density = if body.fog_density > 0.0 { body.fog_density } else { 0.2 };
                    (crate::water_material::volume_color(body.fog_color, cur, &tuning, rs), density * tuning.water_absorb)
                }
            };
            u.under = Vec4::new(body.height, extinction * tuning.under_absorb, tuning.under_scatter, 1.0);
            u.under_color = colour.extend(0.0);
        }
    }
}

/// Dome and ocean follow the camera.
#[allow(clippy::type_complexity)]
fn follow_camera(
    cfg: Res<WorldConfig>,
    cameras: Query<&Transform, (With<Camera3d>, Without<SkyDome>, Without<OceanPlane>)>,
    mut dome: Query<&mut Transform, (With<SkyDome>, Without<OceanPlane>)>,
    mut ocean: Query<&mut Transform, (With<OceanPlane>, Without<SkyDome>)>,
) {
    let Some(cam) = cameras.iter().next() else { return };
    let p = cam.translation;
    for mut t in &mut dome {
        t.translation = p;
        t.scale = Vec3::splat(cfg.sky_radius);
    }
    for mut t in &mut ocean {
        // snap so the plane does not swim relative to the world
        t.translation.x = (p.x / 64.0).round() * 64.0;
        t.translation.z = (p.z / 64.0).round() * 64.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_areas_precede_zones_subtract_weight_and_share_ten_slots() {
        if !crate::pak_available() { return; }
        let mut cache = TodCache::default();
        let mut areas = crate::area_tod::AreaToDState::default();
        let area = load_tod("main_world", Some(186), 0).unwrap();
        areas.inputs.push((area.clone(), 0.25));
        areas.total = 0.25;
        let a = load_tod("main_world", Some(179), 0).unwrap();
        let b = load_tod("main_world", Some(210), 0).unwrap();
        let actual = cache.sample_zones("main_world", &[(179, 0.8), (210, 0.6)], 0, 3.8, &areas, Some(179)).unwrap();
        let expected = TimeOfDay::sample_blended(&[(&area, 0.25), (&a, 0.8 - 0.25), (&b, 0.6 - 0.25)], 3.8, Lighting::default());
        assert_eq!(actual.sun, expected.sun);
        assert_eq!(actual.fog, expected.fog);
        // No spatial candidates: native current-zone fallback gets weight 1.
        let actual = cache.sample_zones("main_world", &[], 0, 3.8, &areas, Some(179)).unwrap();
        let expected = TimeOfDay::sample_blended(&[(&area, 0.25), (&a, 1.0)], 3.8, Lighting::default());
        assert_eq!(actual.sun, expected.sun);
        // Original manager300A4270 rejects the 11th input; area total in producer
        // still includes all area weights, independently of that manager cap.
        areas.inputs = (0..10).map(|_| (Arc::new(area.as_ref().clone()), 0.01)).collect();
        areas.total = 0.1;
        let actual = cache.sample_zones("main_world", &[(179, 1.0)], 0, 3.8, &areas, Some(179)).unwrap();
        let expected = area.sample(3.8, Lighting::default());
        assert!((actual.sun - expected.sun).length() < 0.00001);
    }

    #[test]
    fn native_directional_light_switches_at_the_horizon() {
        let mut sample = TimeOfDay::default().sample(0.0, Lighting::default());
        assert_eq!(directional_light_dir(&sample), sample.moon_dir);
        assert!((directional_light_dir(&sample) + sample.sun_dir).length() > 0.7);
        for elevation in [-f32::EPSILON, 0.0, f32::EPSILON] {
            sample.sun_dir = Vec3::new(1.0, 0.0, elevation).normalize();
            assert_eq!(directional_light_dir(&sample), if elevation > 0.0 { sample.sun_dir } else { sample.moon_dir });
        }
        let noon = TimeOfDay::default().sample(12.0, Lighting::default());
        assert_eq!(directional_light_dir(&noon), noon.sun_dir);
    }

    #[test]
    fn native_border_clock_override_world_change_and_unload() {
        if !crate::pak_available() {
            return;
        }
        let mut world = World::load("main_world").unwrap();
        world.load_sector_zones().unwrap();
        let mut materials = Assets::<SkyMaterial>::default();
        let dome_material = materials.add(SkyMaterial { params: default(), box_12: None, box_34: None, box_5: None });
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(0.25)))
            .insert_resource(WorldConfig::new("main_world"))
            .insert_resource(WorldInfo(Arc::new(world)))
            .insert_resource(WorldClock { hour: 10.0, zone: None, ..default() })
            .insert_resource(AtmosphereTuning::default())
            .insert_resource(Environment { sample: None, zone: None, hour: 0.0, ocean_fog: Vec3::ZERO, ocean_density: 0.0 })
            .init_resource::<TodCache>()
            .init_resource::<SkySelection>()
            .init_resource::<crate::area_tod::AreaToDState>()
            .init_resource::<GlobalAmbientLight>()
            .init_resource::<ClearColor>()
            .init_resource::<Assets<Image>>()
            .insert_resource(materials)
            .insert_resource(SkyAssets { dome_material, ocean_material: None })
            .add_systems(Update, (update_environment, update_sky).chain());
        let at = cry_to_bevy(Vec3::new(15712.0, 15808.0 - 0.01, 150.0));
        let camera = app.world_mut().spawn((Camera3d::default(), GlobalTransform::from_translation(at))).id();
        app.update();
        let a = app.world().resource::<Environment>().sample.as_ref().unwrap().clone();
        assert_eq!(app.world().resource::<Environment>().zone, Some(179));
        assert_eq!(app.world().resource::<TodCache>().zones.len(), 2);
        *app.world_mut().get_mut::<GlobalTransform>(camera).unwrap() = GlobalTransform::from_translation(at - Vec3::Z * 0.02);
        app.update();
        let b = app.world().resource::<Environment>().sample.as_ref().unwrap().clone();
        assert_eq!(app.world().resource::<Environment>().zone, Some(210));
        assert!((a.sun - b.sun).length() < 0.01);
        assert!((a.fog - b.fog).length() < 0.01);
        app.world_mut().resource_mut::<WorldClock>().hour = 3.8;
        app.update();
        let info = app.world().resource::<WorldInfo>();
        let pos = bevy_to_cry(app.world().get::<GlobalTransform>(camera).unwrap().translation());
        let weights = info.0.environment_weights(pos);
        let inputs: Vec<_> = weights.iter().map(|(zone, weight)| (load_tod("main_world", Some(*zone), 0).unwrap(), *weight)).collect();
        let refs: Vec<_> = inputs.iter().map(|(tod, weight)| (tod.as_ref(), *weight)).collect();
        let expected = TimeOfDay::sample_blended(&refs, 3.8, Lighting::default());
        let env = app.world().resource::<Environment>();
        assert_eq!(env.sample.as_ref().unwrap().sun, expected.sun, "clock change has no frame-rate dependent lag");
        assert_eq!(env.sample.as_ref().unwrap().sun_dir, expected.sun_dir);
        for _ in 0..8 { app.update(); }
        let selection = app.world().resource::<SkySelection>();
        assert!(selection.opacity < 0.001, "equal border weights hide discrete assets");
        app.world_mut().resource_mut::<WorldClock>().zone = Some(179);
        for _ in 0..9 { app.update(); }
        assert_eq!(app.world().resource::<SkySelection>().opacity, 1.0);
        app.world_mut().resource_mut::<WorldClock>().layer = 1;
        app.update();
        assert_eq!(app.world().resource::<SkySelection>().layer, 0);
        for _ in 0..7 { app.update(); }
        assert_eq!(app.world().resource::<SkySelection>().layer, 1);
        assert_eq!(app.world().resource::<SkySelection>().opacity, 0.0);
        let cache = app.world().resource::<TodCache>();
        assert_eq!(cache.zone_sky[&(Some(179), 1)].material.as_deref(), Some("Materials/sky/sky_salpi_nuimari_cbt4"));
        assert_eq!(cache.sky_boxes.len(), 1);
        assert!(cache.zone_sky.len() <= 2);
        app.world_mut().resource_mut::<WorldClock>().zone = Some(186);
        app.update();
        assert_eq!(app.world().resource::<TodCache>().zones.len(), 1);
        assert!(app.world().resource::<TodCache>().zones.contains_key(&(186, 1)));
        app.world_mut().resource_mut::<WorldConfig>().world = "login2".into();
        app.update();
        let cache = app.world().resource::<TodCache>();
        assert_eq!(cache.world.as_deref(), Some("login2"));
        assert_eq!(cache.lighting.unwrap().dawn_hour, 6.0);
        app.world_mut().resource_mut::<WorldConfig>().environment = false;
        app.update();
        assert!(app.world().resource::<Environment>().sample.is_none());
        assert!(app.world().resource::<TodCache>().zones.is_empty());
        assert!(app.world().resource::<TodCache>().world_default.is_empty());
        assert!(app.world().resource::<TodCache>().sky_boxes.is_empty());
        assert!(app.world().resource::<TodCache>().zone_sky.is_empty());
        assert_eq!(app.world().resource::<SkySelection>().zone, None);
        let sky = app.world().resource::<SkyAssets>();
        let material = app.world().resource::<Assets<SkyMaterial>>().get(&sky.dome_material).unwrap();
        assert!(material.box_12.is_none() && material.box_34.is_none() && material.box_5.is_none());
        app.world_mut().resource_mut::<WorldConfig>().environment = true;
        app.update();
        assert!(app.world().resource::<Environment>().sample.is_some());
    }
}
