use super::*;
use crate::{
    DominionCommand, NoticeEvent, WorldState,
    game::{ClientMsg, ServerMsg},
};

pub(crate) fn cases() -> Vec<(u16, &'static [u8], DominionEvent)> {
    vec![
        (
            0x21,
            include_bytes!("../../tests/fixtures/dominion/sc021.body.bin"),
            DominionEvent::NationalTaxRate {
                id: 0x1234,
                tax_rate: 0x89abcdefu32 as i32,
                previous_tax_rate: 0x76543210,
                changed_time: 0x8123456789abcdefu64 as i64,
            },
        ),
        (
            0x22,
            include_bytes!("../../tests/fixtures/dominion/sc022.body.bin"),
            DominionEvent::NationalMonumentChanged {
                id: 0x1234,
                monument_db_id: 0x8123456789abcdefu64 as i64,
                position: DominionPosition {
                    x: 0x8fedcba987654321u64 as i64,
                    y: 0x123456789abcdef0,
                    z: 3.5,
                },
            },
        ),
        (
            0x23,
            include_bytes!("../../tests/fixtures/dominion/sc023.body.bin"),
            DominionEvent::TaxBalanced {
                id: 0x1234,
                peace_tax_money: 0x89abcdefu32 as i32,
                peace_tax_aa_point: 0x76543210,
            },
        ),
        (
            0x24,
            include_bytes!("../../tests/fixtures/dominion/sc024.body.bin"),
            DominionEvent::NonPvpStarted {
                id: 0x1234,
                started: 0x8123456789abcdefu64 as i64,
                duration_minutes: 0x1234,
            },
        ),
        (
            0x25,
            include_bytes!("../../tests/fixtures/dominion/sc025.body.bin"),
            DominionEvent::NonPvpStopped { id: 0x1234 },
        ),
        (
            0xeb,
            include_bytes!("../../tests/fixtures/dominion/sc0eb.body.bin"),
            DominionEvent::SiegeReinforced { id: 0x1234 },
        ),
    ]
}

pub(crate) fn dd01(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xdd, 1, 0, 0x2a];
    bytes.extend(opcode.to_le_bytes());
    bytes.extend(body);
    bytes
}

fn batch(raw: &[u8], count: u16) -> Vec<u8> {
    use std::io::Write;
    let mut compressor =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    compressor.write_all(raw).unwrap();
    let mut bytes = vec![0xdd, 4];
    bytes.extend(count.to_le_bytes());
    bytes.extend(compressor.finish().unwrap());
    bytes
}

pub(crate) fn row() -> DominionData {
    let mut row = DominionData {
        zone_group_id: 0x1234,
        expedition_id: 77,
        tax_rate: -7,
        cur_house_tax_money: 11,
        cur_hunt_tax_money: 12,
        peace_tax_money: 13,
        cur_house_tax_aa_point: 14,
        peace_tax_aa_point: 15,
        last_paid_time: 21,
        last_siege_end_time: 22,
        reign_start_time: 23,
        last_tax_rate_changed_time: 24,
        last_national_tax_rate_changed_time: 25,
        non_pvp_start: 100,
        non_pvp_duration: 20,
        non_pvp_end_time: 120,
        ..Default::default()
    };
    row.siege_timers.siege_period = 4;
    row.siege_timers.defender = DominionParticipant {
        expedition_id: 77,
        nr: 101,
        limit: 4,
        members: vec![11, 22, 33, 22],
        ..Default::default()
    };
    row.siege_timers.offense = DominionParticipant {
        expedition_id: 88,
        limit: 3,
        members: vec![44],
        ..Default::default()
    };
    row
}

fn received(row: DominionData) -> Event {
    Event::Dominion(DominionEvent::Data {
        data: row,
        newly_declared: 2,
        final_data_by_request: 255,
    })
}

fn member(expedition_id: u32, character_id: u32, added: bool) -> Event {
    Event::Notice(NoticeEvent::SiegeMember {
        zone_group: 0x1234,
        expedition_id,
        character_id,
        added,
    })
}

