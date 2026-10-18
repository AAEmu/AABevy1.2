//! Original brush ordinary-shadow eligibility: --all-cast reproduces the previous behavior.
//! --unload-check --shot-after 24 verifies every actual brush part disappears after leaving.

use aa_world::{
    AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy,
    objects::{Kind, ObjectSector, ObjectState},
};
use bevy::{light::NotShadowCaster, prelude::*};
use std::collections::HashSet;

#[derive(Resource, Default)]
struct Fixture {
    all_cast: bool,
    unload: bool,
    moved: bool,
    done: bool,
    parts: HashSet<Entity>,
    noncasters: HashSet<Entity>,
    last: f32,
}
fn main() {
    let flag = |s: &str| std::env::args().any(|a| a == s);
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 100.0;
    config.unload_radius = 220.0;
    config.brush_distance = 150.0;
    config.vegetation_distance = 100.0;
    config.tree_distance = 150.0;
    config.water_distance = 180.0;
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Native brush shadow eligibility"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: 10.0,
            ..default()
        })
        .insert_resource(Fixture {
            all_cast: flag("--all-cast"),
            unload: flag("--unload-check"),
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, check)
        .run();
}
fn setup(mut commands: Commands) {
    // Original non-casting stone wall (asset name verdura_stonewell_c), at its bbox centre.
    let target = Vec3::new(12469.8774, 15276.4282, 154.8817);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(cry_to_bevy(target + Vec3::new(-7.0, -10.0, 4.0)))
            .looking_at(cry_to_bevy(target), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50.0_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
fn check(
    mut commands: Commands,
    time: Res<Time>,
    objects: Res<ObjectState>,
    fresh: Query<(Entity, &ChildOf, Has<NotShadowCaster>), Added<Mesh3d>>,
    parts: Query<&ChildOf, With<Mesh3d>>,
    all: Query<()>,
    sectors: Query<&ObjectSector>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut fixture: ResMut<Fixture>,
) {
    for (entity, parent, excluded) in &fresh {
        let Ok(sector) = sectors.get(parent.parent()) else {
            continue;
        };
        if sector.kind != Kind::Brush {
            continue;
        }
        fixture.parts.insert(entity);
        if excluded {
            fixture.noncasters.insert(entity);
            if fixture.all_cast {
                commands.entity(entity).remove::<NotShadowCaster>();
            }
        }
    }
    if time.elapsed_secs() - fixture.last > 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "brush shadows: seen_parts={} authored_noncasters={} all_cast={} objects={} pending={} failed={}",
            fixture.parts.len(),
            fixture.noncasters.len(),
            fixture.all_cast,
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 16.0 {
        assert!(
            !fixture.noncasters.is_empty(),
            "draw original non-casting brushes first"
        );
        assert!(
            fixture.parts.len() > fixture.noncasters.len(),
            "retain original ordinary casters"
        );
        camera.single_mut().unwrap().translation = cry_to_bevy(Vec3::splat(-100000.0));
        fixture.moved = true;
    }
    if fixture.moved && !fixture.done && time.elapsed_secs() > 21.0 {
        assert!(
            fixture.parts.iter().all(|&e| all.get(e).is_err()),
            "all recorded caster/non-caster mesh entities must despawn"
        );
        assert!(
            parts.iter().all(|p| sectors.get(p.parent()).is_err()),
            "no streamed brush/vegetation mesh parts remain"
        );
        assert_eq!(objects.stats.entities, 0);
        info!(
            "native brush shadow unload PASSED: {} recorded parts / {} authored non-casters gone",
            fixture.parts.len(),
            fixture.noncasters.len()
        );
        fixture.done = true;
    }
}
