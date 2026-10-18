//! Original initial trial tasks, wire receipts, and callback-time context.
use super::*;
use crate::{
    game::{ClientMsg, ServerMsg},
    wire::Writer,
};
#[test]
fn initial_receipts_native_lane_goldens_truncations_and_dd04_alignment() {
    let rows = [
        (
            0x171,
            vec![255, 255, 255, 255, 0, 0, 0, 128],
            TrialEvent::ImprisonOrTrialOffered {
                crime_value: -1,
                jail_minutes: i32::MIN,
            },
        ),
        (
            0x172,
            vec![3, 0, 65, 0, 255, 255, 255, 255, 255],
            TrialEvent::JuryInvited {
                defendant_name: vec![65, 0, 255],
                trial_id: u32::MAX,
            },
        ),
        (
            0x174,
            vec![255, 0, 0, 0, 0, 0, 0, 0, 128, 255, 255, 255, 255],
            TrialEvent::JurySeated {
                raw_is_west: 255,
                trial_id: 0,
                court: i32::MIN,
                jury_number: -1,
            },
        ),
    ];
    let mut batch = Writer::new();
    for (op, body, expected) in &rows {
        let mut r = Reader::new(body);
        assert_eq!(TrialEvent::read(*op, &mut r).unwrap(), *expected);
        assert!(r.is_empty());
        for n in 0..body.len() {
            assert!(TrialEvent::read(*op, &mut Reader::new(&body[..n])).is_err());
        }
        batch.u16(0).u16(*op).raw(body);
    }
    batch.u16(0).u16(0x1bc).u32(99);
    use std::io::Write;
    let mut zip = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    zip.write_all(&batch.finish()).unwrap();
    let mut packet = Writer::new();
    packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(9).raw(&zip.finish().unwrap());
    let ServerMsg::Batch(actual) = ServerMsg::decode(&packet.finish()).unwrap() else {
        panic!()
    };
    assert_eq!(actual.len(), 4);
    for (i, (_, _, e)) in rows.iter().enumerate() {
        assert!(matches!(&actual[i],ServerMsg::Trial(v)if v==e));
    }
    assert!(matches!(actual[3], ServerMsg::SetBreath { breath: 99 }));
}
#[test]
fn initial_receipts_do_not_invent_manager_membership_or_seating() {
    let mut s = Trials::default();
    s.trial_id = 7;
    s.is_jury = true;
    s.jury_number = Some(3);
    for e in [
        TrialEvent::ImprisonOrTrialOffered {
            crime_value: -1,
            jail_minutes: 0,
        },
        TrialEvent::JuryInvited {
            defendant_name: vec![255; 129],
            trial_id: 8,
        },
        TrialEvent::JurySeated {
            raw_is_west: 255,
            trial_id: 9,
            court: -1,
            jury_number: 4,
        },
    ] {
        let (mut expected, before) = (s.clone(), s.clone());
        expected.last_receipt = Some(e.clone());
        let (_, ds) = s.apply_with_dispatch(&Event::Trial(e.clone()));
        assert!(ds.is_empty());
        assert_eq!(s, expected);
        assert_eq!((s.trial_id, s.is_jury, s.jury_number), (before.trial_id, before.is_jury, before.jury_number));
    }
    s.apply(&Event::CharacterSelected(Box::default()));
    assert_eq!(s, Trials::default());
}
#[test]
fn final_statement_task_is_outer_handler_action_even_after_wrong_trial_refusal() {
    let mut s = Trials::default();
    s.trial_id = 7;
    s.defendant_context = Some(7);
    s.phase = 1;
    let (_, ds) = s.apply_with_dispatch(&Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 999,
        phase: 4,
        current_jury: 0,
        remaining_time_ms: 0,
    }));
    assert_eq!(
        ds.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        [
            TrialNotice::Status { phase: 4, current_jury: 0 },
            TrialNotice::SkipFinalStatementTask { trial_id: 999 }
        ]
    );
    assert_eq!((ds[1].1.trial_id, ds[1].1.phase), (7, 1));
    let (_, ds) = s.apply_with_dispatch(&Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 999,
        phase: 5,
        current_jury: 0,
        remaining_time_ms: 0,
    }));
    assert_eq!(
        ds.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
        [TrialNotice::Status { phase: 5, current_jury: 0 }, TrialNotice::CloseSkipFinalStatementTask]
    );
    s.defendant_context = None;
    s.is_jury = true;
    let (_, ds) = s.apply_with_dispatch(&Event::Trial(TrialEvent::PhaseChanged {
        trial_id: 999,
        phase: 4,
        current_jury: 0,
        remaining_time_ms: 0,
    }));
    assert_eq!(ds.len(), 1);
}
#[test]
fn captured_task_choices_do_not_reuse_manager_trial_gate() {
    let s = Trials::default();
    for (command, op, body) in [
        (TrialCommand::ReplyImprisonOrTrial { request_trial: false }, 0x6f, vec![0]),
        (TrialCommand::ReplyImprisonOrTrial { request_trial: true }, 0x6f, vec![1]),
        (
            TrialCommand::ReplyJuryInvitation {
                accept: false,
                trial_id: u32::MAX,
            },
            0x71,
            vec![0, 255, 255, 255, 255],
        ),
        (TrialCommand::ReplyJuryInvitation { accept: true, trial_id: 0 }, 0x71, vec![1, 0, 0, 0, 0]),
        (TrialCommand::SkipFinalStatement { trial_id: u32::MAX }, 0x70, vec![255; 4]),
    ] {
        assert!(command.validate_received(&s).is_ok());
        assert_eq!(command.opcode(), op);
        assert_eq!(ClientMsg::Trial(command).encode_body(), body);
        let mut expected = vec![0xdd, 1, 0, 19, op as u8, 0];
        expected.extend(body);
        assert_eq!(ClientMsg::Trial(command).encode(19), expected);
    }
}
