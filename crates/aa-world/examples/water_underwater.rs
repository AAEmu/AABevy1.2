//! Original WaterVolume.Outof ray lengths through the production post pipeline.
//! --eye 8 --pitch 65 --wall 16; --empty / --dry / --legacy-volume / --baseline.
//! --world-pond probes the real lake's camera crossing gates at +/- 1/3 cm and zero.
//! Surface refraction is checked separately by water_refraction --below.
//! --ocean checks only the source-supported absence of an angular post mask;
//! its existing calibrated scatter remains an explicit non-native control.
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

const BACK: Vec3 = Vec3::new(0.4, 0.12, 0.03);
const FOG: Vec3 = Vec3::new(0.02, 0.16, 0.28);
const DENSITY: f32 = 0.12;

#[derive(Resource, Default)]
struct Crossings {
    step: usize,
    changed_at: f32,
    pending: bool,
}
const HEIGHTS: [f32; 5] = [127.03, 127.01, 127.0, 126.99, 126.97];

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn number(name: &str, fallback: f32) -> f32 {
    let a: Vec<_> = std::env::args().collect();
    a.windows(2)
        .find(|a| a[0] == name)
        .and_then(|a| a[1].parse().ok())
        .unwrap_or(fallback)
}

fn main() {
    let mut config = aa_world::WorldConfig::new("main_world");
    config.environment = flag("--world-pond");
    config.objects = false;
    config.vegetation = false;
    config.max_loads = 0;
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native water volume Outof"),
            aa_dev::DevPlugin,
            aa_world::AaWorldPlugin { config },
        ))
        .insert_resource(ClearColor(Color::linear_rgb(BACK.x, BACK.y, BACK.z)))
        .init_resource::<Crossings>()
        .add_systems(Startup, setup)
        .add_systems(Last, crossing)
        .add_observer(pixels)
        .run();
}

fn camera_transform() -> Transform {
    if flag("--world-pond") {
        return Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(
            15549.375, 15380.0, HEIGHTS[0],
        )))
        .looking_to(Vec3::new(0.0, 0.5, -0.8660254), Vec3::Y);
    }
    let eye = Vec3::new(0.0, -number("--eye", 8.0), 0.0);
    let pitch = number("--pitch", 65.0).to_radians();
    Transform::from_translation(eye).looking_to(Vec3::new(0.0, pitch.sin(), -pitch.cos()), Vec3::Y)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let transform = camera_transform();
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
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection {
            fov: 90_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
        transform,
        PostUniform {
            under: Vec4::new(0.0, DENSITY, 1.0, if flag("--dry") { 0.0 } else { 1.0 }),
            // Private shader tag, existing padding: 1 = original volume Outof;
            // -1 = qualified sea without an invented cone; 0 = legacy fallback.
            under_color: FOG.extend(if flag("--ocean") {-1.0} else if flag("--legacy-volume") { 0.0 } else { 1.0 }),
            ..default()
        },
    ));
    if !flag("--empty") && !flag("--world-pond") {
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1000.0, 1000.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::linear_rgb(BACK.x, BACK.y, BACK.z),
                unlit: true,
                ..default()
            })),
            Transform::from_translation(
                transform.translation + transform.forward() * number("--wall", 16.0),
            )
            .with_rotation(transform.rotation),
        ));
    }
}

fn crossing(
    time: Res<Time>,
    mut state: ResMut<Crossings>,
    mut cameras: Query<(&mut Transform, &PostUniform), With<Camera3d>>,
) {
    if !flag("--world-pond") || time.elapsed_secs() < 3.0 || state.step == HEIGHTS.len() {
        return;
    }
    let Ok((mut camera, post)) = cameras.single_mut() else {
        return;
    };
    if state.pending && time.elapsed_secs() - state.changed_at >= 0.35 {
        let height = HEIGHTS[state.step];
        let expected = height <= 127.0;
        info!(
            "Water crossing: CryZ={height} underwater={} volume_tag={} expected={expected}",
            post.under.w, post.under_color.w
        );
        if !flag("--baseline") {
            assert_eq!(
                post.under.w > 0.5,
                expected,
                "real pond crossing at {height}"
            );
            if expected {
                assert_eq!(post.under_color.w, 1.0);
            }
        }
        state.step += 1;
        state.pending = false;
    }
    if !state.pending && state.step < HEIGHTS.len() {
        camera.translation.y = HEIGHTS[state.step];
        state.changed_at = time.elapsed_secs();
        state.pending = true;
    }
}

