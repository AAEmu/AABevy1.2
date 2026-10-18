//! Original library Light_587 and stained-glass cookie on a neutral receiver.
//! --off / --red-only / --green-only / --blue-only / --budget-two are controls.
//! --unload-check removes the cell roots, then verifies child lights and cookies release.
//! --world visits the authored library location with the normal cell streamer.
//! --pos/--look accept global Cry xyz; --collision-check verifies the original room's floor.

use aa_world::objects::lights::{
    self, LocalLight, LocalLightPlugin, LocalLightSettings, LocalLightStats,
};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

#[derive(Resource)]
struct Fixture {
    roots: Vec<Entity>,
    world: bool,
    unload: bool,
    phase: u8,
}

fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}

fn vector_arg(name: &str) -> Option<Vec3> {
    let args: Vec<_> = std::env::args().collect();
    let value = args.get(args.iter().position(|arg| arg == name)? + 1)?;
    let v: Vec<f32> = value
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (v.len() == 3 && v.iter().all(|v| v.is_finite()))
        .then(|| cry_to_bevy(Vec3::new(v[0], v[1], v[2])))
}

fn main() {
    let xml = aa_pak::Pak::global()
        .read("game/worlds/instance_library/cells/000_004/client/entities.xml")
        .unwrap();
    let xml = std::str::from_utf8(&xml).unwrap();
    let mut light = aa_world::entities::parse_lights(xml)
        .into_iter()
        .find(|p| p.name == "Light_587")
        .unwrap();
    for (index, name) in ["--red-only", "--green-only", "--blue-only"]
        .into_iter()
        .enumerate()
    {
        if flag(name) {
            let value = light.diffuse[index];
            light.diffuse = Vec3::ZERO;
            light.diffuse[index] = value;
        }
    }
    let mut config = WorldConfig::new("instance_library");
    config.load_radius = 110.0;
    config.unload_radius = 180.0;
    config.brush_distance = 110.0;
    config.vegetation = false;
    config.shadows = false;
    let origin = cry_to_bevy(aa_world::coords::cell_origin(0, 4) + light.matrix.w_axis.truncate());
    let forward = cry_to_bevy(light.matrix.x_axis.truncate().normalize());
    let up = cry_to_bevy(light.matrix.y_axis.truncate().normalize());
    let receiver = origin + forward * (light.radius * 0.5);
    let size = light.radius * (light.projector_fov.to_radians() * 0.5).tan();
    let world = flag("--world");
    if flag("--collision-check") {
        let terrain = aa_world::TerrainQuery::new("instance_library");
        let collision = aa_world::WorldCollision::new("instance_library", terrain);
        let cell = collision.build_blocking(0, 4).unwrap();
        println!("library room collision: {:?}", cell.stats);
        assert_eq!(cell.stats.brushes, 30287);
        let from = Vec3::new(872.0, 4200.0, 258.0);
        let hit = collision.raycast(from, Vec3::NEG_Z, 4.0).unwrap();
        println!("library floor receiver: {hit:?}");
        assert!(!hit.terrain && (hit.point.z - 256.55).abs() < 0.1);
        drop(cell);
        collision.release_outside(-100000.0, -100000.0, 0.0);
        assert_eq!(collision.stats().cells, 0);
        return;
    }
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins("aa-world native RGB projector"))
        .add_plugins(aa_dev::DevPlugin);
    if world {
        app.add_plugins(AaWorldPlugin { config })
            .insert_resource(WorldClock {
                hour: 0.0,
                ..default()
            });
    } else {
        app.insert_resource(config).add_plugins(LocalLightPlugin);
    }
    app.insert_resource(LocalLightSettings {
        enabled: !flag("--off"),
        max_lights: if flag("--budget-two") { 2 } else { 64 },
        ..default()
    })
    .insert_resource(GlobalAmbientLight {
        brightness: 0.0,
        ..default()
    })
    .insert_resource(ClearColor(Color::BLACK))
    .add_systems(
        Startup,
        move |mut commands: Commands,
              mut meshes: ResMut<Assets<Mesh>>,
              mut materials: ResMut<Assets<StandardMaterial>>| {
            let roots = if world {
                Vec::new()
            } else {
                lights::spawn_cell(&mut commands, (0, 4), &[light.clone()])
            };
            commands.insert_resource(Fixture {
                roots,
                world,
                unload: flag("--unload-check"),
                phase: 0,
            });
            if !world {
                commands.spawn((
                    Mesh3d(meshes.add(Rectangle::new(size * 1.25, size * 1.25))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::WHITE,
                        perceptual_roughness: 1.0,
                        reflectance: 0.0,
                        ..default()
                    })),
                    Transform::from_translation(receiver).looking_to(forward, up),
                ));
            }
            let camera_position = if world {
                // Clear view of Light_587's original floor (its native proxy top is 256.55066).
                cry_to_bevy(aa_world::coords::cell_origin(0, 4) + Vec3::new(872.0, 115.0, 262.0))
            } else {
                receiver - forward * (size * 1.5)
            };
            let target = if world {
                cry_to_bevy(
                    aa_world::coords::cell_origin(0, 4) + Vec3::new(872.0, 104.0, 256.55066),
                )
            } else {
                receiver
            };
            commands.spawn((
                Camera3d::default(),
                Exposure { ev100: 12.0 },
                Tonemapping::None,
                DebandDither::Disabled,
                Transform::from_translation(vector_arg("--pos").unwrap_or(camera_position))
                    .looking_at(vector_arg("--look").unwrap_or(target), up),
                Projection::Perspective(PerspectiveProjection {
                    fov: 50.0_f32.to_radians(),
                    near: 0.1,
                    far: 2000.0,
                    ..default()
                }),
            ));
        },
    )
    .add_systems(Update, verify)
    .run();
}

