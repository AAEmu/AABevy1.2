//! Sky, sun, ambient light, fog parameters and ocean from the time of day data.
//!
//! * [`WorldClock`] holds the hour (0..24); `speed` advances it (game hours per real second).
//! * The zone under the camera selects `zone/<id>/time_of_day.xml` (cached); the environment is sampled at the
//!   clock and smoothed, then drives:
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
use crate::env::{EnvSample, Lighting, TimeOfDay};
use crate::post::{Post, PostUniform};
use crate::sky_material::{SkyBox, SkyMaterial, ZoneSky, load_sky_box};
use crate::water::WaterStyle;
use crate::water_material::{SEA, WaterMaterial, WaterStyles};
use crate::stream::{WorldConfig, WorldFocus, WorldInfo};
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
    /// Exponent of the horizon -> zenith blend (larger keeps the horizon colour higher up).
    pub sky_exp: f32,
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
    /// Ambient light: how much of the sky dome's colour (horizon / zenith, blue grey) replaces `Sky color` (green yellow in the data).
    pub ambient_dome: f32,
    /// Water: multiplier of the volume's fog colour (the lit colour of the water body).
    pub water_body: f32,
    /// Water: absorption per metre = volume fog density x this.
    pub water_absorb: f32,
    /// Water: reflection gain on the Fresnel term.
    pub water_refl: f32,
    /// Water: sun glint multiplier.
    pub water_sun: f32,
    /// Water: screen space reflections on (1) or off (0).
    pub ssr: f32,
    /// Water: refraction offset (fraction of the screen per unit of bump).
    pub refraction: f32,
    /// Water: spread of the reflection rays (roughness of the surface).
    pub water_rough: f32,
    /// Sea: multiplier of the zone's ocean fog colour.
    pub sea_body: f32,
}

impl Default for AtmosphereTuning {
    fn default() -> Self {
        AtmosphereTuning {
            radiance: 540.0,
            fog_gain: 1.0,
            fog_ext: 0.04,
            glow: 0.05,
            sky_gain: 2.7,
            sky_exp: 1.0,
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
            ambient_gain: 0.9,
            ambient_sat: 0.3,
            ambient_dome: 1.0,
            ambient_spec: 0.2,
            water_body: 20.0,
            water_absorb: 6.0,
            water_refl: 0.5,
            water_sun: 3.0,
            ssr: 1.0,
            refraction: 0.012,
            water_rough: 0.06,
            sea_body: 2.5,
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
                "water_body" => t.water_body = v,
                "water_absorb" => t.water_absorb = v,
                "water_refl" => t.water_refl = v,
                "water_sun" => t.water_sun = v,
                "ssr" => t.ssr = v,
                "refraction" => t.refraction = v,
                "water_rough" => t.water_rough = v,
                "sea_body" => t.sea_body = v,
                _ => {}
            }
        }
        t
    }
}

/// The environment currently applied (smoothed).
#[derive(Resource, Clone, Debug)]
pub struct Environment {
    pub sample: Option<EnvSample>,
    pub zone: Option<u16>,
    pub hour: f32,
    /// The zone's `<Ocean FogColor>` (linear): the colour of the sea.
    pub ocean_fog: Vec3,
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
    zones: HashMap<(u16, u32), Option<Arc<TimeOfDay>>>,
    world_default: HashMap<u32, Option<Arc<TimeOfDay>>>,
    lighting: Option<Lighting>,
    sector_rx: Option<Mutex<Receiver<World>>>,
    zone_sky: HashMap<Option<u16>, ZoneSky>,
    sky_boxes: HashMap<String, Option<SkyBox>>,
}

