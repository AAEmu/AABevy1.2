use super::*;
use aa_net::{
    Unit,
    model::{EquipDetail, Item},
};

fn unit() -> Unit {
    let mut unit = crate::hud::fixture_spawn().unit;
    unit.model_id = 11;
    unit.equipment.slots[aa_net::model::slot::HAIR] = Some(Item {
        template_id: 24024,
        ..Default::default()
    });
    unit.equipment.slots[aa_net::model::slot::MAINHAND] = Some(Item {
        template_id: 26060,
        id: 1234,
        count: 1,
        detail: ItemDetail::Equip(EquipDetail::default()),
        ..Default::default()
    });
    unit
}
fn key(unit: &Unit) -> Key {
    Key::new(
        unit.model_id,
        Race::Nuian,
        Gender::Female,
        &unit.equipment,
        &unit.model_params,
    )
}
fn hood(unit: &mut Unit) {
    unit.equipment.slots[aa_net::model::slot::HEAD] = Some(Item {
        template_id: 18230,
        id: 77,
        count: 1,
        detail: ItemDetail::Equip(EquipDetail::default()),
        ..Default::default()
    });
}
fn world() -> (World, Entity) {
    let mut world = World::new();
    let unit = unit();
    let actor = world
        .spawn((
            RemoteUnit::fixture(unit.obj_id, Vec3::ZERO),
            Applied {
                key: key(&unit),
                failed: None,
            },
        ))
        .id();
    let mut units = Units::default();
    units.by_id.insert(unit.obj_id, unit);
    world.insert_resource(units);
    (world, actor)
}
#[test]
fn visual_key_ignores_item_wear_but_tracks_images_dyes_and_customization() {
    let mut unit = unit();
    hood(&mut unit);
    let original = key(&unit);
    let item = unit.equipment.slots[aa_net::model::slot::HEAD]
        .as_mut()
        .unwrap();
    item.id += 17;
    item.grade = 6;
    item.count = 15;
    let ItemDetail::Equip(detail) = &mut item.detail else {
        panic!()
    };
    detail.durability = 3;
    assert_eq!(key(&unit), original);
    let ItemDetail::Equip(detail) = &mut unit.equipment.slots[aa_net::model::slot::HEAD]
        .as_mut()
        .unwrap()
        .detail
    else {
        panic!()
    };
    detail.image_item_template_id = 22874;
    assert_ne!(key(&unit), original);
    let image = key(&unit);
    let ItemDetail::Equip(detail) = &mut unit.equipment.slots[aa_net::model::slot::HEAD]
        .as_mut()
        .unwrap()
        .detail
    else {
        panic!()
    };
    detail.dye_item_id = 100;
    assert_ne!(key(&unit), image);
    let dyed = key(&unit);
    unit.model_params.skin_color = 32;
    assert_ne!(key(&unit), dyed);
}

