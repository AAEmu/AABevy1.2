//! Diagnostic view of an original cave's serialized first-LOD collision proxy.
//! Colours/wireframe are diagnostic; native projected detail/atlas rendering remains unsupported.
//! --unload-check moves the camera away and checks the normal collision-cell release path.

use aa_world::collide::{CollisionConfig, Mask};
use aa_world::{TerrainQuery, WorldCollision, bevy_to_cry, cry_to_bevy};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;

#[derive(Component)]
struct ProxyView;

#[derive(Resource, Default)]
struct Fixture {
    unload: bool,
    moved: bool,
    verified: bool,
    last: f32,
    follow_at: f32,
}

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins(
            "aa-world native cave collision proxy",
        ))
        .add_plugins(aa_dev::DevPlugin)
        .insert_resource(ClearColor(Color::srgb(0.012, 0.018, 0.028)))
        .insert_resource(Fixture {
            unload: std::env::args().any(|arg| arg == "--unload-check"),
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, follow)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cell = (20, 8);
    let collision = WorldCollision::with_config(
        "main_world",
        TerrainQuery::new("main_world"),
        CollisionConfig {
            radius: 20.0,
            keep: 50.0,
            ..default()
        },
    );
    let built = collision.build_blocking(cell.0, cell.1).unwrap();
    assert_eq!(
        (built.stats.voxels_colliding, built.stats.voxel_triangles),
        (7, 45413)
    );
    info!("original cave collision: {:?}", built.stats);
    drop(built);
    let objects = aa_world::placement::load_cell_objects("main_world", cell.0, cell.1);
    let voxel = &objects.voxels[0];
    let proxy = voxel.compiled_collision().unwrap();
    let transform =
        Mat4::from_translation(aa_world::coords::cell_origin(cell.0, cell.1)) * voxel.mat4();
    let points: Vec<_> = proxy
        .positions
        .iter()
        .map(|p| cry_to_bevy(transform.transform_point3(*p)).to_array())
        .collect();
    let mut faces = Vec::new();
    let mut edges = Vec::new();
    let mut colours = Vec::new();
    for (tri, id) in proxy
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&proxy.materials)
    {
        faces.extend(tri.map(|i| points[i as usize]));
        let colour = match id {
            0 => [0.045, 0.17, 0.22, 1.0],
            1 => [0.18, 0.095, 0.025, 1.0],
            _ => [0.12, 0.05, 0.16, 1.0],
        };
        colours.extend([colour; 3]);
        for [a, b] in [[0, 1], [1, 2], [2, 0]] {
            edges.extend([points[tri[a] as usize], points[tri[b] as usize]]);
        }
    }
    let fill = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, faces)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    let wire = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, edges);
    for (mesh, colour) in [(fill, Color::WHITE), (wire, Color::srgb(0.18, 0.68, 0.74))] {
        commands.spawn((
            ProxyView,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: colour,
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
        ));
    }
    for (point, normal) in [
        (
            Vec3::new(21354.6667, 9055.3333, 454.91738),
            Vec3::new(0.26708772, -0.33918378, 0.90200806),
        ),
        (
            Vec3::new(21322.6667, 9063.3309, 472.96565),
            Vec3::new(0.61825323, 0.7845947, -0.04662632),
        ),
        (
            Vec3::new(21325.3333, 9054.6667, 438.62933),
            Vec3::new(-0.2663303, 0.3331916, 0.904462),
        ),
    ] {
        let hit = collision
            .raycast_mask(point + normal * 0.5, -normal, 1.0, Mask::STRUCTURES)
            .unwrap();
        assert!((hit.t - 0.5).abs() < 0.004 && hit.point.distance(point) < 0.008);
        commands.spawn((
            ProxyView,
            Mesh3d(meshes.add(Sphere::new(0.55))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.65, 0.08),
                unlit: true,
                ..default()
            })),
            Transform::from_translation(cry_to_bevy(hit.point)),
        ));
    }
    commands.insert_resource(collision);
    let centre = Vec3::new(21342.0, 9053.0, 465.0);
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_translation(cry_to_bevy(centre + Vec3::new(78.0, -90.0, 65.0)))
            .looking_at(cry_to_bevy(centre), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 42.0_f32.to_radians(),
            ..default()
        }),
    ));
    commands.spawn((
        Text::new("Original cave 020_008 / first compiled voxel\n2,901 vertices / 5,642 native proxy triangles\nGold: verified native floor and wall rays\nDiagnostic colours and edges; voxel surface atlas remains unsupported"),
        TextFont { font_size: FontSize::Px(22.0), ..default() },
        TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, top: px(20.0), left: px(24.0), ..default() },
    ));
}

fn follow(
    mut commands: Commands,
    time: Res<Time>,
    collision: Res<WorldCollision>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    views: Query<Entity, With<ProxyView>>,
    mut text: Query<&mut Text>,
    mut fixture: ResMut<Fixture>,
) {
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 10.0 {
        camera.single_mut().unwrap().translation =
            cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        for entity in &views {
            commands.entity(entity).despawn();
        }
        fixture.moved = true;
        text.single_mut().unwrap().0 = "Camera moved outside authored cells\nChecking collision cell and diagnostic mesh release...".into();
    }
    if time.elapsed_secs() > fixture.follow_at {
        fixture.follow_at = time.elapsed_secs() + 0.25;
        let focus = bevy_to_cry(camera.single().unwrap().translation);
        collision.follow(focus.x, focus.y);
    }
    if time.elapsed_secs() > fixture.last + 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "voxel fixture: {:?}, diagnostic_entities={}",
            collision.stats(),
            views.iter().count()
        );
    }
    if fixture.moved && !fixture.verified && time.elapsed_secs() > 13.0 {
        let stats = collision.stats();
        assert_eq!(
            (
                stats.cells,
                stats.loading,
                stats.triangles,
                stats.bytes,
                views.iter().count()
            ),
            (0, 0, 0, 0, 0)
        );
        assert!(
            collision
                .raycast_mask(
                    Vec3::new(21354.6667, 9055.3333, 455.5),
                    Vec3::NEG_Z,
                    2.0,
                    Mask::STRUCTURES
                )
                .is_none()
        );
        info!("voxel fixture: camera-follow unload passed; cells/jobs/proxy entities = 0");
        text.single_mut().unwrap().0 = "Camera-follow unload PASSED\nCollision cells: 0 / pending jobs: 0 / diagnostic entities: 0\nOriginal cave-floor structure ray is now absent".into();
        fixture.verified = true;
    }
}
