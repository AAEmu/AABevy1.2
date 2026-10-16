//! Sky, sun, ambient light, fog and ocean from the time of day data.
//!
//! * [`WorldClock`] holds the hour (0..24); `speed` advances it (game hours per real second).
//! * The zone under the camera selects `zone/<id>/time_of_day.xml` (cached); the environment is sampled at the
//!   clock and smoothed, then drives:
//!   * a directional sun / moon light (colour and strength from `Sun color * multiplier`, direction from
//!     the sun path of `lighting.xml`; at night the light comes from the moon side),
//!   * ambient light (`Sky color` / `Ambient ground color`),
//!   * the camera's distance fog and clear colour (`Fog color`),
//!   * a sky dome with a horizon -> zenith gradient, sun glow and a sun disc,
//!   * a large ocean plane at the world's ocean level, centred on the camera.
//!
//! All of it is deliberately simple geometry with the engine's colours, no custom shaders.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, channel};

use bevy::asset::RenderAssetUsages;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::Face;

use crate::coords::{bevy_to_cry, cry_to_bevy};
use crate::env::{EnvSample, Lighting, TimeOfDay};
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
}

impl Default for WorldClock {
    fn default() -> Self {
        WorldClock { hour: 13.0, speed: 0.0, zone: None }
    }
}

/// The environment currently applied (smoothed).
#[derive(Resource, Clone, Debug)]
pub struct Environment {
    pub sample: Option<EnvSample>,
    pub zone: Option<u16>,
    pub hour: f32,
}

#[derive(Component)]
pub struct SunLight;
#[derive(Component)]
pub struct SkyDome;
#[derive(Component)]
pub struct SunDisc;
#[derive(Component)]
pub struct OceanPlane;

#[derive(Resource)]
pub struct SkyAssets {
    dome_mesh: Handle<Mesh>,
    sun_material: Handle<StandardMaterial>,
    ocean_material: Handle<StandardMaterial>,
}

/// Cached zone time of day files.
#[derive(Resource, Default)]
pub struct TodCache {
    zones: HashMap<u16, Option<Arc<TimeOfDay>>>,
    world_default: Option<Option<Arc<TimeOfDay>>>,
    lighting: Option<Lighting>,
    sector_rx: Option<Mutex<Receiver<World>>>,
}

const DOME_RINGS: usize = 24;
const DOME_SEGMENTS: usize = 48;

/// Registers the sky systems.
pub fn build(app: &mut App) {
    app.init_resource::<WorldClock>()
        .insert_resource(Environment { sample: None, zone: None, hour: 13.0 })
        .init_resource::<TodCache>()
        .add_systems(Startup, setup_sky)
        .add_systems(Update, (receive_zones, update_clock, update_environment).chain())
        .add_systems(PostUpdate, follow_camera.before(bevy::transform::TransformSystems::Propagate));
}

