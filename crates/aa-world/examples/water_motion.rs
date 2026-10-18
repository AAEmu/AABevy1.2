//! Original WaterSurface UV/motion GPU controls and original river/pond streaming fixture.
//! Default: numeric oracle. --world [--pond] [--legacy] [--unload-check]: original volumes.

use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::water_material::{WaterMaterial, WaterMaterialPlugin, WaterParams};
use bevy::asset::AssetId;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;
use std::collections::{HashMap, HashSet};

#[derive(Resource)]
struct Controls([[u8; 3]; 4]);

#[derive(Resource, Default)]
struct Fixture {
    reference: u8,
    pond: bool,
    legacy: bool,
    unload: bool,
    moved: bool,
    done: bool,
    meshes: HashSet<AssetId<Mesh>>,
    controls: HashMap<AssetId<WaterMaterial>, Handle<WaterMaterial>>,
    last: f32,
}

fn main() {
    let flag = |s: &str| std::env::args().any(|a| a == s);
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native water authored UV / wave motion"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 180.0;
        config.unload_radius = 300.0;
        config.water_distance = 300.0;
        config.brush_distance = 180.0;
        config.tree_distance = 180.0;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: if flag("--reference05") {
                    2.80975842
                } else if flag("--reference10") {
                    3.76261711
                } else {
                    10.0
                },
                ..default()
            })
            .insert_resource(Fixture {
                reference: if flag("--reference05") {
                    5
                } else if flag("--reference10") {
                    10
                } else {
                    0
                },
                pond: flag("--pond"),
                legacy: flag("--legacy"),
                unload: flag("--unload-check"),
                ..default()
            })
            .add_systems(Startup, world_camera)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(WaterMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}

fn sample(rgba: &[u8], width: u32, height: u32, uv: Vec2) -> Vec2 {
    let p = uv * Vec2::new(width as f32, height as f32) - Vec2::splat(0.5);
    let lo = p.floor();
    let f = p - lo;
    let texel = |x: i32, y: i32| {
        let i =
            (y.rem_euclid(height as i32) * width as i32 + x.rem_euclid(width as i32)) as usize * 4;
        Vec2::new(rgba[i] as f32, rgba[i + 1] as f32) / 255.0
    };
    texel(lo.x as i32, lo.y as i32)
        .lerp(texel(lo.x as i32 + 1, lo.y as i32), f.x)
        .lerp(
            texel(lo.x as i32, lo.y as i32 + 1).lerp(texel(lo.x as i32 + 1, lo.y as i32 + 1), f.x),
            f.y,
        )
}

fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let mut expected = [[0; 3]; 4];
    for i in 0..4 {
        let no_flow = i >= 2;
        let time = if i % 2 == 0 { 0.0 } else { 4.0 };
        let mtl = CryMaterial::load(if no_flow {
            "materials/ocean/lake_tank.mtl"
        } else {
            "materials/ocean/valley_river_watervolume1.mtl"
        })
        .unwrap();
        assert_eq!(mtl.gen_mask & 0x200 != 0, no_flow);
        let path = mtl.texture(&TextureMap::Bump).unwrap().pak_path();
        let bytes = aa_cry::read(&path).unwrap();
        let mut dds = Dds::parse(&bytes).unwrap();
        dds.swapped_rg = false;
        let rgba = dds.decode_rgba8(0, 0);
        let uv = Vec2::new(0.173, 0.283);
        let param = |s: &str| mtl.public_param(s).unwrap().parse::<f32>().unwrap();
        let speed = param("WavesSpeed");
        let drift = Vec2::new(time * speed * 0.0125, 0.0);
        let taps = if no_flow {
            [
                uv * 2.0 + drift * 2.0,
                uv * 3.0 - drift * 2.0,
                uv * 4.0 + drift * 2.0,
                uv * 5.0 - drift * 2.0,
            ]
        } else {
            [
                uv * 4.0 + drift * 2.0,
                uv * 2.0 + drift * 3.0,
                uv * 8.0 + drift * 2.0,
                uv * 16.0 + drift * 3.0,
            ]
        };
        let samples = taps.map(|uv| sample(&rgba, dds.width, dds.height, uv));
        let big = param("BigWavesScale");
        let small = param("SmallWavesScale");
        let bump = param("BumpScale");
        let xy = ((samples[0] + samples[1] - Vec2::ONE) * big
            + (samples[2] + samples[3] - Vec2::ONE) * small)
            * bump;
        let normal = Vec3::new(xy.x, 1.0, -xy.y).normalize();
        let colour = normal * 0.5 + Vec3::splat(0.5);
        expected[i] = colour.to_array().map(|v| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        let image = images.add(dds_to_image(&dds, DdsOptions::linear()).unwrap());
        let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![uv.to_array(); 4]);
        let params = WaterParams {
            waves: Vec4::new(big, small, speed, 0.2),
            refl: Vec4::new(0.0, 0.0, bump, 1.0),
            motion: Vec4::new(1.0, if no_flow { 1.0 } else { 0.0 }, time, 1.0),
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(WaterMaterial {
                params,
                normal_map: Some(image),
                foam_map: None,
                env_map: None,
            })),
            Transform::from_xyz(i as f32 * 2.0 - 3.0, 0.0, -1.0)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1.5, 1.5))),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(i as f32 * 2.0 - 3.0, 0.0, 1.0)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        info!(
            "native water GPU case{i}: NO_FLOW={no_flow} time={time} uv={uv:?} taps={taps:?} raw_samples={samples:?} expected_srgb={:?}",
            expected[i]
        );
    }
    commands.insert_resource(Controls(expected));
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 6.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 6.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new("Original river flow t=0 / t=4 / pond NO_FLOW t=0 / t=4\nTop: production wave shader; bottom: independent original DDS bilinear controls\nNative mesh UVs + PB_time 0.0125 + original material scales; raw texture XY"),TextFont {font_size:FontSize::Px(20.0),..default()},TextColor(Color::WHITE),Node {position_type:PositionType::Absolute,top:px(12.0),left:px(24.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for i in 0..4 {
        let x = (w as i32 / 2 + (i as i32 * 2 - 3) * h as i32 / 6) as u32;
        let gpu = image.get_pixel(x, h / 3).0;
        let cpu = image.get_pixel(x, 2 * h / 3).0;
        for c in 0..3 {
            assert!(
                gpu[c].abs_diff(cpu[c]) <= 3 && gpu[c].abs_diff(controls.0[i][c]) <= 3,
                "water case{i} GPU={gpu:?} CPU={cpu:?} expected={:?}",
                controls.0[i]
            );
        }
        info!("native water UV/motion GPU case{i} PASSED GPU={gpu:?} CPU={cpu:?}");
    }
}

fn world_camera(mut commands: Commands, fixture: Res<Fixture>) {
    let (p, t, fov) = if fixture.reference != 0 {
        // Original reference/ingame/combat_{triple_slash,return}/camera.json before_key_press.
        let (p, yaw) = if fixture.reference == 5 {
            (
                Vec3::new(15573.67773, 15381.66406, 127.979904),
                -115.528885_f32,
            )
        } else {
            (
                Vec3::new(15572.85498, 15379.30811, 127.979904),
                -148.365387_f32,
            )
        };
        let yaw = yaw.to_radians();
        (p, p + Vec3::new(-yaw.sin(), yaw.cos(), 0.0), 1.24354708)
    } else if fixture.pond {
        (
            Vec3::new(15570.0, 15335.0, 139.0),
            Vec3::new(15580.0, 15375.0, 127.0),
            60.0_f32.to_radians(),
        )
    } else {
        (
            Vec3::new(9245.0, 12815.0, 211.0),
            Vec3::new(9218.0, 12792.0, 189.4),
            60.0_f32.to_radians(),
        )
    };
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov, ..default() }),
        Transform::from_translation(aa_world::cry_to_bevy(p))
            .looking_at(aa_world::cry_to_bevy(t), Vec3::Y),
    ));
}

fn world_check(
    mut fixture: ResMut<Fixture>,
    time: Res<Time>,
    mut surfaces: Query<(
        &aa_world::water::WaterSurface,
        &Mesh3d,
        &mut MeshMaterial3d<WaterMaterial>,
    )>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    objects: Res<aa_world::objects::ObjectState>,
) {
    for (_, mesh, mut material) in &mut surfaces {
        fixture.meshes.insert(mesh.id());
        if fixture.legacy
            && time.elapsed_secs() > 8.0
            && !fixture.controls.values().any(|h| h.id() == material.id())
        {
            if let Some(handle) = fixture.controls.get(&material.id()) {
                material.0 = handle.clone();
            } else if let Some(source) = materials.get(material.id()) {
                let mut control = source.clone();
                control.params.motion.x = 0.0;
                control.params.motion.y = 0.0;
                let original = material.id();
                let handle = materials.add(control);
                fixture.controls.insert(original, handle.clone());
                material.0 = handle;
            }
        }
    }
    if time.elapsed_secs() > fixture.last + 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "native water world: active_styles={} captured_meshes={} vertices={} pending_models={} failed_models={}",
            surfaces.iter().count(),
            fixture.meshes.len(),
            fixture
                .meshes
                .iter()
                .filter_map(|id| meshes.get(*id))
                .map(Mesh::count_vertices)
                .sum::<usize>(),
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 12.0 {
        assert!(!fixture.meshes.is_empty());
        camera.single_mut().unwrap().translation =
            aa_world::cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        fixture.moved = true;
    }
    if fixture.moved && !fixture.done && time.elapsed_secs() > 16.0 {
        assert_eq!(surfaces.iter().count(), 0);
        assert!(fixture.meshes.iter().all(|id| meshes.get(*id).is_none()));
        info!(
            "native water unload PASSED: all {} cell mesh IDs and water entities released; shared style texture cache retains its existing lifetime",
            fixture.meshes.len()
        );
        fixture.done = true;
    }
}
