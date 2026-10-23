use super::*;
use crate::game::{ClientMsg, ServerMsg};
use crate::model::CharacterState;
use crate::state::WorldState;

fn bytes(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    f(&mut w);
    w.finish()
}
fn apply(state: &mut Actabilities, event: ActabilityEvent) {
    state.apply(&Event::Actability(event));
}
fn snapshot(state: &mut Actabilities, last: bool, values: &[(u32, i32, u8)]) {
    apply(
        state,
        ActabilityEvent::Snapshot {
            last,
            entries: values
                .iter()
                .map(|&(id, point, step)| ActabilityValue { id, point, step })
                .collect(),
        },
    );
}
fn batch(packets: &[(u16, Vec<u8>)]) -> Vec<ServerMsg> {
    let raw = bytes(|w| {
        for (op, body) in packets {
            w.u16(0).u16(*op).raw(body);
        }
        w.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
    });
    let len = raw.len() as u16;
    let wire = bytes(|w| {
        w.raw(&[0xdd, 4])
            .u16(packets.len() as u16 + 1)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
    });
    let ServerMsg::Batch(messages) = ServerMsg::decode(&wire).unwrap() else {
        panic!()
    };
    messages
}

#[test]
fn expert_commands_are_exact_native_id_auto_flag_and_empty_expand() {
    for auto_use_aa_point in [false, true] {
        for (command, op) in [
            (
                ActabilityCommand::Upgrade {
                    id: 0x01020304,
                    auto_use_aa_point,
                },
                0xff,
            ),
            (
                ActabilityCommand::Downgrade {
                    id: 0x01020304,
                    auto_use_aa_point,
                },
                0x100,
            ),
        ] {
            let request = ClientMsg::Actability(command);
            assert_eq!(request.opcode(), op);
            assert_eq!(
                request.encode_body(),
                [4, 3, 2, 1, u8::from(auto_use_aa_point)]
            );
            assert!(command.validate().is_ok());
        }
    }
    let request = ClientMsg::Actability(ActabilityCommand::Expand);
    assert_eq!(request.opcode(), 0x101);
    assert!(request.encode_body().is_empty());
    for command in [
        ActabilityCommand::Upgrade {
            id: 0,
            auto_use_aa_point: true,
        },
        ActabilityCommand::Downgrade {
            id: 0,
            auto_use_aa_point: false,
        },
    ] {
        assert!(command.validate().is_err());
    }
}

#[test]
fn snapshot_tier_and_expand_preserve_dd04_boundaries_and_reject_every_truncation() {
    let packets = vec![
        (
            0x136,
            bytes(|w| {
                w.bool(true)
                    .u8(2)
                    .u32(5)
                    .i32(12345)
                    .u8(1)
                    .u32(99)
                    .i32(-2)
                    .u8(255);
            }),
        ),
        (
            0x1c0,
            bytes(|w| {
                w.bool(false).u32(5).u8(0);
            }),
        ),
        (0x1c1, vec![7]),
    ];
    let messages = batch(&packets);
    assert_eq!(messages.len(), 4);
    assert_eq!(messages.last(), Some(&ServerMsg::Snow(true)));
    for (i, (op, body)) in packets.iter().enumerate() {
        let mut r = Reader::new(body);
        let expected = ActabilityEvent::read(*op, &mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(messages[i], ServerMsg::Actability(expected));
        for cut in 0..body.len() {
            assert!(ActabilityEvent::read(*op, &mut Reader::new(&body[..cut])).is_err());
        }
    }
    let ServerMsg::Actability(ActabilityEvent::Snapshot { entries, .. }) = &messages[0] else {
        panic!()
    };
    assert_eq!(
        entries[1],
        ActabilityValue {
            id: 99,
            point: -2,
            step: 255
        }
    );
    let empty = bytes(|w| {
        w.bool(true).u8(0);
    });
    assert_eq!(batch(&[(0x136, empty)]).len(), 2);
}

#[test]
fn bad_counts_zero_and_duplicate_ids_fail_before_a_snapshot_is_exposed() {
    for raw in [
        vec![1, 255],
        bytes(|w| {
            w.bool(true).u8(1).u32(0).i32(2).u8(0);
        }),
        bytes(|w| {
            w.bool(true).u8(2).u32(1).i32(2).u8(0).u32(1).i32(3).u8(1);
        }),
    ] {
        assert!(ActabilityEvent::read(0x136, &mut Reader::new(&raw)).is_err());
        assert!(matches!(batch(&[(0x136, raw)]).as_slice(),[ServerMsg::DecodeFailed{..}, ServerMsg::Snow(true)]));
    }
    assert!(ActabilityEvent::read(0x1c0, &mut Reader::new(&[1, 0, 0, 0, 0, 1])).is_err());
}

#[test]
fn snapshots_complete_in_chunks_and_new_selection_empty_lists_and_logout_clear_stale_state() {
    let mut state = Actabilities::default();
    snapshot(&mut state, false, &[(1, 100, 0)]);
    assert!(!state.loaded);
    snapshot(&mut state, true, &[(2, 20000, 1)]);
    assert!(state.loaded);
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.count_at_step(1), 1);
    snapshot(&mut state, true, &[]);
    assert!(state.loaded && state.entries.is_empty());
    snapshot(&mut state, true, &[(3, 300, 0)]);
    state.apply(&Event::CharacterSelected(Box::new(CharacterState {
        expanded_expert: 4,
        ..Default::default()
    })));
    assert!(!state.loaded && state.entries.is_empty());
    assert_eq!(state.expanded_expert, Some(4));
    state.apply(&Event::LeftWorld { target: 1 });
    assert_eq!(state, Actabilities::default());
    snapshot(&mut state, false, &[(9, 900, 1)]);
    state.apply(&Event::Disconnected {
        message: "test".into(),
    });
    assert_eq!(state, Actabilities::default());
}