#[test]
fn territory_fixed_native_literals_reject_every_prefix_and_leave_exact_suffix() {
    for (opcode, body, expected) in cases() {
        assert!(DominionEvent::handles(opcode));
        for end in 0..body.len() {
            assert!(
                DominionEvent::read_lane(opcode, &mut Reader::new(&body[..end])).is_err(),
                "{opcode:x}/{end}"
            );
            assert!(
                ServerMsg::decode(&dd01(opcode, &body[..end])).is_err(),
                "{opcode:x}/{end}"
            );
        }
        let mut with_suffix = body.to_vec();
        with_suffix.extend([0xde, 0xad, 0xbe, 0xef]);
        let mut reader = Reader::new(&with_suffix);
        assert_eq!(
            DominionEvent::read_lane(opcode, &mut reader).unwrap(),
            expected
        );
        assert_eq!(reader.rest(), [0xde, 0xad, 0xbe, 0xef]);
        assert!(
            matches!(ServerMsg::decode(&dd01(opcode, body)).unwrap(), ServerMsg::Dominion(actual) if actual == expected)
        );
    }
    assert!(!DominionEvent::handles(0x20));
    assert!(!DominionEvent::handles(0xec));
    assert!(!DominionEvent::handles(0xed));
}

#[test]
fn territory_python_dd04_preserves_repeated_receipts_and_following_packet() {
    let frame = include_bytes!("../../tests/fixtures/dominion/repeated-deltas.dd04.bin");
    let ServerMsg::Batch(messages) = ServerMsg::decode(frame).unwrap() else {
        panic!()
    };
    assert_eq!(messages.len(), 15);
    assert!(
        matches!(&messages[0], ServerMsg::Notice(NoticeEvent::DominionTaxRate { zone_group: 0x1234, tax_rate }) if *tax_rate == 0x89abcdefu32 as i32)
    );
    assert_eq!(messages[0], messages[1]);
    for (pair, (_, _, expected)) in messages[2..14].as_chunks::<2>().0.iter().zip(cases()) {
        for message in pair {
            assert!(matches!(message, ServerMsg::Dominion(actual) if *actual == expected));
        }
    }
    assert!(
        matches!(&messages[14], ServerMsg::Expedition(crate::ExpeditionEvent::Policies(rows)) if rows.is_empty())
    );
    for (opcode, body, _) in cases() {
        for end in 0..body.len() {
            let mut raw = vec![0, 0];
            raw.extend(opcode.to_le_bytes());
            raw.extend(&body[..end]);
            let ServerMsg::Batch(messages) = ServerMsg::decode(&batch(&raw, 1)).unwrap() else {
                panic!()
            };
            assert!(matches!(
                messages.as_slice(),
                [ServerMsg::DecodeFailed { .. }]
            ));
        }
    }
}

#[test]
fn territory_scalar_extremes_and_nonfinite_position_bits_remain_received_values() {
    let national = [
        0xff, 0xff, 0, 0, 0, 0x80, 0xff, 0xff, 0xff, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0x80,
    ];
    assert_eq!(
        DominionEvent::read_lane(0x21, &mut Reader::new(&national)).unwrap(),
        DominionEvent::NationalTaxRate {
            id: u16::MAX,
            tax_rate: i32::MIN,
            previous_tax_rate: i32::MAX,
            changed_time: i64::MIN
        },
    );
    let monument = [
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0, 0, 0, 0, 0, 0, 0, 0x80,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0x34, 0x12, 0xc0, 0x7f,
    ];
    let DominionEvent::NationalMonumentChanged {
        id,
        monument_db_id,
        position,
    } = DominionEvent::read_lane(0x22, &mut Reader::new(&monument)).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (id, monument_db_id, position.x, position.y),
        (u16::MAX, i64::MAX, i64::MIN, i64::MAX)
    );
    assert_eq!(position.z.to_bits(), 0x7fc01234);
}

#[test]
fn territory_request_literals_preserve_opcode_width_order_and_local_only_identity() {
    for (command, expected) in [
        (
            DominionCommand::ChangeTaxRate {
                zone_group_id: 0x1234,
                tax_rate: 0x89abcdefu32 as i32,
            },
            vec![
                0xdd, 1, 0, 0x2a, 0x12, 0, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x89,
            ],
        ),
        (
            DominionCommand::ChangeNationalTaxRate {
                zone_group_id: 0x1234,
                tax_rate: 0x89abcdefu32 as i32,
            },
            vec![
                0xdd, 1, 0, 0x2a, 0x13, 0, 0x34, 0x12, 0xef, 0xcd, 0xab, 0x89,
            ],
        ),
        (
            DominionCommand::RequestCharacterBrief {
                requested_id: 0x89abcdef,
            },
            vec![0xdd, 1, 0, 0x2a, 0x2d, 0, 0xef, 0xcd, 0xab, 0x89],
        ),
    ] {
        assert_eq!(ClientMsg::Dominion(command).encode(0x2a), expected);
    }
}

