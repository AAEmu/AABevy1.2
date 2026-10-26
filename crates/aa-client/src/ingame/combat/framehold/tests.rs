use super::*;
use crate::ingame::{
    anim::{ClipSet, Motion, drive_locomotion},
    units::{Asleep, RemoteUnit},
};
use aa_net::state::Buffs;

fn buff(id: u32, index: u32, kind: BuffKind) -> Buff {
    Buff {
        buff_id: id,
        index,
        kind,
        ..default()
    }
}

fn fixture() -> (App, Entity, aa_net::model::Unit) {
    let rig =
        aa_anim::pose::shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr").unwrap();
    let cal = aa_anim::Cal::shared("objects/characters/nuian/male/nude/nu_m_base.cal").unwrap();
    let library = ClipLibrary::default();
    let clips = ClipSet::load(&rig, cal, &library).unwrap();
    let mut animator = Animator::new(rig, Vec::new());
    animator.play(clips.idle.clone(), true, 0.0);
    let mut app = App::new();
    app.add_message::<NetEvent>()
        .init_resource::<Frameholds>()
        .init_resource::<Time>()
        .insert_resource(SkillBook::open().unwrap())
        .init_resource::<Buffs>()
        .insert_resource(library)
        .add_systems(Update, (observe, sync).chain());
    let mut unit = crate::ingame::seating::fixture_unit(200, 47, None);
    unit.obj_id = 200;
    unit.buffs.clear();
    let entity = app
        .world_mut()
        .spawn((
            RemoteUnit::fixture(200, Vec3::ZERO),
            animator,
            Locomotion::new(clips),
            Motion::default(),
        ))
        .id();
    receive(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    (app, entity, unit)
}

// Observe existing public playback state without adding a test API to aa-anim.
fn playback(app: &App, actor: Entity) -> String {
    let a = app.world().get::<Animator>(actor).unwrap();
    let playing = |p: &aa_anim::animator::Playing| {
        (
            Arc::as_ptr(&p.clip.clip) as usize,
            p.time,
            p.looping,
            p.loops,
        )
    };
    format!(
        "{:?}",
        (
            a.current.as_ref().map(playing),
            a.then.as_ref().map(playing),
            a.speed,
            a.has_layer()
        )
    )
}

fn receive(app: &mut App, event: Event) {
    app.world_mut().write_message(NetEvent(event));
    app.update();
}
fn held(app: &App, entity: Entity) -> bool {
    !app.world()
        .get::<Animator>(entity)
        .unwrap()
        .starts_enabled()
}
fn create(app: &mut App, id: u32, index: u32) {
    receive(
        app,
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(id, index, BuffKind::Unknown),
        },
    );
}

#[test]
fn receipt_order_is_imperative_and_duplicate_snapshots_do_not_reassert() {
    let (mut app, actor, mut unit) = fixture();
    create(&mut app, 1012, 1);
    create(&mut app, 1023, 2);
    assert!(held(&app, actor));
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert!(
        !held(&app, actor),
        "removing A releases even with B present"
    );
    unit.buffs = vec![buff(1023, 2, BuffKind::Good)];
    receive(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    receive(&mut app, Event::OwnUnitSnapshot(Box::new(unit.clone())));
    assert!(
        !held(&app, actor),
        "existing-ID snapshots do not run ApplyAttrs"
    );
    receive(
        &mut app,
        Event::BuffUpdated {
            obj_id: 200,
            index: 2,
            stacks: 3,
            charged: 2,
        },
    );
    assert!(!held(&app, actor));
    create(&mut app, 1023, 2);
    assert!(held(&app, actor), "same-index admitted Created reasserts");
    receive(&mut app, Event::UnitsRemoved(vec![200]));
    assert!(!held(&app, actor));
    receive(&mut app, Event::UnitSpawned(Box::new(unit)));
    assert!(held(&app, actor), "genuine new actor applies its snapshot");
}

#[test]
fn refused_capacity_and_missing_instance_do_not_change_permission() {
    let (mut app, actor, mut unit) = fixture();
    receive(&mut app, Event::UnitsRemoved(vec![200]));
    let book = app.world().resource::<SkillBook>();
    let mut good: Vec<_> = app
        .world()
        .resource::<Frameholds>()
        .native
        .rules
        .iter()
        .filter(|(id, rule)| {
            rule.kind == BuffKind::Good && !book.buff(**id).is_some_and(|look| look.framehold)
        })
        .map(|(id, _)| *id)
        .collect();
    good.sort_unstable();
    assert!(good.len() >= 32);
    unit.buffs = good[..32]
        .iter()
        .enumerate()
        .map(|(i, id)| buff(*id, 100 + i as u32, BuffKind::Good))
        .collect();
    receive(&mut app, Event::UnitSpawned(Box::new(unit)));
    create(&mut app, 1023, 7);
    assert!(
        !held(&app, actor),
        "native Good allocation cap rejects before hold"
    );
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 7,
        },
    );
    assert!(!held(&app, actor));
    create(&mut app, 1012, 8);
    assert!(held(&app, actor), "Bad has its independent capacity");
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 77,
        },
    );
    assert!(held(&app, actor), "missing index cannot release");
}

