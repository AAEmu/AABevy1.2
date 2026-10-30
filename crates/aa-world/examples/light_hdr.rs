//! Native ordinary-point HDR inputs against original binary-upload receiver references.
//! --styled uses style6 at.125s; --legacy zeros HDR only in cloned diagnostic placements.
//! --world visits original dock Light_116; --day keeps that camera at12h.
//! --unload-check retires the original streamed cell/light entities.

use aa_world::objects::lights::{self, LocalLight, LocalLightPlugin, LocalLightStats};
use aa_world::objects::lights::styles::LightStyleSettings;
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;

const HDR: [f32; 6] = [-1.0, 0.0, 0.5, 0.75, 0.8, 1.5];
#[allow(clippy::approx_constant)] // native powf results, kept bit-exact
const NATIVE_FACTORS: [f32; 6] = [0.5, 1.0, 1.4142135, 1.6817929, 1.7411011, 2.828427];
const BASE: Vec3 = Vec3::new(0.7, 0.4, 0.2);
const DOCK_GUID: u64 = 0x405c2f3b9f41a176;
#[derive(Resource)]
struct Fixture {
    roots: Vec<Entity>,
    references: Vec<Entity>,
    world: bool,
    legacy: bool,
    unload: bool,
    styled: bool,
    moved: bool,
    checked: bool,
    done: bool,
}
fn flag(name: &str) -> bool { std::env::args().any(|a| a == name) }

fn main() {
    let world = flag("--world");
    let mut cfg = WorldConfig::new("main_world");
    cfg.load_radius = 180.0;
    cfg.unload_radius = 300.0;
    cfg.brush_distance = 200.0;
    cfg.water_distance = 220.0;
    cfg.vegetation = false;
    cfg.shadows = false;
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("Native point-light HDR"), aa_dev::DevPlugin));
    if world {
        app.add_plugins(AaWorldPlugin { config: cfg })
            .insert_resource(WorldClock { hour: if flag("--day") { 12.0 } else { 0.0 }, ..default() });
    } else {
        app.insert_resource(cfg).add_plugins(LocalLightPlugin)
            .insert_resource(GlobalAmbientLight { brightness: 0.0, ..default() });
    }
    app.insert_resource(LightStyleSettings { enabled: true, seconds: Some(0.125) })
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Fixture { roots: Vec::new(), references: Vec::new(), world,
            legacy: flag("--legacy"), unload: flag("--unload-check"), styled: flag("--styled"),
            moved: false, checked: false, done: false })
        .add_systems(Startup, setup)
        .add_systems(PreUpdate, diagnostic_unity)
        .add_systems(Update, verify)
        .add_observer(pixels)
        .run();
}

fn setup(
    mut commands: Commands,
    mut fixture: ResMut<Fixture>,
    cfg: Res<WorldConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let transform = if fixture.world {
        let data = aa_world::placement::load_cell_objects("main_world", 18, 7);
        let lamp = data.lights.iter().find(|p| p.guid == DOCK_GUID).expect("original dock lamp");
        assert_eq!(lamp.hdr_dynamic, 1.0);
        info!("native HDR scene guid={:x} hdr={} base_rgb={:?} diagnostic_unity={}", lamp.guid, lamp.hdr_dynamic, lamp.diffuse, fixture.legacy);
        Transform::from_translation(cry_to_bevy(Vec3::new(18961.0, 7745.0, 105.5)))
            .looking_at(cry_to_bevy(Vec3::new(18967.62, 7734.7, 104.0)), Vec3::Y)
    } else {
        let style = if fixture.styled { 6 } else { 0 };
        let gain = if fixture.styled { 0.125 } else { 1.0 };
        for (i, hdr) in HDR.into_iter().enumerate() {
            let x = (i as f32 - 2.5) * 6.0;
            let source_hdr = if fixture.legacy { 0.0 } else { hdr };
            let xml = format!(r#"<Mission><Objects><Entity EntityClass="Light" Name="hdr{i}" EntityGuid="{:x}" Pos="{x},3,2"><Properties Radius="3"><Color clrDiffuse="0.7,0.4,0.2" fHDRDynamic="{source_hdr}"/><Options bCastShadow="0"/><Style nLightStyle="{style}"/></Properties></Entity></Objects></Mission>"#, i + 1);
            fixture.roots.extend(lights::spawn_cell(&mut commands, (0, 0), &aa_world::entities::parse_lights(&xml)));
            // Independent original38179800 uploads; keep native multiplication order.
            let factor = if fixture.legacy { 1.0 } else { NATIVE_FACTORS[i] };
            let reference = (BASE * gain) * factor;
            fixture.references.push(commands.spawn((
                PointLight { color: Color::linear_rgb(reference.x, reference.y, reference.z),
                    intensity: std::f32::consts::TAU * 2.0 * 0.16 * 9.0 * lights::lux_per_unit(&cfg),
                    range: 3.0, shadow_maps_enabled: false, ..default() },
                Transform::from_xyz(x, 2.0, 3.0),
            )).id());
            for z in [-3.0, 3.0] {
                commands.spawn((Mesh3d(meshes.add(Rectangle::new(4.5, 4.5))),
                    MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::WHITE,
                        perceptual_roughness: 1.0, reflectance: 0.0, ..default() })),
                    Transform::from_xyz(x, 0.0, z).with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))));
            }
        }
        commands.spawn((Text::new("Original point-light HDR -1 / 0 / .5 / .75 / .8 / 1.5\nTop: production; bottom: original binary-upload references. --styled / --legacy"),
            TextFont { font_size: FontSize::Px(16.0), ..default() },
            Node { position_type: PositionType::Absolute, left: px(12.0), top: px(12.0), ..default() }));
        Transform::from_xyz(0.0, 30.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z)
    };
    let projection = if fixture.world {
        Projection::Perspective(PerspectiveProjection { fov: 50.0_f32.to_radians(), near: 0.1, far: 20000.0, ..default() })
    } else {
        Projection::Orthographic(OrthographicProjection { scaling_mode: bevy::camera::ScalingMode::FixedVertical { viewport_height: 20.0 },
            near: 0.1, far: 100.0, ..OrthographicProjection::default_3d() })
    };
    let exposure = if fixture.world { Exposure::BLENDER } else { Exposure { ev100: if fixture.styled { 6.0 } else { 9.0 } } };
    commands.spawn((Camera3d::default(), transform, projection, exposure,
        Tonemapping::None, DebandDither::Disabled, Msaa::Off));
}