// Literal original exp2 fog with an independent segment/plane intersection in
// f64. Unlike the old ocean approximation, volume Outof has no angular window,
// scatter integral, receiver-depth absorption, or underwater colour gain.
fn expected(pixel: UVec2, size: UVec2) -> Vec3 {
    let uv = (pixel.as_vec2() + Vec2::splat(0.5)) / size.as_vec2();
    let local = Vec3::new(
        (uv.x * 2.0 - 1.0) * size.x as f32 / size.y as f32,
        1.0 - uv.y * 2.0,
        -1.0,
    );
    let transform = camera_transform();
    let ray = transform.rotation * local.normalize();
    let dist = if flag("--empty") {
        1.0e9 / (-local.normalize().z)
    } else {
        number("--wall", 16.0) / (-local.normalize().z)
    };
    let depth = number("--eye", 8.0).max(0.0);
    let mut path = f64::from(dist);
    if ray.y > 0.0 {
        path = path.min(f64::from(depth) / f64::from(ray.y));
    }
    if flag("--dry") {
        return BACK;
    }
    if flag("--legacy-volume") || flag("--ocean") {
        let k = f64::from(DENSITY);
        let receiver_depth = (f64::from(depth) - f64::from(ray.y) * f64::from(dist)).max(0.0);
        let kt = k * (f64::from(ray.y) - 1.0);
        let integral = |s: f64| {
            if kt.abs() < 1e-6 {
                s
            } else {
                (kt * s).exp_m1() / kt
            }
        };
        let scatter = FOG * ((-k * f64::from(depth)).exp() * integral(path) * k) as f32;
        let far = FOG * ((-k * f64::from(depth)).exp() * integral(20000.0) * k) as f32;
        let seen = BACK * (-k * (path + receiver_depth)).exp() as f32 + scatter;
        let t = ((ray.y - 0.62) / 0.1).clamp(0.0, 1.0);
        let window = if ray.y > 1e-3 && f64::from(depth) / f64::from(ray.y) < f64::from(dist) {
            t * t * (3.0 - 2.0 * t)
        } else {
            1.0
        };
        return if flag("--ocean") {seen} else {far.lerp(seen, window)};
    }
    // Renderer 380BF625 multiplies the serialized density by the original
    // f32 log2(e) constant 382E6E4C before the shader's exp2.
    FOG.lerp(
        BACK,
        (-f64::from(DENSITY) * 1.4426950216293335 * path).exp2() as f32,
    )
}

fn display(c: Vec3) -> [u8; 3] {
    // Default post film curve and final gamma->linear->sRGB, with bloom,
    // grading, adaptation and shafts disabled by PostUniform::default.
    let curve = |x: f32| x * (0.9 * 6.2 * x + 0.5) / (x * (0.9 * 6.2 * x + 1.7) + 0.8 * 0.06);
    c.to_array().map(|x| {
        let linear = (curve(x) / curve(1.8)).clamp(0.0, 1.0).powf(2.2);
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (srgb * 255.0).round() as u8
    })
}

fn pixels(shot: On<ScreenshotCaptured>, crossings: Res<Crossings>) {
    if flag("--world-pond") {
        assert_eq!(
            crossings.step,
            HEIGHTS.len(),
            "all real lake crossings must settle before capture"
        );
        return;
    }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let size = UVec2::new(image.width(), image.height());
    for q in [
        Vec2::new(0.5, 0.1),
        Vec2::new(0.5, 0.5),
        Vec2::new(0.5, 0.9),
        Vec2::new(0.1, 0.5),
        Vec2::new(0.9, 0.5),
    ] {
        let pixel = (q * size.as_vec2()).as_uvec2();
        let gpu = image.get_pixel(pixel.x, pixel.y).0;
        let cpu = display(expected(pixel, size));
        info!("Water Outof {pixel:?}: GPU{gpu:?} original CPU{cpu:?}");
        if !flag("--baseline") {
            assert!(
                gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(b) <= 3),
                "Water Outof {pixel:?}: GPU{gpu:?} CPU{cpu:?}"
            );
        }
    }
}