#[test]
fn parked_and_recreated_renderers_receive_the_logical_bit_without_overwriting_manual_pause() {
    let (mut app, actor, _) = fixture();
    app.world_mut().get_mut::<Animator>(actor).unwrap().paused = true;
    create(&mut app, 1012, 1);
    let animator = app
        .world_mut()
        .entity_mut(actor)
        .take::<Animator>()
        .unwrap();
    app.world_mut()
        .entity_mut(actor)
        .insert(Asleep::parked(animator));
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    let parked = app.world().get::<Asleep>(actor).unwrap().animator();
    assert!(parked.starts_enabled());
    assert!(parked.paused);
    create(&mut app, 1012, 1);
    assert!(
        !app.world()
            .get::<Asleep>(actor)
            .unwrap()
            .animator()
            .starts_enabled()
    );
    let rig = app
        .world()
        .get::<Asleep>(actor)
        .unwrap()
        .animator()
        .rig
        .clone();
    app.world_mut()
        .entity_mut(actor)
        .remove::<Asleep>()
        .insert(Animator::new(rig, Vec::new()));
    app.update();
    assert!(
        held(&app, actor),
        "fresh visual admission restores logical permission"
    );
    receive(&mut app, Event::LeftWorld { target: 0 });
    assert!(!held(&app, actor));
}

#[test]
fn held_locomotion_does_not_record_a_rejected_run_and_retries_on_release() {
    let (mut app, actor, _) = fixture();
    app.add_systems(Update, drive_locomotion.after(sync));
    create(&mut app, 1012, 1);
    let before = playback(&app, actor);
    app.world_mut().get_mut::<Motion>(actor).unwrap().speed = 5.4;
    app.update();
    assert_eq!(playback(&app, actor), before);
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert_ne!(playback(&app, actor), before);
}

#[test]
fn own_snapshot_does_not_replace_history_but_a_new_own_spawn_does() {
    let (mut app, actor, mut unit) = fixture();
    app.world_mut()
        .entity_mut(actor)
        .remove::<RemoteUnit>()
        .insert(LocalPlayer { obj_id: 200 });
    let mut state = aa_net::model::CharacterState::default();
    state.character.id = 47;
    let mut spawn = aa_net::PlayerSpawn {
        obj_id: 200,
        state,
        unit: unit.clone(),
    };
    spawn.obj_id = 200;
    unit.buffs = vec![buff(1012, 1, BuffKind::Bad), buff(1023, 2, BuffKind::Good)];
    spawn.unit = unit.clone();
    receive(&mut app, Event::PlayerSpawn(Box::new(spawn.clone())));
    assert!(held(&app, actor));
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert!(!held(&app, actor));
    receive(&mut app, Event::OwnUnitSnapshot(Box::new(unit)));
    assert!(!held(&app, actor), "appearance snapshot cannot reassert B");
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 2,
        },
    );
    assert!(
        !held(&app, actor),
        "history still knows B after ignoring the appearance snapshot"
    );
    receive(&mut app, Event::PlayerSpawn(Box::new(spawn)));
    assert!(held(&app, actor));
}

#[test]
fn held_death_selection_does_not_mark_a_corpse_until_the_start_can_succeed() {
    let (mut app, actor, _) = fixture();
    app.add_systems(Update, super::super::ensure_corpses.after(sync));
    create(&mut app, 1012, 1);
    let before = playback(&app, actor);
    app.world_mut().entity_mut(actor).insert(Dead { at: 0.0 });
    app.update();
    assert!(app.world().get::<super::super::Corpse>(actor).is_none());
    assert_eq!(playback(&app, actor), before);
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert!(app.world().get::<super::super::Corpse>(actor).is_some());
    assert_ne!(playback(&app, actor), before);
}

