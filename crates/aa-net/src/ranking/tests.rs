use super::*;
use crate::{game::ServerMsg, wire::Writer};
fn row(rank_id: u32, score: i32) -> RankCharacterRow {
    RankCharacterRow {
        account_id: u32::MAX,
        account: b"A\0tail".to_vec(),
        character_id: 42,
        rank_id,
        world_id: 255,
        v1: score,
        raw_sum_v1: 255,
        v2: i32::MIN,
        raw_sum_v2: 2,
        recorded_raw: u64::MAX,
    }
}
fn board() -> RankSnapshot {
    RankSnapshot {
        board_id: -1,
        updated_raw: u64::MAX,
        reset_raw: 1,
        entry_count: 2,
        entries: vec![
            RankSnapshotEntry {
                character_id: 42,
                rank_id: 7,
                world_id: 3,
                v1: 10,
                v2: 20,
                updated_raw: 0,
                ranking: 1,
            },
            RankSnapshotEntry {
                character_id: 43,
                rank_id: 99,
                world_id: 255,
                v1: i32::MIN,
                v2: i32::MAX,
                updated_raw: u64::MAX,
                ranking: -1,
            },
        ],
        scope_count: 2,
        scopes: vec![
            RankSnapshotScope { scope_id: 5, v1: 10, v2: 20 },
            RankSnapshotScope { scope_id: 5, v1: 11, v2: 21 },
        ],
    }
}
fn personal(rows: Vec<RankCharacterRow>) -> RankingEvent {
    RankingEvent::Personal {
        type_id: 999,
        error: 0,
        ack_time_raw: 123,
        count: rows.len() as i32,
        rows,
    }
}
fn receipt(value: RankingEvent) -> Event {
    Event::Ranking(value)
}
fn write_row(w: &mut Writer, row: &RankCharacterRow) {
    w.u32(row.account_id)
        .blob(&row.account)
        .u32(row.character_id)
        .u32(row.rank_id)
        .u8(row.world_id)
        .i32(row.v1)
        .u8(row.raw_sum_v1)
        .i32(row.v2)
        .u8(row.raw_sum_v2)
        .u64(row.recorded_raw);
}
fn write_board(w: &mut Writer, b: &RankSnapshot) {
    w.i64(b.board_id).u64(b.updated_raw).u64(b.reset_raw).i32(b.entry_count);
    for r in &b.entries {
        w.u32(r.character_id)
            .u32(r.rank_id)
            .u8(r.world_id)
            .i32(r.v1)
            .i32(r.v2)
            .u64(r.updated_raw)
            .i32(r.ranking);
    }
    w.i32(b.scope_count);
    for r in &b.scopes {
        w.i32(r.scope_id).i32(r.v1).i32(r.v2);
    }
}
#[test]
fn exact_lane_native_wire_fields_truncations_and_dd04() {
    let mut a = Writer::new();
    a.u32(999).i16(-1).u64(u64::MAX).i32(1);
    write_row(&mut a, &row(7, 11));
    let mut b = Writer::new();
    b.u32(7).i16(0);
    write_board(&mut b, &board());
    let cases = [
        (
            0x1fa,
            a.finish(),
            RankingEvent::Personal {
                type_id: 999,
                error: -1,
                ack_time_raw: u64::MAX,
                count: 1,
                rows: vec![row(7, 11)],
            },
        ),
        (
            0x1fb,
            b.finish(),
            RankingEvent::Snapshot {
                rank_id: 7,
                error: 0,
                board: board(),
            },
        ),
        (0x1f9, vec![255; 4], RankingEvent::RewardMail { rank_id: u32::MAX }),
        (
            0x205,
            vec![255; 5],
            RankingEvent::Alarm {
                rank_id: u32::MAX,
                raw_kind: 255,
            },
        ),
    ];
    let mut batch = Writer::new();
    for (op, body, want) in &cases {
        let mut r = Reader::new(body);
        assert_eq!(RankingEvent::read(*op, &mut r).unwrap(), *want);
        assert!(r.is_empty());
        for n in 0..body.len() {
            assert!(RankingEvent::read(*op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
        }
        batch.u16(0).u16(*op).raw(body);
    }
    batch.u16(0).u16(0x1bc).u32(17);
    use std::io::Write;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&batch.finish()).unwrap();
    let mut p = Writer::new();
    p.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(4).raw(&z.finish().unwrap());
    let ServerMsg::Batch(decoded) = ServerMsg::decode(&p.finish()).unwrap() else {
        panic!()
    };
    assert_eq!(decoded.len(), 5);
    for (i, (_, _, want)) in cases.iter().enumerate() {
        assert!(matches!(&decoded[i],ServerMsg::Ranking(actual)if actual==want));
    }
    assert!(matches!(decoded[4], ServerMsg::SetBreath { breath: 17 }));
}
#[test]
fn executed_original_existing_key_cache_and_callback_goldens() {
    let mut cache = Rankings::default();
    assert_eq!(
        cache.apply_with_notices(&receipt(personal(vec![row(7, 11), row(7, 22)]))).1,
        vec![RankingNotice::PlayerRecords]
    );
    assert_eq!(cache.by_id[&7].personal.v1, 22);
    let event = receipt(RankingEvent::Snapshot {
        rank_id: 7,
        error: 0,
        board: board(),
    });
    assert!(cache.apply_with_notices(&event).1.is_empty());
    assert_eq!(cache.by_id[&7].snapshot, Some(board()));
    assert_eq!(
        cache.by_id[&7].snapshot.as_ref().unwrap().scopes.iter().map(|s| s.scope_id).collect::<Vec<_>>(),
        vec![5, 5]
    );
    assert_eq!(
        cache.apply_with_notices(&receipt(personal(vec![row(7, 33)]))).1,
        vec![RankingNotice::PlayerRecords]
    );
    assert_eq!(cache.by_id[&7].snapshot, Some(board()), "personal memcpy preserves board vectors");
    let before = cache.by_id.clone();
    for error in [-1, 1] {
        assert!(
            cache
                .apply_with_notices(&receipt(RankingEvent::Personal {
                    type_id: 999,
                    error,
                    ack_time_raw: 0,
                    count: 1,
                    rows: vec![row(7, 0)]
                }))
                .1
                .is_empty()
        );
        assert!(
            cache
                .apply_with_notices(&receipt(RankingEvent::Snapshot {
                    rank_id: 7,
                    error,
                    board: board()
                }))
                .1
                .is_empty()
        );
        assert_eq!(cache.by_id, before);
    }
    cache.apply(&receipt(RankingEvent::Snapshot {
        rank_id: 8,
        error: 0,
        board: board(),
    }));
    assert!(!cache.by_id.contains_key(&8));
    assert_eq!(cache.apply_with_notices(&receipt(personal(vec![]))).1, vec![RankingNotice::PlayerRecords]);
    let mut empty = Rankings::default();
    assert_eq!(
        empty.apply_with_notices(&receipt(personal(vec![]))).1,
        vec![RankingNotice::Error(792), RankingNotice::PlayerRecords]
    );
}
#[test]
fn cache_safe_caps_atomic_refusal_and_full_raw_receipt() {
    let mut cache = Rankings::default();
    cache.apply(&receipt(personal(vec![row(7, 11)])));
    let before = cache.by_id.clone();
    for event in [
        personal(vec![row(9, 1); 17]),
        personal(vec![RankCharacterRow {
            account: vec![255; 17],
            ..row(9, 1)
        }]),
        RankingEvent::Snapshot {
            rank_id: 7,
            error: 0,
            board: RankSnapshot {
                entry_count: 101,
                entries: vec![board().entries[0].clone(); 101],
                ..board()
            },
        },
        RankingEvent::Snapshot {
            rank_id: 7,
            error: 0,
            board: RankSnapshot {
                scope_count: 101,
                scopes: vec![board().scopes[0].clone(); 101],
                ..board()
            },
        },
    ] {
        assert!(cache.apply_with_notices(&receipt(event.clone())).1.is_empty());
        assert_eq!(cache.by_id, before);
        assert_eq!(cache.last_receipt, Some(event));
    }
    // Negative signed native counts consume zero rows; impossible allocations are refused before reserving.
    let mut w = Writer::new();
    w.u32(7).i16(0).u64(0).i32(-1);
    let event = RankingEvent::read(0x1fa, &mut Reader::new(&w.finish())).unwrap();
    assert_eq!(cache.apply_with_notices(&receipt(event)).1, vec![RankingNotice::PlayerRecords]);
    let mut w = Writer::new();
    w.u32(7).i16(0).u64(0).i32(i32::MAX);
    assert!(RankingEvent::read(0x1fa, &mut Reader::new(&w.finish())).is_err());
}
#[test]
fn actual_identity_app_isolation_matching_spawn_retention_and_raw_notices() {
    let mut cache = Rankings::default();
    let mut header = crate::model::CharacterState::default();
    header.character.id = 42;
    cache.apply(&Event::CharacterSelected(Box::new(header.clone())));
    cache.apply(&receipt(personal(vec![RankCharacterRow {
        character_id: 999,
        ..row(7, 11)
    }])));
    assert_eq!(cache.character_id, Some(42), "no inferred identity from first row");
    let spawn = crate::session::PlayerSpawn {
        obj_id: 7,
        state: header.clone(),
        unit: crate::state::tests::unit(7),
    };
    cache.apply(&Event::PlayerSpawn(Box::new(spawn)));
    cache.apply(&Event::LeftWorld { target: 0 });
    assert!(cache.by_id.contains_key(&7));
    for event in [RankingEvent::RewardMail { rank_id: 7 }, RankingEvent::Alarm { rank_id: 7, raw_kind: 255 }] {
        assert!(cache.apply_with_notices(&receipt(event.clone())).1.is_empty());
        assert_eq!(cache.last_receipt, Some(event));
        assert_eq!(cache.by_id.len(), 1);
    }
    header.character.id = 43;
    cache.apply(&Event::CharacterSelected(Box::new(header)));
    assert!(cache.by_id.is_empty());
    assert_eq!(cache.character_id, Some(43));
    cache.apply(&Event::Disconnected { message: String::new() });
    assert_eq!(cache, Rankings::default());
}

#[test]
fn empty_map_native_error_precedes_ack_assignment_dispatch_view() {
    let mut cache = Rankings::default();
    let (_, views) = cache.apply_with_dispatch(&receipt(personal(vec![])));
    assert_eq!(views.len(), 2);
    assert_eq!(views[0].0, RankingNotice::Error(792));
    assert_eq!(views[0].1.ack_time_raw, None);
    assert_eq!(views[1].0, RankingNotice::PlayerRecords);
    assert_eq!(views[1].1.ack_time_raw, Some(123));
    assert_eq!(cache, views[1].1);
}
