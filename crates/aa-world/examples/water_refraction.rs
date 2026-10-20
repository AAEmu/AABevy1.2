//! Native WaterSurface POINT/CLAMP refraction. --ortho / --viewport / --legacy / --no-depth.
//! Opaque pixel-aligned receivers exercise the production depth and transmission copy.
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

const BLUE: Vec3 = Vec3::new(0.04, 0.12, 0.6);
const RED: Vec3 = Vec3::new(0.7, 0.03, 0.02);
const CLEAR: Vec3 = Vec3::new(0.02, 0.4, 0.08);
const SURFACE: f32 = 8.0;

#[derive(Clone)]
struct Receiver {
    lo: Vec2,
    hi: Vec2,
    depth: f32,
    colour: Vec3,
}
#[derive(Resource)]
struct Controls(Vec<(UVec2, [u8; 3], String)>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native water refraction"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::linear_rgb(CLEAR.x, CLEAR.y, CLEAR.z)))
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

fn setup(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let flag = |s: &str| std::env::args().any(|a| a == s);
    let ortho = flag("--ortho");
    let sub = flag("--viewport");
    let legacy = flag("--legacy");
    let no_depth = flag("--no-depth");
    let old_sampling = legacy || no_depth;
    let target = UVec2::new(window.physical_width(), window.physical_height());
    let origin = if sub { UVec2::new(80, 40) } else { UVec2::ZERO };
    let size = if sub {
        target - UVec2::new(240, 120)
    } else {
        target
    };
    let viewport = Rect::from_corners(origin.as_vec2(), (origin + size).as_vec2());
    let fov = 2.0 * (3.0_f32 / SURFACE).atan();
    let projection = if ortho {
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 6.0,
            },
            near: 0.0,
            far: 100.0,
            ..OrthographicProjection::default_3d()
        })
    } else {
        Projection::Perspective(PerspectiveProjection {
            fov,
            near: 0.1,
            far: 100.0,
            ..default()
        })
    };
    let world = |pixel: Vec2, depth: f32| {
        let factor = if ortho { 1.0 } else { depth / SURFACE };
        let q = (pixel - origin.as_vec2() - size.as_vec2() * 0.5) * (6.0 / size.y as f32) * factor;
        Vec3::new(q.x, SURFACE - depth, q.y)
    };
    let mut receivers = Vec::<Receiver>::new();
    let mut controls = Vec::new();
    for i in 0..12 {
        let point = if i == 11 {
            Vec2::new(
                (origin.x + size.x - 5) as f32 + 0.5,
                (origin.y + size.y - 55) as f32 + 0.5,
            )
        } else {
            let x = origin.x as f32 + (i % 4) as f32 * size.x as f32 / 4.0 + size.x as f32 / 8.0;
            let y = origin.y as f32 + 110.0 + (i / 4) as f32 * (size.y as f32 - 210.0) / 3.0;
            Vec2::new(x.floor() + 0.5, y.floor() + 0.5)
        };
        let rgba = if i == 10 {
            [0, 255, 0, 255]
        } else {
            [255, 128, 0, 255]
        };
        // Independent literal native wave -> compressed bump -> viewport-UV arithmetic.
        let wave = Vec3::new(
            (rgba[0] as f32 / 255.0 * 2.0 - 1.0) * 0.5,
            1.0,
            -(rgba[1] as f32 / 255.0 * 2.0 - 1.0) * 0.5,
        )
        .normalize();
        let native_delta = Vec2::new(wave.x, -wave.z) * 0.015 * size.as_vec2();
        let native = (point + native_delta).clamp(
            viewport.min + Vec2::splat(0.5),
            viewport.max - Vec2::splat(0.5),
        );
        let depth = match i {
            5 => 0.05,
            7 => 0.4,
            _ => 4.0,
        };
        let name = match i {
            0 => "strict foreground",
            1 => "equal depth",
            2 => "strict behind",
            3 => "shifted clear",
            4 => "base clear/shifted foreground",
            5 => "shallow fixed offset",
            6 => "deep fixed offset",
            7 => "middle fixed offset",
            8 => "point texel",
            9 => "native ignores fitted strength",
            10 => "signed XY",
            _ => "viewport edge clamp",
        };
        if i != 4 {
            let width = if i == 3 { 1.0 } else { 50.0 };
            receivers.push(Receiver {
                lo: point - Vec2::new(width, 35.0),
                hi: point + Vec2::new(width, 35.0),
                depth: SURFACE + depth,
                colour: BLUE,
            });
        }
        if i != 3 {
            let lo = if i == 11 {
                Vec2::new(viewport.max.x - 1.0, point.y - 20.0)
            } else {
                native.floor() - Vec2::new(if i == 8 { 0.0 } else { 3.0 }, 1.0)
            };
            let hi = if i == 11 {
                Vec2::new(viewport.max.x, point.y + 20.0)
            } else {
                lo + Vec2::new(if i == 8 { 1.0 } else { 7.0 }, 3.0)
            };
            let candidate_depth = match i {
                0 | 4 => SURFACE - 0.0004,
                1 => SURFACE,
                2 => SURFACE + 0.0004,
                _ => SURFACE + depth,
            };
            // On equal-background cases the red patch replaces the same-depth blue
            // receiver through a microscopic depth separation, not draw-order luck.
            receivers.push(Receiver {
                lo,
                hi,
                depth: if i >= 5 {
                    candidate_depth - 0.00001
                } else {
                    candidate_depth
                },
                colour: RED,
            });
        }
        let scene = |p: Vec2| -> (f32, Vec3) {
            let p = p.clamp(Vec2::splat(0.5), target.as_vec2() - Vec2::splat(0.5));
            if !viewport.contains(p) {
                return (0.0, CLEAR);
            }
            let p = p.floor() + Vec2::splat(0.5);
            receivers
                .iter()
                .filter(|r| p.x >= r.lo.x && p.y >= r.lo.y && p.x < r.hi.x && p.y < r.hi.y)
                .min_by(|a, b| a.depth.total_cmp(&b.depth))
                .map_or((0.0, CLEAR), |r| {
                    (
                        if ortho {
                            (100.0 - r.depth) / 100.0
                        } else {
                            0.1 / r.depth
                        },
                        r.colour,
                    )
                })
        };
        let surface_depth = if ortho {
            (100.0 - SURFACE) / 100.0
        } else {
            0.1 / SURFACE
        };
        // Same native/legacy offset for this deep one-texel receiver: only the
        // native POINT versus shared linear scene sampler can change its colour.
        let fitted = if i == 9 {
            0.0
        } else if i == 8 {
            0.015
        } else {
            0.012
        };
        let sample = if old_sampling {
            let (base, _) = scene(point);
            let along = (Vec3::new(0.0, 8.0, 0.0) - world(point, SURFACE))
                .normalize()
                .y;
            let thickness = if no_depth || base == 0.0 {
                60.0
            } else {
                depth / along
            };
            let candidate = point
                + Vec2::new(wave.x, -wave.z)
                    * fitted
                    * thickness.clamp(0.0, 1.0)
                    * target.as_vec2();
            if no_depth || base <= 0.0 || scene(candidate).0 <= surface_depth * 1.0001 {
                candidate
            } else {
                point
            }
        } else if scene(native).0 < surface_depth {
            native
        } else {
            point
        };
        let colour = if old_sampling {
            let p = sample - Vec2::splat(0.5);
            let f = p.fract();
            let lo = p.floor() + Vec2::splat(0.5);
            scene(lo).1.lerp(scene(lo + Vec2::X).1, f.x).lerp(
                scene(lo + Vec2::Y).1.lerp(scene(lo + Vec2::ONE).1, f.x),
                f.y,
            )
        } else {
            scene(sample).1
        };
        info!(
            "Water refraction{i} {name} ortho={ortho} subviewport={sub} legacy={legacy}: point={point:?} native={native:?} sample={sample:?} expected={:?}",
            encoded(colour)
        );
        controls.push((point.as_uvec2(), encoded(colour), name.to_owned()));
        let normal = images.add(Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &rgba,
            TextureFormat::Rgba8Unorm,
            bevy::asset::RenderAssetUsages::default(),
        ));
        let bounds = Vec2::new(90.0, 64.0);
        let p = world(point, SURFACE);
        let extent =
            (world(point + bounds * 0.5, SURFACE) - world(point - bounds * 0.5, SURFACE)).abs();
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(extent.x, extent.z))),
            MeshMaterial3d(materials.add(WaterMaterial {
                params: WaterParams {
                    body: Vec4::ZERO,
                    refl: Vec4::new(0.0, 0.0, 0.5, 1.0),
                    waves: Vec4::new(1.0, 0.0, 0.0, 1.0),
                    sun: Vec4::new(0.0, 0.0, 0.0, 1000.0),
                    misc: Vec4::new(0.0, fitted, 0.0, 0.0),
                    motion: Vec4::new(1.0, 0.0, 0.0, if legacy { -3.0 } else { 0.0 }),
                    ..default()
                },
                normal_map: Some(normal),
                foam_map: None,
                env_map: None,
                environment_2d: None,
                gloss_map: None,
                caustics_map: None,
            })),
            Transform::from_translation(p)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
    }
    for r in receivers {
        let center = world((r.lo + r.hi) * 0.5, r.depth);
        let extent = (world(r.hi, r.depth) - world(r.lo, r.depth)).abs();
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(extent.x, extent.z))),
            MeshMaterial3d(standards.add(StandardMaterial {
                base_color: Color::linear_rgb(r.colour.x, r.colour.y, r.colour.z),
                unlit: true,
                ..default()
            })),
            Transform::from_translation(center)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
    }
    commands.insert_resource(Controls(controls));
    let mut camera = commands.spawn((
        Camera3d::default(),
        Camera {
            viewport: sub.then_some(Viewport {
                physical_position: origin,
                physical_size: size,
                ..default()
            }),
            ..default()
        },
        projection,
        bevy::pbr::ScreenSpaceTransmission::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 8.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    if !no_depth {
        camera.insert(DepthPrepass);
    }
    commands.spawn((Text::new(format!("Native water refraction: ortho={ortho}, subviewport={sub}, legacy={legacy}\nStrict foreground / equal / behind / shifted clear; base-clear foreground / shallow / deep / middle; point / fitted-strength-zero / signed XY / edge")),TextFont {font_size:FontSize::Px(16.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    for (i, (p, expected, name)) in controls.0.iter().enumerate() {
        let gpu = image.get_pixel(p.x, p.y).0;
        assert!(
            gpu.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 3),
            "refraction{i} {name}: GPU{gpu:?} CPU{expected:?}"
        );
        info!("Water refraction{i} {name} PASSED GPU{gpu:?} CPU{expected:?}");
    }
}