#[test]
fn same_frame_overlap_receipts_are_folded_in_order_before_the_actor_is_synchronized() {
    let (mut app, actor, _) = fixture();
    for event in [
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(1012, 1, BuffKind::Unknown),
        },
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(1023, 2, BuffKind::Unknown),
        },
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert!(!held(&app, actor));
    create(&mut app, 1023, 2);
    assert!(held(&app, actor));
    // Visual eviction alone is not a fresh native actor initialization.
    let replacement = Animator::new(
        app.world().get::<Animator>(actor).unwrap().rig.clone(),
        Vec::new(),
    );
    app.world_mut().entity_mut(actor).insert(replacement);
    app.update();
    assert!(held(&app, actor));
}

#[test]
fn an_absent_end_action_releases_without_stopping_the_existing_action() {
    let (mut app, actor, _) = fixture();
    app.world_mut().entity_mut(actor).insert(Acting {
        until: 100.0,
        layered: false,
    });
    create(&mut app, 1039, 1);
    assert!(held(&app, actor));
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert!(!held(&app, actor));
    assert!(
        app.world().get::<Acting>(actor).is_some(),
        "1039 has no anim_end action"
    );
    create(&mut app, 1012, 2);
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 2,
        },
    );
    assert!(
        app.world().get::<Acting>(actor).is_none(),
        "1012's end48 is an explicit stop marker"
    );
}

fn ordered_fixture() -> (App, Entity) {
    let (mut app, actor, _) = fixture();
    app.insert_resource(State::new(crate::Screen::InGame))
        .init_resource::<Time<Virtual>>()
        .init_resource::<aa_net::state::Units>()
        .init_resource::<aa_net::state::Doodads>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<crate::ingame::skill_motion::Inbox>()
        .init_resource::<Combat>()
        .insert_resource(crate::ingame::emote::Emotes::from_env())
        .add_systems(
            Update,
            super::super::buffs::ensure_visuals
                .after(sync)
                .before(super::super::handle_events),
        )
        .add_systems(Update, super::super::handle_events.after(sync))
        .add_systems(
            Update,
            sync_after_receipts
                .after(super::super::handle_events)
                .after(sync),
        )
        .add_systems(
            Update,
            super::super::buffs::reconcile_buffs.after(sync_after_receipts),
        );
    app.world_mut()
        .get_entity_mut(actor)
        .unwrap()
        .insert(GlobalTransform::default());
    app.world_mut()
        .spawn((LocalPlayer { obj_id: 100 }, GlobalTransform::default()));
    (app, actor)
}

fn emote(id: u32) -> Event {
    Event::Emote(aa_net::EmoteEvent::Expressed {
        obj_id: 200,
        target_obj_id: 0,
        emotion_id: id,
    })
}

#[test]
fn an_emote_before_a_same_batch_hold_starts_then_freezes() {
    let (mut app, actor) = ordered_fixture();
    app.world_mut().write_message(NetEvent(emote(10))); // actual fist_em_clap
    app.world_mut().write_message(NetEvent(Event::BuffCreated {
        obj_id: 200,
        buff: buff(1039, 1, BuffKind::Unknown),
    }));
    app.update();
    let a = app.world().get::<Animator>(actor).unwrap();
    assert!(!a.starts_enabled());
    assert!(
        a.current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("fist_em_clap")
    );
    assert!(app.world().get::<Acting>(actor).is_some());
}

#[test]
fn an_emote_before_a_same_batch_release_is_rejected_without_replay() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1039, 1);
    let before = playback(&app, actor);
    app.world_mut().write_message(NetEvent(emote(11))); // actual fist_em_greet
    app.world_mut().write_message(NetEvent(Event::BuffRemoved {
        obj_id: 200,
        index: 1,
    }));
    app.update();
    assert!(!held(&app, actor));
    assert_eq!(playback(&app, actor), before);
    assert!(app.world().get::<Acting>(actor).is_none());
}

#[test]
fn a_receipt_end_stop_does_not_replay_over_a_later_start_in_the_same_batch() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1012, 1);
    for event in [
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
        emote(10),
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(1039, 2, BuffKind::Unknown),
        },
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert!(held(&app, actor));
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("fist_em_clap")
    );
    assert!(
        app.world().get::<Acting>(actor).is_some(),
        "the old end48 stop preceded this admitted start"
    );
}

