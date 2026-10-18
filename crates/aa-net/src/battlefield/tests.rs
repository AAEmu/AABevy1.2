use super::*;
use crate::game::{ClientMsg, ServerMsg};
fn fold(s: &mut Battlefield, e: BattlefieldEvent) {
    s.apply(&Event::Battlefield(e));
}
fn request(s: &mut Battlefield, id: u32, c: u8) {
    fold(s, BattlefieldEvent::Requested(BattlefieldCommand::Apply { battlefield_id: id, corps: c }));
}
fn applied(id: u32, c: u8) -> BattlefieldEvent {
    BattlefieldEvent::Applied {
        battlefield_id: id,
        corps: c,
        error: 0,
    }
}
fn queue(id: u32) -> BattlefieldEvent {
    BattlefieldEvent::QueueState(BattlefieldQueue {
        battlefield_id: id,
        corps_size: 3,
        counts: [9, 1, 9],
    })
}
fn check(op: u16, body: &[u8], e: BattlefieldEvent) {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 1]).u16(op).raw(body);
    assert_eq!(ServerMsg::decode(&w.finish()).unwrap(), ServerMsg::Battlefield(e.clone()));
    for cut in 0..body.len() {
        assert!(BattlefieldEvent::read(op, &mut Reader::new(&body[..cut])).is_err(), "{op:x} cut{cut}");
    }
    let mut w = Writer::new();
    w.u16(0).u16(op).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
    let raw = w.finish();
    let n = raw.len() as u16;
    let mut w = Writer::new();
    w.raw(&[0xdd, 4]).u16(2).u8(1).u16(n).u16(!n).raw(&raw);
    assert_eq!(
        ServerMsg::decode(&w.finish()).unwrap(),
        ServerMsg::Batch(vec![ServerMsg::Battlefield(e), ServerMsg::Snow(true)])
    );
}
#[test]
fn queue_wire_goldens_all_truncations_and_dd04_alignment() {
    check(
        0x18b,
        &[7, 0, 0, 0, 2, 0x34, 0x12],
        BattlefieldEvent::Applied {
            battlefield_id: 7,
            corps: 2,
            error: 0x1234,
        },
    );
    check(0x18c, &[0x34, 0x12, 1, 0], BattlefieldEvent::Cancelled { raw: 0x11234 });
    check(0x18d, &[7, 0, 0, 0, 3, 9, 0, 1, 0, 9, 0], queue(7));
    let i = BattlefieldInvitation {
        zone_id: 8,
        instance_id: 9,
        ruleset_id: 10,
        corps: 1,
        qualified_id: 0x0102030405060708,
    };
    check(
        0x18e,
        &[8, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0, 1, 8, 7, 6, 5, 4, 3, 2, 1],
        BattlefieldEvent::Invited(i),
    );
    check(
        0x18f,
        &[8, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0, 1],
        BattlefieldEvent::InviteTimedOut {
            zone_id: 8,
            instance_id: 9,
            ruleset_id: 10,
            corps: 1,
        },
    );
}
#[test]
fn commands_have_exact_native_wire_and_validation() {
    let c = BattlefieldCommand::Apply {
        battlefield_id: 0x01020304,
        corps: 2,
    };
    assert_eq!(c.opcode(), 0xe1);
    assert_eq!(ClientMsg::Battlefield(c).encode_body(), [4, 3, 2, 1, 2]);
    assert_eq!(BattlefieldCommand::Cancel.opcode(), 0xe2);
    assert!(ClientMsg::Battlefield(BattlefieldCommand::Cancel).encode_body().is_empty());
    assert!(BattlefieldCommand::Apply { battlefield_id: 0, corps: 0 }.validate().is_err());
    assert!(BattlefieldCommand::Apply { battlefield_id: 7, corps: 3 }.validate().is_err());
}
#[test]
fn native_default_and_matching_receipts_do_not_invent_population() {
    let mut s = Battlefield::default();
    assert_eq!(s.joined_infos(), (0, 0, -1));
    fold(&mut s, queue(7));
    fold(&mut s, applied(7, 2));
    assert!(s.applied.is_none());
    request(&mut s, 7, 2);
    assert_eq!(s.joined_infos(), (0, 0, -1));
    fold(&mut s, applied(7, 1));
    assert!(s.applied.is_none());
    fold(&mut s, applied(8, 2));
    assert!(s.applied.is_none());
    fold(&mut s, applied(7, 2));
    assert_eq!(s.joined_infos(), (0, 0, 2));
    assert!(s.queue.is_none());
    fold(&mut s, queue(8));
    assert!(s.queue.is_none());
    fold(&mut s, queue(7));
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    assert!(s.cancel_pending);
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Cancelled { raw: 0xffff_ffff });
    assert_eq!(s, Battlefield::default());
    fold(&mut s, queue(7));
    assert!(s.queue.is_none());
    request(&mut s, 8, 0);
    fold(
        &mut s,
        BattlefieldEvent::Applied {
            battlefield_id: 8,
            corps: 0,
            error: 91,
        },
    );
    assert_eq!(s.last_error, Some(91));
    assert!(s.applied.is_none());
}
#[test]
fn invitation_rejects_wrong_corps_stops_queue_updates_and_timeout_matches_all_fields() {
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, applied(7, 2));
    fold(&mut s, queue(7));
    let mut i = BattlefieldInvitation {
        zone_id: 8,
        instance_id: 9,
        ruleset_id: 10,
        corps: 2,
        qualified_id: 11,
    };
    fold(&mut s, BattlefieldEvent::Invited(i));
    assert!(s.invitation.is_none());
    i.corps = 1;
    fold(&mut s, BattlefieldEvent::Invited(i));
    assert_eq!(s.invitation, Some(i));
    fold(
        &mut s,
        BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 7,
            corps_size: 99,
            counts: [0; 3],
        }),
    );
    assert_eq!(s.joined_infos(), (6, 3, 2));
    for (z, n, r, c) in [(0, 9, 10, 1), (8, 0, 10, 1), (8, 9, 0, 1), (8, 9, 10, 0)] {
        fold(
            &mut s,
            BattlefieldEvent::InviteTimedOut {
                zone_id: z,
                instance_id: n,
                ruleset_id: r,
                corps: c,
            },
        );
        assert!(s.invitation.is_some());
    }
    fold(
        &mut s,
        BattlefieldEvent::InviteTimedOut {
            zone_id: 8,
            instance_id: 9,
            ruleset_id: 10,
            corps: 1,
        },
    );
    assert!(s.invitation.is_none());
}
#[test]
fn lifecycle_worldstate_and_failed_sends_keep_default_received_state() {
    for e in [
        Event::CharacterSelected(Box::default()),
        Event::LeftWorld { target: 0 },
        Event::Disconnected { message: "test".into() },
        Event::Kicked {
            reason: 1,
            message: "test".into(),
        },
    ] {
        let mut w = crate::state::WorldState::default();
        w.apply(&Event::Battlefield(BattlefieldEvent::Requested(BattlefieldCommand::Apply {
            battlefield_id: 7,
            corps: 0,
        })));
        w.apply(&Event::Battlefield(applied(7, 0)));
        w.apply(&Event::Battlefield(queue(7)));
        assert!(w.battlefield.queue.is_some());
        w.apply(&e);
        assert_eq!(w.battlefield, Battlefield::default());
    }
    let mut s = Battlefield::default();
    fold(
        &mut s,
        BattlefieldEvent::RequestRejected {
            command: BattlefieldCommand::Apply { battlefield_id: 7, corps: 2 },
            message: "not sent".into(),
        },
    );
    assert_eq!(s.joined_infos(), (0, 0, -1));
    assert!(s.requesting.is_none());
}

