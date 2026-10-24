use super::*;

#[test]
fn unused_hud_surfaces_do_not_exhaust_actor_scene_permits() {
    let mut world = World::new();
    let mut spawn = crate::hud::fixture_spawn();
    spawn.unit.model_id = 11;
    let bridge = crate::hud::api::HudBridge::default();
    bridge.0.borrow_mut().character_id = spawn.character().id;
    crate::hud::enchant::dispatch(
        None,
        &bridge.0,
        &aa_net::Event::PlayerSpawn(Box::new(spawn.clone())),
        None,
    );
    world.insert_non_send(bridge);
    world.insert_resource(Player { spawn: Some(spawn) });
    world.insert_resource(Host::default());
    let ui = UiRuntime::new("en_us", 1000., 700.).unwrap();
    ui.exec(
        "@crowded_modelviews",
        r#"
        for i=1,20 do UIParent:CreateWidget('modelview','unused'..i,'UIParent') end
        local wanted=UIParent:CreateWidget('modelview','wanted','UIParent')
        wanted:Init('player',true)
    "#,
    )
    .unwrap();
    world.insert_non_send(ui);
    drive(&mut world);
    let host = world.resource::<Host>();
    assert_eq!(host.views.len(), 21);
    assert_eq!(host.views.values().filter(|v| v.layer.is_some()).count(), 1);
    assert_eq!(
        host.views
            .values()
            .filter(|v| v.create_model && v.source.is_some())
            .count(),
        1
    );
}

#[test]
fn widget_camera_cannot_change_world_ui_hdr_or_msaa_selection() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_systems(Update, crate::hud::cameras::match_world_camera);
    app.world_mut()
        .spawn((Camera3d::default(), ModelViewCamera, Msaa::Off));
    app.world_mut()
        .spawn((Camera3d::default(), bevy::camera::Hdr, Msaa::Sample4));
    let ui = app
        .world_mut()
        .spawn((Camera2d, aa_ui::render::UiCamera, Msaa::Off))
        .id();
    app.update();
    assert!(app.world().get::<bevy::camera::Hdr>(ui).is_some());
    assert_eq!(app.world().get::<Msaa>(ui), Some(&Msaa::Sample4));
}

fn source() -> Source {
    Source {
        character: 47,
        object: 100,
        race: Race::Nuian,
        gender: Gender::Female,
        params: ModelParams::default_face(),
        equipment: Equipment::default(),
    }
}

#[test]
fn teleport_suspension_preserves_snapshots_revisions_and_pending_completion_fences() {
    let mut world = World::new();
    world.init_resource::<crate::hud::HudBuilt>();
    world.resource_mut::<crate::hud::HudBuilt>().up = true;
    world.init_resource::<crate::hud::HudSuspended>();
    world.init_resource::<aa_ui::render::UiRenderState>();
    let spawn = crate::hud::fixture_spawn();
    let mut source = source();
    source.character = spawn.character().id;
    source.object = spawn.obj_id;
    world.insert_resource(Player { spawn: Some(spawn) });
    let mut view = View::new(Some(FIRST_LAYER));
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::Init {
            unit: "player".into(),
            create_model: true,
        },
        Some(&source),
        None,
    );
    let revision = view.revision;
    view.settled = revision;
    let root = world.spawn(Transform::default()).id();
    let camera = world.spawn(Camera::default()).id();
    view.actor = Some(root);
    view.camera = Some(camera);
    let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
    ui.exec(
        "@parked_preview",
        "parked=UIParent:CreateWidget('modelview','parked','UIParent')",
    )
    .unwrap();
    let id = ui.take_modelview_work().surfaces[0].view;
    world.insert_non_send(ui);
    let mut host = Host::default();
    host.views.insert(id, view);
    host.pending.insert((id, revision));
    world.insert_resource(host);
    assert!(crate::hud::suspend(&mut world));
    // The loading runtime has different IDs and no native Init replay for the HUD.
    world.insert_non_send(UiRuntime::new("en_us", 800., 600.).unwrap());
    drive(&mut world);
    assert_eq!(world.resource::<Host>().views[&id].revision, revision);
    assert_eq!(world.resource::<Host>().views[&id].actor, Some(root));
    assert!(!world.get::<Camera>(camera).unwrap().is_active);
    assert!(crate::hud::resume(&mut world));
    world
        .get_non_send::<UiRuntime>()
        .unwrap()
        .exec("@restored_preview", "parked:ResetEquips()")
        .unwrap();
    world
        .resource::<Host>()
        .tx
        .send(Ready {
            view: id,
            revision,
            result: Err("old pending result".into()),
        })
        .unwrap();
    drive(&mut world);
    let retained = &world.resource::<Host>().views[&id];
    assert!(retained.revision > revision);
    assert_eq!(
        retained.settled, revision,
        "obsolete completion cannot settle the new reset"
    );
    assert_eq!(retained.actor, Some(root));
    assert_eq!(retained.trial.as_ref().unwrap().equipment, source.equipment);
    world.remove_non_send::<UiRuntime>();
    drive(&mut world);
    assert!(world.resource::<Host>().views.is_empty());
    assert!(world.get_entity(root).is_err() && world.get_entity(camera).is_err());
}