#[test]
fn synchronous_permission_rules_do_not_wait_for_the_hud_catalog() {
    let (mut app, actor, mut unit) = fixture();
    assert!(!app.world().resource::<Buffs>().native.catalog_loaded());
    app.insert_resource(Frameholds::default());
    unit.buffs = vec![buff(1039, 1, BuffKind::Bad)];
    app.world_mut()
        .write_message(NetEvent(Event::UnitSpawned(Box::new(unit.clone()))));
    app.world_mut().write_message(NetEvent(Event::BuffRemoved {
        obj_id: 200,
        index: 1,
    }));
    app.update();
    assert!(!held(&app, actor));
    assert!(
        app.world()
            .resource::<Frameholds>()
            .native
            .of(200)
            .unwrap()
            .is_empty()
    );
    receive(&mut app, Event::UnitSpawned(Box::new(unit)));
    assert!(
        !held(&app, actor),
        "a duplicate snapshot cannot undo the observed release"
    );
    let rules = (*app.world().resource::<Frameholds>().native.rules).clone();
    app.world_mut()
        .resource_mut::<Buffs>()
        .native
        .set_catalog(rules);
    app.update();
    assert!(!held(&app, actor));
}

#[test]
fn removing_a_knockdown_while_held_retires_its_old_action_even_when_standup_is_rejected() {
    let (mut app, actor, _) = fixture();
    let look = app.world().resource::<SkillBook>().buff(1172).unwrap();
    assert!(look.knock_down);
    let (rig, cal) = {
        let a = app.world().get::<Animator>(actor).unwrap();
        (
            a.rig.clone(),
            app.world()
                .get::<Locomotion>(actor)
                .unwrap()
                .clips
                .cal
                .clone(),
        )
    };
    let knocked = bind_clip(
        &rig,
        &cal,
        app.world().resource::<ClipLibrary>(),
        "all_re_combat_knockback",
    )
    .unwrap();
    app.world_mut()
        .get_mut::<Animator>(actor)
        .unwrap()
        .play(knocked, true, 0.0);
    create(&mut app, 1039, 1);
    app.init_resource::<aa_net::state::Units>()
        .init_resource::<aa_net::state::Doodads>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<crate::ingame::skill_motion::Inbox>();
    app.world_mut()
        .spawn((LocalPlayer { obj_id: 100 }, GlobalTransform::default()));
    app.world_mut().entity_mut(actor).insert((
        GlobalTransform::default(),
        Acting {
            until: 1.0e7,
            layered: false,
        },
        super::super::buffs::fixture_knockdown(look),
    ));
    app.add_systems(Update, super::super::buffs::reconcile_buffs.after(sync));
    app.update();
    assert!(held(&app, actor));
    assert!(
        app.world().get::<Acting>(actor).is_none(),
        "the consumed knockdown must not own the unit forever"
    );
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert!(!held(&app, actor));
    assert!(app.world().get::<Acting>(actor).is_none());
}

#[test]
fn host_decode_diagnostics_do_not_erase_known_runtime_release_instances() {
    for diagnostic in [
        Event::GameDecodeFailed {
            message: "bounded fixture".into(),
        },
        Event::UnhandledGamePacket {
            level: aa_net::game::LEVEL_COMPRESSED,
            opcode: 0x3ff,
            len: 1,
        },
    ] {
        let (mut app, actor, _) = fixture();
        create(&mut app, 1039, 1);
        receive(&mut app, diagnostic);
        assert!(
            held(&app, actor),
            "a host diagnostic is not a native hold/release operation"
        );
        receive(
            &mut app,
            Event::BuffRemoved {
                obj_id: 200,
                index: 1,
            },
        );
        assert!(
            !held(&app, actor),
            "later decoded removal still owns its known instance"
        );
        create(&mut app, 1039, 2);
        assert!(held(&app, actor));
        receive(
            &mut app,
            Event::BuffRemoved {
                obj_id: 200,
                index: 2,
            },
        );
        assert!(!held(&app, actor));
    }
}

fn records(app: &App, actor: Entity) -> Vec<(u32, u32, bool, Vec<Entity>)> {
    super::super::buffs::fixture_records(
        app.world()
            .get::<super::super::buffs::BuffVisuals>(actor)
            .unwrap(),
    )
}

