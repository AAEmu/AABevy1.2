use super::*;
use aa_net::model::{CharacterState, Doodad, Equipment, ModelParams};
use aa_net::{PlayerSpawn, Pos};

pub(super) fn bond() -> DoodadBond {
    DoodadBond {
        owner_obj_id: 123,
        attach_point: 2,
        kind: 2,
        space: 1,
        spot: 0,
    }
}

pub(crate) fn unit(id: u32, character: u32, bond: Option<DoodadBond>) -> Unit {
    Unit {
        obj_id: id,
        name: "seat fixture".into(),
        kind: UnitKind::Character { char_id: character },
        owner_name: String::new(),
        pos: Pos::new(1.0, 2.0, 3.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id: 1,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 100,
        mp: 100,
        max_hp: 100,
        max_mp: 100,
        dead: false,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: bond,
    }
}

pub(super) fn selection() -> Event {
    let mut state = CharacterState::default();
    state.character.id = 42;
    Event::CharacterSelected(Box::new(state))
}

pub(super) fn spawn(value: Option<DoodadBond>) -> Event {
    let Event::CharacterSelected(state) = selection() else {
        unreachable!()
    };
    Event::PlayerSpawn(Box::new(PlayerSpawn {
        obj_id: 47,
        state: *state,
        unit: unit(47, 42, value),
    }))
}

pub(super) fn active(value: Option<DoodadBond>) -> Seats {
    let mut seats = Seats::default();
    seats.apply(&selection());
    seats.apply(&spawn(value));
    seats
}

pub(super) fn received(value: DoodadBond) -> Event {
    Event::DoodadBond(DoodadBondEvent::Bonded {
        unit_obj_id: 47,
        bond: value,
    })
}

pub(super) fn completion(request: UnbondDoodadRequest) -> Event {
    Event::DoodadBond(DoodadBondEvent::UnbondRequested(request))
}

fn ack(actor: u32, character: u32, owner: u32) -> Event {
    Event::DoodadBond(DoodadBondEvent::Unbonded {
        character_obj_id: actor,
        character_id: character,
        doodad_obj_id: owner,
    })
}

#[test]
fn successful_send_detaches_only_its_receipt_and_cached_history_does_not_reseat() {
    let mut seats = active(Some(bond()));
    let original = seats.wanted(47).unwrap();
    let request = seats.capture_unbond().unwrap();
    assert_eq!(seats.wanted(47), Some(original), "enqueue is not a write");
    assert!(seats.capture_unbond().is_none());
    assert!(!seats.own_free());
    seats.apply(&completion(request));
    assert_eq!(
        seats.history.own(),
        Some(&bond()),
        "received history stays intact"
    );
    assert!(seats.wanted(47).is_none());
    assert!(seats.own_free());
    assert!(seats.capture_unbond().is_none());
    for _ in 0..10 {
        seats.apply(&Event::ServerError { code: 1 });
        assert!(seats.wanted(47).is_none());
    }
    seats.apply(&received(bond()));
    let fresh = seats.wanted(47).unwrap();
    assert_eq!(fresh.generation, original.generation);
    assert_ne!(fresh.receipt, original.receipt);
    assert!(!seats.own_free());
    seats.apply(&received(bond()));
    assert_eq!(
        seats.wanted(47),
        Some(fresh),
        "occupied duplicate is not accepted"
    );
}

#[test]
fn failure_unrelated_completion_and_absent_sentinel_leave_seating_intact() {
    let mut seats = active(Some(bond()));
    let original = seats.wanted(47);
    let request = seats.capture_unbond().unwrap();
    let wrong = UnbondDoodadRequest {
        character_id: 999,
        ..request
    };
    seats.apply(&completion(wrong));
    seats.apply(&Event::DoodadBond(DoodadBondEvent::BondAbsent {
        unit_obj_id: 47,
    }));
    assert!(
        seats.capture_unbond().is_none(),
        "unmatched completion cannot consume capture"
    );
    assert_eq!(seats.wanted(47), original);
    seats.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRejected {
        request,
        message: "socket closed".into(),
    }));
    assert_eq!(seats.wanted(47), original);
    assert_eq!(seats.capture_unbond(), Some(request));
}

