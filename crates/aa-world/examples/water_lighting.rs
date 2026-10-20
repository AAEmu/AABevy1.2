//! Native static ENV/foam cSunShading controls: --hdr / --night / --legacy.
//! Uses cached original material flags and production environment refresh.
use aa_cry::material::Material as CryMaterial;
use aa_world::{
    env::{Lighting, TimeOfDay},
    sky::{AtmosphereTuning, Environment},
    water::WaterStyle,
    water_material::{WaterMaterial, WaterMaterialPlugin, WaterStyles, update_water},
};
use bevy::{
    camera::Exposure,
    core_pipeline::{
        prepass::DepthPrepass,
        tonemapping::{DebandDither, Tonemapping},
    },
    prelude::*,
    render::{
        render_resource::{
            Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
        },
        view::screenshot::ScreenshotCaptured,
    },
};

#[derive(Resource)]
struct ExpectedLight {
    native: Vec3,
    legacy: Vec3,
}
const BACK: Vec3 = Vec3::splat(0.35);
const DEPTH: f32 = 0.001;

struct Card {
    source: Handle<WaterMaterial>,
    point: Vec2,
    term: usize,
    factor: f32,
    mask: u64,
    foam: f32,
}
#[derive(Resource, Default)]
struct Pending(Vec<Card>);
#[derive(Resource, Default)]
struct Controls(Vec<(Vec2, [u8; 3])>);

