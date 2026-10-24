//! Pixel-aligned opaque receivers through the production water material/depth/copy.
//! Native WARP evidence separately establishes the original PS equations. This
//! oracle preserves the port's axial-gap/clear adapter, including orthographic
//! axial depth 8 versus clip W 1; it does not assert native float-LERP identity.
use super::{Controls, encoded, texture};
use aa_world::water_material::{WaterMaterial, WaterParams};
use bevy::{
    camera::Viewport,
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
};

const SURFACE: f32 = 8.0;

struct Receiver {
    bounds: Rect,
    depth: f32,
}

pub(super) fn setup(
    mut commands: Commands,
    window: Single<&Window, With<bevy::window::PrimaryWindow>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut opaque: ResMut<Assets<StandardMaterial>>,
) {
    let args: Vec<String> = std::env::args().collect();
    let flag = |s: &str| args.iter().any(|a| a == s);
    let below = flag("--below");
    let ortho = flag("--ortho");
    let no_depth = flag("--no-depth");
    let no_copy = flag("--no-transmission");
    let invalid = flag("--invalid");
    let number = |name: &str| {
        args.windows(2)
            .find(|a| a[0] == name)
            .map(|a| a[1].parse::<usize>().unwrap())
    };
    let tag = number("--mode").map_or(if flag("--legacy") { 0.0 } else { -1.0 }, |m| m as f32);
    let qualified = tag == -1.0 && !invalid && !no_depth && !no_copy;
    let supported = !below || qualified || (tag > 0.5 && !no_depth && !no_copy);
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
    let viewport = Rect::from_corners(origin.as_vec2(), (origin + size).as_vec2());
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
    // None denotes clear depth, retained as raw zero until the existing 1e4 gap adapter.
    let cases = [
        ("near-to-deep", Some(0.66), Some(2.0)),
        ("deep-to-near", Some(2.0), Some(0.66)),
        ("foreground", Some(0.66), Some(-0.0004)),
        ("equality", Some(0.66), Some(0.0)),
        ("strict-behind", Some(0.66), Some(0.0004)),
        ("shift-clear", Some(0.66), None),
        ("base-clear-foreground", None, Some(-0.0004)),
        ("base-clear-near", None, Some(0.1)),
        ("both-clear", None, None),
        ("zero-offset", Some(0.66), Some(2.0)),
        ("same-depth", Some(0.66), Some(0.66)),
        ("signed-xy", Some(2.0), Some(0.1)),
        ("point-texel", Some(2.0), Some(0.66)),
        ("edge-clamp", Some(2.0), Some(0.66)),
        ("base-clear-equality", None, Some(0.0)),
        ("base-clear-deep", None, Some(2.0)),
    ];
    let selected_case = number("--receiver-case");
    assert!(selected_case.is_none_or(|i| i < cases.len()));
    let mut receivers = Vec::new();
    let mut controls = Vec::new();
    for (case, (name, base_gap, shifted_gap)) in cases.into_iter().enumerate() {
        if selected_case.is_some_and(|i| i != case) {
            continue;
        }
        for (shape, (fbyte, gbyte)) in [(255, 0), (191, 64)].into_iter().enumerate() {
            let cell = case * 2 + shape;
            let lo = origin.as_vec2()
                + Vec2::new((cell % 8) as f32, (cell / 8) as f32) * size.as_vec2()
                    / Vec2::new(8.0, 4.0);
            let hi = lo + size.as_vec2() / Vec2::new(8.0, 4.0);
            let mut point = ((lo + hi) * 0.5).floor() + Vec2::splat(0.5);
            if case == 13 {
                point.x = if shape == 0 {
                    viewport.min.x + 0.5
                } else {
                    viewport.max.x - 0.5
                };
            }
            let rgba = if case == 11 || (case == 13 && shape == 0) {
                [0, 255, 0, 255]
            } else {
                [255, 128, 0, 255]
            };
            // Independent literal wave-normal arithmetic, at distances below flattening.
            let n = Vec3::new(
                (rgba[0] as f32 / 255.0 * 2.0 - 1.0) * 0.5,
                1.0,
                -(rgba[1] as f32 / 255.0 * 2.0 - 1.0) * 0.5,
            )
            .normalize();
            let scale = if case == 9 { 0.0 } else { 0.05 };
            let soft = (0.2_f32 * base_gap.unwrap_or(1e4)).clamp(0.0, 1.0);
            let candidate = (point + Vec2::new(n.x, -n.z) * scale * soft * 0.85 * size.as_vec2())
                .clamp(
                    viewport.min + Vec2::splat(0.5),
                    viewport.max - Vec2::splat(0.5),
                );
            let pixel_rect = |p: Vec2| Rect::from_corners(p.floor(), p.floor() + Vec2::ONE);
            if let Some(gap) = base_gap {
                receivers.push(Receiver {
                    bounds: pixel_rect(point),
                    depth: SURFACE + gap,
                });
            }
            if let Some(gap) = shifted_gap {
                // The one texel candidate cannot replace the base texel when the
                // offset is zero or clamps back to it.
                if point.floor() != candidate.floor() {
                    receivers.push(Receiver {
                        bounds: pixel_rect(candidate),
                        depth: SURFACE + gap,
                    });
                }
            }
            let raw = |depth: f32| {
                if ortho {
                    (100.0 - depth) / 100.0
                } else {
                    0.01 / depth
                }
            };
            let scene = |p: Vec2| {
                receivers
                    .iter()
                    .filter(|r| r.bounds.contains(p.floor() + Vec2::splat(0.5)))
                    .min_by(|a, b| a.depth.total_cmp(&b.depth))
                    .map(|r| (raw(r.depth), r.depth))
            };
            let base = scene(point);
            let shifted = scene(candidate);
            let accepted = shifted.is_none_or(|(z, _)| z < raw(SURFACE));
            let use_shifted = qualified && below && accepted && !flag("--base-depth");
            let receiver = if use_shifted { shifted } else { base };
            let gap = if no_depth {
                1e4
            } else {
                receiver.map_or(1e4, |(_, d)| (d - SURFACE).max(0.0))
            };
            let f = fbyte as f32 / 255.0;
            let g = gbyte as f32 / 255.0 * 0.7 + 0.3;
            let s = (0.7 * gap).clamp(0.0, 1.0) - 0.045;
            let k = f
                * (f - 2.0 * g * s).clamp(0.0, 1.0)
                * (s - (s - 0.079).clamp(0.0, 1.0).powf(1.1)).clamp(0.0, 1.0);
            let sky = Vec3::new(0.25, 0.7, 1.2);
            let expected = if !supported || flag("--foam-off") {
                Vec3::ZERO
            } else if qualified {
                Vec3::splat(k * sky.x * if ortho { 0.67 } else { 1.0 })
            } else {
                k * sky
            };
            controls.push((point.as_uvec2(),encoded(expected),format!("receiver={case}:{name} shape={shape} pixel={},{} accepted={accepted} gap={gap}",point.x as u32,point.y as u32)));
            let bounds = if case == 13 {
                Rect::from_corners(
                    Vec2::new(point.x - 10.0, lo.y + 4.0),
                    Vec2::new(point.x + 10.0, hi.y - 4.0),
                )
            } else {
                Rect::from_corners(lo + Vec2::splat(4.0), hi - Vec2::splat(4.0))
            };
            let extent = (world(bounds.max, SURFACE) - world(bounds.min, SURFACE)).abs();
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(extent.x, extent.z))),
                MeshMaterial3d(materials.add(WaterMaterial {
                    params: WaterParams {
                        body: Vec4::ZERO,
                        refl: Vec4::new(0.0, 0.0, 0.5, 1.0),
                        waves: Vec4::new(1.0, 0.0, 0.0, 1.0),
                        sun: Vec4::new(0.0, 0.0, 0.0, 1000.0),
                        sun_dir: Vec4::new(0.0, 1.0, 0.0, 1.0),
                        sky_h: Vec4::ZERO,
                        sky_z: Vec4::ZERO,
                        misc: Vec4::ZERO,
                        foam: Vec4::new(0.0, 0.0, 0.0, 0.2),
                        motion: Vec4::new(tag, scale, 0.0, 0.0),
                        ocean_foam: Vec4::new(0.7, -0.045, 0.079, 1.1),
                        ocean_sun: Vec4::new(
                            0.0,
                            0.0,
                            0.0,
                            if flag("--foam-off") { 0.0 } else { 1.0 },
                        ),
                        ocean_sky: sky.extend(0.0),
                        caustics: Vec4::new(0.0, 0.0, 0.05, if invalid { 6.0 } else { 4.0 }),
                        ..default()
                    },
                    normal_map: Some(texture(&mut images, rgba)),
                    foam_map: Some(texture(&mut images, [fbyte, fbyte, fbyte, 255])),
                    gloss_map: Some(texture(&mut images, [gbyte, gbyte, gbyte, 255])),
                    env_map: None,
                    environment_2d: None,
                    caustics_map: None,
                })),
                Transform::from_translation(world((bounds.min + bounds.max) * 0.5, SURFACE))
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            ));
        }
    }
    let black = opaque.add(StandardMaterial {
        base_color: Color::BLACK,
        unlit: true,
        cull_mode: None,
        ..default()
    });
    for r in receivers {
        let extent = (world(r.bounds.max, r.depth) - world(r.bounds.min, r.depth)).abs();
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(extent.x, extent.z))),
            MeshMaterial3d(black.clone()),
            Transform::from_translation(world((r.bounds.min + r.bounds.max) * 0.5, r.depth))
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
    }
    info!(
        "Ocean receiver controls: {} below={below} ortho={ortho} qualified={qualified} base_depth={}",
        controls.len(),
        flag("--base-depth")
    );
    commands.insert_resource(Controls(controls));
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
    if !no_copy {
        camera.insert(bevy::pbr::ScreenSpaceTransmission::default());
    }
}