#[test]
fn accepted_changed_rebond_updates_full_outbound_capture_and_consumes_native_gate() {
    let mut seats = active(Some(bond()));
    let old = seats.capture_unbond().unwrap();
    seats.apply(&completion(old));
    let changed = DoodadBond {
        owner_obj_id: 456,
        spot: -7,
        space: -3,
        kind: 1,
        ..bond()
    };
    seats.apply(&received(changed));
    let current = seats.wanted(47).unwrap();
    assert_eq!(current.bond, changed);
    seats.apply(&received(bond()));
    assert_eq!(
        seats.wanted(47),
        Some(current),
        "only one fresh receipt is admitted"
    );
    assert_eq!(seats.capture_unbond().unwrap().bond, changed);
}

#[test]
fn new_snapshot_supersedes_suppression_and_old_equal_completion_cannot_detach_it() {
    let mut seats = active(Some(bond()));
    let original = seats.wanted(47).unwrap();
    let request = seats.capture_unbond().unwrap();
    seats.apply(&Event::OwnUnitSnapshot(Box::new(unit(
        47,
        42,
        Some(bond()),
    ))));
    let fresh = seats.wanted(47).unwrap();
    assert_eq!(fresh.generation, original.generation);
    assert_ne!(fresh.receipt, original.receipt);
    assert!(
        seats.capture_unbond().is_none(),
        "equal completion still outstanding"
    );
    seats.apply(&completion(request));
    assert_eq!(seats.wanted(47), Some(fresh));
    seats.apply(&received(bond()));
    assert_eq!(
        seats.wanted(47),
        Some(fresh),
        "ignored equal receipt cannot invalidate next capture"
    );
    let request = seats.capture_unbond().unwrap();
    seats.apply(&completion(request));
    assert!(seats.wanted(47).is_none());
    seats.apply(&Event::OwnUnitSnapshot(Box::new(unit(
        47,
        42,
        Some(bond()),
    ))));
    assert!(
        seats.wanted(47).is_some(),
        "fresh snapshot also re-arms after completed send"
    );
}

#[test]
fn late_completion_keeps_protocol_history_in_sync_and_fresh_changed_receipt_is_authority() {
    let mut seats = active(Some(bond()));
    let mut actual = seats.history.clone();
    let request = seats.capture_unbond().unwrap();
    let snapshot = Event::OwnUnitSnapshot(Box::new(unit(47, 42, Some(bond()))));
    actual.apply(&snapshot);
    seats.apply(&snapshot);
    let newer = seats.wanted(47).unwrap();
    let sent = completion(request);
    actual.apply(&sent);
    seats.apply(&sent);
    assert_eq!(
        seats.history, actual,
        "presentation cannot retract a completed socket write from protocol history"
    );
    assert_eq!(
        seats.wanted(47),
        Some(newer),
        "old capture still cannot retire the newer visual receipt"
    );
    let changed = DoodadBond {
        owner_obj_id: 456,
        spot: -3,
        ..bond()
    };
    let receipt = received(changed);
    assert!(actual.apply(&receipt));
    seats.apply(&receipt);
    assert_eq!(seats.history, actual);
    assert_eq!(
        seats.wanted(47).unwrap().bond,
        changed,
        "accepted current received authority wins"
    );
    assert_eq!(
        seats.capture_unbond().unwrap().bond,
        changed,
        "capture stays valid for session's full-bond guard"
    );
}

#[test]
fn stale_ack_guards_and_same_owner_wire_ambiguity_follow_the_network_reducer() {
    let mut seats = active(Some(bond()));
    let request = seats.capture_unbond().unwrap();
    seats.apply(&completion(request));
    let changed = DoodadBond { spot: -4, ..bond() };
    seats.apply(&received(changed));
    let current = seats.wanted(47);
    for event in [ack(99, 42, 123), ack(47, 43, 123), ack(47, 42, 999)] {
        seats.apply(&event);
        assert_eq!(seats.wanted(47), current);
    }
    seats.apply(&ack(47, 42, 123));
    assert!(
        seats.wanted(47).is_none(),
        "same-owner ACK carries no spot/epoch"
    );
    assert!(seats.own_free());
}