fn main() {
    let exposure = Exposure { ev100: 0.0 };
    let night = std::env::args().any(|a| a == "--night");
    let hdr = std::env::args().any(|a| a == "--hdr");
    let colour = if night { "0:0:0" } else { "0.2:0.5:0.8" };
    let sky = if night { "0:0:0" } else { "0.8:0.5:0.2" };
    let power = if hdr { 0.6 } else { 0.0 };
    let xml = format!(
        r#"<TimeOfDay sRGB="1">
      <Variable Name="Sun color"><Spline Keys="0:({colour}):0,1:({colour}):0,"/></Variable>
      <Variable Name="Sky color"><Spline Keys="0:({sky}):0,1:({sky}):0,"/></Variable>
      <Variable Name="Sun color multiplier"><Spline Keys="0:0.4:0,1:0.4:0,"/></Variable>
      <Variable Name="Sky color multiplier"><Spline Keys="0:0.75:0,1:0.75:0,"/></Variable>
      <Variable Name="HDR dynamic power factor"><Spline Keys="0:{power}:0,1:{power}:0,"/></Variable>
    </TimeOfDay>"#
    );
    let mut sample = TimeOfDay::parse(&xml)
        .unwrap()
        .sample(10.0, Lighting::default());
    // Full original ToD SSE -> setters/getters -> renderer RGB upload goldens.
    let native = if night {
        Vec3::ZERO
    } else if hdr {
        Vec3::new(1.0306873, 0.87153703, 0.7123868)
    } else {
        Vec3::new(0.68, 0.575, 0.47000003)
    };
    assert!(sample.native_sun_sky.abs_diff_eq(native, 1e-6));
    let expected = ExpectedLight {
        native,
        legacy: sample.sun + sample.sky,
    };
    sample.day = 1.0;
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native static water light"),
            aa_dev::DevPlugin,
            WaterMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Environment {
            sample: Some(sample),
            zone: None,
            hour: 10.0,
            ocean_fog: Vec3::ZERO, ocean_density: 0.0,
        })
        .insert_resource(AtmosphereTuning {
            radiance: exposure.exposure().recip(),
            water_foam: 1.0,
            water_sun: 1.0,
            ..default()
        })
        .insert_resource(expected)
        .init_resource::<WaterStyles>()
        .init_resource::<Pending>()
        .init_resource::<Controls>()
        .add_systems(Startup, setup)
        .add_systems(Update, (update_water, prepare).chain())
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
    mut styles: ResMut<WaterStyles>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut pending: ResMut<Pending>,
) {
    for (row, name) in [
        "lake_tank",
        "valley_river_watervolume2",
        "swamp_rescue",
        "water_plantation_cave",
    ]
    .into_iter()
    .enumerate()
    {
        let path = format!("materials/ocean/{name}");
        let mtl = CryMaterial::load(&format!("{path}.mtl")).unwrap();
        assert!(mtl.shader.eq_ignore_ascii_case("WaterSurface") && mtl.gen_mask & 1 != 0);
        let scalar = |s: &str, d: f32| {
            mtl.public_param(s)
                .and_then(|v| v.parse().ok())
                .unwrap_or(d)
        };
        let source = styles.material(
            &WaterStyle::new(&path, 2, [0.0; 3], 0.0),
            &mut materials,
            &mut images,
        );
        for term in 0..2 {
            for (column, factor) in [-1.0_f32, 0.0, 1.0].into_iter().enumerate() {
                let point = Vec2::new(
                    (term * 3 + column) as f32 * 2.0 - 5.0,
                    3.0 - row as f32 * 2.0,
                );
                pending.0.push(Card {
                    source: source.clone(),
                    point,
                    term,
                    factor,
                    mask: mtl.gen_mask,
                    foam: scalar("FoamMultiplier", 0.75),
                });
            }
        }
    }
    commands.spawn((
        Camera3d::default(),
        bevy::pbr::ScreenSpaceTransmission::default(),
        DepthPrepass,
        aa_world::post::PostUniform::default(),
        Exposure { ev100: 0.0 },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 10.0,
            },
            near: 0.0,
            far: 100.0,
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 8.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    ));
    commands.spawn((Text::new(format!("Native static water cSunShading, legacy radiometry={}\nRows: lake_tank(SUN off) / river2(on) / swamp2D(off,no foam) / plantation(mode1 fallback)\nENV -1 / 0 / +1 ; foam -1 / 0 / +1. Thin CPU references; --hdr / --night.",std::env::args().any(|a|a=="--legacy"))),TextFont {font_size:FontSize::Px(17.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}

fn prepare(
    mut commands: Commands,
    expected_light: Res<ExpectedLight>,
    mut pending: ResMut<Pending>,
    mut controls: ResMut<Controls>,
    mut materials: ResMut<Assets<WaterMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    if pending.0.is_empty() {
        return;
    }
    let legacy = std::env::args().any(|a| a == "--legacy");
    let foam = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    ));
    let mut cube = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        &[255, 255, 255, 255],
        TextureFormat::Rgba8Unorm,
        bevy::asset::RenderAssetUsages::default(),
    );
    cube.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    let cube = images.add(cube);
    for (i, c) in pending.0.drain(..).enumerate() {
        let source = materials.get(&c.source).unwrap();
        // Assert the actual cached production-style path, before isolating shader terms.
        let static_light = if source.params.motion.x >= 2.0 {
            if c.mask & 0x2000 != 0 {
                expected_light.native
            } else {
                Vec3::ONE
            }
        } else {
            expected_light.legacy
        };
        let foam_gain = if c.mask & 0x1000 != 0 {
            static_light * c.foam
        } else {
            Vec3::ZERO
        };
        assert!(
            source
                .params
                .env_lit
                .truncate()
                .abs_diff_eq(static_light, 1e-6)
        );
        assert!(
            source
                .params
                .foam_col
                .truncate()
                .abs_diff_eq(foam_gain, 1e-6)
        );
        let mut material = source.clone();
        let light = if legacy {
            expected_light.legacy
        } else {
            static_light
        };
        let gain = if c.term == 0 {
            light
        } else if c.mask & 0x1000 != 0 {
            light * c.foam
        } else {
            Vec3::ZERO
        };
        // Multiplication by -1/0/+1 commutes with the binary native feature gate.
        // Together with signed source-gate unit tests this covers negative authored gains.
        let gain = gain * c.factor;
        let p = Vec3::new(c.point.x, 0.0, -c.point.y);
        let v = (Vec3::new(0.0, 8.0, 0.0) - p).normalize();
        let thickness = DEPTH / v.y;
        let lum = 2.0 * 3.0 * 0.333 - 1.0;
        let reach = (material.params.foam.x * (0.2 / lum) * thickness).clamp(0.0, 1.0);
        let f = 1.0 - reach;
        let blend = ((f - 0.2) / 0.5).clamp(0.0, 1.0);
        let foam_factor = f * blend * blend * (3.0 - 2.0 * blend) * lum;
        let expected = if c.term == 0 {
            gain
        } else {
            BACK + gain * foam_factor
        };
        material.params.body = Vec4::ZERO;
        material.params.refl.y = 0.0;
        material.params.waves = Vec4::ZERO;
        material.params.misc.x = 0.0;
        material.params.sun = Vec3::ZERO.extend(10000.0);
        material.params.env_lit = if c.term == 0 {
            gain.extend(1.0)
        } else {
            Vec4::ZERO
        };
        material.params.motion.w = if c.term == 0 { 8.0 } else { 0.0 };
        material.params.sky_h = Vec4::new(0.2, 0.8, 0.3, 0.0);
        material.env_map = Some(cube.clone());
        material.environment_2d = Some(foam.clone());
        material.normal_map = None;
        material.params.foam_col = if c.term == 1 {
            gain.extend(0.0)
        } else {
            Vec4::ZERO
        };
        material.params.motion.z = 0.0;
        material.foam_map = Some(foam.clone());
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(1.7, 1.25))),
            MeshMaterial3d(materials.add(material)),
            Transform::from_translation(p)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        ));
        for (y, z, height, colour) in [(-DEPTH, p.z, 1.25, BACK), (0.0, p.z + 0.8, 0.18, expected)]
        {
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.7, height))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(colour.x, colour.y, colour.z),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(p.x, y, z)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            ));
        }
        controls.0.push((c.point, encoded(expected)));
        info!(
            "Water native light{i}: mask{:x} term{} factor{} legacy={legacy} gain{gain:?} expected{:?}",
            c.mask,
            c.term,
            c.factor,
            encoded(expected)
        );
    }
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    assert_eq!(controls.0.len(), 24);
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, expected)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 10.0) as u32;
        let y = (h as f32 * 0.5 - p.y * h as f32 / 10.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(expected).all(|(a, b)| a.abs_diff(*b) <= 3),
            "water native light{i}: GPU{gpu:?} CPU{expected:?}"
        );
        info!("Water native light{i} PASSED GPU{gpu:?} CPU{expected:?}");
    }
}