#[test]
fn reset_uses_copied_model_header_before_first_actor_or_after_init_without_assembly() {
    let mut world = World::new();
    let mut view = View::new(Some(FIRST_LAYER));
    let mut source = source();
    source.equipment.slots[0] = aa_net::model::Item::template_only(18230);
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::Init {
            unit: "player".into(),
            create_model: false,
        },
        Some(&source),
        None,
    );
    assert!(!view.create_model && view.actor.is_none());
    view.trial.as_mut().unwrap().equipment.slots[0] = None;
    dispatch(&mut world, &mut view, ModelViewOp::ResetEquips, None, None);
    assert!(view.create_model && view.actor.is_none());
    assert_eq!(view.trial.as_ref().unwrap().equipment, source.equipment);
    assert_eq!(view.trial.as_ref().unwrap().selector, 0);
    view.trial.as_mut().unwrap().equipment.slots[0] = None;
    dispatch(&mut world, &mut view, ModelViewOp::ClearModel, None, None);
    assert!(!view.bound && !view.create_model);
    assert_eq!(view.reference_heel, Some(0.));
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::SetIngameShopMode(true),
        None,
        None,
    );
    assert!(
        view.camera_ops.is_empty(),
        "cleared context has no imported shop camera rows"
    );
    dispatch(&mut world, &mut view, ModelViewOp::ResetEquips, None, None);
    assert!(view.create_model && view.actor.is_none() && !view.bound);
    assert_eq!(view.trial.as_ref().unwrap().equipment, source.equipment);
    view.retire(&mut world);
    dispatch(&mut world, &mut view, ModelViewOp::ResetEquips, None, None);
    assert!(view.source.is_none() && view.trial.is_none() && !view.create_model);
}

#[test]
fn same_object_init_preserves_initial_snapshot_and_clear_invalidates_work() {
    let mut world = World::new();
    let mut view = View::new(Some(FIRST_LAYER));
    let mut source = source();
    source.equipment.slots[0] = aa_net::model::Item::template_only(18230);
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::Init {
            unit: "player".into(),
            create_model: true,
        },
        Some(&source),
        None,
    );
    let revision = view.revision;
    source.equipment.slots[0] = None;
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::Init {
            unit: "player".into(),
            create_model: false,
        },
        Some(&source),
        None,
    );
    assert_eq!(view.revision, revision);
    assert!(view.create_model);
    assert_eq!(
        view.trial
            .as_ref()
            .unwrap()
            .equipment
            .get(0)
            .unwrap()
            .template_id,
        18230
    );
    view.yaw = 0.7;
    let image = Handle::<Image>::default();
    view.target = Some((image.clone(), UVec2::splat(512)));
    dispatch(&mut world, &mut view, ModelViewOp::ClearModel, None, None);
    assert!(!view.bound && view.source.is_some() && view.trial.is_some());
    assert!(view.revision > revision);
    assert_eq!(view.yaw, 0.7);
    assert_eq!(view.target.as_ref().unwrap().0, image);
    dispatch(
        &mut world,
        &mut view,
        ModelViewOp::Init {
            unit: "player".into(),
            create_model: true,
        },
        Some(&source),
        None,
    );
    assert!(view.bound);
    assert_eq!(view.trial.as_ref().unwrap().equipment, source.equipment);
}

