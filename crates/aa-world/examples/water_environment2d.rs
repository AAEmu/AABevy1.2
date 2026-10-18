//! Original 2D environment DDS signed reflection UVs, native mip/filter and SRV controls.
use aa_cry::texture::Dds;
use aa_world::{
    water::WaterStyle,
    water_material::{
        AUTHORED_WATER_ENVIRONMENT_2D, WaterMaterial, WaterMaterialPlugin, WaterParams, WaterStyles,
    },
};
use bevy::{
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::VertexAttributeValues,
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
            aa_dev::default_plugins("Native WaterSurface 2D environment"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .init_resource::<WaterStyles>()
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

fn linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
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

// Independent decoded BC texels. Native SRV decodes each texel before filtering.
fn sample(dds: &Dds, uv: Vec2, mip: u32, srgb: bool) -> Vec3 {
    let w = (dds.width >> mip).max(1);
    let h = (dds.height >> mip).max(1);
    let rgba = dds.decode_rgba8(0, mip);
    let p = uv * Vec2::new(w as f32, h as f32) - Vec2::splat(0.5);
    let lo = p.floor();
    let f = p - lo;
    let texel = |x: i32, y: i32| {
        let i = (y.rem_euclid(h as i32) * w as i32 + x.rem_euclid(w as i32)) as usize * 4;
        let channel = |i| {
            let v = rgba[i] as f32 / 255.0;
            if srgb { linear(v) } else { v }
        };
        Vec3::new(channel(i), channel(i + 1), channel(i + 2))
    };
    texel(lo.x as i32, lo.y as i32)
        .lerp(texel(lo.x as i32 + 1, lo.y as i32), f.x)
        .lerp(
            texel(lo.x as i32, lo.y as i32 + 1).lerp(texel(lo.x as i32 + 1, lo.y as i32 + 1), f.x),
            f.y,
        )
}

fn setup(
    mut commands: Commands,
    mut styles: ResMut<WaterStyles>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let w = windows.single().unwrap().resolution.physical_width() as f32;
    let h = windows.single().unwrap().resolution.physical_height() as f32;
    let mut controls = Vec::new();
    let fractional = std::env::args().any(|a| a == "--fractional");
    let uvs = [
        Vec2::new(-0.63, -0.21),
        Vec2::new(0.37, -0.21),
        Vec2::new(-0.63, 0.79),
        Vec2::new(0.37, 0.79),
        Vec2::new(0.624528372, -0.780695495),
        Vec2::new(0.851842846, -0.523756631),
        Vec2::new(-0.999, 0.001),
        Vec2::new(0.001, 0.001),
    ];
    for (asset, name) in [
        "swamp_rescue",
        "rescue_watertank",
        "river",
        "fleet_watervolume",
    ]
    .into_iter()
    .enumerate()
    {
        let path = format!("materials/ocean/{name}");
        let source = styles.material(
            &WaterStyle::new(&path, 2, [0.0; 3], 0.0),
            &mut materials,
            &mut images,
        );
        let source = materials.get(&source).unwrap().clone();
        assert_eq!(source.params.motion.x, AUTHORED_WATER_ENVIRONMENT_2D);
        let cry = aa_cry::material::Material::load(&format!("{path}.mtl")).unwrap();
        let tex = cry
            .texture(&aa_cry::material::TextureMap::Environment)
            .unwrap()
            .pak_path();
        let bytes = aa_cry::read(&tex).unwrap();
        let dds = Dds::parse(&bytes).unwrap();
        let srgb = images
            .get(source.environment_2d.as_ref().unwrap())
            .unwrap()
            .texture_descriptor
            .format
            .is_srgb();
        for (lodrow, mip) in [0.0_f32, if fractional { 1.5 } else { 2.0 }]
            .into_iter()
            .enumerate()
        {
            for (column, uv) in uvs.into_iter().enumerate() {
                let center =
                    Vec2::new(column as f32 * 2.0 - 7.0, 3.5 - (asset * 2 + lodrow) as f32);
                let screen =
                    Vec2::new(w * 0.5 + center.x * h / 10.0, h * 0.5 - center.y * h / 10.0);
                let pixel = screen.floor().as_uvec2();
                let derivative = Vec2::new(
                    2.0_f32.powf(mip) / dds.width as f32,
                    2.0_f32.powf(mip) / dds.height as f32,
                );
                let mut mesh = Mesh::from(Rectangle::new(1.7, 0.65));
                let VertexAttributeValues::Float32x3(positions) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
                else {
                    panic!("rectangle")
                };
                let coordinates: Vec<[f32; 2]> = positions
                    .iter()
                    .map(|p| (uv + Vec2::new(p[0], -p[1]) * derivative * h / 10.0).to_array())
                    .collect();
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, coordinates);
                let sample_uv = if mip == 0.0 {
                    uv
                } else {
                    uv + (pixel.as_vec2() + Vec2::splat(0.5) - screen) * derivative
                };
                let colour = sample(&dds, sample_uv, mip.floor() as u32, srgb).lerp(
                    sample(&dds, sample_uv, mip.ceil() as u32, srgb),
                    mip.fract(),
                );
                let mut material = source.clone();
                material.params = WaterParams {
                    env_lit: Vec4::ONE,
                    // w=7 deliberately differs from the implicit mip level: cube calibration
                    // must not leak into the 2D tex2D path.
                    foam: Vec4::new(0.0, 0.0, 7.0, 0.0),
                    sky_h: Vec4::new(uv.x, 0.2, -uv.y, 1.0),
                    misc: Vec4::ZERO,
                    motion: Vec4::new(
                        AUTHORED_WATER_ENVIRONMENT_2D,
                        0.0,
                        0.0,
                        if mip == 0.0 { 8.0 } else { 9.0 },
                    ),
                    ..default()
                };
                material.normal_map = None;
                material.foam_map = None;
                commands.spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(materials.add(material)),
                    Transform::from_xyz(center.x, 0.0, -center.y)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
                commands.spawn((
                    Mesh3d(meshes.add(Rectangle::new(1.7, 0.1))),
                    MeshMaterial3d(standards.add(StandardMaterial {
                        base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                        unlit: true,
                        ..default()
                    })),
                    Transform::from_xyz(center.x, 0.0, -center.y + 0.4)
                        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                ));
                let label = format!("{name} srgb{srgb} mip{mip} uv{sample_uv:?}");
                info!("native 2D ENV {label}: CPU{colour:?}");
                controls.push(Control {
                    pixel,
                    expected: encoded(colour),
                    label,
                });
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
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 6.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new("Original probe04 / probe01 / blue_sky / fleet DDS; LOD0/2 (--fractional: LOD0/1.5)\nSigned Cry XY + repeat seams + original negative receiver rays; native linear/sRGB views. Thin CPU swatches."),
        TextFont {font_size:FontSize::Px(17.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(8.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let mut worst = 0;
    for control in &controls.0 {
        let gpu = image.get_pixel(control.pixel.x, control.pixel.y).0;
        let error = gpu
            .iter()
            .zip(control.expected)
            .map(|(a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        worst = worst.max(error);
        assert!(
            error <= 3,
            "{}: GPU{gpu:?} CPU{:?} difference{error}",
            control.label,
            control.expected
        );
    }
    info!(
        "native 2D ENV GPU {} controls PASSED, max_byte_error={worst}",
        controls.0.len()
    );
}