#[test]
fn rejected_buff_pose_is_consumed_before_a_same_batch_release() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1039, 1);
    let before = playback(&app, actor);
    app.world_mut().write_message(NetEvent(Event::BuffCreated {
        obj_id: 200,
        buff: buff(63, 2, BuffKind::Unknown),
    }));
    app.world_mut().write_message(NetEvent(Event::BuffRemoved {
        obj_id: 200,
        index: 1,
    }));
    app.update();
    assert_eq!(playback(&app, actor), before);
    assert!(!held(&app, actor));
    assert!(
        records(&app, actor)
            .iter()
            .any(|(index, id, posed, _)| (*index, *id, *posed) == (2, 63, false))
    );
    app.update();
    assert_eq!(
        playback(&app, actor),
        before,
        "reconciliation cannot retry the denied receipt"
    );
}

#[test]
fn admitted_buff_pose_starts_before_a_same_batch_hold() {
    let (mut app, actor) = ordered_fixture();
    app.world_mut().write_message(NetEvent(Event::BuffCreated {
        obj_id: 200,
        buff: buff(63, 2, BuffKind::Unknown),
    }));
    app.world_mut().write_message(NetEvent(Event::BuffCreated {
        obj_id: 200,
        buff: buff(1039, 1, BuffKind::Unknown),
    }));
    app.update();
    assert!(held(&app, actor));
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("all_re_combat_stun")
    );
    assert!(
        records(&app, actor)
            .iter()
            .any(|(index, id, posed, _)| (*index, *id, *posed) == (2, 63, true))
    );
}

#[test]
fn ordinary_buff_and_emote_starts_keep_their_receipt_order_after_reconciliation() {
    for buff_last in [false, true] {
        let (mut app, actor) = ordered_fixture();
        let created = Event::BuffCreated {
            obj_id: 200,
            buff: buff(63, 2, BuffKind::Unknown),
        };
        let events = if buff_last {
            [emote(10), created]
        } else {
            [created, emote(10)]
        };
        for event in events {
            app.world_mut().write_message(NetEvent(event));
        }
        app.update();
        let expected = if buff_last {
            "all_re_combat_stun"
        } else {
            "fist_em_clap"
        };
        assert!(
            app.world()
                .get::<Animator>(actor)
                .unwrap()
                .current
                .as_ref()
                .unwrap()
                .clip
                .source
                .as_deref()
                .unwrap()
                .contains(expected)
        );
        let after = playback(&app, actor);
        app.update();
        assert_eq!(playback(&app, actor), after);
    }
}

#[test]
fn first_component_and_same_batch_remove_create_have_one_final_record() {
    let (mut app, actor) = ordered_fixture();
    assert!(
        app.world()
            .get::<super::super::buffs::BuffVisuals>(actor)
            .is_none()
    );
    for event in [
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(63, 1, BuffKind::Unknown),
        },
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
        Event::BuffCreated {
            obj_id: 200,
            buff: buff(63, 2, BuffKind::Unknown),
        },
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    let rows = records(&app, actor);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].0, rows[0].1, rows[0].2), (2, 63, true));
}

#[test]
fn admitted_duplicate_reasserts_and_rekey_keeps_fx_without_artificial_removal() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 63, 1);
    let sentinel = app.world_mut().spawn_empty().id();
    super::super::buffs::fixture_held(
        &mut app
            .world_mut()
            .get_mut::<super::super::buffs::BuffVisuals>(actor)
            .unwrap(),
        sentinel,
    );
    receive(&mut app, emote(10));
    create(&mut app, 63, 1);
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("all_re_combat_stun")
    );
    receive(&mut app, emote(10));
    create(&mut app, 63, 2);
    let rows = records(&app, actor);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, 2);
    assert!(rows[0].3.contains(&sentinel));
    assert!(app.world().get_entity(sentinel).is_ok());
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    assert_eq!(
        records(&app, actor).len(),
        1,
        "old rekey index cannot end the current instance"
    );
}

#[test]
fn rejected_pose_does_not_retry_after_distance_fx_eviction_or_parked_wake() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1039, 1);
    let animator = app
        .world_mut()
        .entity_mut(actor)
        .take::<Animator>()
        .unwrap();
    app.world_mut()
        .entity_mut(actor)
        .insert(Asleep::parked(animator));
    create(&mut app, 63, 2);
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    let a = app.world_mut().entity_mut(actor).take::<Asleep>().unwrap();
    let a = a.into_animator();
    app.world_mut().entity_mut(actor).insert(a);
    let before = playback(&app, actor);
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
        GlobalTransform::from(Transform::from_xyz(200.0, 0.0, 0.0));
    app.update();
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() = GlobalTransform::default();
    app.update();
    assert_eq!(playback(&app, actor), before);
    assert!(
        records(&app, actor)
            .iter()
            .any(|(_, id, posed, _)| *id == 63 && !posed)
    );
}

