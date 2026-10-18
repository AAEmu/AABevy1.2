//! GPU pixel-footprint check of the original star shader using the real Sirius record.
//! A bright ungraded sprite on black makes the actual rasterized width measurable in the PNG.
//! `cargo run -p aa-world --example star_size -- --size 1920x1080 --shot shots/aa-world/night/star_size_1080.png`

use aa_world::night_sky::{StarMaterial, StarParams, parse_stars};
use aa_world::{AaWorldPlugin, WorldConfig, cry_to_bevy};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;

fn main() {
    let mut config = WorldConfig::new("main_world");
    config.environment = false;
    config.post = false;
    config.terrain_detail = false;
    config.objects = false;
    config.vegetation = false;
    config.shadows = false;
    config.load_radius = 0.0;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world native star pixel size"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(check_pixels)
        .run();
}

fn check_pixels(shot: On<ScreenshotCaptured>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (width, height) = image.dimensions();
    let expected = match (width, height) {
        (1920, 1080) => 11,
        (1280, 720) => 7,
        (320, 180) => 3,
        _ => {
            warn!("star-size: no fixed regression answer for {width}x{height}");
            return;
        }
    };
    let (mut left, mut top, mut right, mut bottom, mut count) = (width, height, 0, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0.iter().any(|v| *v > 5) {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
            count += 1;
        }
    }
    assert_eq!(
        count,
        expected * expected,
        "one fully covered native star footprint"
    );
    assert_eq!(
        (right - left + 1, bottom - top + 1),
        (expected, expected),
        "native star pixel diameter"
    );
    info!("star-size GPU check: {width}x{height}, footprint={expected}x{expected}, pixels={count}");
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let stars = parse_stars(&aa_pak::Pak::global().read("libs/sky/stars.dat").unwrap()).unwrap();
    let sirius = stars[0];
    let direction = cry_to_bevy(sirius.direction());
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![direction.to_array(); 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![sirius.color_magnitude.map(|v| v as f32 / 255.0); 4],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StarMaterial {
            // High test intensity makes every covered pixel visible; native size cvar remains 1.
            params: StarParams {
                values: Vec4::new(10000.0, 0.0, 1.0, 0.0),
            },
        })),
        Transform::default(),
        bevy::camera::visibility::NoFrustumCulling,
    ));
    // Outside authored cells so the ungraded black pixel fixture has no terrain / sky behind it.
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Msaa::Off,
        Transform::from_translation(Vec3::splat(1_000_000.0)).looking_to(direction, Vec3::Y),
    ));
}