#[test]
#[ignore = "needs original character, rig and animation assets"]
fn failed_preparation_reconciles_latest_playback_without_losing_the_retained_clip() {
    let game = aa_anim::GameDb::open().unwrap();
    let assets = aa_anim::CharacterDesc::default_for(&game, Race::Nuian, Gender::Female)
        .unwrap()
        .resolve(&game)
        .unwrap();
    let rig = aa_anim::pose::shared_rig(&assets.skeleton).unwrap();
    for (name, stopped, queued) in [
        ("onehand_ba_combat_idle", false, false),
        ("nonexistent_preview_clip", false, false),
        ("onehand_ba_combat_idle", true, false),
        ("onehand_ba_combat_idle", false, true),
        ("nonexistent_preview_clip", false, true),
        ("onehand_ba_combat_idle", true, true),
    ] {
        let mut world = World::new();
        world.init_resource::<ClipLibrary>();
        let mut spawn = crate::hud::fixture_spawn();
        let mut source = source();
        source.character = spawn.character().id;
        source.object = spawn.obj_id;
        spawn.unit.model_id = 11;
        world.insert_resource(Player { spawn: Some(spawn) });
        let root = world.spawn(Animator::new(rig.clone(), Vec::new())).id();
        world.entity_mut(root).insert(Character {
            assets: std::sync::Arc::new(assets.clone()),
            model_root: root,
            parts: Vec::new(),
        });
        let mut view = View::new(Some(FIRST_LAYER));
        view.actor = Some(root);
        view.source = Some(source);
        view.bound = true;
        view.create_model = true;
        view.animation = Some(("fist_ba_relaxed_rand_idle".into(), true));
        bind_animation(&mut world, &view).unwrap();
        let old_clip = world
            .get::<Animator>(root)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .clone();
        {
            let mut animator = world.get_mut::<Animator>(root).unwrap();
            animator.current.as_mut().unwrap().time = 0.37;
            animator.speed = 0.75;
            animator.paused = !queued; // same-frame queued controls start from a playing actor
        }
        if !queued {
            view.animation = Some((name.into(), false));
            view.stopped = stopped;
        }
        view.revision = 2;
        let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
        ui.exec(
            "@retained_preview",
            "retained=UIParent:CreateWidget('modelview','retained','UIParent')",
        )
        .unwrap();
        let id = ui.take_modelview_work().surfaces[0].view;
        if queued {
            let command = if stopped {
                "retained:StopAnimation()".into()
            } else {
                format!("retained:PlayAnimation('{name}',false)")
            };
            ui.exec("@same_frame_control", &command).unwrap();
        }
        world.insert_non_send(ui);
        let mut host = Host::default();
        host.views.insert(id, view);
        host.pending.insert((id, 2));
        host.tx
            .send(Ready {
                view: id,
                revision: 2,
                result: Err("injected asset preparation failure".into()),
            })
            .unwrap();
        world.insert_resource(host);
        drive(&mut world);
        assert_eq!(world.resource::<Host>().views[&id].actor, Some(root));
        assert_eq!(world.resource::<Host>().views[&id].settled, 2);
        let animator = world.get::<Animator>(root).unwrap();
        assert_eq!(animator.paused, stopped);
        assert_eq!(animator.speed, 0.75);
        let current = animator.current.as_ref().unwrap();
        if stopped || name == "nonexistent_preview_clip" {
            assert_eq!(current.clip.source, old_clip);
            assert_eq!(current.time, 0.37);
            assert!(current.looping);
        } else {
            assert_ne!(current.clip.source, old_clip);
            assert_eq!(current.time, 0.);
            assert!(!current.looping);
        }
    }
}

#[test]
fn missing_or_unsupported_source_does_not_create_a_template_actor() {
    let mut world = World::new();
    let mut view = View::new(Some(FIRST_LAYER));
    for (unit, source) in [("player", None), ("target", Some(source()))] {
        dispatch(
            &mut world,
            &mut view,
            ModelViewOp::Init {
                unit: unit.into(),
                create_model: true,
            },
            source.as_ref(),
            None,
        );
        assert!(view.source.is_none() && view.actor.is_none());
    }
}

#[test]
fn retiring_a_runtime_removes_actor_camera_light_and_layered_descendants() {
    let mut world = World::new();
    let root = world
        .spawn((Transform::default(), PreviewActor(FIRST_LAYER)))
        .id();
    let child = world.spawn((Transform::default(), ChildOf(root))).id();
    let camera = world.spawn(Camera::default()).id();
    let light = world.spawn(Transform::default()).id();
    layers(&mut world);
    assert_eq!(
        world.get::<RenderLayers>(child),
        Some(&render_layer(FIRST_LAYER))
    );
    let mut view = View::new(Some(FIRST_LAYER));
    view.actor = Some(root);
    view.camera = Some(camera);
    view.light = Some(light);
    view.destroy(&mut world);
    for entity in [root, child, camera, light] {
        assert!(world.get_entity(entity).is_err());
    }
}