#[test]
fn initial_snapshot_applies_available_controller_poses_in_native_group_order() {
    for hold_first in [false, true] {
        let (mut app, actor) = ordered_fixture();
        receive(&mut app, Event::UnitsRemoved(vec![200]));
        let rig = app.world().get::<Animator>(actor).unwrap().rig.clone();
        let idle = app
            .world()
            .get::<Locomotion>(actor)
            .unwrap()
            .clips
            .idle
            .clone();
        let mut animator = Animator::new(rig, Vec::new());
        animator.play(idle, true, 0.0);
        app.world_mut().entity_mut(actor).insert(animator);
        let before = playback(&app, actor);
        let mut unit = crate::ingame::seating::fixture_unit(200, 47, None);
        let ordinary = buff(63, 2, BuffKind::Bad);
        let hold = buff(1039, 1, BuffKind::Bad);
        unit.buffs = if hold_first {
            vec![hold, ordinary]
        } else {
            vec![ordinary, hold]
        };
        receive(&mut app, Event::UnitSpawned(Box::new(unit)));
        assert!(held(&app, actor));
        if hold_first {
            assert_eq!(playback(&app, actor), before);
        } else {
            assert!(
                app.world()
                    .get::<Animator>(actor)
                    .unwrap()
                    .current
                    .as_ref()
                    .unwrap()
                    .clip
                    .source
                    .as_deref()
                    .unwrap()
                    .contains("all_re_combat_stun")
            );
        }
        assert!(
            records(&app, actor)
                .iter()
                .any(|(_, id, posed, _)| *id == 63 && *posed == !hold_first)
        );
    }
}

#[test]
fn capacity_rejection_cannot_start_through_the_raw_lane_visual_list() {
    let (mut app, actor) = ordered_fixture();
    let book = app.world().resource::<SkillBook>();
    let mut ids: Vec<_> = app
        .world()
        .resource::<Frameholds>()
        .native
        .rules
        .iter()
        .filter(|(id, rule)| {
            rule.kind == BuffKind::Bad
                && book.buff(**id).is_none_or(|l| {
                    l.fx.is_empty()
                        && l.start_anim.is_none()
                        && !l.framehold
                        && !l.knock_down
                        && !l.ragdoll
                        && l.instrument_start.iter().all(Option::is_none)
                })
        })
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    assert!(ids.len() >= 24);
    let mut unit = crate::ingame::seating::fixture_unit(200, 47, None);
    unit.buffs = ids[..24]
        .iter()
        .enumerate()
        .map(|(i, id)| buff(*id, 100 + i as u32, BuffKind::Bad))
        .collect();
    receive(&mut app, Event::UnitsRemoved(vec![200]));
    app.world_mut()
        .resource_mut::<Buffs>()
        .apply(&Event::UnitSpawned(Box::new(unit.clone())));
    receive(&mut app, Event::UnitSpawned(Box::new(unit)));
    let created = Event::BuffCreated {
        obj_id: 200,
        buff: buff(63, 9, BuffKind::Unknown),
    };
    app.world_mut().resource_mut::<Buffs>().apply(&created);
    assert_eq!(app.world().resource::<Buffs>().of(200).len(), 25);
    receive(&mut app, created);
    assert_eq!(records(&app, actor).len(), 24);
    assert!(!records(&app, actor).iter().any(|(_, id, _, _)| *id == 63));
}

#[test]
fn an_end_stop_is_imperative_even_when_its_buff_start_was_denied() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1039, 1);
    create(&mut app, 63, 2);
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 1,
        },
    );
    receive(&mut app, emote(10));
    assert!(app.world().get::<Acting>(actor).is_some());
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 2,
        },
    );
    assert!(app.world().get::<Acting>(actor).is_none());
}