#[test]
fn territory_incremental_reducers_touch_only_documented_existing_fields() {
    for (_, _, receipt) in cases() {
        let mut state = Dominion::default();
        state.apply(&Event::Dominion(receipt.clone()));
        assert!(state.rows.is_empty());
        assert_eq!(state.last_receipt, Some(receipt.clone()));
        let original = row();
        state.apply(&received(original.clone()));
        let mut expected = original.clone();
        match &receipt {
            DominionEvent::NationalTaxRate {
                tax_rate,
                changed_time,
                ..
            } => {
                expected.national_tax_rate = *tax_rate as u16;
                expected.last_national_tax_rate_changed_time = *changed_time;
            }
            DominionEvent::NationalMonumentChanged {
                monument_db_id,
                position,
                ..
            } => {
                expected.national_monument_db_id = *monument_db_id;
                expected.national_monument_position = position.clone();
            }
            DominionEvent::TaxBalanced {
                peace_tax_money,
                peace_tax_aa_point,
                ..
            } => {
                expected.cur_house_tax_money = 0;
                expected.cur_hunt_tax_money = 0;
                expected.peace_tax_money = *peace_tax_money;
                expected.cur_house_tax_aa_point = 0;
                expected.peace_tax_aa_point = *peace_tax_aa_point;
            }
            DominionEvent::NonPvpStarted {
                started,
                duration_minutes,
                ..
            } => {
                expected.non_pvp_start = *started;
                expected.non_pvp_end_time = started.wrapping_add(i64::from(*duration_minutes) * 60);
            }
            DominionEvent::NonPvpStopped { .. } => expected.non_pvp_end_time = 0,
            DominionEvent::SiegeReinforced { .. } => expected.siege_timers.defender.limit = 100,
            _ => panic!(),
        }
        assert!(state.apply(&Event::Dominion(receipt.clone())));
        assert_eq!(state.rows[&0x1234], expected);
        assert_eq!(state.last_receipt, Some(receipt));
        assert_eq!(state.owner_change_blocked(77), Some(true));
        assert_eq!(state.owner_change_blocked(88), Some(true));
    }
}

#[test]
fn territory_notices_use_defender_first_duplicate_append_and_first_match_swap_remove() {
    let mut state = Dominion::default();
    assert!(!state.apply(&member(77, 5, true)));
    assert!(!state.apply(&Event::Notice(NoticeEvent::DominionTaxRate {
        zone_group: 0x1234,
        tax_rate: 50
    })));
    state.apply(&received(row()));
    let last = state.last_receipt.clone();
    let mut expected = row();
    assert!(state.apply(&Event::Notice(NoticeEvent::DominionTaxRate {
        zone_group: 0x1234,
        tax_rate: 50
    })));
    expected.tax_rate = 50;
    assert_eq!(state.rows[&0x1234], expected);
    assert!(!state.apply(&member(77, 99, true)), "full limit");
    assert!(!state.apply(&member(99, 11, false)), "foreign side");
    assert!(state.apply(&member(77, 11, false)));
    assert_eq!(
        state.rows[&0x1234].siege_timers.defender.members,
        [22, 22, 33]
    );
    assert!(state.apply(&member(77, 22, true)));
    assert_eq!(
        state.rows[&0x1234].siege_timers.defender.members,
        [22, 22, 33, 22]
    );
    assert!(state.apply(&member(77, 22, false)));
    assert_eq!(
        state.rows[&0x1234].siege_timers.defender.members,
        [22, 22, 33]
    );
    assert!(!state.apply(&member(77, 99, false)));
    assert!(state.apply(&member(88, 44, true)));
    assert_eq!(state.rows[&0x1234].siege_timers.offense.members, [44, 44]);
    // A malformed duplicate team identity still follows native defender-first ordering.
    state
        .rows
        .get_mut(&0x1234)
        .unwrap()
        .siege_timers
        .offense
        .expedition_id = 77;
    assert!(state.apply(&member(77, 66, true)));
    assert_eq!(
        state.rows[&0x1234].siege_timers.defender.members,
        [22, 22, 33, 66]
    );
    assert_eq!(state.rows[&0x1234].siege_timers.offense.members, [44, 44]);
    state
        .rows
        .get_mut(&0x1234)
        .unwrap()
        .siege_timers
        .defender
        .limit = 0;
    assert!(!state.apply(&member(77, 1, true)));
    assert_eq!(
        state.last_receipt, last,
        "Notice folding cannot invent a Dominion receipt"
    );
}

#[test]
fn territory_member_remove_uses_first_duplicate_when_last_member_differs() {
    let mut state = Dominion::default();
    let mut data = row();
    data.siege_timers.defender.members = vec![11, 22, 22, 33];
    state.apply(&received(data));
    assert!(state.apply(&member(77, 22, false)));
    assert_eq!(
        state.rows[&0x1234].siege_timers.defender.members,
        [11, 33, 22]
    );
}

