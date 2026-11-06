use super::*;
use aa_world::cry_to_bevy;

const DEG: f32 = 0.017453292;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn same(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() > 1.0 - 1e-6
}

#[test]
fn full_helm_turns_the_rudder_one_radian_about_its_own_up_axis() {
    for steering in [1.0f32, -1.0, 0.5] {
        let rotation = rudder_rotation(Quat::IDENTITY, steering);
        let (axis, angle) = rotation.to_axis_angle();
        assert!(
            close(angle, steering.abs()),
            "steering {steering}: {angle} rad"
        );
        // cry Z (up) is Bevy +Y
        assert!(close(axis.y.abs(), 1.0), "{axis:?}");
    }
    // a right (D, +127) helm swings the rudder's aft edge (cry -Y) to starboard (cry +X)
    let aft = cry_to_bevy(Vec3::new(0.0, -1.0, 0.0));
    assert!(rudder_rotation(Quat::IDENTITY, 1.0).mul_vec3(aft).x > 0.5);
    assert!(rudder_rotation(Quat::IDENTITY, -1.0).mul_vec3(aft).x < -0.5);
    // the turn is applied in the rudder's own frame, after its authored rotation
    let base = Quat::from_rotation_x(0.3);
    let turned = rudder_rotation(base, 0.4);
    assert!(
        same(turned, base * Quat::from_rotation_y(0.4)),
        "{turned:?}"
    );
    assert!(same(rudder_rotation(base, 0.0), base));
}

#[test]
fn the_wheel_target_rounds_up_to_45_degree_steps() {
    // x = (int)(deg * 7): 0 stays, 1..45 is one step, the truncating division stops full left at -315
    assert_eq!(wheel_target(0.0, WHEEL_OPTION), 0.0);
    assert!(close(wheel_target(0.002, WHEEL_OPTION), 0.0), "x = 0");
    assert!(
        close(wheel_target(0.003, WHEEL_OPTION), 45.0 * DEG),
        "x = 1"
    );
    assert!(close(wheel_target(0.1, WHEEL_OPTION), 45.0 * DEG), "x = 40");
    assert!(close(wheel_target(-0.1, WHEEL_OPTION), 0.0), "x = -40");
    assert!(
        close(wheel_target(-0.2, WHEEL_OPTION), -45.0 * DEG),
        "x = -80"
    );
    assert!(
        close(wheel_target(1.0, WHEEL_OPTION), 405.0 * DEG),
        "full right, x = 401"
    );
    assert!(
        close(wheel_target(-1.0, WHEEL_OPTION), -315.0 * DEG),
        "full left, x = -401"
    );
}

#[test]
fn the_wheel_turns_at_half_the_option_per_second_and_stops_on_the_target() {
    assert!(close(wheel_step(0.0, 2.0, WHEEL_OPTION, 0.1), 0.35));
    assert!(close(wheel_step(0.0, -2.0, WHEEL_OPTION, 0.1), -0.35));
    assert_eq!(wheel_step(1.9, 2.0, WHEEL_OPTION, 0.1), 2.0);
    assert_eq!(wheel_step(-1.9, -2.0, WHEEL_OPTION, 0.1), -2.0);
    assert_eq!(wheel_step(2.0, 2.0, WHEEL_OPTION, 0.1), 2.0);
}

#[test]
fn the_steering_state_follows_the_drawn_wheel() {
    let mut helm = ShipHelm::new("prefab://Prefabs/galleon.xml/galleon.step_10galleon_normal");
    helm.steering = 1.0;
    helm.step_wheel(0.1, WHEEL_OPTION);
    assert_eq!(helm.state, SteeringState::Growing);
    for _ in 0..100 {
        helm.step_wheel(0.1, WHEEL_OPTION);
    }
    assert!(close(helm.wheel, 405.0 * DEG));
    assert_eq!(
        helm.state,
        SteeringState::Still,
        "on the target the wheel stands"
    );
    helm.steering = -1.0;
    helm.step_wheel(0.1, WHEEL_OPTION);
    assert_eq!(helm.state, SteeringState::Shrinking);
}

#[test]
fn the_rowboat_the_solo_ship_and_the_duck_boat_have_no_wheel() {
    for normal in [
        "prefab://prefabs/smallboat.xml/smallboat.smallboat_a",
        "prefab://prefabs/soloship.xml/soloship.soloship_a_body",
        "prefab://prefabs/coupleduckship.xml/coupleduckship.coupleduckship_body",
    ] {
        assert!(!wheel_turns(normal), "{normal}");
    }
    assert!(wheel_turns(
        "prefab://Prefabs/galleon.xml/galleon.step_10galleon_normal"
    ));
    assert!(wheel_turns(
        "prefab://prefabs/boxship.xml/boxship.boxship_normal"
    ));
}

