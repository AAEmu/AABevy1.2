//! Original WaterSurface static cube directions; --folded isolates the retained fallback.
use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::water_material::{
    AUTHORED_WATER_CUBE, AUTHORED_WATER_UV, WaterMaterial, WaterMaterialPlugin, WaterParams,
};
use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
};

#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native water static cube rays"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

// D3D11/WebGPU/Vulkan cube face selection. These inputs avoid ties and seam taps.
fn face_uv(d: Vec3) -> (usize, Vec2) {
    let a = d.abs();
    let (face, st, major) = if a.x > a.y && a.x > a.z {
        if d.x > 0.0 {
            (0, Vec2::new(-d.z, -d.y), a.x)
        } else {
            (1, Vec2::new(d.z, -d.y), a.x)
        }
    } else if a.y > a.z {
        if d.y > 0.0 {
            (2, Vec2::new(d.x, d.z), a.y)
        } else {
            (3, Vec2::new(d.x, -d.z), a.y)
        }
    } else if d.z > 0.0 {
        (4, Vec2::new(d.x, -d.y), a.z)
    } else {
        (5, Vec2::new(-d.x, -d.y), a.z)
    };
    (face, st / major * 0.5 + Vec2::splat(0.5))
}

fn sample(dds: &Dds, direction: Vec3, mip: u32) -> Vec3 {
    let (face, uv) = face_uv(direction);
    let size = UVec2::new(dds.width >> mip, dds.height >> mip);
    let rgba = dds.decode_rgba8(face as u32, mip);
    let p = (uv * size.as_vec2() - Vec2::splat(0.5)).clamp(Vec2::ZERO, size.as_vec2() - Vec2::ONE);
    let lo = p.floor();
    let f = p - lo;
    let texel = |x: u32, y: u32| {
        let i = (y * size.x + x) as usize * 4;
        Vec3::new(rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32) / 255.0
    };
    texel(lo.x as u32, lo.y as u32)
        .lerp(texel((lo.x as u32 + 1).min(size.x - 1), lo.y as u32), f.x)
        .lerp(
            texel(lo.x as u32, (lo.y as u32 + 1).min(size.y - 1)).lerp(
                texel(
                    (lo.x as u32 + 1).min(size.x - 1),
                    (lo.y as u32 + 1).min(size.y - 1),
                ),
                f.x,
            ),
            f.y,
        )
}

fn srgb(value: Vec3) -> [u8; 3] {
    value.to_array().map(|v| {
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
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let folded = std::env::args().any(|a| a == "--folded");
    let bytes = aa_cry::read("textures/cubemaps/camera_river_cubemap.dds").unwrap();
    let dds = Dds::parse(&bytes).unwrap();
    assert_eq!(
        (dds.width, dds.height, dds.faces, dds.mips),
        (256, 256, 6, 9)
    );
    let image = images.add(
        dds_to_image(
            &dds,
            DdsOptions {
                srgb: false,
                bc_supported: true,
                repeat: false,
            },
        )
        .unwrap(),
    );
    // Original pond polygon/terrain/ATI2 time0 receiver rays; each is away from a face seam.
    let negative = [
        Vec3::new(0.624528372, -0.780695495, -0.021882798),
        Vec3::new(0.780794208, -0.624488258, -0.019360260),
        Vec3::new(0.851842846, -0.523756631, -0.006538977),
        Vec3::new(0.843639190, -0.536893914, -0.004223975),
        Vec3::new(0.879247095, -0.473444819, -0.052673986),
        Vec3::new(0.928589251, -0.371090071, -0.003763189),
    ];
    let mut rays = vec![
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    rays.extend(negative);
    rays.extend(negative.map(|d| Vec3::new(d.x, d.y, -d.z)));
    let mut controls = Vec::new();
    for (mip_row, mip) in [0u32, 2].into_iter().enumerate() {
        for (index, ray) in rays.iter().enumerate() {
            let column = index % 9;
            let row = mip_row * 2 + index / 9;
            let center = Vec2::new(column as f32 * 1.8 - 7.2, 3.0 - row as f32 * 1.9);
            let sample_ray = if folded {
                Vec3::new(ray.x, ray.y, ray.z.abs())
            } else {
                *ray
            };
            let colour = sample(&dds, sample_ray, mip);
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.4, 1.3))),
                MeshMaterial3d(materials.add(WaterMaterial {
                    params: WaterParams {
                        // Diagnostic8 injects this known reflected Bevy ray into the production sampler.
                        sky_h: aa_world::cry_to_bevy(*ray).extend(1.0),
                        foam: Vec4::new(0.0, 0.0, mip as f32, 0.0),
                        env_lit: Vec4::ONE,
                        misc: Vec4::ZERO,
                        motion: Vec4::new(
                            if folded {
                                AUTHORED_WATER_UV
                            } else {
                                AUTHORED_WATER_CUBE
                            },
                            0.0,
                            0.0,
                            8.0,
                        ),
                        ..default()
                    },
                    normal_map: None,
                    foam_map: None,
                    env_map: Some(image.clone()),
                    environment_2d: None,
                    gloss_map: None,
                    caustics_map: None,
                })),
                Transform::from_xyz(center.x, 0.0, -center.y)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.4, 0.15))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(center.x, 0.0, -center.y + 0.8)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            ));
            controls.push((center, srgb(colour)));
            info!(
                "Cube control{index} mip{mip} folded={folded} original_ray{ray:?} face_uv{:?} linear{colour:?}",
                face_uv(sample_ray)
            );
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 10.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 6.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new(format!("Original camera_river cubemap / folded fallback={folded}. Top2 rows LOD0, bottom2 LOD2.\nSix face axes +six actual negative pond receiver rays +their positive controls. Thin independent CPU swatches.")),
        TextFont {font_size:FontSize::Px(18.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (point, expected)) in controls.0.iter().enumerate() {
        let gpu = image
            .get_pixel(
                (w as f32 * 0.5 + point.x * h as f32 / 10.0) as u32,
                (h as f32 * 0.5 - point.y * h as f32 / 10.0) as u32,
            )
            .0;
        assert!(
            gpu.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 3),
            "cube{i}: GPU{gpu:?} CPU{expected:?}"
        );
        info!("Water cube{i} PASSED GPU{gpu:?} CPU{expected:?}");
    }
}
