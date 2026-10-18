//! Initial sleeping RigidBodyEx CGF slots in their original streamed cells.
//! Default lamp head (9,12), --machine (21,8); --off hides only these slots.
//! --unload-check moves out of the world and verifies the actual part entities retire.
//! Dynamic simulation, wake, damage and flow events remain unsupported.

use aa_world::objects::{ObjectState, StaticEntityPart};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;
use std::collections::HashSet;

#[derive(Resource)]
struct Fixture {
    cell: (i32, i32),
    guid: u64,
    off: bool,
    unload: bool,
    captured: HashSet<Entity>,
    checked: bool,
    moved: bool,
    done: bool,
}

fn main() {
    let flag = |arg: &str| std::env::args().any(|a| a == arg);
    let machine = flag("--machine");
    let (cell, guid, offset, look_offset) = if machine {
        ((21, 8), 0x4f7b306e75bcc637, Vec3::new(6.0, -9.0, 5.0), Vec3::new(0.0, 0.0, 1.0))
    } else {
        ((9, 12), 0x4dca223e56291138, Vec3::new(-3.0, -5.0, 1.0), Vec3::ZERO)
    };
    let data = aa_world::placement::load_cell_objects("main_world", cell.0, cell.1);
    let prop = data.entity_props.iter().find(|p| p.guid == guid && p.sleeping_rigid)
        .expect("original authored sleeping rigid fixture");
    let origin = aa_world::coords::cell_origin(cell.0, cell.1);
    let target = origin + prop.translation();
    info!("sleeping rigid fixture cell={cell:?} guid={guid:x} model={} matrix={:?} target={target:?}", prop.model, prop.matrix);
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 150.0;
    config.unload_radius = 300.0;
    config.brush_distance = 180.0;
    config.water_distance = 180.0;
    config.tree_distance = 180.0;
    config.vegetation = false;
    App::new()
        .add_plugins((aa_dev::default_plugins("Authored sleeping rigid props"), aa_dev::DevPlugin))
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock { hour: 10.0, ..default() })
        .insert_resource(Fixture {
            cell, guid, off: flag("--off"), unload: flag("--unload-check"), captured: HashSet::new(),
            checked: false, moved: false, done: false,
        })
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((Camera3d::default(),
                Transform::from_translation(cry_to_bevy(target + offset))
                    .looking_at(cry_to_bevy(target + look_offset), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: 50.0_f32.to_radians(), near: 0.1, far: 20000.0, ..default()
                })));
        })
        .add_systems(Update, check)
        .run();
}

fn check(
    time: Res<Time>,
    mut fixture: ResMut<Fixture>,
    objects: Res<ObjectState>,
    mut parts: Query<(Entity, &StaticEntityPart, &mut Visibility)>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    alive: Query<()>,
    mut last: Local<f32>,
) {
    for (entity, part, mut visibility) in &mut parts {
        if part.cell == fixture.cell && part.guid == fixture.guid {
            fixture.captured.insert(entity);
            if fixture.off { *visibility = Visibility::Hidden; }
        }
    }
    if time.elapsed_secs() - *last >= 2.0 {
        *last = time.elapsed_secs();
        info!("sleeping rigid drawn/seen={} total_entity_parts={} pending_models={} off={}",
            fixture.captured.len(), objects.stats.entity_props, objects.stats.models_pending, fixture.off);
    }
    if !fixture.checked && time.elapsed_secs() > 10.0 {
        assert!(!fixture.captured.is_empty(), "original sleeping rigid must enter streamed render bins");
        info!("sleeping rigid source/lifetime capture PASSED: {} actual part entities", fixture.captured.len());
        fixture.checked = true;
    }
    if !fixture.unload || fixture.done { return; }
    if !fixture.moved && time.elapsed_secs() > 12.0 {
        assert!(fixture.checked);
        camera.single_mut().unwrap().translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        fixture.moved = true;
    } else if fixture.moved && time.elapsed_secs() > 18.0 {
        for entity in &fixture.captured {
            assert!(alive.get(*entity).is_err(), "actual sleeping rigid part must despawn after cell unload");
        }
        assert_eq!(objects.stats.entity_props, 0);
        info!("sleeping rigid unload PASSED: {} actual part entities released", fixture.captured.len());
        fixture.done = true;
    }
}