#[test]
fn the_throttle_sign_picks_the_clip_and_its_size_the_speed() {
    assert_eq!(throttle_clip(0.5), (ThrottleClip::RunF, 0.5));
    assert_eq!(throttle_clip(-0.25), (ThrottleClip::RunB, 0.25));
    assert_eq!(throttle_clip(0.0), (ThrottleClip::Idle, 1.0));
}

#[test]
fn the_rudder_part_follows_its_ships_steering() {
    let mut app = App::new();
    app.add_systems(Update, turn_rudders);
    let base = Quat::from_rotation_y(0.2);
    let root = app
        .world_mut()
        .spawn(ShipHelm {
            steering: 1.0,
            ..default()
        })
        .id();
    let rudder = app
        .world_mut()
        .spawn((
            Rudder { base },
            Transform::from_rotation(base),
            ChildOf(root),
        ))
        .id();
    app.update();
    let rotation = app.world().get::<Transform>(rudder).unwrap().rotation;
    assert!(same(rotation, rudder_rotation(base, 1.0)), "{rotation:?}");
    app.world_mut().get_mut::<ShipHelm>(root).unwrap().steering = 0.0;
    app.update();
    assert!(same(
        app.world().get::<Transform>(rudder).unwrap().rotation,
        base
    ));
}

fn bound_doodad(obj_id: u32, parent_obj_id: u32, attach_point: u8) -> aa_net::model::Doodad {
    aa_net::model::Doodad {
        obj_id,
        template_id: 2374,
        owner_obj_id: 0,
        parent_obj_id,
        attach_point,
        pos: aa_net::wire::Pos::new(0.0, -14.0, 6.9),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 0,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    }
}

#[test]
fn only_the_doodad_at_the_helms_point_of_a_ship_is_its_wheel() {
    let mut app = App::new();
    app.add_systems(Update, mark_helm_wheels);
    let mut doodads = Doodads::default();
    doodads.by_id.insert(1, bound_doodad(1, 9, HELMS_POINT));
    // a repair doodad at point 36 of the same ship
    doodads.by_id.insert(2, bound_doodad(2, 9, 36));
    app.insert_resource(doodads);
    let ship = app.world_mut().spawn(ShipHelm::default()).id();
    let cart = app.world_mut().spawn_empty().id();
    let base = Transform::from_xyz(0.0, 0.5, 0.0);
    let mut drawn = |obj_id, parent| {
        let model = app.world_mut().spawn(base).id();
        let visual = DoodadVisual {
            path: String::new(),
            model: Some(model),
            func_group_id: 0,
            rendered_func_group_id: 0,
            bond_helpers: default(),
            ladder: None,
            growth: None,
            remaining_ms: 0.0,
            total_ms: 0.0,
            base_scale: 1.0,
        };
        app.world_mut().spawn((RemoteDoodad { obj_id }, visual, ChildOf(parent)));
        model
    };
    let (wheel, repair, on_cart) = (drawn(1, ship), drawn(2, ship), drawn(1, cart));
    app.update();
    let marked = app.world().get::<HelmWheel>(wheel).expect("the $helms doodad's model");
    assert_eq!((marked.ship, marked.base), (ship, base));
    assert!(app.world().get::<HelmWheel>(repair).is_none());
    assert!(app.world().get::<HelmWheel>(on_cart).is_none(), "not under a ship root");
}

fn wheel_app(normal: &str, steering: f32) -> (App, Entity, [Entity; 2]) {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .add_systems(Update, turn_helm_wheels);
    let mut helm = ShipHelm::new(normal);
    helm.steering = steering;
    let ship = app.world_mut().spawn(helm).id();
    let base = Transform::from_xyz(0.0, 1.0, 0.0);
    let wheels = [0, 1].map(|_| app.world_mut().spawn((HelmWheel { ship, base }, base)).id());
    // the first update has no elapsed time yet
    app.update();
    (app, ship, wheels)
}

#[test]
fn the_wheel_turns_about_its_own_axle_once_a_frame() {
    let (mut app, ship, wheels) = wheel_app(
        "prefab://Prefabs/galleon.xml/galleon.step_10galleon_normal",
        1.0,
    );
    app.update();
    let helm = *app.world().get::<ShipHelm>(ship).unwrap();
    // two marked wheels still step the ship once: 0.1 s at 3.5 rad/s
    assert!(close(helm.wheel, 0.35), "{helm:?}");
    assert_eq!(helm.state, SteeringState::Growing);
    for wheel in wheels {
        let transform = *app.world().get::<Transform>(wheel).unwrap();
        assert_eq!(
            transform,
            wheel_transform(Transform::from_xyz(0.0, 1.0, 0.0), 0.35)
        );
        // the axle is cry Y, Bevy -Z
        let (axis, angle) = transform.rotation.to_axis_angle();
        assert!(
            close(angle, 0.35) && close(axis.z.abs(), 1.0),
            "{axis:?} {angle}"
        );
    }
}

