//! Native WaterSurface sun/reflection/Fresnel stages. --legacy checks the retained sea-stage arithmetic.
use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::water_material::{WaterMaterial, WaterMaterialPlugin, WaterParams};
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
            aa_dev::default_plugins("Native water normal stages"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

// Independent wrapping bilinear sample of the original DDS's raw XY channels.
fn sample(rgba: &[u8], size: UVec2, uv: Vec2) -> Vec2 {
    let p = uv * size.as_vec2() - Vec2::splat(0.5);
    let lo = p.floor();
    let f = p - lo;
    let at = |x: i32, y: i32| {
        let i = (y.rem_euclid(size.y as i32) * size.x as i32 + x.rem_euclid(size.x as i32))
            as usize
            * 4;
        Vec2::new(rgba[i] as f32, rgba[i + 1] as f32) / 255.0
    };
    at(lo.x as i32, lo.y as i32)
        .lerp(at(lo.x as i32 + 1, lo.y as i32), f.x)
        .lerp(
            at(lo.x as i32, lo.y as i32 + 1).lerp(at(lo.x as i32 + 1, lo.y as i32 + 1), f.x),
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
    let legacy = std::env::args().any(|a| a == "--legacy");
    let uv = Vec2::new(0.173, 0.283);
    let mut controls = Vec::new();
    for row in 0..4 {
        let no_flow = row >= 2;
        let seconds = if row % 2 == 0 { 0.0 } else { 4.0 };
        let material = CryMaterial::load(if no_flow {
            "materials/ocean/lake_tank.mtl"
        } else {
            "materials/ocean/valley_river_watervolume1.mtl"
        })
        .unwrap();
        assert!(material.shader.eq_ignore_ascii_case("WaterSurface"));
        assert_eq!(material.gen_mask & 1, 1);
        assert_eq!(material.gen_mask & 0x200 != 0, no_flow);
        let param = |s: &str| material.public_param(s).unwrap().parse::<f32>().unwrap();
        let bytes = aa_cry::read(&material.texture(&TextureMap::Bump).unwrap().pak_path()).unwrap();
        let mut dds = Dds::parse(&bytes).unwrap();
        dds.swapped_rg = false;
        let rgba = dds.decode_rgba8(0, 0);
        let speed = param("WavesSpeed");
        let big = param("BigWavesScale");
        let small = param("SmallWavesScale");
        let scale = param("BumpScale");
        let drift = Vec2::new(seconds * 0.0125 * speed, 0.0);
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
        let samples = taps.map(|t| sample(&rgba, UVec2::new(dds.width, dds.height), t));
        let xy = ((samples[0] + samples[1] - Vec2::ONE) * big
            + (samples[2] + samples[3] - Vec2::ONE) * small)
            * scale;
        let wave = Vec3::new(xy.x, 1.0, -xy.y).normalize();
        let image = images.add(dds_to_image(&dds, DdsOptions::linear()).unwrap());
        for (depth_index, depth) in [2.0_f32, 160.0, 300.0].into_iter().enumerate() {
            for (stage, mode) in [3.0_f32, 4.0, 5.0, 6.0].into_iter().enumerate() {
                let column = depth_index * 4 + stage;
                let center = Vec2::new(column as f32 * 1.5 - 8.25, 3.0 - row as f32 * 1.9);
                let point = Vec3::new(center.x, 8.0 - depth, -center.y);
                let view = (Vec3::new(0.0, 8.0, 0.0) - point).normalize();
                let distance = point.distance(Vec3::new(0.0, 8.0, 0.0));
                // Literal WaterSurfacePS arithmetic, in Cry->Bevy coordinates.
                // The legacy branch independently retains the old sea-stage calculation.
                let sun = if legacy {
                    let t = ((distance - 60.0) / 200.0).clamp(0.0, 1.0);
                    Vec3::Y
                        .lerp(wave, 1.0 - t * t * (3.0 - 2.0 * t))
                        .normalize()
                } else {
                    wave
                };
                let compressed = if legacy {
                    Vec3::Y.lerp(sun, 0.2)
                } else {
                    Vec3::new(sun.x * 0.15, sun.y, sun.z * 0.15)
                };
                let reflected = compressed.normalize();
                let ndot = view
                    .dot(if legacy { reflected } else { compressed })
                    .abs()
                    .clamp(0.0, 1.0);
                let fresnel = ((0.2 + 0.8 * (1.0 - ndot).powi(5)) * 0.7).clamp(0.0, 1.0);
                let colour = match stage {
                    0 => sun * 0.5 + Vec3::splat(0.5),
                    1 => reflected * 0.5 + Vec3::splat(0.5),
                    2 => Vec3::new(ndot, fresnel, 0.0),
                    _ => compressed * 0.5 + Vec3::splat(0.5),
                };
                let mut mesh = Mesh::from(Rectangle::new(1.2, 1.3));
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![uv.to_array(); 4]);
                commands.spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(materials.add(WaterMaterial {
                        params: WaterParams {
                            waves: Vec4::new(big, small, speed, 0.2),
                            refl: Vec4::new(0.2, 0.7, scale, 1.0),
                            motion: Vec4::new(
                                1.0,
                                if no_flow { 1.0 } else { 0.0 },
                                seconds,
                                mode + if legacy { 10.0 } else { 0.0 },
                            ),
                            ..default()
                        },
                        normal_map: Some(image.clone()),
                        foam_map: None,
                        env_map: None,
                        environment_2d: None,
                        gloss_map: None,
                        caustics_map: None,
                    })),
                    Transform::from_translation(point)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
                // Put CPU swatches at the same depth. A low shared plane would occlude distant cards.
                commands.spawn((
                    Mesh3d(meshes.add(Rectangle::new(1.2, 0.15))),
                    MeshMaterial3d(standards.add(StandardMaterial {
                        base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                        unlit: true,
                        ..default()
                    })),
                    Transform::from_xyz(point.x, point.y, point.z + 0.8)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
                info!(
                    "Water normal control row{row} depth{depth} stage{mode} legacy={legacy}: wave{wave:?} sun{sun:?} compressed{compressed:?} reflection{reflected:?} ndot{ndot} fresnel{fresnel}"
                );
                controls.push((center, srgb(colour)));
            }
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
            far: 1000.0,
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 8.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new(format!("Native water normal stages; legacy-stage control={legacy}. Rows: river0/4s, pond0/4s.\nColumns: sun / reflected / NdotI,Fresnel / compressed; repeated at2 /160 /300m. Thin CPU swatches.")),TextFont {font_size:FontSize::Px(18.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
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
            "water normal{i}: GPU{gpu:?} CPU{expected:?}"
        );
        info!("Water normal{i} PASSED GPU{gpu:?} CPU{expected:?}");
    }
}