#[test]
fn own_source_uses_complete_native_equipment_and_refuses_identity_or_history_gaps() {
    let (mut world, actor) = world();
    let (shared, _rt) = crate::hud::look_test_runtime();
    let mut spawn = crate::hud::fixture_spawn();
    spawn.unit = unit();
    spawn.unit.obj_id = spawn.obj_id;
    let stale = spawn.unit.equipment.clone();
    world.insert_resource(Player { spawn: Some(spawn) });
    world.insert_non_send(crate::hud::api::HudBridge(shared.clone()));
    world
        .entity_mut(actor)
        .remove::<RemoteUnit>()
        .insert(LocalPlayer { obj_id: 100 });
    let current = source(&world, actor).unwrap();
    assert!(stale.get(aa_net::model::slot::MAINHAND).is_some());
    assert!(
        current.equipment.slots.iter().all(Option::is_none),
        "known-empty received equipment must win over stale player and Units snapshots"
    );
    shared.borrow_mut().disconnected = true;
    assert!(source(&world, actor).is_none());
    shared.borrow_mut().disconnected = false;
    shared.borrow_mut().character_id = 48;
    assert!(
        source(&world, actor).is_none(),
        "do not reuse a prior character's native records"
    );
    shared.borrow_mut().character_id = 47;
    world
        .resource_mut::<Player>()
        .spawn
        .as_mut()
        .unwrap()
        .obj_id = 101;
    assert!(
        source(&world, actor).is_none(),
        "the current actor must still match the local entity"
    );
}
#[test]
fn old_completions_cannot_mark_failure_or_change_newer_source_or_reused_actor() {
    let (mut world, actor) = world();
    let original = world.get::<Applied>(actor).unwrap().key.clone();
    hood(world.resource_mut::<Units>().by_id.get_mut(&100).unwrap());
    let current = source(&world, actor).unwrap().key;
    apply(
        &mut world,
        Ready {
            root: actor,
            key: original.clone(),
            result: Err("old failure".into()),
        },
    );
    assert!(world.get::<Applied>(actor).unwrap().failed.is_none());
    apply(
        &mut world,
        Ready {
            root: actor,
            key: current.clone(),
            result: Err("current failure".into()),
        },
    );
    assert_eq!(
        world.get::<Applied>(actor).unwrap().failed,
        Some(current.clone())
    );
    assert_eq!(world.get::<Applied>(actor).unwrap().key, original);
    world.despawn(actor);
    let replacement = world
        .spawn((
            RemoteUnit::fixture(100, Vec3::ZERO),
            Applied {
                key: original.clone(),
                failed: None,
            },
        ))
        .id();
    apply(
        &mut world,
        Ready {
            root: actor,
            key: current,
            result: Err("retired completion".into()),
        },
    );
    assert!(world.get::<Applied>(replacement).unwrap().failed.is_none());
    assert_eq!(world.get::<Applied>(replacement).unwrap().key, original);
}
#[test]
fn actual_prepared_hood_replaces_parts_on_same_actor_and_retains_playback_and_drawn_state() {
    actual_refresh(false);
}
#[test]
fn actual_prepared_hood_keeps_parked_animator_parked() {
    actual_refresh(true);
}
fn actual_refresh(parked: bool) {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        aa_cry::CryPlugin,
        aa_anim::AnimPlugin,
        aa_anim::facial::FacialPlugin,
    ));
    app.init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
    app.add_systems(Update, weapons::attach_carried);
    let db = Db::open().unwrap();
    let unit = unit();
    let initial = Source {
        key: key(&unit),
        equipment: unit.equipment.clone(),
    };
    let mut newer = unit.clone();
    hood(&mut newer);
    let mut state =
        SystemState::<(Commands, CryAssets, Res<aa_anim::ClipLibrary>)>::new(app.world_mut());
    let actor = {
        let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
        let build = super::super::prepare::prepare_unit(&db, &cry.loader(), &clips, &unit);
        super::super::units::spawn_unit(&mut commands, &mut cry, &clips, &newer, build).unwrap()
    };
    state.apply(app.world_mut());
    app.world_mut().entity_mut(actor).insert((
        weapons::Drawn::after_action(20.0, weapons::Out::Blade { dual: false }),
        weapons::StaffSide(true),
    ));
    assert_eq!(
        app.world().get::<Applied>(actor).unwrap().key,
        initial.key,
        "spawn acknowledges the worker snapshot, not future gear"
    );
    aa_anim::facial::enable_character_face(app.world_mut(), actor).unwrap();
    let mut units = Units::default();
    units.by_id.insert(unit.obj_id, newer);
    app.insert_resource(units);
    app.update();
    app.world_mut().spawn((
        Camera3d::default(),
        Projection::default(),
        GlobalTransform::from(Transform::from_xyz(0.0, 0.0, 50.0).looking_at(Vec3::ZERO, Vec3::Y)),
    ));
    app.world_mut()
        .run_system_cached(super::super::units::cull_units)
        .unwrap();
    assert!(!app.world().get::<RemoteUnit>(actor).unwrap().casts_shadow());
    let (rig, bones, time) = {
        let a = app.world().get::<Animator>(actor).unwrap();
        (
            a.rig.clone(),
            a.bones.clone(),
            a.current.as_ref().map(|p| p.time),
        )
    };
    if parked {
        let animator = app
            .world_mut()
            .entity_mut(actor)
            .take::<Animator>()
            .unwrap();
        app.world_mut()
            .entity_mut(actor)
            .insert(Asleep::parked(animator));
    }
    let root = app.world().get::<Character>(actor).unwrap().model_root;
    let old_parts = app.world().get::<Character>(actor).unwrap().parts.clone();
    let placement = *app.world().get::<Transform>(actor).unwrap();
    let current = source(app.world(), actor).unwrap();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let loader = state.get_mut(app.world_mut()).unwrap().loader();
    state.apply(app.world_mut());
    let prepared = prepare(&db, &loader, &current).unwrap();
    apply(
        app.world_mut(),
        Ready {
            root: actor,
            key: current.key.clone(),
            result: Ok(prepared),
        },
    );
    app.world_mut()
        .run_system_cached(weapons::attach_carried)
        .unwrap();
    assert_eq!(app.world().get::<Applied>(actor).unwrap().key, current.key);
    assert!(app.world().get::<Applied>(actor).unwrap().failed.is_none());
    let character = app.world().get::<Character>(actor).unwrap();
    assert_eq!(character.model_root, root);
    assert!(
        old_parts
            .iter()
            .all(|e| app.world().get_entity(*e).is_err())
    );
    assert_ne!(character.parts, old_parts);
    let mut meshes = Vec::new();
    let mut stack = character.parts.clone();
    while let Some(entity) = stack.pop() {
        if let Some(children) = app.world().get::<Children>(entity) {
            stack.extend(children.iter());
        }
        if app.world().get::<Mesh3d>(entity).is_some() {
            meshes.push(entity);
        }
    }
    // A cached false policy need not change distance just to reach new meshes.
    if !app.world().get::<RemoteUnit>(actor).unwrap().casts_shadow() {
        assert!(!meshes.is_empty());
        assert!(
            meshes
                .iter()
                .all(|&e| app.world().get::<bevy::light::NotShadowCaster>(e).is_some())
        );
    }
    assert_eq!(*app.world().get::<Transform>(actor).unwrap(), placement);
    assert_eq!(app.world().get::<Animator>(actor).is_none(), parked);
    let animator = app
        .world()
        .get::<Animator>(actor)
        .or_else(|| app.world().get::<Asleep>(actor).map(Asleep::animator))
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&animator.rig, &rig));
    assert_eq!(animator.bones, bones);
    assert_eq!(animator.current.as_ref().map(|p| p.time), time);
    assert!(app.world().get::<weapons::Drawn>(actor).is_some());
    assert!(
        app.world().get::<weapons::StaffSide>(actor).is_none(),
        "a replacement without a staff cannot inherit its old hip/bow state"
    );
    assert!(
        app.world()
            .get::<aa_anim::facial::FacialAnimator>(actor)
            .is_some()
    );
}
