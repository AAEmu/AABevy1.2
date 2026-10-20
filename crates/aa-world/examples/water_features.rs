//! Source material -> cached production style -> signed foam/glint GPU controls.
//! --legacy restores only the previously unconditional authored multipliers.
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
        render_resource::{Extent3d, TextureDimension, TextureFormat},
        view::screenshot::ScreenshotCaptured,
    },
};

const SUN: Vec3 = Vec3::new(0.04, 0.06, 0.08);
const SKY: Vec3 = Vec3::new(0.03, 0.04, 0.05);
const BACK: Vec3 = Vec3::splat(0.35);
const DEPTH: f32 = 0.001;

struct Card {
    source: Handle<WaterMaterial>,
    point: Vec2,
    term: usize,
    factor: f32,
    mask: u64,
    sun: f32,
    foam: f32,
}
#[derive(Resource, Default)]
struct Pending(Vec<Card>);
#[derive(Resource, Default)]
struct Controls(Vec<(Vec2, [u8; 3])>);

fn main() {
    let exposure = Exposure { ev100: 0.0 };
    let mut sample = TimeOfDay::default().sample(10.0, Lighting::default());
    sample.sun = SUN;
    sample.sky = SKY;
    sample.native_sun_sky = SUN + SKY;
    sample.day = 1.0;
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native water feature flags"),
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
                    sun: scalar("SunMultiplier", 0.1),
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
    commands.spawn((Text::new(format!("Native foam/glint eligibility, old multipliers={}\nRows: lake_tank / decimal river2 / swamp(no foam) / plantation cave(no glint/foam)\nColumns: glint -1 / 0 / +1 ; foam -1 / 0 / +1. Thin CPU references; all other terms fixed.",std::env::args().any(|a|a=="--legacy"))),TextFont {font_size:FontSize::Px(18.0),..default()},Node {position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}

fn prepare(
    mut commands: Commands,
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
    for (i, c) in pending.0.drain(..).enumerate() {
        let source = materials.get(&c.source).unwrap();
        // Assert the actual cached production-style path, before isolating shader terms.
        let sun = if c.mask & 0x2020 != 0 {
            SUN * c.sun
        } else {
            Vec3::ZERO
        };
        let static_light = if source.params.motion.x >= 2.0 {
            if c.mask & 0x2000 != 0 { SUN + SKY } else { Vec3::ONE }
        } else { SUN + SKY };
        let foam_gain = if c.mask & 0x1000 != 0 {
            static_light * c.foam
        } else {
            Vec3::ZERO
        };
        assert!(source.params.sun.truncate().abs_diff_eq(sun, 1e-6));
        assert!(
            source
                .params
                .foam_col
                .truncate()
                .abs_diff_eq(foam_gain, 1e-6)
        );
        let mut material = source.clone();
        let old_sun = SUN * c.sun;
        let old_foam = static_light * c.foam;
        let gain = if c.term == 0 {
            if legacy { old_sun } else { sun }
        } else if legacy {
            old_foam
        } else {
            foam_gain
        };
        // Multiplication by -1/0/+1 commutes with the binary native feature gate.
        // Together with signed source-gate unit tests this covers negative authored gains.
        let gain = gain * c.factor;
        let p = Vec3::new(c.point.x, 0.0, -c.point.y);
        let v = (Vec3::new(0.0, 8.0, 0.0) - p).normalize();
        let reflected = (-v).reflect(Vec3::Y);
        let thickness = DEPTH / v.y;
        let lum = 2.0 * 3.0 * 0.333 - 1.0;
        let reach = (material.params.foam.x * (0.2 / lum) * thickness).clamp(0.0, 1.0);
        let f = 1.0 - reach;
        let blend = ((f - 0.2) / 0.5).clamp(0.0, 1.0);
        let foam_factor = f * blend * blend * (3.0 - 2.0 * blend) * lum;
        let expected = BACK + gain * if c.term == 0 { 1.0 } else { foam_factor };
        material.params.body = Vec4::ZERO;
        material.params.refl.y = 0.0;
        material.params.waves = Vec4::ZERO;
        material.params.misc.x = 0.0;
        material.params.sun = if c.term == 0 {
            gain.extend(10000.0)
        } else {
            Vec3::ZERO.extend(10000.0)
        };
        material.params.sun_dir = reflected.extend(1.0);
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
            "Water feature{i}: mask{:x} term{} factor{} legacy={legacy} gain{gain:?} expected{:?}",
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
            "water feature{i}: GPU{gpu:?} CPU{expected:?}"
        );
        info!("Water feature{i} PASSED GPU{gpu:?} CPU{expected:?}");
    }
}
