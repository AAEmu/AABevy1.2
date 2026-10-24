use super::*;

fn received(actor: u32, value: DoodadBond) -> Event {
    Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: actor, bond: value })
}

fn sent() -> Event { Event::DoodadBond(DoodadBondEvent::UnbondRequested(request())) }

fn owner_doodad() -> crate::model::Doodad {
    crate::model::Doodad {
        obj_id: bond().owner_obj_id, template_id: 20847, owner_obj_id: 0, parent_obj_id: 0,
        attach_point: 0, pos: Pos::default(), rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0,
        has_loot_item: false, func_group_id: 1951, owner_id: 0, ucc_id: 0, item_template_id: 0,
        type2: 0, time_left: 0, plant_time: 0, quest_glow: 0, family: 0, puzzle_group: 0,
        owner_type: 0, owner_db_id: 0, data: 0,
    }
}

#[test]
fn successful_send_preserves_history_and_next_native_bond_consumes_eligibility_even_if_equal() {
    let values = rebond_goldens().into_iter().chain([
        DoodadBond { attach_point: 0, ..bond() }, DoodadBond { kind: 0, ..bond() },
        DoodadBond { space: i32::MIN, ..bond() }, DoodadBond { spot: i32::MAX, ..bond() },
    ]);
    for value in values {
        let mut state = active(Some(bond()));
        let history = state.by_unit.clone();
        assert!(state.apply(&sent()));
        assert_eq!(state.by_unit, history);
        assert_eq!(state.own(), Some(&bond())); assert!(state.own_known());
        assert!(state.matches_request(&request()));
        assert!(state.apply(&received(7, value)), "{value:?}");
        assert!(!state.own_rebond_eligible);
        assert_eq!(state.own(), Some(&value)); assert!(state.own_known());
        assert_eq!(state.matches_request(&request()), value == bond());
        let capture = UnbondDoodadRequest { bond: value, ..request() };
        assert!(state.matches_request(&capture));
        assert!(!state.apply(&received(7, value)), "occupied duplicate");
        assert!(!state.apply(&received(7, DoodadBond { spot: value.spot.wrapping_add(1), ..value })), "occupied conflict");
        assert_eq!(state.own(), Some(&value));
        assert!(state.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRequested(capture))));
        assert!(state.apply(&received(7, bond())), "another successful send reopens the gate");
    }
}

#[test]
fn rejected_stale_or_unknown_completions_cannot_open_the_receive_gate() {
    let mut captures = vec![UnbondDoodadRequest { character_id: 43, ..request() },
        UnbondDoodadRequest { actor_obj_id: 8, ..request() }];
    captures.extend([
        DoodadBond { owner_obj_id: 99, ..bond() }, DoodadBond { attach_point: 3, ..bond() },
        DoodadBond { kind: 3, ..bond() }, DoodadBond { space: -1, ..bond() },
        DoodadBond { spot: -1, ..bond() },
    ].map(|value| UnbondDoodadRequest { bond: value, ..request() }));
    for capture in captures {
        let mut state = active(Some(bond()));
        assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRequested(capture))));
        assert!(!state.own_rebond_eligible);
        assert!(!state.apply(&received(7, DoodadBond { spot: 1, ..bond() })));
    }
    let mut state = active(Some(bond()));
    assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRejected { request: request(), message: "not sent".into() })));
    assert!(!state.own_rebond_eligible);
    for gap in [false, true] {
        let mut state = active(None);
        if gap { state.apply(&Event::GameDecodeFailed { message: "gap".into() }); }
        assert!(!state.apply(&sent()));
        assert!(!state.own_rebond_eligible);
        assert_eq!(state.own_known(), !gap);
    }
}