#[test]
fn simultaneous_created_receipts_apply_both_fx_distance_crossings_without_losing_handles() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 63, 2);
    let initial = records(&app, actor)
        .into_iter()
        .find(|r| r.1 == 63)
        .unwrap()
        .3;
    assert!(
        !initial.is_empty(),
        "original buff63 has a held head effect"
    );
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
        GlobalTransform::from(Transform::from_xyz(200.0, 0.0, 0.0));
    create(&mut app, 1039, 1);
    assert!(
        records(&app, actor)
            .into_iter()
            .find(|r| r.1 == 63)
            .unwrap()
            .3
            .is_empty()
    );
    assert!(
        initial
            .iter()
            .all(|id| app.world().get_entity(*id).is_err())
    );
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() = GlobalTransform::default();
    create(&mut app, 1039, 1);
    let returned = records(&app, actor)
        .into_iter()
        .find(|r| r.1 == 63)
        .unwrap()
        .3;
    assert_eq!(returned.len(), initial.len());
    assert!(
        returned
            .iter()
            .all(|id| app.world().get_entity(*id).is_ok())
    );
    app.update();
    assert_eq!(
        records(&app, actor)
            .into_iter()
            .find(|r| r.1 == 63)
            .unwrap()
            .3,
        returned
    );
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
        GlobalTransform::from(Transform::from_xyz(200.0, 0.0, 0.0));
    create(&mut app, 1039, 1);
    assert!(
        returned
            .iter()
            .all(|id| app.world().get_entity(*id).is_err())
    );
}

#[test]
fn a_parked_buff_uses_the_retained_original_head_bone_and_keeps_it_on_wake() {
    let (mut app, actor) = ordered_fixture();
    let rig = app.world().get::<Animator>(actor).unwrap().rig.clone();
    let bones: Vec<_> = rig
        .skeleton
        .bones
        .iter()
        .enumerate()
        .map(|(i, _)| {
            app.world_mut()
                .spawn((
                    Transform::default(),
                    GlobalTransform::from(Transform::from_xyz(0.0, 2.0 + i as f32 * 0.01, 0.0)),
                    ChildOf(actor),
                ))
                .id()
        })
        .collect();
    app.world_mut().get_mut::<Animator>(actor).unwrap().bones = bones;
    let head = super::super::find_bone(
        app.world().get::<Animator>(actor).unwrap(),
        super::super::bone_names(2),
    )
    .unwrap();
    let animator = app
        .world_mut()
        .entity_mut(actor)
        .take::<Animator>()
        .unwrap();
    app.world_mut()
        .entity_mut(actor)
        .insert(Asleep::parked(animator));
    create(&mut app, 63, 2);
    let held = records(&app, actor)
        .into_iter()
        .find(|r| r.1 == 63)
        .unwrap()
        .3;
    assert!(!held.is_empty());
    for entity in &held {
        assert_eq!(
            app.world().get::<ChildOf>(*entity).map(ChildOf::parent),
            Some(head)
        );
    }
    let parked = app.world_mut().entity_mut(actor).take::<Asleep>().unwrap();
    app.world_mut()
        .entity_mut(actor)
        .insert(parked.into_animator());
    app.update();
    assert_eq!(
        records(&app, actor)
            .into_iter()
            .find(|r| r.1 == 63)
            .unwrap()
            .3,
        held
    );
    for entity in held {
        assert_eq!(
            app.world().get::<ChildOf>(entity).map(ChildOf::parent),
            Some(head)
        );
    }
}

#[test]
fn active_native_gliding_suppresses_only_the_ordinary_buff_start_block() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 1029, 3); // actual Good gliding descriptor, no framehold
    assert!(!held(&app, actor));
    let before = playback(&app, actor);
    create(&mut app, 63, 2);
    assert_eq!(playback(&app, actor), before);
    assert!(
        records(&app, actor)
            .iter()
            .any(|(_, id, posed, _)| *id == 63 && !posed)
    );
    receive(&mut app, emote(10));
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("fist_em_clap")
    );
    receive(
        &mut app,
        Event::BuffRemoved {
            obj_id: 200,
            index: 3,
        },
    );
    let after = playback(&app, actor);
    app.update();
    assert_eq!(
        playback(&app, actor),
        after,
        "removing gliding cannot retry the consumed ordinary start"
    );
    create(&mut app, 63, 2);
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .clip
            .source
            .as_deref()
            .unwrap()
            .contains("all_re_combat_stun")
    );
}