fn diagnostic_unity(fixture: Res<Fixture>, mut lights: Query<&mut LocalLight>) {
    // Fixture-only control: zero cloned placement HDR, leave every other authored input intact.
    if fixture.legacy {
        for mut slot in &mut lights { slot.placement.hdr_dynamic = 0.0; }
    }
}

fn verify(
    mut commands: Commands,
    time: Res<Time>,
    stats: Res<LocalLightStats>,
    lights: Query<(&LocalLight, &PointLight)>,
    slots: Query<Entity, With<LocalLight>>,
    points: Query<&PointLight>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut fixture: ResMut<Fixture>,
) {
    if !fixture.checked && time.elapsed_secs() > 8.0 {
        assert!(stats.rendered > 0 && stats.rendered <= 64);
        if fixture.world {
            let (slot, point) = lights.iter().find(|(slot, _)| slot.placement.guid == DOCK_GUID).expect("admitted actual dock source");
            let wanted = slot.placement.diffuse * if fixture.legacy { 1.0 } else { 2.0 };
            assert_eq!(point.color.to_linear().to_vec3(), wanted);
        } else {
            assert_eq!(lights.iter().count(), 6);
            for (slot, light) in &lights {
                let i = slot.placement.guid as usize - 1;
                let gain = if fixture.styled { 0.125 } else { 1.0 };
                let factor = if fixture.legacy { 1.0 } else { NATIVE_FACTORS[i] };
                assert!(light.color.to_linear().to_vec3().abs_diff_eq((BASE * gain) * factor, 2e-6));
            }
        }
        info!("native HDR source/upload PASSED rendered={} shadowed={} legacy={}", stats.rendered, stats.shadowed, fixture.legacy);
        fixture.checked = true;
    }
    if !fixture.unload || fixture.done { return; }
    if !fixture.moved && time.elapsed_secs() > 12.0 {
        assert!(fixture.checked);
        if fixture.world {
            cameras.single_mut().unwrap().translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        } else {
            for e in fixture.roots.iter().chain(&fixture.references) { commands.entity(*e).despawn(); }
        }
        fixture.moved = true;
    } else if fixture.moved && time.elapsed_secs() > 18.0 {
        assert_eq!(lights.iter().count(), 0);
        assert_eq!(slots.iter().count(), 0);
        assert_eq!(stats.rendered, 0);
        if !fixture.world { assert_eq!(points.iter().count(), 0); }
        info!("native HDR unload PASSED: zero authored/rendered light components; no HDR assets allocated");
        fixture.done = true;
    }
}

fn pixels(shot: On<ScreenshotCaptured>, fixture: Res<Fixture>) {
    if fixture.world || fixture.unload { return; }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, hdr) in HDR.into_iter().enumerate() {
        let x = (w as f32 * 0.5 + (i as f32 - 2.5) * 6.0 * h as f32 / 20.0) as u32;
        let upper = image.get_pixel(x, (h as f32 * 0.5 - 3.0 * h as f32 / 20.0) as u32).0;
        let lower = image.get_pixel(x, (h as f32 * 0.5 + 3.0 * h as f32 / 20.0) as u32).0;
        assert!(upper.iter().any(|&v| v > 5) && upper.iter().all(|&v| v < 250), "HDR{hdr}: meaningful unclipped receiver required, got{upper:?}");
        assert!(upper.iter().zip(lower).all(|(a, b)| a.abs_diff(b) <= 3), "HDR{hdr}: production{upper:?} native reference{lower:?}");
        info!("native HDR{hdr} GPU PASSED production={upper:?} native_reference={lower:?}");
    }
}