#[test]
fn actor_replacement_and_removal_cannot_reuse_a_pending_completion() {
    for removed in [false, true] {
        let mut seats = active(Some(bond()));
        let original = seats.wanted(47).unwrap();
        let request = seats.capture_unbond().unwrap();
        if removed {
            seats.apply(&Event::UnitsRemoved(vec![47]));
        }
        seats.apply(&spawn(Some(bond())));
        let fresh = seats.wanted(47).unwrap();
        assert_ne!(fresh.generation, original.generation);
        assert!(seats.capture_unbond().is_none());
        seats.apply(&completion(request));
        assert_eq!(seats.wanted(47), Some(fresh));
        assert_eq!(seats.capture_unbond(), Some(request));
    }
}

#[test]
fn gaps_and_wrong_snapshots_never_turn_unknown_into_a_free_seat() {
    let mut seats = Seats::default();
    assert!(!seats.own_free());
    seats.apply(&selection());
    assert!(!seats.own_free());
    seats.apply(&spawn(None));
    assert!(seats.own_free());
    for gap in [
        Event::GameDecodeFailed {
            message: "truncated".into(),
        },
        Event::UnhandledGamePacket {
            level: aa_net::game::LEVEL_COMPRESSED,
            opcode: 0x999,
            len: 1,
        },
    ] {
        seats.apply(&gap);
        assert!(!seats.own_free());
        assert!(seats.capture_unbond().is_none());
        seats.apply(&Event::OwnUnitSnapshot(Box::new(unit(47, 99, None))));
        seats.apply(&Event::OwnUnitSnapshot(Box::new(unit(99, 42, None))));
        assert!(!seats.own_free());
        seats.apply(&Event::OwnUnitSnapshot(Box::new(unit(47, 42, None))));
        assert!(seats.own_free());
    }
}

#[test]
fn owner_loss_and_reuse_do_not_resurrect_bonds_without_fresh_authority() {
    let doodad = Doodad {
        obj_id: 123,
        template_id: 1236,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: Pos::default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 1951,
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
    };
    let mut seats = active(Some(bond()));
    seats.apply(&Event::DoodadsSpawned(vec![doodad.clone()]));
    assert!(
        seats.wanted(47).is_some(),
        "first owner observation is not replacement"
    );
    seats.apply(&Event::DoodadsRemoved(vec![123]));
    assert!(seats.wanted(47).is_none());
    assert!(!seats.own_free());
    seats.apply(&Event::DoodadsSpawned(vec![doodad.clone()]));
    assert!(seats.wanted(47).is_none());
    seats.apply(&received(bond()));
    assert!(seats.wanted(47).is_some());
    seats.apply(&Event::DoodadsSpawned(vec![doodad]));
    assert!(
        seats.wanted(47).is_none(),
        "replacement invalidates owner lifetime"
    );
    assert!(
        seats.pending.is_none(),
        "owner loss creates no outbound request"
    );
}

#[test]
fn teardown_forgets_capture_but_keeps_actor_epochs_distinct() {
    for event in [
        selection(),
        Event::LeftWorld { target: 1 },
        Event::Disconnected {
            message: "closed".into(),
        },
        Event::Kicked {
            reason: 1,
            message: "kicked".into(),
        },
    ] {
        let mut seats = active(Some(bond()));
        let original = seats.wanted(47).unwrap();
        seats.capture_unbond().unwrap();
        seats.apply(&event);
        assert!(seats.pending.is_none());
        assert!(seats.wanted(47).is_none());
        assert!(!seats.own_free());
        seats.apply(&selection());
        seats.apply(&spawn(Some(bond())));
        assert_ne!(seats.wanted(47).unwrap().generation, original.generation);
    }
}

#[test]
fn receipt_fold_runs_before_input_without_an_in_game_screen() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .init_resource::<Seats>()
        .add_systems(Update, track);
    app.world_mut().write_message(NetEvent(selection()));
    app.update();
    app.world_mut().write_message(NetEvent(spawn(Some(bond()))));
    app.update();
    assert!(app.world().resource::<Seats>().wanted(47).is_some());
    let request = app
        .world_mut()
        .resource_mut::<Seats>()
        .capture_unbond()
        .unwrap();
    for event in [completion(request), received(bond())] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert!(
        app.world().resource::<Seats>().wanted(47).is_some(),
        "fresh same-batch receipt re-arms"
    );
    app.world_mut().write_message(NetEvent(ack(47, 42, 123)));
    app.update();
    assert!(app.world().resource::<Seats>().own_free());
}
