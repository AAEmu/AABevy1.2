//! --case 0..17: depth/material/global-colour GPU controls.
//! --world [--global|--ellipsoid] [--hour H] [--zone Z] [--off] [--unload-check|--transition-check].
use aa_world::fog::{self, FogMaterial, FogVolume, FogVolumePart, FogVolumePlugin, GlobalFogColor};
use bevy::{
    asset::AssetId,
    camera::Exposure,
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
};
use std::collections::HashSet;
const BACK: Vec3 = Vec3::new(0.12, 0.24, 0.4);
const FOG: Vec3 = Vec3::new(0.7, 0.4, 0.2);
const FOV: f32 = 50.0;
fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn case() -> usize {
    let a: Vec<_> = std::env::args().collect();
    a.iter()
        .position(|v| v == "--case")
        .and_then(|i| a.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
fn scalar_arg<T: std::str::FromStr>(name: &str) -> Option<T> {
    let args: Vec<_> = std::env::args().collect();
    args.iter().position(|v| v == name).and_then(|i| args.get(i + 1))?.parse().ok()
}
fn vector_arg(name: &str) -> Option<Vec3> {
    let args: Vec<_> = std::env::args().collect();
    let text = args
        .iter()
        .position(|v| v == name)
        .and_then(|i| args.get(i + 1))?;
    let values: Vec<f32> = text.split(',').map(|v| v.parse().unwrap()).collect();
    assert_eq!(values.len(), 3);
    Some(Vec3::new(values[0], values[1], values[2]))
}
#[derive(Resource)]
struct Control {
    id: usize,
    volume: FogVolume,
    eye: Vec3,
    wall_z: f32,
    no_depth: bool,
    pane: bool,
    color: Vec3,
}
#[derive(Resource)]
struct RestoreGlobal(aa_world::env::EnvSample);
#[derive(Resource, Default)]
struct WorldCheck {
    moved: bool,
    done: bool,
    last: f32,
    parts: HashSet<Entity>,
    meshes: HashSet<AssetId<Mesh>>,
    materials: HashSet<AssetId<FogMaterial>>,
    phase: u8,
    validated: u8,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native authored fog volumes"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 80.;
        config.unload_radius = 220.;
        config.brush_distance = 200.;
        config.vegetation_distance = 90.;
        config.tree_distance = 200.;
        config.water_distance = 200.;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: scalar_arg("--hour").unwrap_or(10.),
                zone: scalar_arg("--zone"),
                ..default()
            })
            .init_resource::<WorldCheck>()
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(FogVolumePlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_systems(Update, restore_global)
            .add_observer(pixels);
    }
    app.run();
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
) {
    let id = case();
    assert!(id <= 17);
    let shape = if [4, 5, 6, 10].contains(&id) { 0 } else { 1 };
    let xml = format!(
        r#"<Mission><Objects><Entity EntityClass="FogVolume" Pos="0,8,0"><Properties VolumeType="{shape}" GlobalDensity="0.2" color_Color="0.7,0.4,0.2" FallOffScale="0"><Size x="4" y="4" z="4"/></Properties></Entity></Objects></Mission>"#
    );
    let mut v = fog::parse(&xml).remove(0);
    let mut eye = Vec3::ZERO;
    let mut wall_z = -12.;
    match id {
        1 => wall_z = -8.,
        2 => wall_z = -5.,
        3 | 5 => eye.z = -8.,
        6 => v.matrix.w_axis.x = 5.,
        7 => {
            v.direction = Vec3::Y;
            v.falloff = 0.2;
            v.shift = 1.;
        }
        8 => {
            v.direction = Vec3::NEG_Y;
            v.falloff = 0.2;
            v.shift = 1.;
        }
        9 => {
            v.matrix *= Mat4::from_rotation_z(0.7) * Mat4::from_scale(Vec3::new(1., 1.7, 0.6));
        }
        10 => v.matrix.w_axis.x = 1.95,
        _ => {}
    }
    let exposure = Exposure { ev100: 0. };
    let mut color = FOG;
    if id >= 13 {
        v.global_color = true;
        let tod = |color: &str, multiplier: f32, hdr: f32| {
            aa_world::env::TimeOfDay::parse(&format!(r#"<TimeOfDay sRGB="1">
              <Variable Name="Fog color"><Spline Keys="0:({color}):0,1:({color}):0,"/></Variable>
              <Variable Name="Fog color multiplier"><Spline Keys="0:{multiplier}:0,1:{multiplier}:0,"/></Variable>
              <Variable Name="HDR dynamic power factor"><Spline Keys="0:{hdr}:0,1:{hdr}:0,"/></Variable>
              </TimeOfDay>"#)).unwrap()
        };
        let day = tod("0.07:0.12:0.2", 1.2, 0.5);
        let night = tod("0.02:0.04:0.08", 0.5, -1.0);
        let lighting = aa_world::env::Lighting::default();
        let sample = match id {
            14 => night.sample(0., lighting),
            15 => aa_world::env::TimeOfDay::sample_blended(&[(&day, 1.), (&night, 3.)], 10., lighting),
            _ => day.sample(10., lighting),
        };
        // Independent numeric expectation from the raw source inputs, with no
        // calls to the sampled/material colour implementation under test.
        let (raw, mult, hdr) = match id {
            14 => (Vec3::new(0.02, 0.04, 0.08), 0.5_f64, -1.0_f64),
            15 => (Vec3::new(0.0325, 0.06, 0.11), 0.675, -0.625),
            _ => (Vec3::new(0.07, 0.12, 0.2), 1.2, 0.5),
        };
        color = raw * (mult * 2_f64.powf(hdr)) as f32;
        if id == 16 { color = BACK; }
        commands.insert_resource(aa_world::sky::Environment {
            sample: (id != 16 && id != 17).then_some(sample.clone()),
            zone: Some(179), hour: 10., ocean_fog: Vec3::ZERO, ocean_density: 0.0,
        });
        commands.insert_resource(aa_world::sky::AtmosphereTuning {
            radiance: exposure.exposure().recip(), ..default()
        });
        if id == 17 { commands.insert_resource(RestoreGlobal(sample)); }
    }
    // Exact native RGB controls use a unit scene conversion, independent of the world adapter.
    let params = v.params((0, 0), exposure.exposure().recip());
    let mut fog_entity = commands.spawn((
        Mesh3d(meshes.add(fog::mesh(&v))),
        MeshMaterial3d(materials.add(FogMaterial { params })),
        Transform::default(),
    ));
    if v.global_color { fog_entity.insert(GlobalFogColor { density: v.density }); }
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(100., 100.))),
        MeshMaterial3d(stdm.add(StandardMaterial {
            base_color: Color::linear_rgb(BACK.x, BACK.y, BACK.z),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(0., 0., wall_z),
    ));
    if id == 12 {
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(100., 100.))),
            MeshMaterial3d(stdm.add(StandardMaterial {
                base_color: Color::linear_rgba(0.8, 0.1, 0.1, 0.5),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            })),
            Transform::from_xyz(0., 0., -4.),
        ));
    }
    let mut camera = commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV.to_radians(),
            near: 0.1,
            far: 1000.,
            ..default()
        }),
        exposure,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_translation(eye).looking_to(Vec3::NEG_Z, Vec3::Y),
    ));
    if id != 11 {
        camera.insert(DepthPrepass);
    }
    commands.insert_resource(Control {
        id,
        volume: v,
        eye,
        wall_z,
        no_depth: id == 11,
        pane: id == 12,
        color,
    });
    commands.spawn((Text::new(format!("Native FogVolume GPU control {id}: real opaque depth, analytic bounds/height integral\n0 box /1 depth middle /2 opaque in front /3 inside box /4 ellipsoid /5 inside ellipsoid\n6 ray miss /7-8 height gradients /9 rotated nonuniform /10 soft edge /11 no depth /12 alpha pane\nCentre pixel checked against independent numerical density integration.")),TextFont{font_size:FontSize::Px(18.),..default()},Node{position_type:PositionType::Absolute,left:px(15.),top:px(12.),..default()}));
}
fn restore_global(time: Res<Time>, restore: Option<Res<RestoreGlobal>>, env: Option<ResMut<aa_world::sky::Environment>>) {
    if time.elapsed_secs() > 1.0 {
        if let (Some(restore), Some(mut env)) = (restore, env) {
            if env.sample.is_none() { env.sample = Some(restore.0.clone()); }
        }
    }
}
fn numerical(c: &Control, ray: Vec3) -> f32 {
    if c.no_depth {
        return 0.;
    }
    let p = c.volume.params((0, 0), 1.);
    let length = (c.wall_z - c.eye.z) / ray.z;
    let steps = 100_000;
    let dt = f64::from(length) / f64::from(steps);
    let mut integral = 0f64;
    for i in 0..steps {
        let at = c.eye + ray * ((f64::from(i) + 0.5) * dt) as f32;
        let q = p
            .local_from_world
            .transform_vector3(at - p.center.truncate());
        let inside = if c.volume.box_shape {
            q.abs().max_element() <= 1.
        } else {
            q.length_squared() <= 1.
        };
        if inside {
            integral += dt * f64::from(-(at - p.base.truncate()).dot(p.gradient.truncate())).exp();
        }
    }
    if !c.volume.box_shape {
        // Native discriminant is four times one minus squared normalized-ray impact distance.
        let e = p
            .local_from_world
            .transform_vector3(c.eye - p.center.truncate());
        let d = p.local_from_world.transform_vector3(ray).normalize();
        let impact = e.cross(d).length_squared();
        let soft = c.volume.soft_edges * ((e.length() - 0.95) * 20.).clamp(0., 1.);
        let arg = (4. * (1. - impact) * soft + 1. - soft).clamp(0., 1.);
        integral *= f64::from(arg * (2. - arg));
    }
    (1. - (-f64::from(c.volume.density) * integral).exp()) as f32
}
fn pixels(shot: On<ScreenshotCaptured>, c: Res<Control>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    let (x, y) = (w / 2, h / 2);
    let ndc = Vec2::new(
        (x as f32 + 0.5) * 2. / w as f32 - 1.,
        1. - (y as f32 + 0.5) * 2. / h as f32,
    );
    let scale = (FOV.to_radians() * 0.5).tan();
    let ray = Vec3::new(ndc.x * w as f32 / h as f32 * scale, ndc.y * scale, -1.).normalize();
    let alpha = numerical(&c, ray);
    let mut color = BACK.lerp(c.color, alpha);
    if c.pane {
        color = color.lerp(Vec3::new(0.8, 0.1, 0.1), 0.5);
    }
    let rgb = color.to_array().map(|v| {
        ((if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        }) * 255.)
            .round() as u8
    });
    let gpu = image.get_pixel(x, y).0;
    assert!(
        gpu.iter().zip(rgb).all(|(a, b)| a.abs_diff(b) <= 2),
        "fog GPU case{} GPU{gpu:?} CPU{rgb:?} alpha{alpha}",
        c.id
    );
    info!(
        "Native fog GPU case{} PASSED GPU{gpu:?} independent CPU{rgb:?} alpha{alpha}",
        c.id
    );
}
fn world_setup(mut commands: Commands) {
    let target = if flag("--global") {
        Vec3::new(12587.382, 10548.999, 119.5)
    } else if flag("--ellipsoid") {
        Vec3::new(21731.574, 8731.525, 507.83643)
    } else {
        Vec3::new(22064.125, 8376.249, 447.38528)
    };
    let offset = if flag("--global") {
        Vec3::new(-60., -40., 1.)
    } else if flag("--inside") {
        Vec3::new(0., -1., -5.)
    } else {
        Vec3::new(40., -60., 8.)
    };
    let eye = vector_arg("--camera").unwrap_or(target + offset);
    let look = vector_arg("--target").unwrap_or(target + Vec3::Z * 2.);
    info!("Original fog camera Cry{eye:?} looking at Cry{look:?}");
    let mut camera = commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(eye))
            .looking_at(aa_world::cry_to_bevy(look), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV.to_radians(),
            near: 0.1,
            far: 20000.,
            ..default()
        }),
    ));
    if flag("--transmission") { camera.insert(bevy::pbr::ScreenSpaceTransmission::default()); }
}
fn world_check(
    mut commands: Commands,
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    mut check: ResMut<WorldCheck>,
    parts: Query<
        (
            Entity,
            &FogVolumePart,
            &Mesh3d,
            &MeshMaterial3d<FogMaterial>,
        ),
        Added<FogVolumePart>,
    >,
    all: Query<Entity, With<FogVolumePart>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<FogMaterial>>,
    env: Res<aa_world::sky::Environment>,
    tuning: Res<aa_world::sky::AtmosphereTuning>,
    mut clock: ResMut<aa_world::WorldClock>,
    mut config: ResMut<aa_world::WorldConfig>,
) {
    for (e, p, m, a) in &parts {
        info!("Original FogVolume cell{:?} guid{:016X}", p.cell, p.guid);
        check.parts.insert(e);
        check.meshes.insert(m.0.id());
        check.materials.insert(a.0.id());
        if flag("--off") {
            commands.entity(e).insert(Visibility::Hidden);
        }
    }
    let now = time.elapsed_secs();
    if flag("--transition-check") {
        let phase = if now > 20. { 4 } else if now > 16. { 3 } else if now > 12. { 2 } else if now > 8. { 1 } else { 0 };
        if phase != check.phase {
            check.phase = phase;
            clock.hour = if phase == 1 { 0. } else { 10. };
            clock.zone = (phase == 2).then_some(179);
            config.environment = phase != 3;
            info!("Global fog transition phase{phase}: hour{} zone{:?} environment{}", clock.hour, clock.zone, config.environment);
        }
        let settled = match phase { 1 => now > 9., 2 => now > 13., 3 => now > 17., 4 => now > 21., _ => false };
        if settled && check.validated < phase {
            assert!(!check.materials.is_empty());
            let expected = env.sample.as_ref().map(|s| (s.native_fog * tuning.radiance).extend(0.07)).unwrap_or(Vec4::ZERO);
            assert_eq!(env.sample.is_none(), phase == 3);
            for handle in &check.materials {
                let actual = materials.get(*handle).unwrap().params.color;
                assert!((actual - expected).abs().max_element() < 1e-5, "phase{phase} actual{actual:?} expected{expected:?}");
            }
            assert_eq!(check.materials.len(), 1);
            check.validated = phase;
            info!("Native global fog phase{phase} PASSED: raw native colour{expected:?}, same material retained");
        }
    }
    if now - check.last > 2. {
        check.last = now;
        info!(
            "fog scene volumes={} unsupported={} over_budget={} objects={} pending={} failed={}",
            objects.stats.fog_volumes,
            objects.stats.fog_unsupported,
            objects.stats.fog_over_budget,
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed
        );
        if flag("--global") { info!("Global fog env zone{:?} hour{} raw{:?}", env.zone, env.hour, env.sample.as_ref().map(|s| s.native_fog)); }
    }
    let unload_at = if flag("--transition-check") { 24. } else { 16. };
    if (flag("--unload-check") || flag("--transition-check")) && now > unload_at && !check.moved {
        assert!(!check.parts.is_empty());
        if flag("--transition-check") { assert_eq!(check.validated, 4); }
        check.moved = true;
        for mut c in &mut camera {
            c.translation = Vec3::splat(-100_000.);
        }
    }
    if check.moved && now > unload_at + 7. && !check.done {
        assert!(check.parts.iter().all(|e| all.get(*e).is_err()));
        assert!(check.meshes.iter().all(|id| !meshes.contains(*id)));
        assert!(check.materials.iter().all(|id| !materials.contains(*id)));
        assert_eq!(objects.stats.fog_volumes, 0);
        check.done = true;
        info!(
            "Native fog cell unload PASSED: {} entities, {} meshes, {} materials released",
            check.parts.len(),
            check.meshes.len(),
            check.materials.len()
        );
    }
}