/// Registers the sky systems.
pub fn build(app: &mut App) {
    crate::post::build(app);
    crate::sky_material::build(app);
    crate::water_material::build(app);
    crate::sky_clouds::build(app);
    app.init_resource::<WorldClock>()
        .insert_resource(Environment { sample: None, zone: None, hour: 13.0, ocean_fog: Vec3::new(0.01, 0.07, 0.3) })
        .init_resource::<TodCache>()
        .insert_resource(AtmosphereTuning::from_env())
        .add_systems(Startup, setup_sky)
        .add_systems(Update, (receive_zones, update_clock, update_environment, update_ambient, update_sky, crate::sky_clouds::update_clouds, update_post, crate::water_material::update_water).chain())
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
    // sector -> zone ownership comes from the big world.xml: parse it off the main thread
    let (tx, rx) = channel();
    let mut world = (*info.0).clone();
    std::thread::spawn(move || {
        if world.load_sector_zones().is_ok() {
            let _ = tx.send(world);
        }
    });
    cache.sector_rx = Some(Mutex::new(rx));
    cache.lighting = aa_pak::Pak::global()
        .read_string(&format!("worlds/{}/lighting.xml", cfg.world))
        .ok()
        .and_then(|x| Lighting::parse(&x));

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
        let style = WaterStyle::new(SEA, 0, [0.0; 3], 0.05);
        let material = water_styles.material(&style, &mut water_materials, &mut images);
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

fn receive_zones(mut cache: ResMut<TodCache>, mut info: ResMut<WorldInfo>) {
    let received = match &cache.sector_rx {
        Some(rx) => rx.lock().unwrap().try_recv().ok(),
        None => None,
    };
    if let Some(world) = received {
        info.0 = Arc::new(world);
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
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<SunLight>>,
    cameras: Query<(Entity, &GlobalTransform, Has<PostUniform>), With<Camera3d>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
    focus: Query<Entity, With<WorldFocus>>,
) {
    if !cfg.environment {
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
    let zone = clock.zone.or_else(|| info.0.zone_at(cry_pos));

    let layer = clock.layer;
    let tod = zone
        .and_then(|z| cache.zones.entry((z, layer)).or_insert_with(|| load_tod(&cfg.world, Some(z), layer)).clone())
        .or_else(|| cache.world_default.entry(layer).or_insert_with(|| load_tod(&cfg.world, None, layer)).clone());
    let Some(tod) = tod else { return };
    let lighting = cache.lighting.unwrap_or_default();
    let target = tod.sample(clock.hour, lighting);
    // glide between zones / times
    let k = 1.0 - (-time.delta_secs() * 3.0).exp();
    let cur = match (&env.sample, env.zone == zone) {
        (Some(prev), true) => prev.lerp(&target, k.max(0.02)),
        _ => target.clone(),
    };
    env.zone = zone;
    env.hour = clock.hour;
    if let Some((c, _)) = zone_sky_of(&mut cache, &cfg.world, zone).ocean_fog {
        env.ocean_fog = crate::env::srgb_to_linear(Vec3::from(c) / 255.0);
    }

    // --- sun / moon ---
    let elev = cur.sun_dir.z;
    let light_dir_cry = if elev >= -0.02 { cur.sun_dir } else { -cur.sun_dir };
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

fn desaturate(c: Vec3, s: f32) -> Vec3 {
    let l = luminance(c);
    Vec3::splat(l) + (c - Vec3::splat(l)) * s
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
    focus: Query<Entity, With<WorldFocus>>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    if !cfg.environment || tuning.ambient_env < 0.5 {
        return;
    }
    let Some(cur) = &env.sample else { return };
    let focus = focus.iter().next();
    let Some(camera) = cameras.iter().find(|e| Some(*e) == focus).or_else(|| cameras.iter().next()) else { return };

    let dome = unit_luminance(cur.horizon * 0.6 + cur.zenith * 0.4);
    let sky_dir = lerp3(unit_luminance(cur.sky), dome, tuning.ambient_dome);
    let ground_dir = lerp3(unit_luminance(cur.ambient_ground), dome, tuning.ambient_dome * 0.5);
    let scale = cfg.ambient_scale * tuning.ambient_gain;
    let top = desaturate(sky_dir, tuning.ambient_sat) * luminance(cur.sky) * scale;
    let bottom = desaturate(ground_dir, tuning.ambient_sat) * luminance(cur.ambient_ground) * scale;
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
fn zone_sky_of(cache: &mut TodCache, world: &str, zone: Option<u16>) -> ZoneSky {
    cache
        .zone_sky
        .entry(zone)
        .or_insert_with(|| {
            zone.and_then(|z| aa_pak::Pak::global().read_string(&format!("worlds/{world}/zone/{z}/env.xml")).ok())
                .map(|xml| ZoneSky::parse(&xml, "Zone"))
                .unwrap_or_default()
        })
        .clone()
}

/// The sky dome's gradient, sun and sky box.
#[allow(clippy::too_many_arguments)]
fn update_sky(
    env: Res<Environment>,
    cfg: Res<WorldConfig>,
    tuning: Res<AtmosphereTuning>,
    assets: Option<Res<SkyAssets>>,
    mut cache: ResMut<TodCache>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    mut images: ResMut<Assets<Image>>,
    exposure: Query<Option<&Exposure>, With<PostUniform>>,
) {
    let (Some(cur), Some(assets)) = (&env.sample, assets) else { return };
    if !cfg.environment {
        return;
    }
    // the zone's sky box (loaded once)
    let zone_sky = zone_sky_of(&mut cache, &cfg.world, env.zone);
    let sky_box = zone_sky.material.as_ref().and_then(|m| {
        cache
            .sky_boxes
            .entry(m.clone())
            .or_insert_with(|| {
                let b = load_sky_box(m, &mut images);
                info!("aa-world: sky box {m}: {:?}", b);
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
    m.params.horizon = (cur.horizon * rs * tuning.sky_gain).extend(tuning.sky_exp);
    m.params.zenith = (cur.zenith * rs * tuning.sky_gain).extend(0.0);
    m.params.sun_dir = sun_bevy.extend(sun_up * cur.day.max(0.05));
    m.params.sun_color = (sun_col * rs * tuning.sun_sky).extend(tuning.disc);
    match &sky_box {
        Some(b) => {
            m.params.box_sky = (cur.cloud_sky * b.sky_multiplier * rs * tuning.sky_gain).extend(b.opacity * tuning.box_gain);
            m.params.box_sun = (cur.cloud_sun * b.sun_multiplier * rs * tuning.sky_gain).extend(b.attenuation);
            let a = (zone_sky.angle + tuning.box_yaw).to_radians();
            m.params.box_params = Vec4::new(b.alpha_saturation, a.cos(), a.sin(), 1.0);
            m.box_12 = Some(b.box_12.clone());
            m.box_34 = Some(b.box_34.clone());
            m.box_5 = Some(b.box_5.clone());
        }
        None => {
            m.params.box_params.w = 0.0;
        }
    }
}

/// The post pass parameters of the applied environment (fog, film curve, grading).
fn update_post(
    env: Res<Environment>,
    cfg: Res<WorldConfig>,
    tuning: Res<AtmosphereTuning>,
    mut cameras: Query<(&Post, &mut PostUniform, Option<&Exposure>)>,
) {
    let Some(cur) = &env.sample else { return };
    for (post, mut u, exposure) in &mut cameras {
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
        u.sun_glow = (cur.sun * rs * tuning.glow * cur.day).extend(0.0);
        u.film = film;
        u.grade = grade * Vec4::new(tuning.saturation, 1.0, 1.0, tuning.exposure);
        u.balance = balance.truncate().extend(if post.fog { 1.0 } else { 0.0 });
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