#[test]
fn unrelated_or_invalid_events_do_not_consume_own_eligibility_or_open_remote_gate() {
    let mut state = active(Some(bond()));
    let mut remote = player(Some(bond())).unit;
    remote.obj_id = 8; remote.kind = UnitKind::Character { char_id: 43 };
    state.apply(&Event::UnitSpawned(Box::new(remote)));
    state.apply(&sent());
    let mut wrong = player(None).unit;
    wrong.kind = UnitKind::Character { char_id: 43 };
    let mut foreign = player(None).unit; foreign.obj_id = 99;
    let ignored = [
        received(99, bond()), received(7, DoodadBond { owner_obj_id: 0, ..bond() }),
        received(8, DoodadBond { spot: 1, ..bond() }),
        Event::DoodadBond(DoodadBondEvent::UnbondRequested(UnbondDoodadRequest { actor_obj_id: 8, character_id: 43, ..request() })),
        Event::DoodadBond(DoodadBondEvent::BondAbsent { unit_obj_id: 7 }),
        Event::DoodadBond(DoodadBondEvent::UnbondRejected { request: request(), message: "later rejection".into() }),
        Event::OwnUnitSnapshot(Box::new(wrong)), Event::OwnUnitSnapshot(Box::new(foreign)),
        Event::UnhandledGamePacket { level: 1, opcode: 0x777, len: 12 },
        Event::UnhandledGamePacket { level: 4, opcode: 0x777, len: 0 },
        Event::DoodadsRemoved(vec![999]), Event::UnitsRemoved(vec![999]),
    ];
    for event in ignored {
        state.apply(&event);
        assert!(state.own_rebond_eligible, "{event:?}");
        assert_eq!(state.own(), Some(&bond())); assert!(state.own_known());
    }
    for (actor, character, owner) in [(99, 42, bond().owner_obj_id), (7, 43, bond().owner_obj_id), (7, 42, 999)] {
        assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: actor, character_id: character, doodad_obj_id: owner })));
        assert!(state.own_rebond_eligible);
    }
    assert!(state.apply(&Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: 8, character_id: 43, doodad_obj_id: bond().owner_obj_id })));
    assert!(state.own_rebond_eligible, "a remote matched ACK is unrelated");
    assert!(state.apply(&received(8, bond())), "free remote actor still receives normally");
    assert!(state.own_rebond_eligible);
    assert!(state.apply(&received(7, DoodadBond { owner_obj_id: 999, ..bond() })));
    assert!(!state.own_rebond_eligible);
}

#[test]
fn snapshots_ack_loss_removal_reuse_and_teardown_retire_private_eligibility() {
    let mut owner = crate::state::tests::unit(bond().owner_obj_id);
    owner.kind = UnitKind::Character { char_id: 55 };
    let mut replaced_actor = player(Some(bond())).unit;
    replaced_actor.kind = UnitKind::Character { char_id: 43 };
    let mut events = vec![
        Event::OwnUnitSnapshot(Box::new(player(Some(bond())).unit)),
        Event::OwnUnitSnapshot(Box::new(player(None).unit)),
        Event::PlayerSpawn(Box::new(player(Some(bond())))),
        Event::UnitSpawned(Box::new(replaced_actor)), Event::UnitSpawned(Box::new(owner)),
        Event::DoodadsSpawned(vec![owner_doodad()]),
        Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: 7, character_id: 42, doodad_obj_id: bond().owner_obj_id }),
        Event::GameDecodeFailed { message: "gap".into() },
        Event::UnhandledGamePacket { level: 4, opcode: 0x777, len: 1 },
        Event::UnitsRemoved(vec![7]), Event::DoodadsRemoved(vec![bond().owner_obj_id]),
        Event::LeftWorld { target: 1 }, Event::Disconnected { message: "closed".into() },
        Event::Kicked { reason: 1, message: "kick".into() }, Event::CharacterSelected(Box::new(selected())),
    ];
    events.extend([Phase::Idle, Phase::Authenticating, Phase::WorldSelect, Phase::CharacterSelect,
        Phase::ReadyToConnect, Phase::Connecting].map(Event::PhaseChanged));
    for event in events {
        let mut state = active(Some(bond()));
        state.apply(&Event::DoodadsSpawned(vec![owner_doodad()]));
        state.apply(&sent());
        state.apply(&event);
        assert!(!state.own_rebond_eligible, "{event:?}");
        if matches!(event, Event::OwnUnitSnapshot(_) | Event::PlayerSpawn(_)) {
            assert!(state.own_known());
        }
        // A fresh actual snapshot can restore B but never inherits old local eligibility.
        state.apply(&Event::CharacterSelected(Box::new(selected())));
        state.apply(&Event::PlayerSpawn(Box::new(player(Some(bond())))));
        assert!(!state.apply(&received(7, DoodadBond { spot: 1, ..bond() })));
    }
}

#[test]
fn different_owner_old_ack_is_rejected_but_same_owner_old_ack_is_indistinguishable() {
    for next in [bond(), DoodadBond { spot: -1, kind: 3, ..bond() }, DoodadBond { owner_obj_id: 999, ..bond() }] {
        let mut state = active(Some(bond()));
        state.apply(&sent()); state.apply(&received(7, next));
        let old_ack = Event::DoodadBond(DoodadBondEvent::Unbonded {
            character_obj_id: 7, character_id: 42, doodad_obj_id: bond().owner_obj_id,
        });
        let same_owner = next.owner_obj_id == bond().owner_obj_id;
        assert_eq!(state.apply(&old_ack), same_owner);
        assert!(state.own_known());
        assert_eq!(state.own(), if same_owner { None } else { Some(&next) });
        assert!(!state.own_rebond_eligible);
    }
}