fn dome_vertices() -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut pos = Vec::new();
    for r in 0..=DOME_RINGS {
        // elevation from slightly below the horizon to the zenith
        let e = -0.12 + (r as f32 / DOME_RINGS as f32) * (std::f32::consts::FRAC_PI_2 + 0.12);
        for s in 0..=DOME_SEGMENTS {
            let a = s as f32 / DOME_SEGMENTS as f32 * std::f32::consts::TAU;
            pos.push([a.cos() * e.cos(), e.sin(), a.sin() * e.cos()]);
        }
    }
    let mut idx = Vec::new();
    let stride = (DOME_SEGMENTS + 1) as u32;
    for r in 0..DOME_RINGS as u32 {
        for s in 0..DOME_SEGMENTS as u32 {
            let a = r * stride + s;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            // seen from inside
            idx.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    (pos, idx)
}

fn setup_sky(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    info: Res<WorldInfo>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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

    let (pos, idx) = dome_vertices();
    let n = pos.len();
    let mut dome = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    dome.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    dome.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3f32, 0.5, 0.8, 1.0]; n]);
    dome.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, -1.0, 0.0]; n]);
    dome.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; n]);
    dome.insert_indices(Indices::U32(idx));
    let dome_mesh = meshes.add(dome);
    let dome_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        fog_enabled: false,
        cull_mode: Some(Face::Back),
        ..default()
    });
    commands.spawn((
        SkyDome,
        Mesh3d(dome_mesh.clone()),
        MeshMaterial3d(dome_material),
        Transform::from_scale(Vec3::splat(cfg.sky_radius)),
        NotShadowCaster,
        NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    ));
    let sun_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.95, 0.8),
        unlit: true,
        fog_enabled: false,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        SunDisc,
        Mesh3d(meshes.add(Circle::new(cfg.sky_radius * 0.035))),
        MeshMaterial3d(sun_material.clone()),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        bevy::camera::visibility::NoFrustumCulling,
    ));
    let ocean_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.06, 0.28, 0.5, 0.88),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.12,
        reflectance: 0.5,
        cull_mode: None,
        ..default()
    });
    if info.0.ocean_level > 0.0 {
        commands.spawn((
            OceanPlane,
            Mesh3d(meshes.add(Plane3d::default().mesh().size(cfg.sky_radius * 2.4, cfg.sky_radius * 2.4))),
            MeshMaterial3d(ocean_material.clone()),
            Transform::from_xyz(0.0, info.0.ocean_level, 0.0),
            NotShadowCaster,
        ));
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
    commands.insert_resource(SkyAssets { dome_mesh, sun_material, ocean_material });
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

fn load_tod(world: &str, zone: Option<u16>) -> Option<Arc<TimeOfDay>> {
    let pak = aa_pak::Pak::global();
    let candidates = zone
        .map(|z| format!("worlds/{world}/zone/{z}/time_of_day.xml"))
        .into_iter()
        .chain([format!("worlds/{world}/time_of_day.xml")]);
    for path in candidates {
        if let Ok(xml) = pak.read_string(&path) {
            if let Some(t) = TimeOfDay::parse(&xml) {
                return Some(Arc::new(t));
            }
        }
    }
    None
}

fn lerp3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

fn smooth(prev: &EnvSample, next: &EnvSample, t: f32) -> EnvSample {
    EnvSample {
        sun_dir: lerp3(prev.sun_dir, next.sun_dir, t).normalize_or(next.sun_dir),
        sun: lerp3(prev.sun, next.sun, t),
        sky: lerp3(prev.sky, next.sky, t),
        ambient_ground: lerp3(prev.ambient_ground, next.ambient_ground, t),
        fog: lerp3(prev.fog, next.fog, t),
        fog_density: prev.fog_density + (next.fog_density - prev.fog_density) * t,
        ocean_fog: lerp3(prev.ocean_fog, next.ocean_fog, t),
        sky_srgb: lerp3(prev.sky_srgb, next.sky_srgb, t),
        fog_srgb: lerp3(prev.fog_srgb, next.fog_srgb, t),
        night_horizon: lerp3(prev.night_horizon, next.night_horizon, t),
        night_zenith: lerp3(prev.night_zenith, next.night_zenith, t),
        day: prev.day + (next.day - prev.day) * t,
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn srgb_to_linear(v: Vec3) -> Vec3 {
    let f = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    Vec3::new(f(v.x), f(v.y), f(v.z))
}

#[allow(clippy::too_many_arguments)]
pub fn update_environment(
    mut commands: Commands,
    time: Res<Time>,
    cfg: Res<WorldConfig>,
    info: Res<WorldInfo>,
    clock: Res<WorldClock>,
    mut env: ResMut<Environment>,
    mut cache: ResMut<TodCache>,
    assets: Option<Res<SkyAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut sun: Query<(&mut DirectionalLight, &mut Transform), With<SunLight>>,
    mut discs: Query<&mut Visibility, With<SunDisc>>,
    cameras: Query<(Entity, &GlobalTransform, Option<&DistanceFog>), With<Camera3d>>,
    focus: Query<Entity, With<WorldFocus>>,
) {
    if !cfg.environment {
        return;
    }
    let Some(assets) = assets else { return };
    let focus_entity = focus.iter().next();
    let Some((cam_entity, cam_tf, fog)) = cameras
        .iter()
        .find(|(e, ..)| Some(*e) == focus_entity)
        .or_else(|| cameras.iter().next())
    else {
        return;
    };
    let cry_pos = bevy_to_cry(cam_tf.translation());
    let zone = clock.zone.or_else(|| info.0.zone_at(cry_pos));

    let tod = zone
        .and_then(|z| cache.zones.entry(z).or_insert_with(|| load_tod(&cfg.world, Some(z))).clone())
        .or_else(|| cache.world_default.get_or_insert_with(|| load_tod(&cfg.world, None)).clone());
    let Some(tod) = tod else { return };
    let lighting = cache.lighting.unwrap_or_default();
    let target = tod.sample(clock.hour, lighting);
    // glide between zones / times
    let k = 1.0 - (-time.delta_secs() * 3.0).exp();
    let cur = match (&env.sample, env.zone == zone) {
        (Some(prev), true) => smooth(prev, &target, k.max(0.02)),
        _ => target.clone(),
    };
    env.zone = zone;
    env.hour = clock.hour;

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
    // --- ambient ---
    let hemi = (cur.sky + cur.ambient_ground) * 0.5;
    let hemi_lum = luminance(hemi).max(1e-4);
    ambient.color = Color::linear_rgb((hemi.x / hemi_lum).min(2.0), (hemi.y / hemi_lum).min(2.0), (hemi.z / hemi_lum).min(2.0));
    ambient.brightness = cfg.ambient_scale * hemi_lum;

    // --- sky colours (LDR) ---
    let day = cur.day;
    let night_scale = 0.18;
    let horizon_day = srgb_to_linear(cur.fog_srgb * 1.1).min(Vec3::splat(1.0));
    let zenith_day = srgb_to_linear(cur.sky_srgb);
    let horizon_night = srgb_to_linear(cur.night_horizon) * night_scale;
    let zenith_night = srgb_to_linear(cur.night_zenith) * night_scale * 0.6;
    let horizon = lerp3(horizon_night, horizon_day, day);
    let zenith = lerp3(zenith_night, zenith_day, day);
    let sun_glow_col = srgb_to_linear(Vec3::new(1.0, 0.85, 0.6)) * (0.35 * day);
    if let Some(mut mesh) = meshes.get_mut(&assets.dome_mesh) {
        let sun_dir_bevy = cry_to_bevy(cur.sun_dir).normalize();
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            let colors: Vec<[f32; 4]> = pos
                .iter()
                .map(|p| {
                    let d = Vec3::from(*p).normalize();
                    let e = d.y.max(0.0);
                    let t = e.powf(0.55);
                    let mut c = lerp3(horizon, zenith, t);
                    // below the horizon: fog colour
                    if d.y < 0.0 {
                        c = horizon;
                    }
                    let g = d.dot(sun_dir_bevy).max(0.0);
                    c += sun_glow_col * (g.powf(12.0) * 0.6 + g.powf(180.0) * 2.0);
                    [c.x, c.y, c.z, 1.0]
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
    }
    if let Some(mut m) = materials.get_mut(&assets.sun_material) {
        let c = cur.sun / luminance(cur.sun).max(1e-3);
        m.base_color = Color::linear_rgb(c.x.min(1.0) * 1.0, c.y.min(1.0), c.z.min(1.0));
    }
    for mut vis in &mut discs {
        *vis = if elev > -0.03 { Visibility::Inherited } else { Visibility::Hidden };
    }
    if let Some(mut m) = materials.get_mut(&assets.ocean_material) {
        let deep = srgb_to_linear(Vec3::new(32.0, 112.0, 208.0) / 255.0) * (0.25 + 0.75 * day);
        m.base_color = Color::linear_rgba(deep.x, deep.y, deep.z, 0.9);
    }

    // --- fog + clear colour on the camera ---
    let fog_color = lerp3(horizon_night, horizon_day, day);
    let visibility = cfg.fog_visibility / (0.5 + cur.fog_density * 2.0);
    let new_fog = DistanceFog {
        color: Color::linear_rgb(fog_color.x, fog_color.y, fog_color.z),
        directional_light_color: Color::linear_rgb(sun_glow_col.x * 1.5, sun_glow_col.y * 1.5, sun_glow_col.z * 1.5),
        directional_light_exponent: 30.0,
        falloff: FogFalloff::from_visibility_squared(visibility),
    };
    let _ = fog;
    commands.entity(cam_entity).insert(new_fog);
    clear.0 = Color::linear_rgb(fog_color.x, fog_color.y, fog_color.z);
    env.sample = Some(cur);
}

/// Dome, sun disc and ocean follow the camera.
fn follow_camera(
    cfg: Res<WorldConfig>,
    env: Res<Environment>,
    cameras: Query<&Transform, (With<Camera3d>, Without<SkyDome>, Without<OceanPlane>, Without<SunDisc>)>,
    mut dome: Query<&mut Transform, (With<SkyDome>, Without<OceanPlane>, Without<SunDisc>)>,
    mut ocean: Query<&mut Transform, (With<OceanPlane>, Without<SkyDome>, Without<SunDisc>)>,
    mut disc: Query<&mut Transform, (With<SunDisc>, Without<SkyDome>, Without<OceanPlane>)>,
) {
    let Some(cam) = cameras.iter().next() else { return };
    let p = cam.translation;
    if let Some(sample) = &env.sample {
        for mut t in &mut disc {
            t.translation = p + cry_to_bevy(sample.sun_dir).normalize() * cfg.sky_radius * 0.96;
            t.look_at(p, Vec3::Y);
        }
    }
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
