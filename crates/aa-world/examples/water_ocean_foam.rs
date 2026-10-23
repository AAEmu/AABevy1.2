//! Qualified ocean foam scalar lighting and clip-W camera fade in the production shader.
//! --below / --ortho / --viewport exercise both sides and projection conventions.
//! --legacy / --invalid / --no-depth / --no-transmission retain their old paths.
//! --mode 1/2/3 checks authored-volume modes; --foam-off checks the feature gate.
//! --baseline reports the unchanged shader without asserting the corrected formula.
use aa_world::water_material::{WaterMaterial, WaterMaterialPlugin, WaterParams};
use bevy::{
    camera::Viewport,
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat},
        view::screenshot::ScreenshotCaptured,
    },
    window::PrimaryWindow,
};

#[derive(Resource)]
struct Controls(Vec<(UVec2, [u8; 3], String)>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native ocean foam light"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

fn encoded(c: Vec3) -> [u8; 3] {
    c.to_array().map(|v| {
        ((if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    })
}

fn texture(images: &mut Assets<Image>, rgba: [u8; 4]) -> Handle<Image> {
    images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &rgba,
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    ))
}

fn setup(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
) {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|s| s == name);
    let below = flag("--below");
    let ortho = flag("--ortho");
    let no_depth = flag("--no-depth");
    let no_transmission = flag("--no-transmission");
    let invalid = flag("--invalid");
    let mode = args
        .iter()
        .position(|s| s == "--mode")
        .map(|i| args[i + 1].parse::<f32>().unwrap());
    let tag = mode.unwrap_or(if flag("--legacy") { 0.0 } else { -1.0 });
    let qualified = tag == -1.0 && !invalid && !no_depth && !no_transmission;
    let foam_on = !flag("--foam-off");
    let target = UVec2::new(window.physical_width(), window.physical_height());
    let origin = if flag("--viewport") {
        UVec2::new(80, 40)
    } else {
        UVec2::ZERO
    };
    let size = if flag("--viewport") {
        target - UVec2::new(180, 100)
    } else {
        target
    };
    let projection = if ortho {
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 2.0,
            },
            near: 0.0,
            far: 100.0,
            ..OrthographicProjection::default_3d()
        })
    } else {
        Projection::Perspective(PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_2,
            near: 0.01,
            far: 100.0,
            ..default()
        })
    };
    // Camera at the origin looks along -Y above water, +Y below it.
    // Each screen rectangle lies at its own axial depth; constant clip W makes
    // the native perspective interpolation exact. Orthographic clip W is one.
    let world = |pixel: Vec2, depth: f32| {
        let q = (pixel - origin.as_vec2() - size.as_vec2() * 0.5)
            * (2.0 / size.y as f32)
            * if ortho { 1.0 } else { depth };
        Vec3::new(
            q.x,
            if below { depth } else { -depth },
            if below { -q.y } else { q.y },
        )
    };
    let mut controls = Vec::new();
    // Unequal colour channels discriminate scalar truncation from RGB lighting.
    // Reflection-derived shadow remains fixed to the port's existing one; the
    // original WARP controls with reflection=1 independently isolate that case.
    let lights = [
        (
            Vec3::new(2.0, 0.1, 0.4),
            Vec3::new(0.25, 0.7, 1.2),
            1.0,
            "colour sun+sky",
        ),
        (Vec3::ZERO, Vec3::new(0.25, 0.7, 1.2), 1.0, "colour sky"),
        (
            Vec3::new(0.0, 2.0, 1.0),
            Vec3::new(0.0, 0.4, 0.8),
            1.0,
            "zero red",
        ),
        (
            Vec3::new(2.0, 0.1, 0.4),
            Vec3::new(0.1, 0.7, 1.2),
            0.0,
            "zero NdotL",
        ),
    ];
    for (row, depth) in [0.1_f32, 0.33, 0.5, 1.0, 1.33, 8.0].into_iter().enumerate() {
        for (light_index, (sun, sky, ndotl, name)) in lights.into_iter().enumerate() {
            for (shape, (foam_byte, gloss_byte)) in [(191_u8, 0_u8), (255, 0), (255, 64)]
                .into_iter()
                .enumerate()
            {
                let column = light_index * 3 + shape;
                let center = origin.as_vec2()
                    + Vec2::new(
                        (column as f32 + 0.5) * size.x as f32 / 12.0,
                        (row as f32 + 0.5) * size.y as f32 / 6.0,
                    );
                let extent_pixels =
                    Vec2::new(size.x as f32 / 12.0 - 8.0, size.y as f32 / 6.0 - 8.0);
                let extent = (world(center + extent_pixels * 0.5, depth)
                    - world(center - extent_pixels * 0.5, depth))
                .abs();
                let foam_map = texture(&mut images, [foam_byte, foam_byte, foam_byte, 255]);
                let gloss_map = texture(&mut images, [gloss_byte, gloss_byte, gloss_byte, 255]);
                let params = WaterParams {
                    body: Vec4::ZERO,
                    refl: Vec4::new(0.0, 0.0, 0.0, 1.0),
                    waves: Vec4::ZERO,
                    sun: Vec4::new(0.0, 0.0, 0.0, 1000.0),
                    sun_dir: if ndotl > 0.0 {
                        Vec4::new(0.0, 1.0, 0.0, 1.0)
                    } else {
                        Vec4::new(1.0, 0.0, 0.0, 1.0)
                    },
                    sky_h: Vec4::ZERO,
                    sky_z: Vec4::ZERO,
                    misc: Vec4::ZERO,
                    foam: Vec4::new(0.0, 0.0, 0.0, 0.2),
                    motion: Vec4::new(tag, 0.05, 0.0, 0.0),
                    ocean_foam: Vec4::new(0.7, -0.045, 0.079, 1.1),
                    ocean_sun: sun.extend(if foam_on { 1.0 } else { 0.0 }),
                    ocean_sky: sky.extend(0.0),
                    caustics: Vec4::new(0.0, 0.0, 0.05, if invalid { 6.0 } else { 4.0 }),
                    ..default()
                };
                // Independent literal source formula, no production helper.
                // Clear scene depth fully saturates the native softness term.
                let f = foam_byte as f32 / 255.0;
                let gloss = gloss_byte as f32 / 255.0 * 0.7 + 0.3;
                let softness = 1.0_f32 - 0.045;
                let shape = f
                    * (f - softness * (2.0 * gloss)).clamp(0.0, 1.0)
                    * (softness - (softness - 0.079).clamp(0.0, 1.0).powf(1.1)).clamp(0.0, 1.0);
                let fade = ((if ortho { 1.0_f32 } else { depth }) - 0.33).clamp(0.0, 1.0);
                let supported_below = qualified || (tag > 0.5 && !no_depth && !no_transmission);
                let expected = if !foam_on || (below && !supported_below) {
                    Vec3::ZERO
                } else if qualified {
                    Vec3::splat(shape * (sun.x * ndotl + sky.x) * fade)
                } else {
                    shape * (sun * ndotl + sky)
                };
                controls.push((
                    center.floor().as_uvec2(),
                    encoded(expected),
                    format!("{name} depth={depth} foam={foam_byte} gloss={gloss_byte}"),
                ));
                commands.spawn((
                    Mesh3d(meshes.add(Rectangle::new(extent.x, extent.z))),
                    MeshMaterial3d(materials.add(WaterMaterial {
                        params,
                        normal_map: None,
                        foam_map: Some(foam_map),
                        env_map: None,
                        environment_2d: None,
                        gloss_map: Some(gloss_map),
                        caustics_map: None,
                    })),
                    Transform::from_translation(world(center, depth))
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
            }
        }
    }
    info!(
        "Ocean foam controls: {} below={below} ortho={ortho} qualified={qualified}",
        controls.len()
    );
    commands.insert_resource(Controls(controls));
    let mut camera = commands.spawn((
        Camera3d::default(),
        Camera {
            viewport: flag("--viewport").then_some(Viewport {
                physical_position: origin,
                physical_size: size,
                ..default()
            }),
            ..default()
        },
        projection,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::IDENTITY.looking_at(
            if below { Vec3::Y } else { Vec3::NEG_Y },
            if below { Vec3::Z } else { Vec3::NEG_Z },
        ),
    ));
    if !no_depth {
        camera.insert(DepthPrepass);
    }
    if !no_transmission {
        camera.insert(bevy::pbr::ScreenSpaceTransmission::default());
    }
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let baseline = std::env::args().any(|a| a == "--baseline");
    let mut worst = 0;
    for (pixel, expected, label) in &controls.0 {
        let got = image.get_pixel(pixel.x, pixel.y).0;
        let error = got
            .iter()
            .zip(expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        worst = worst.max(error);
        if !baseline {
            assert!(error <= 2, "{label}: GPU{got:?} CPU{expected:?}");
        }
        info!("Ocean foam {label}: GPU{got:?} CPU{expected:?}");
    }
    info!(
        "Ocean foam: {} GPU controls {}; max_byte_error={worst}",
        controls.0.len(),
        if baseline { "BASELINE" } else { "PASSED" }
    );
}