#[test]
fn labor_points_are_deltas_and_unknown_or_downgraded_bases_stay_unknown() {
    let mut state = Actabilities::default();
    snapshot(&mut state, true, &[(5, 20000, 1)]);
    state.apply(&Event::LaborChanged {
        amount: -30,
        action: 5,
        point: 2,
        step: 1,
    });
    assert_eq!(state.get(5).unwrap().point, Some(20002));
    state.apply(&Event::LaborChanged {
        amount: -30,
        action: 99,
        point: 3,
        step: 2,
    });
    assert_eq!(
        state.get(99),
        Some(&Actability {
            id: 99,
            point: None,
            step: 2
        })
    );
    apply(
        &mut state,
        ActabilityEvent::TierChanged {
            is_upgrade: false,
            id: 5,
            step: 0,
        },
    );
    assert_eq!(state.get(5).unwrap().point, None);
    state.apply(&Event::LaborChanged {
        amount: -30,
        action: 5,
        point: 2,
        step: 0,
    });
    assert_eq!(state.get(5).unwrap().point, None);
    assert!(!state.apply(&Event::LaborChanged {
        amount: 5,
        action: 0,
        point: 0,
        step: 0
    }));
    assert!(state.get(0).is_none());
}

#[test]
fn only_matching_authoritative_expert_acks_change_tiers_capacity_and_pending() {
    let mut state = WorldState::default();
    snapshot(&mut state.actabilities, true, &[(5, 10000, 0)]);
    state.stats.vocation = 50;
    let up = ActabilityCommand::Upgrade {
        id: 5,
        auto_use_aa_point: true,
    };
    let down = ActabilityCommand::Downgrade {
        id: 5,
        auto_use_aa_point: true,
    };
    apply(&mut state.actabilities, ActabilityEvent::Requested(up));
    apply(&mut state.actabilities, ActabilityEvent::Requested(down));
    apply(
        &mut state.actabilities,
        ActabilityEvent::Requested(ActabilityCommand::Expand),
    );
    assert_eq!(state.actabilities.get(5).unwrap().step, 0);
    assert_eq!(state.actabilities.expanded_expert, None);
    state.apply(&Event::ServerError { code: 10 });
    assert_eq!(state.actabilities.pending.len(), 3);
    apply(
        &mut state.actabilities,
        ActabilityEvent::TierChanged {
            is_upgrade: true,
            id: 9,
            step: 2,
        },
    );
    assert_eq!(state.actabilities.pending.len(), 3);
    apply(
        &mut state.actabilities,
        ActabilityEvent::TierChanged {
            is_upgrade: true,
            id: 5,
            step: 1,
        },
    );
    assert_eq!(
        state.actabilities.pending,
        vec![down, ActabilityCommand::Expand]
    );
    assert_eq!(state.actabilities.get(5).unwrap().point, Some(10000));
    apply(
        &mut state.actabilities,
        ActabilityEvent::TierChanged {
            is_upgrade: false,
            id: 5,
            step: 0,
        },
    );
    assert_eq!(state.actabilities.pending, vec![ActabilityCommand::Expand]);
    apply(
        &mut state.actabilities,
        ActabilityEvent::Expanded { count: 6 },
    );
    assert_eq!(state.actabilities.expanded_expert, Some(6));
    assert!(state.actabilities.pending.is_empty());
    assert_eq!(
        state.stats.vocation, 50,
        "expand ACK must not guess the lane's currency change"
    );
    let invalid = ActabilityCommand::Upgrade {
        id: 0,
        auto_use_aa_point: false,
    };
    apply(
        &mut state.actabilities,
        ActabilityEvent::RequestRejected {
            command: invalid,
            message: "invalid id".into(),
        },
    );
    assert!(state.actabilities.pending.is_empty());
    assert!(state.actabilities.last_error.is_some());
}