#[test]
fn snapshot_gliding_predicate_sees_the_full_manager_before_any_apply() {
    for gliding in [false, true] {
        let (mut app, actor) = ordered_fixture();
        receive(&mut app, Event::UnitsRemoved(vec![200]));
        let before = playback(&app, actor);
        let mut unit = crate::ingame::seating::fixture_unit(200, 47, None);
        unit.buffs = vec![buff(108, 2, BuffKind::Good)]; // all_re_combat_struggle
        if gliding {
            unit.buffs.push(buff(1029, 3, BuffKind::Good));
        }
        receive(&mut app, Event::UnitSpawned(Box::new(unit)));
        assert!(!held(&app, actor), "gliding does not disable CChar starts");
        if gliding {
            assert_eq!(playback(&app, actor), before);
        } else {
            assert_ne!(playback(&app, actor), before);
        }
        assert!(
            records(&app, actor)
                .iter()
                .any(|(_, id, posed, _)| *id == 108 && *posed == !gliding)
        );
    }
}


#[test]
fn r5_gliding_does_not_suppress_knockdown_created_or_snapshot() {
    for snapshot in [false, true] {
        let (mut app, actor) = ordered_fixture();
        if snapshot {
            receive(&mut app, Event::UnitsRemoved(vec![200]));
            let mut unit = crate::ingame::seating::fixture_unit(200, 47, None);
            unit.buffs = vec![buff(1029, 3, BuffKind::Good), buff(1172, 7, BuffKind::Bad)];
            receive(&mut app, Event::UnitSpawned(Box::new(unit)));
        } else {
            create(&mut app, 1029, 3);
            create(&mut app, 1172, 7);
        }
        assert!(!held(&app, actor));
        assert!(records(&app, actor).iter().any(|(_, id, posed, _)| *id == 1172 && *posed),
            "native knockdown precedes the gliding-only ordinary block: snapshot={snapshot}");
        assert!(app.world().get::<Animator>(actor).unwrap().current.as_ref().unwrap()
            .clip.source.as_deref().unwrap().contains("all_re_combat_knockback"));
    }
}

#[test]
fn r5_removed_buff_uses_current_fx_distance_but_animation_admission_is_independent() {
    for return_near in [true, false] {
        let (mut app, actor) = ordered_fixture();
        *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
            GlobalTransform::from(Transform::from_xyz(200.0, 0.0, 0.0));
        create(&mut app, 1511, 4); // actual end49 = all_re_combat_struggle
        if return_near {
            *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() = GlobalTransform::default();
        }
        receive(&mut app, Event::BuffRemoved { obj_id: 200, index: 4 });
        assert!(records(&app, actor).iter().all(|(_, id, _, _)| *id != 1511));
        assert!(app.world().get::<Animator>(actor).unwrap().current.as_ref().unwrap()
            .clip.source.as_deref().unwrap().contains("all_re_combat_struggle"),
            "enabled end command must not depend on the FX visibility cache: return_near={return_near}");
    }
}

#[test]
fn r5_passive_parked_distance_maintenance_retires_fx_and_never_replays_pose() {
    let (mut app, actor) = ordered_fixture();
    create(&mut app, 63, 2);
    let initial = records(&app, actor).into_iter().find(|r| r.1 == 63).unwrap().3;
    assert!(!initial.is_empty());
    let animator = app.world_mut().entity_mut(actor).take::<Animator>().unwrap();
    let before = animator.current.as_ref().unwrap().clip.source.clone();
    app.world_mut().entity_mut(actor).insert(Asleep::parked(animator));
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() =
        GlobalTransform::from(Transform::from_xyz(200.0, 0.0, 0.0));
    app.update(); // no receipt: actual parked component must still maintain FX distance
    assert!(records(&app, actor).into_iter().find(|r| r.1 == 63).unwrap().3.is_empty());
    assert!(initial.iter().all(|id| app.world().get_entity(*id).is_err()));
    *app.world_mut().get_mut::<GlobalTransform>(actor).unwrap() = GlobalTransform::default();
    app.update();
    let returned = records(&app, actor).into_iter().find(|r| r.1 == 63).unwrap().3;
    assert_eq!(returned.len(), initial.len());
    assert!(returned.iter().all(|id| app.world().get_entity(*id).is_ok()));
    let parked = app.world_mut().entity_mut(actor).take::<Asleep>().unwrap();
    assert_eq!(parked.animator().current.as_ref().unwrap().clip.source, before);
    app.world_mut().entity_mut(actor).insert(parked.into_animator());
    app.update();
    assert_eq!(records(&app, actor).into_iter().find(|r| r.1 == 63).unwrap().3, returned);
}
