//! Exact cached WaterSurfaceVS foam projections; retained sea/fallback controls.
use aa_world::water_material::{WaterMaterial, WaterMaterialPlugin, WaterParams};
use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
    window::PrimaryWindow,
};

struct Control {
    pixel: UVec2,
    expected: [u8; 3],
    label: String,
}
#[derive(Resource)]
struct Controls(Vec<Control>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native water foam projection"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

fn encoded(v: Vec3) -> [u8; 3] {
    v.to_array().map(|v| {
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let size = Vec2::new(
        windows.single().unwrap().resolution.physical_width() as f32,
        windows.single().unwrap().resolution.physical_height() as f32,
    );
    let mut controls = Vec::new();
    // Source coefficient bits, not re-evaluation of the source's folded expression.
    // All 13 unmodified original cached VS variants executed these 54 controls on
    // D3D11 WARP with stream output: native float32 UVs match exactly.
    for native in [true, false] {
        let a = if native {
            f32::from_bits(0x3ea3ad19)
        } else {
            0.3197
        };
        let b = if native {
            f32::from_bits(0x3e8f3776)
        } else {
            0.2797
        };
        for xy in [
            Vec2::ZERO,
            Vec2::new(1.0, -1.0),
            Vec2::new(15564.0, 15346.0),
            Vec2::new(9268.0, 12755.0),
            Vec2::new(14168.5, 14263.5),
            Vec2::new(-15564.0, -15346.0),
        ] {
            for seconds in [0.0_f32, 4.0, 123.75] {
                for speed in [-4.0_f32, 0.0, 1.5] {
                    let phase = seconds * 0.025 * speed;
                    let phase = phase - phase.floor();
                    let taps = [
                        xy * a - Vec2::splat(phase),
                        Vec2::new(xy.y, xy.x) * b + Vec2::splat(phase),
                    ];
                    for (tap, uv) in taps.into_iter().enumerate() {
                        let i = controls.len();
                        let center =
                            Vec2::new((i % 18) as f32 * 1.4 - 11.9, 5.775 - (i / 18) as f32 * 1.05);
                        let point = Vec2::new(
                            size.x * 0.5 + center.x * size.y / 14.4,
                            size.y * 0.5 - center.y * size.y / 14.4,
                        )
                        .floor()
                        .as_uvec2();
                        let uv = uv - uv.floor();
                        let colour = Vec3::new(uv.x, uv.y, 0.2);
                        let params = WaterParams {
                            sky_h: Vec4::new(xy.x, xy.y, 0.0, 0.0),
                            foam: Vec4::new(0.0, speed, 0.0, 0.0),
                            motion: Vec4::new(
                                if native { 1.0 } else { 0.0 },
                                0.0,
                                seconds,
                                30.0 + tap as f32,
                            ),
                            ..default()
                        };
                        commands.spawn((
                            Mesh3d(meshes.add(Rectangle::new(1.25, 0.62))),
                            MeshMaterial3d(materials.add(WaterMaterial {
                                params,
                                normal_map: None,
                                foam_map: None,
                                env_map: None,
                                environment_2d: None,
                                gloss_map: None,
                                caustics_map: None,
                            })),
                            Transform::from_xyz(center.x, 0.0, -center.y)
                                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                        ));
                        commands.spawn((
                            Mesh3d(meshes.add(Rectangle::new(1.25, 0.1))),
                            MeshMaterial3d(standards.add(StandardMaterial {
                                base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                                unlit: true,
                                ..default()
                            })),
                            Transform::from_xyz(center.x, 0.0, -center.y + 0.4)
                                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                        ));
                        controls.push(Control {
                            pixel: point,
                            expected: encoded(colour),
                            label: format!(
                                "native{native} xy{xy:?} seconds{seconds} speed{speed} tap{tap}"
                            ),
                        });
                    }
                }
            }
        }
    }
    info!(
        "native foam projection: {} controls; upper six rows native / lower six retained sea-fallback",
        controls.len()
    );
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 14.4,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 8.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((
        Text::new("Native cached foam UVs / retained sea-fallback UVs; 216 signed position/time/speed/tap controls\nThin bars: independent float32 CPU. Original pond/river/lake and negative world coordinates."),
        TextFont {font_size:FontSize::Px(15.0),..default()},
        Node {position_type:PositionType::Absolute,left:px(12.0),top:px(8.0),..default()},
    ));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let mut worst = 0;
    for control in &controls.0 {
        let got = image.get_pixel(control.pixel.x, control.pixel.y).0;
        let error = got
            .iter()
            .zip(control.expected)
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        worst = worst.max(error);
        assert!(
            error <= 3,
            "{}: GPU{got:?} CPU{:?} max difference{error}",
            control.label,
            control.expected
        );
    }
    info!(
        "native foam projection: {} GPU controls PASSED; max_byte_error={worst}",
        controls.0.len()
    );
}