#[test]
fn downgrade_catalog_reconciliation_requires_applied_matching_ack_and_sorted_lane_limits() {
    let mut state = Actabilities::default();
    snapshot(&mut state, true, &[(5, 100, 2)]);
    let ack = Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, id: 5, step: 1 });
    let limits = [10000, 20000, 30000];
    assert!(!state.resolve_downgrade_points(&ack, &limits));
    apply(&mut state, ActabilityEvent::Requested(ActabilityCommand::Downgrade { id: 5, auto_use_aa_point: false }));
    assert!(!state.resolve_downgrade_points(&ack, &limits));
    state.apply(&ack);
    assert_eq!(state.get(5).unwrap().point, None);
    assert!(!state.resolve_downgrade_points(&ack, &[20000, 10000]));
    assert!(!state.resolve_downgrade_points(&ack, &[10000]));
    for (id, step, is_upgrade) in [(6, 1, false), (5, 0, false), (5, 1, true)] {
        assert!(!state.resolve_downgrade_points(&Event::Actability(ActabilityEvent::TierChanged { is_upgrade, id, step }), &limits));
    }
    assert!(state.resolve_downgrade_points(&ack, &limits));
    // Exact lane assignment; native would retain old100 via min(old,limit).
    assert_eq!(state.get(5).unwrap().point, Some(20000));
    assert!(state.pending.is_empty());
    assert!(!state.resolve_downgrade_points(&ack, &limits));
}

#[test]
fn later_point_tier_and_snapshot_state_invalidate_old_downgrade_reconciliation() {
    let ack = Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, id: 5, step: 1 });
    let mut base = Actabilities::default();
    snapshot(&mut base, true, &[(5, 30000, 2)]);
    base.apply(&ack);
    let mut delta = base.clone();
    delta.apply(&Event::LaborChanged { amount: -1, action: 5, point: 7, step: 1 });
    assert!(!delta.resolve_downgrade_points(&ack, &[10000, 20000]));
    assert_eq!(delta.get(5).unwrap().point, None, "cannot drop an intervening delta");
    let mut upgraded = base.clone();
    apply(&mut upgraded, ActabilityEvent::TierChanged { is_upgrade: true, id: 5, step: 2 });
    assert!(!upgraded.resolve_downgrade_points(&ack, &[10000, 20000, 30000]));
    let mut received = base.clone();
    snapshot(&mut received, true, &[(5, 123, 1)]);
    assert!(!received.resolve_downgrade_points(&ack, &[10000, 20000]));
    assert_eq!(received.get(5).unwrap().point, Some(123));
    // A different profession's update does not invalidate this received reset.
    base.apply(&Event::LaborChanged { amount: -1, action: 6, point: 1, step: 0 });
    assert!(base.resolve_downgrade_points(&ack, &[10000, 20000]));
}

#[test]
fn kick_and_character_lifecycle_clear_pending_and_unresolved_downgrade_evidence() {
    let ack = Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, id: 5, step: 0 });
    let mut base = Actabilities::default();
    snapshot(&mut base, true, &[(5, 20000, 1)]);
    base.apply(&ack);
    apply(&mut base, ActabilityEvent::Requested(ActabilityCommand::Expand));
    for event in [Event::Kicked { reason: 7, message: "test".into() }, Event::LeftWorld { target: 0 },
        Event::Disconnected { message: "test".into() }, Event::CharacterSelected(Box::default())] {
        let mut state = base.clone();
        state.apply(&event);
        assert!(!state.loaded && state.entries.is_empty() && state.pending.is_empty());
        assert!(!state.resolve_downgrade_points(&ack, &[10000]));
    }
}
