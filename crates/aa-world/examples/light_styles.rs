//! Original LightStyles on production point lights, with independent native-upload references.
//! --phase-one selects1s instead of0.125s; --legacy keeps the former unity style.
//! --world visits an actual styled workshop light; --live follows normal game seconds.
//! --unload-check retires the actual source light roots / streamed cell components.

use aa_world::objects::lights::{self, LocalLight, LocalLightPlugin, LocalLightStats};
use aa_world::objects::lights::styles::{LightStyleSettings, LightStyles};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;

const INDICES: [u16; 6] = [0, 2, 4, 6, 12, 19];
const BASE: Vec3 = Vec3::new(0.7, 0.4, 0.2);
#[derive(Resource)]
struct Fixture {
    roots: Vec<Entity>,
    references: Vec<Entity>,
    world: bool,
    unload: bool,
    expected: [f32; 6],
    moved: bool,
    checked: bool,
    done: bool,
}
fn flag(name: &str) -> bool { std::env::args().any(|a| a == name) }

fn main() {
    let world = flag("--world");
    assert!(world || !flag("--live"), "--live is the original-world motion control; numeric captures use fixed native phases");
    let second = flag("--phase-one");
    let seconds = if second { 1.0 } else { 0.125 };
    let expected = if flag("--legacy") { [1.0; 6] } else if second {
        [1.0, 1.0, 0.58999997, 1.0, 0.5, 1.0]
    } else { [1.0, 1.0, 0.33625, 0.125, 0.75, 0.775] };
    let mut cfg = WorldConfig::new("main_world");
    cfg.load_radius = 120.0;
    cfg.unload_radius = 250.0;
    cfg.brush_distance = 160.0;
    cfg.water_distance = 160.0;
    cfg.vegetation = false;
    cfg.shadows = false;
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("Native local-light styles"), aa_dev::DevPlugin));
    if world {
        app.add_plugins(AaWorldPlugin { config: cfg }).insert_resource(WorldClock { hour: 22.0, ..default() });
    } else {
        app.insert_resource(cfg).add_plugins(LocalLightPlugin)
            .insert_resource(GlobalAmbientLight { brightness: 0.0, ..default() });
    }
    app.insert_resource(LightStyleSettings { enabled: !flag("--legacy"), seconds: (!flag("--live")).then_some(seconds) })
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Fixture { roots: Vec::new(), references: Vec::new(), world,
            unload: flag("--unload-check"), expected, moved: false, checked: false, done: false })
        .add_systems(Startup, setup)
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
        let data = aa_world::placement::load_cell_objects("main_world", 21, 8);
        let light = data.lights.iter().find(|p| p.style == 6 && p.active && !p.hidden).expect("actual styled workshop light");
        let target = aa_world::coords::cell_origin(21, 8) + light.matrix.w_axis.truncate();
        info!("native style scene guid={:x} style={} pos={target:?} color={:?}", light.guid, light.style, light.diffuse);
        Transform::from_translation(cry_to_bevy(target + Vec3::new(4.0, -8.0, 2.0)))
            .looking_at(cry_to_bevy(target - Vec3::Z * 2.0), Vec3::Y)
    } else {
        for (i, style) in INDICES.into_iter().enumerate() {
            let x = (i as f32 - 2.5) * 6.0;
            let xml = format!(r#"<Mission><Objects><Entity EntityClass="Light" Name="style{style}" Pos="{x},3,2"><Properties Radius="3"><Color clrDiffuse="0.7,0.4,0.2" fDiffuseMultiplier="1"/><Options bCastShadow="0"/><Style nLightStyle="{style}"/></Properties></Entity></Objects></Mission>"#);
            fixture.roots.extend(lights::spawn_cell(&mut commands, (0, 0), &aa_world::entities::parse_lights(&xml)));
            // Original38179800 upload goldens, independently executed for each phase.
            let gain = fixture.expected[i];
            fixture.references.push(commands.spawn((
                PointLight { color: Color::linear_rgb(BASE.x * gain, BASE.y * gain, BASE.z * gain),
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
        commands.spawn((Text::new("Original point-light styles 0 / 2 / 4 / 6 / 12 / 19\nTop: production source tables; bottom: original binary-upload references. --phase-one / --legacy"),
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
    let exposure = if fixture.world { Exposure::BLENDER } else { Exposure { ev100: 8.0 } };
    commands.spawn((Camera3d::default(), transform, projection, exposure,
        Tonemapping::None, DebandDither::Disabled, Msaa::Off));
}

fn verify(
    mut commands: Commands,
    time: Res<Time>,
    styles: Res<LightStyles>,
    settings: Res<LightStyleSettings>,
    stats: Res<LocalLightStats>,
    lights: Query<(&LocalLight, &PointLight)>,
    points: Query<&PointLight>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut fixture: ResMut<Fixture>,
) {
    if !fixture.checked && time.elapsed_secs() > 8.0 {
        assert_eq!(styles.count(), 37);
        assert!(stats.rendered > 0 && stats.rendered <= 64);
        if !fixture.world {
            assert_eq!(lights.iter().count(), 6);
            for (slot, light) in &lights {
                let i = INDICES.iter().position(|&i| i == slot.placement.style).unwrap();
                let actual = light.color.to_linear().to_vec3();
                assert!(actual.abs_diff_eq(BASE * fixture.expected[i], 1e-7));
            }
        }
        info!("native styles source/upload PASSED tables={} rendered={} game_seconds={:?}", styles.count(), stats.rendered, settings.seconds);
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
        assert_eq!(stats.rendered, 0);
        if !fixture.world { assert_eq!(points.iter().count(), 0); }
        assert_eq!(styles.count(), 37, "native global source tables retain renderer lifetime");
        info!("native styles unload PASSED: zero source light components; tables remain shared");
        fixture.done = true;
    }
}

fn pixels(shot: On<ScreenshotCaptured>, fixture: Res<Fixture>) {
    if fixture.world || fixture.unload { return; }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, style) in INDICES.into_iter().enumerate() {
        let x = (w as f32 * 0.5 + (i as f32 - 2.5) * 6.0 * h as f32 / 20.0) as u32;
        let upper = image.get_pixel(x, (h as f32 * 0.5 - 3.0 * h as f32 / 20.0) as u32).0;
        let lower = image.get_pixel(x, (h as f32 * 0.5 + 3.0 * h as f32 / 20.0) as u32).0;
        assert!(upper.iter().any(|&v| v > 5) && upper.iter().all(|&v| v < 250), "style{style}: meaningful unclipped receiver required, got{upper:?}");
        assert!(upper.iter().zip(lower).all(|(a, b)| a.abs_diff(b) <= 3), "style{style}: production{upper:?} native reference{lower:?}");
        info!("native style{style} GPU PASSED production={upper:?} native_reference={lower:?}");
    }
}
