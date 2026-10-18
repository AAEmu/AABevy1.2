//! Native steady main bending: GPU geometry oracles; --world [--off] [--at seconds] [--unload-check].
use aa_world::{
    vegetation_color::{
        NO_COLOR_VARIATION, VegetationColor, VegetationColorPlugin, VegetationMaterial,
    },
    vegetation_wind::{self, VegetationWindPart},
};
use bevy::{
    asset::embedded_asset,
    asset::{AssetId, RenderAssetUsages},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::{Indices, MeshTag, PrimitiveTopology},
    prelude::*,
    render::render_resource::AsBindGroup,
    render::view::screenshot::ScreenshotCaptured,
    shader::ShaderRef,
};
use std::collections::{HashMap, HashSet};

fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
fn at() -> Option<f32> {
    let a: Vec<_> = std::env::args().collect();
    a.windows(2)
        .find(|v| v[0] == "--at")
        .and_then(|v| v[1].parse().ok())
}
#[derive(Resource)]
struct Controls(Vec<Vec3>);
#[derive(Resource)]
struct Samples(Vec<(Vec2, [u8; 3])>);
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
struct Oracle {
    #[uniform(50)]
    sample: Vec4,
    #[uniform(51)]
    wind: Vec4,
    #[uniform(52)]
    clock: Vec4,
}
impl bevy::pbr::MaterialExtension for Oracle {
    fn fragment_shader() -> ShaderRef {
        "embedded://vegetation_wind/vegetation_wind_controls.wesl".into()
    }
}
type OracleMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, Oracle>;
#[derive(Resource, Default)]
struct Fixture {
    parts: HashSet<Entity>,
    meshes: HashSet<AssetId<Mesh>>,
    materials: HashSet<AssetId<VegetationMaterial>>,
    controls: HashMap<AssetId<VegetationMaterial>, Handle<VegetationMaterial>>,
    last: f32,
    moved: bool,
    done: bool,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native vegetation main wind"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 100.0;
        config.unload_radius = 220.0;
        config.brush_distance = 150.0;
        config.vegetation_distance = 100.0;
        config.tree_distance = 150.0;
        config.water_distance = 180.0;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: 10.0,
                ..default()
            })
            .init_resource::<Fixture>()
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_check);
    } else {
        embedded_asset!(app, "examples", "vegetation_wind_controls.wesl");
        app.add_plugins((
            VegetationColorPlugin,
            bevy::pbr::MaterialPlugin::<OracleMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, numeric)
        .add_observer(pixels);
    }
    app.run();
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<VegetationMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut oracles: ResMut<Assets<OracleMaterial>>,
) {
    let mut controls = Vec::new();
    let mut samples = Vec::new();
    for i in 0..12 {
        let group = [0.01, 1.0, 2.0, 10.0][i % 4];
        let seconds = [0.0, 3.25, 3600.125][i / 4];
        let shift = Vec3::new(
            (i % 4) as f32 * 3.0 - 4.5,
            if i % 2 == 0 { -1.25 } else { 1.75 },
            (i / 4) as f32 * 3.0 - 3.0,
        );
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat([0.8, 1.0, 1.2, 1.0][i % 4]),
            Quat::from_euler(EulerRot::XYZ, 0.05, 0.1, i as f32 * 0.13),
            shift,
        );
        let p = vegetation_wind::parameters(
            if i == 0 {
                Vec3::ZERO
            } else {
                Vec3::new(1.0, 1.0, 0.0)
            },
            group,
            0.75,
        );
        let vertices = [
            Vec3::new(-0.09, 0.0, 1.44),
            Vec3::new(0.09, 0.0, 1.44),
            Vec3::new(0.0, 0.0, 1.62),
        ];
        let centroid = vertices
            .iter()
            .map(|v| m.transform_point3(vegetation_wind::bend(*v, m, p, seconds, i % 2 == 0, true)))
            .sum::<Vec3>()
            / 3.0;
        controls.push(aa_world::cry_to_bevy(centroid));
        let point = Vec2::new(shift.x, shift.z + 0.35);
        let index = [-1, 0, 1, 256, 511, 512, 768, 1022, 1023, 1024, 1025, -513][i];
        let gain = if [0, 1, 2, 4, 5, 7, 8, 9, 10, 11].contains(&i) {
            80.0
        } else {
            0.4
        };
        let native = vegetation_wind::lookup(index) * gain + 0.5;
        let rgb = Vec3::splat(native).to_array().map(|v| {
            let v = v.clamp(0.0, 1.0);
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        samples.push((point, rgb));
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1.3, 0.3))),
            MeshMaterial3d(oracles.add(OracleMaterial {
                base: StandardMaterial {
                    unlit: true,
                    ..default()
                },
                extension: Oracle {
                    sample: Vec4::new(index as f32, gain, 0.5, 0.0),
                    wind: p,
                    clock: shift.extend(seconds),
                },
            })),
            Transform::from_xyz(point.x, point.y, 3.0),
        ));
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vertices.map(|v| v.to_array()).to_vec(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, -1.0, 0.0]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 3]);
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        let material = mats.add(VegetationMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: None,
                ..default()
            },
            extension: VegetationColor {
                wind: p,
                grass: i % 2 == 0,
                seconds: Some(seconds),
                high_quality: true,
                ..default()
            },
        });
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            MeshTag::with_type::<VegetationColor>(NO_COLOR_VARIATION),
            Transform::from_matrix(Mat4::from_quat(aa_world::coords::CRY_TO_BEVY) * m),
        ));
        commands.spawn((
            Mesh3d(meshes.add(Circle::new(0.11))),
            MeshMaterial3d(standard.add(StandardMaterial {
                base_color: Color::srgb(0.8, 0.0, 0.0),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(controls[i].x, controls[i].y, -4.0),
        ));
    }
    commands.insert_resource(Controls(controls));
    commands.insert_resource(Samples(samples));
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 11.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 1.5, 15.0).looking_at(Vec3::new(0.0, 1.5, 0.0), Vec3::Y),
    ));
    commands.spawn((Text::new("Native main bending GPU geometry / independent CPU centres\nColumns group .01 /1 /2 /10; rows 0 /3.25 /3600.125 game seconds. Alternating grass/leaves.\nRed circles mark expected centres; every white native-deformed triangle must cover its centre."),
        TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(20.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>, samples: Res<Samples>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, p) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 11.0) as u32;
        let y = (h as f32 * 0.5 - (p.y - 1.5) * h as f32 / 11.0) as u32;
        let rgb = image.get_pixel(x, y).0;
        assert!(
            rgb.iter().all(|v| *v > 245),
            "native main bend GPU case{i} at({x},{y}) RGB{rgb:?}"
        );
        info!("native main bend GPU case{i} PASSED at({x},{y}) RGB{rgb:?}");
    }
    for (i, (p, cpu)) in samples.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 11.0) as u32;
        let y = (h as f32 * 0.5 - (p.y - 1.5) * h as f32 / 11.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
            "native lookup GPU case{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("native lookup GPU case{i} PASSED GPU{gpu:?} CPU{cpu:?}");
    }
}
fn world_setup(mut commands: Commands) {
    let target = Vec3::new(12812.47, 14575.70, 158.28);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(12807.0, 14566.0, 162.0)))
            .looking_at(aa_world::cry_to_bevy(target + Vec3::Z * 0.6), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50.0f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
        bevy::core_pipeline::prepass::DepthPrepass,
    ));
}
#[allow(clippy::too_many_arguments)]
fn world_check(
    mut commands: Commands,
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    fresh: Query<
        (
            Entity,
            &Mesh3d,
            &MeshMaterial3d<VegetationMaterial>,
            &VegetationWindPart,
        ),
        Added<VegetationWindPart>,
    >,
    live: Query<&VegetationWindPart>,
    all: Query<()>,
    mut mats: ResMut<Assets<VegetationMaterial>>,
    mut f: ResMut<Fixture>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (e, mesh, mat, wind) in &fresh {
        assert!(wind.bending >= 0.01 && wind.bending <= 10.0 && wind.half_height >= 0.0);
        f.parts.insert(e);
        f.meshes.insert(mesh.0.id());
        f.materials.insert(mat.0.id());
        if flag("--off") || at().is_some() {
            let control = if let Some(h) = f.controls.get(&mat.0.id()) {
                h.clone()
            } else {
                let mut m = mats.get(&mat.0).unwrap().clone();
                if flag("--off") {
                    m.extension.wind.z = 0.0;
                }
                m.extension.seconds = at();
                let h = mats.add(m);
                f.controls.insert(mat.0.id(), h.clone());
                h
            };
            commands.entity(e).insert(MeshMaterial3d(control));
        }
    }
    if time.elapsed_secs() - f.last > 2.0 {
        f.last = time.elapsed_secs();
        info!(
            "native main wind: seen={} live={} meshes={} materials={} objects={} pending={} failed={}",
            f.parts.len(),
            live.iter().count(),
            f.meshes.len(),
            f.materials.len(),
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if flag("--unload-check") && !f.moved && time.elapsed_secs() > 16.0 {
        assert!(f.parts.len() > 10 && f.parts.len() > f.materials.len() * 2);
        camera.single_mut().unwrap().translation = aa_world::cry_to_bevy(Vec3::splat(-100000.0));
        f.moved = true;
    }
    if f.moved && !f.done && time.elapsed_secs() > 22.0 {
        assert!(live.is_empty());
        assert_eq!(objects.stats.entities, 0);
        assert!(f.parts.iter().all(|e| all.get(*e).is_err()));
        info!(
            "native main wind unload PASSED: {} part IDs removed; existing shared model cache lifetime unchanged",
            f.parts.len()
        );
        f.done = true;
    }
}
