//! Destination ownership must survive the frames before its world.xml worker completes.

use super::*;

fn app(world: World) -> App {
    let mut app = App::new();
    app.insert_resource(WorldConfig::new(&world.name))
        .insert_resource(WorldInfo(Arc::new(world.clone())))
        .init_resource::<WorldClock>()
        .insert_resource(AtmosphereTuning::default())
        .insert_resource(Environment {
            sample: None,
            zone: None,
            hour: 0.0,
            ocean_fog: Vec3::ZERO,
            ocean_density: 0.0,
        })
        .insert_resource(TodCache {
            world: Some(world.name),
            ..default()
        })
        .init_resource::<Time>()
        .init_resource::<SkySelection>()
        .init_resource::<crate::area_tod::AreaToDState>()
        .init_resource::<GlobalAmbientLight>()
        .init_resource::<ClearColor>()
        .add_systems(Update, (receive_zones, update_environment).chain());
    app.world_mut().spawn((
        Camera3d::default(),
        WorldFocus,
        GlobalTransform::from_translation(cry_to_bevy(Vec3::new(15712.0, 15808.0, 150.0))),
    ));
    app
}

#[test]
fn switched_original_sector_map_survives_delayed_delivery_and_drives_environment() {
    if !crate::pak_available() {
        return;
    }
    let mut app = app(World::load("login2").unwrap());
    assert!(crate::switch_world(app.world_mut(), "main_world"));
    // Replace only the worker transport, after the real switch/load entry point.
    // Keep delivery pending across two actual environment updates, independent of OS scheduling.
    let (tx, rx) = channel();
    app.world_mut().resource_mut::<TodCache>().sector_rx = Some(Mutex::new(rx));
    for _ in 0..2 {
        app.update();
        assert!(
            app.world()
                .resource::<WorldInfo>()
                .0
                .sector_zones
                .is_empty()
        );
        assert!(
            app.world().resource::<TodCache>().sector_rx.is_some(),
            "refreshing the destination environment dropped its pending sector map"
        );
    }
    let mut original = World::load("main_world").unwrap();
    original.load_sector_zones().unwrap();
    let border = Vec3::new(15712.0, 15808.0, 150.0);
    assert_eq!(original.sector_zone_at(border), Some(210));
    assert_ne!(
        app.world().resource::<Environment>().zone,
        Some(210),
        "this original border must distinguish cell fallback from sector ownership"
    );
    tx.send(original).unwrap();
    app.update();
    let info = app.world().resource::<WorldInfo>();
    assert_eq!(info.0.sector_zone_at(border), Some(210));
    assert_eq!(app.world().resource::<Environment>().zone, Some(210));
    let weights = info.0.environment_weights(border);
    assert_eq!(weights, [(179, 1.0), (210, 1.0)]);
    let cache = app.world().resource::<TodCache>();
    assert!(cache.sector_rx.is_none());
    assert_eq!(cache.zones.len(), 2);
    for zone in [179, 210] {
        assert!(cache.zones.contains_key(&(zone, 0)));
    }
    let inputs: Vec<_> = weights
        .iter()
        .map(|(zone, weight)| (load_tod("main_world", Some(*zone), 0).unwrap(), *weight))
        .collect();
    let refs: Vec<_> = inputs
        .iter()
        .map(|(tod, weight)| (tod.as_ref(), *weight))
        .collect();
    let expected = TimeOfDay::sample_blended(&refs, 10.0, Lighting::default());
    assert_eq!(
        app.world()
            .resource::<Environment>()
            .sample
            .as_ref()
            .unwrap()
            .sun,
        expected.sun
    );
}

#[test]
fn original_instance_switch_resets_old_curves_before_starting_ownership_load() {
    if !crate::pak_available() {
        return;
    }
    let mut app = app(World::load("main_world").unwrap());
    let old = Arc::new(TimeOfDay::default());
    {
        let mut cache = app.world_mut().resource_mut::<TodCache>();
        cache.zones.insert((179, 0), Some(old.clone()));
        cache.world_default.insert(0, Some(old));
    }
    assert!(crate::switch_world(
        app.world_mut(),
        "instance_training_camp"
    ));
    let cache = app.world().resource::<TodCache>();
    assert_eq!(cache.world.as_deref(), Some("instance_training_camp"));
    assert!(cache.zones.is_empty() && cache.world_default.is_empty());
    assert!(cache.sector_rx.is_some());
    assert_eq!(
        cache.lighting.unwrap().dawn_hour,
        Lighting::parse(
            &aa_pak::Pak::global()
                .read_string("worlds/instance_training_camp/lighting.xml")
                .unwrap()
        )
        .unwrap()
        .dawn_hour
    );
    // A late result from the old world cannot replace this instance.
    let (tx, rx) = channel();
    tx.send(World::load("main_world").unwrap()).unwrap();
    app.world_mut().resource_mut::<TodCache>().sector_rx = Some(Mutex::new(rx));
    app.update();
    assert_eq!(
        app.world().resource::<WorldInfo>().0.name,
        "instance_training_camp"
    );
    assert!(app.world().resource::<TodCache>().sector_rx.is_none());
    // Receive the authored instance ownership next. Sector(14,5) is present;
    // (0,0) is unowned even though world.g assigns both cells to zone198.
    let mut original = World::load("instance_training_camp").unwrap();
    original.load_sector_zones().unwrap();
    let at = Vec3::new(928.0, 352.0, 150.0);
    assert_eq!(original.sector_zone_at(at), Some(198));
    assert_eq!(original.sector_zone_at(Vec3::new(32.0, 32.0, 150.0)), None);
    let (tx, rx) = channel();
    tx.send(original).unwrap();
    app.world_mut().resource_mut::<TodCache>().sector_rx = Some(Mutex::new(rx));
    let mut cameras = app
        .world_mut()
        .query_filtered::<&mut GlobalTransform, With<WorldFocus>>();
    *cameras.single_mut(app.world_mut()).unwrap() =
        GlobalTransform::from_translation(cry_to_bevy(at));
    app.update();
    assert_eq!(
        app.world().resource::<WorldInfo>().0.sector_zone_at(at),
        Some(198)
    );
    assert_eq!(app.world().resource::<Environment>().zone, Some(198));
    assert_eq!(app.world().resource::<TodCache>().zones.len(), 1);
    assert!(
        app.world()
            .resource::<TodCache>()
            .zones
            .contains_key(&(198, 0))
    );
}

#[test]
fn disconnected_sector_worker_releases_its_receiver_without_replacing_world() {
    let mut app = app(World {
        name: "sector_fixture".into(),
        ..default()
    });
    let (tx, rx) = channel();
    drop(tx);
    app.world_mut().resource_mut::<TodCache>().sector_rx = Some(Mutex::new(rx));
    // Receive only: a failed worker has no environment/asset I/O to perform.
    app.world_mut().run_system_cached(receive_zones).unwrap();
    assert!(app.world().resource::<TodCache>().sector_rx.is_none());
    assert_eq!(app.world().resource::<WorldInfo>().0.name, "sector_fixture");
}
