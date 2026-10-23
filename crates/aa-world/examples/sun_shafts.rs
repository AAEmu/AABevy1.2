//! Unattended native HDR shaft controls: constant axial depth, ray intensity,
//! signed sun facing and grading order. --depth 0..1 --rays N --away --off
//! --graded --msaa --atten N --stripe; uses the actual world post pipelines.
use aa_world::post::PostUniform;
use bevy::{
    camera::{Camera3dDepthTextureUsage, Hdr},
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::{render_resource::TextureUsages, view::screenshot::ScreenshotCaptured},
};

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn value(name: &str, fallback: f32) -> f32 {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2)
        .find(|a| a[0] == name)
        .and_then(|a| a[1].parse().ok())
        .unwrap_or(fallback)
}

fn main() {
    let mut config = aa_world::WorldConfig::new("main_world");
    config.environment = false;
    config.objects = false;
    config.vegetation = false;
    config.max_loads = 0;
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native HDR sun shafts"),
            aa_dev::DevPlugin,
            aa_world::AaWorldPlugin { config },
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window>,
) {
    let depth = value("--depth", 0.5).clamp(0.001, 1.0);
    let rays = if flag("--off") {
        0.0
    } else {
        value("--rays", 0.5)
    };
    let mut camera = Camera3d::default();
    camera.depth_texture_usages = Camera3dDepthTextureUsage::from(
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    );
    commands.spawn((
        camera,
        Hdr,
        DepthPrepass,
        Tonemapping::None,
        DebandDither::Disabled,
        if flag("--msaa") {
            Msaa::Sample4
        } else {
            Msaa::Off
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 60_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
        Transform::default(),
        PostUniform {
            sun_dir: Vec3::new(0.0, 0.0, if flag("--away") { 1.0 } else { -1.0 }).extend(0.0),
            shafts: Vec4::new(
                rays,
                1000.0 / 999.9,
                value("--atten", 0.0).clamp(0.0, 10.0),
                1000.0,
            ),
            shafts_color: Vec3::splat(1.0 / 3.0_f32.sqrt()).extend(0.0),
            color: if flag("--graded") {
                Vec4::new(1.0, 0.6, 0.8, 1.0)
            } else {
                Vec4::new(1.0, 1.0, 1.0, 0.0)
            },
            ..default()
        },
    ));
    // A black plane supplies known normalized axial depth independently of color.
    let black = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(10000.0, 10000.0))),
        MeshMaterial3d(black.clone()),
        Transform::from_xyz(0.0, 0.0, -depth * 1000.0),
    ));
    if flag("--stripe") {
        let window = windows.single().unwrap();
        let aspect = window.physical_width() as f32 / window.physical_height() as f32;
        let width = 2.0 * 50.0 * 30_f32.to_radians().tan() * aspect * 0.2;
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(width, 10000.0))),
            MeshMaterial3d(black),
            Transform::from_xyz(0.0, 0.0, -50.0),
        ));
    }
}

fn display(x: f32) -> u8 {
    let linear = x.clamp(0.0, 1.0).powf(2.2);
    let srgb = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

fn pixels(shot: On<ScreenshotCaptured>) {
    // Original shader weights sum to 4.5, then /8 *2. UNORM stores clamp
    // and round after the mask and each of the three generation passes.
    let quantize = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    let (sw, sh) = ((w / 4).max(1), (h / 4).max(1));
    let mut mask = Vec::with_capacity((sw * sh) as usize);
    let depth = value("--depth", 0.5).clamp(0.001, 1.0);
    let stripe = flag("--stripe");
    for y in 0..sh {
        for x in 0..sw {
            let uv = Vec2::new((x as f32 + 0.5) / sw as f32, (y as f32 + 0.5) / sh as f32);
            let sum: f32 = [
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 0.65),
                Vec2::new(-1.0, 0.65),
            ]
            .iter()
            .map(|offset| {
                let pixel = ((uv + *offset / Vec2::new(w as f32, h as f32))
                    * Vec2::new(w as f32, h as f32))
                .floor();
                let u = (pixel.x + 0.5) / w as f32;
                if stripe && (0.4..0.6).contains(&u) {
                    0.05
                } else {
                    depth
                }
            })
            .sum();
            mask.push(quantize(sum * 0.333));
        }
    }
    let facing: f32 = if flag("--away") { -1.0 } else { 1.0 } * (1000.0 / 999.9);
    let attenuation = value("--atten", 0.0).clamp(0.0, 10.0);
    for reach in [0.1, 0.025, 0.01] {
        let mut output = vec![0.0; mask.len()];
        for y in 0..sh {
            for x in 0..sw {
                let uv = Vec2::new((x as f32 + 0.5) / sw as f32, (y as f32 + 0.5) / sh as f32);
                let to_sun = Vec2::splat(0.5) - uv;
                let distance = (to_sun * Vec2::new(1.0, 1.333 * h as f32 / w as f32)).length();
                let radial =
                    facing.clamp(0.0, 1.0) * (1.0 - (distance * attenuation).clamp(0.0, 1.0));
                let sum: f32 = (0..8)
                    .map(|i| {
                        sample(&mask, sw, sh, uv + to_sun * (reach * facing * i as f32))
                            * (1.0 - i as f32 / 8.0)
                    })
                    .sum();
                output[(y * sw + x) as usize] = quantize(sum * 0.25 * radial);
            }
        }
        mask = output;
    }
    let rays = if flag("--off") {
        0.0
    } else {
        value("--rays", 0.5)
    };
    for (x, y) in [
        (w / 2, h / 2),
        (w / 4, h / 3),
        (3 * w / 4, 2 * h / 3),
        (w * 2 / 5, h * 3 / 5),
        (w * 3 / 5, h * 2 / 5),
    ] {
        let uv = Vec2::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let mut g = sample(&mask, sw, sh, uv) * rays / 3.0_f32.sqrt();
        if flag("--graded") {
            g = (g * 0.8 - 0.5) * 0.6 + 0.5;
        }
        let expected = display(g);
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().all(|c| c.abs_diff(expected) <= 2),
            "native shafts GPU{gpu:?}, expected {expected} at {x},{y}"
        );
    }
    info!(
        "Native HDR shafts GPU PASSED: depth={depth} rays={rays} attenuation={attenuation} away={} graded={} msaa={} stripe={}",
        flag("--away"),
        flag("--graded"),
        flag("--msaa"),
        flag("--stripe")
    );
}

fn sample(data: &[f32], w: u32, h: u32, uv: Vec2) -> f32 {
    let at = uv * Vec2::new(w as f32, h as f32) - Vec2::splat(0.5);
    let base = at.floor();
    let t = at - base;
    let get = |x: f32, y: f32| {
        data[(y.clamp(0.0, h as f32 - 1.0) as u32 * w + x.clamp(0.0, w as f32 - 1.0) as u32)
            as usize]
    };
    let a = get(base.x, base.y) * (1.0 - t.x) + get(base.x + 1.0, base.y) * t.x;
    let b = get(base.x, base.y + 1.0) * (1.0 - t.x) + get(base.x + 1.0, base.y + 1.0) * t.x;
    a * (1.0 - t.y) + b * t.y
}