#[test]
fn a_ship_without_a_wheel_keeps_its_helm_doodad_still() {
    let (mut app, ship, wheels) =
        wheel_app("prefab://prefabs/smallboat.xml/smallboat.smallboat_a", 1.0);
    app.update();
    let helm = *app.world().get::<ShipHelm>(ship).unwrap();
    assert_eq!((helm.wheel, helm.state), (0.0, SteeringState::Still));
    assert_eq!(
        *app.world().get::<Transform>(wheels[0]).unwrap(),
        Transform::from_xyz(0.0, 1.0, 0.0)
    );
}

#[test]
#[ignore = "requires the game database"]
fn the_ship_models_without_a_wheel_are_the_three_the_decompile_names() {
    let Some(db) = Db::open() else { return };
    let turns =
        |model| ShipHelm::new(&db.vehicle_look(model).expect("a ship look").normal).wheel_turns;
    // 129 rowboat (smallboat), 1288 soloship, 1287 duck boat
    for model in [129, 1288, 1287] {
        assert!(!turns(model), "model {model}");
    }
    // the Eznan Cutter (galleon), the merchant ship, the fishing boat
    for model in [393, 1205, 1360] {
        assert!(turns(model), "model {model}");
    }
}

#[test]
fn the_duck_boats_feet_have_the_throttle_clips() {
    if !aa_cry::pak_available() {
        return;
    }
    let clips = ClipLibrary::default();
    let feet = ThrottleClips::bind(
        "objects/env/06_unit/01_ship/coupleduckship/coupleduckship_foot.chr",
        &clips,
    )
    .expect("coupleduckship_foot.cal has idle, run_f and run_b");
    assert!(feet.idle.is_some() && feet.run_f.is_some() && feet.run_b.is_some());
    assert_eq!(feet.last, -1.0);
    // the galleon's flag only has its sail states
    assert!(
        ThrottleClips::bind(
            "objects/env/06_unit/01_ship/galleon/galleon_flag0.chr",
            &clips
        )
        .is_none()
    );
}

#[test]
fn a_throttle_change_plays_run_f_run_b_or_idle_at_the_throttle_speed() {
    if !aa_cry::pak_available() {
        return;
    }
    let model = "objects/env/06_unit/01_ship/coupleduckship/coupleduckship_foot.chr";
    let clips = ClipLibrary::default();
    let feet = ThrottleClips::bind(model, &clips).expect("the duck feet clips");
    let source = |clip: &Option<BoundClip>| clip.as_ref().and_then(|c| c.source.clone());
    let (idle, run_f, run_b) = (source(&feet.idle), source(&feet.run_f), source(&feet.run_b));
    let rig = aa_anim::pose::shared_rig(model).expect("the duck feet rig");
    let mut app = App::new();
    app.add_systems(Update, play_throttle_anims);
    let ship = app.world_mut().spawn(ShipHelm::default()).id();
    let part = app
        .world_mut()
        .spawn((feet, Animator::new(rig, Vec::new()), ChildOf(ship)))
        .id();
    let mut drive = |throttle: f32| {
        app.world_mut().get_mut::<ShipHelm>(ship).unwrap().throttle = throttle;
        app.update();
        let animator = app.world().get::<Animator>(part).unwrap();
        (
            animator
                .current
                .as_ref()
                .and_then(|p| p.clip.source.clone()),
            animator.speed,
        )
    };
    // the first frame compares with -1: still water plays idle at speed 1
    assert_eq!(drive(0.0), (idle.clone(), 1.0));
    assert_eq!(drive(0.5), (run_f.clone(), 0.5));
    drop(drive);
    app.world_mut()
        .get_mut::<Animator>(part)
        .unwrap()
        .current
        .as_mut()
        .unwrap()
        .time = 0.3;
    let mut drive = |throttle: f32| {
        app.world_mut().get_mut::<ShipHelm>(ship).unwrap().throttle = throttle;
        app.update();
        let animator = app.world().get::<Animator>(part).unwrap();
        let current = animator.current.as_ref().unwrap();
        (current.clip.source.clone(), animator.speed, current.time)
    };
    assert_eq!(
        drive(0.75),
        (run_f, 0.75, 0.3),
        "the same way only retimes the playing clip"
    );
    assert_eq!(drive(-0.25), (run_b, 0.25, 0.0));
    assert_eq!(drive(0.0), (idle, 1.0, 0.0));
}