#[test]
fn territory_reinforcement_and_unsafe_members_preserve_raw_data_and_sticky_uncertainty() {
    for nr in [0, 60, 100, 101, 255] {
        let mut state = Dominion::default();
        let mut data = row();
        data.siege_timers.defender.nr = nr;
        state.apply(&received(data.clone()));
        assert!(
            state.apply(&Event::Dominion(DominionEvent::SiegeReinforced {
                id: 0x1234
            }))
        );
        assert_eq!(state.rows[&0x1234].siege_timers.defender.limit, nr.min(100));
        assert_eq!(
            state.rows[&0x1234].siege_timers.defender.members,
            data.siege_timers.defender.members
        );
    }
    for (count, limit) in [(2, 101), (101, 100)] {
        let mut state = Dominion::default();
        let mut data = row();
        data.siege_timers.defender.members = vec![9; count];
        data.siege_timers.defender.limit = limit;
        let raw = received(data.clone());
        state.apply(&raw);
        state.apply(&member(77, 5, true));
        assert_eq!(state.rows[&0x1234], data);
        assert!(state.coverage_gap);
        assert_eq!(state.owner_change_blocked(77), None);
        state.apply(&received(row()));
        assert!(
            state.coverage_gap,
            "later full flags do not repair missing certainty"
        );
    }
}

#[test]
fn territory_non_pvp_wrapping_minutes_and_stop_do_not_rewrite_snapshot_suffix() {
    let mut state = Dominion::default();
    state.apply(&received(row()));
    for (started, duration_minutes, end) in [
        (1000, 2, 1120),
        (i64::MAX, 1, i64::MIN + 59),
        (i64::MAX, u16::MAX, i64::MIN + 3_932_099),
        (-1, 0, -1),
    ] {
        state.apply(&Event::Dominion(DominionEvent::NonPvpStarted {
            id: 0x1234,
            started,
            duration_minutes,
        }));
        assert_eq!(state.rows[&0x1234].non_pvp_end_time, end);
        assert_eq!(state.rows[&0x1234].non_pvp_duration, 20);
        state.apply(&Event::Dominion(DominionEvent::NonPvpStopped {
            id: 0x1234,
        }));
        assert_eq!(state.rows[&0x1234].non_pvp_start, started);
        assert_eq!(state.rows[&0x1234].non_pvp_end_time, 0);
    }
}

#[test]
fn territory_unknown_dd04_tail_can_recover_a_delta_without_repairing_coverage() {
    let mut raw = vec![0, 0, 0xff, 3, 0x55, 0, 0, 0x23, 0];
    raw.extend(include_bytes!(
        "../../tests/fixtures/dominion/sc023.body.bin"
    ));
    let ServerMsg::Batch(messages) = ServerMsg::decode(&batch(&raw, 2)).unwrap() else {
        panic!()
    };
    assert!(matches!(
        messages.as_slice(),
        [
            ServerMsg::Unhandled {
                level: 4,
                opcode: 0x3ff,
                ..
            },
            ServerMsg::Dominion(DominionEvent::TaxBalanced { .. })
        ]
    ));
    let mut world = WorldState::default();
    world.apply(&received(row()));
    world.apply(&Event::UnhandledGamePacket {
        level: 4,
        opcode: 0x3ff,
        len: 1,
    });
    world.apply(&Event::Dominion(cases()[2].2.clone()));
    assert_eq!(
        world.dominion.rows[&0x1234].peace_tax_money,
        0x89abcdefu32 as i32
    );
    assert!(world.dominion.coverage_gap);
}

#[test]
fn territory_world_lifecycle_clears_derived_end_and_history_but_sends_never_fold() {
    for reset in [
        Event::CharacterSelected(Box::default()),
        Event::LeftWorld { target: 0 },
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
        Event::Disconnected {
            message: String::new(),
        },
    ] {
        let mut world = WorldState::default();
        world.apply(&received(row()));
        world.apply(&Event::GameDecodeFailed {
            message: "gap".into(),
        });
        let before = world.dominion.clone();
        let command = DominionCommand::ChangeTaxRate {
            zone_group_id: 0x1234,
            tax_rate: 50,
        };
        world.apply(&Event::DominionRequested {
            character_id: 42,
            command,
        });
        world.apply(&Event::DominionRequestRejected {
            character_id: 42,
            command,
            message: "not sent".into(),
        });
        assert_eq!(world.dominion, before);
        world.apply(&reset);
        assert_eq!(world.dominion, Dominion::default());
    }
}