fn verify(
    mut commands: Commands,
    time: Res<Time>,
    stats: Res<LocalLightStats>,
    objects: Option<Res<aa_world::objects::ObjectState>>,
    collision: Option<Res<aa_world::WorldCollision>>,
    slots: Query<&LocalLight>,
    spots: Query<&SpotLight>,
    sectors: Query<&aa_world::objects::ObjectSector>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mut fixture: ResMut<Fixture>,
    mut last: Local<f32>,
) {
    if time.elapsed_secs() > *last + 2.0 {
        *last = time.elapsed_secs();
        info!(
            "projectors: authored={} components={} projected_sources={} shadows={} unsupported={} cookies={} pending={} spots={}",
            stats.authored,
            stats.rendered,
            stats.projected_sources,
            stats.shadowed,
            stats.unsupported,
            stats.cookie_assets,
            stats.pending_cookies,
            spots.iter().count()
        );
        if let Some(objects) = &objects {
            info!("library geometry: {:?}", objects.stats);
        }
        assert!(stats.rendered <= if flag("--budget-two") { 2 } else { 64 });
        assert!(stats.shadowed <= 2 && stats.cookie_assets <= 8 && stats.pending_cookies <= 2);
        assert!(spots.iter().all(|spot| !spot.shadow_maps_enabled));
    }
    if fixture.unload
        && fixture.phase == 0
        && time.elapsed_secs() > if fixture.world { 12.0 } else { 8.0 }
    {
        if fixture.world {
            assert!(stats.projected_sources > 0 && spots.iter().count() >= 3);
            cameras.single_mut().unwrap().translation =
                cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        } else {
            assert_eq!(stats.projected_sources, 1);
            assert_eq!(spots.iter().count(), 3);
            for root in fixture.roots.drain(..) {
                commands.entity(root).despawn();
            }
        }
        fixture.phase = 1;
    } else if fixture.unload
        && fixture.phase == 1
        && time.elapsed_secs() > if fixture.world { 16.0 } else { 10.0 }
    {
        assert!(slots.is_empty() && spots.is_empty());
        assert!(sectors.is_empty());
        if let Some(collision) = &collision {
            assert_eq!(collision.stats().cells, 0);
            assert_eq!(collision.stats().loading, 0);
        }
        assert_eq!(stats.rendered, 0);
        assert_eq!(stats.cookie_assets, 0);
        assert_eq!(stats.pending_cookies, 0);
        info!("projector unload: roots, all RGB children and cookie handles released");
        fixture.phase = 2;
    }
}