#[test]
fn native_cancel_is_explicit_two_byte_signed_source_contract() {
    let mut r = Reader::new(&[0xff, 0xff, 0x44, 0x33]);
    assert_eq!(BattlefieldEvent::read_native_cancel_error(&mut r).unwrap(), -1);
    assert_eq!(r.u16().unwrap(), 0x3344);
    assert!(BattlefieldEvent::read_native_cancel_error(&mut Reader::new(&[0])).is_err());
    assert!(BattlefieldEvent::read(0x18c, &mut Reader::new(&[0, 0])).is_err());
}

#[test]
fn successful_cancel_write_blocks_late_admission_and_queue_mutation_without_faking_clear() {
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    fold(&mut s, applied(7, 2));
    assert!(s.applied.is_none());
    let mut s = Battlefield::default();
    request(&mut s, 7, 2);
    fold(&mut s, applied(7, 2));
    fold(&mut s, queue(7));
    fold(&mut s, BattlefieldEvent::Requested(BattlefieldCommand::Cancel));
    fold(
        &mut s,
        BattlefieldEvent::QueueState(BattlefieldQueue {
            battlefield_id: 7,
            corps_size: 10,
            counts: [0; 3],
        }),
    );
    assert_eq!(s.joined_infos(), (6, 3, 2));
    fold(&mut s, BattlefieldEvent::Cancelled { raw: 0 });
    assert_eq!(s.joined_infos(), (0, 0, -1));
}
