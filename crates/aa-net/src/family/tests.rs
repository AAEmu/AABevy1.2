use super::*;
use crate::{
    game::{MAGIC, ServerMsg},
    wire::Writer,
};
fn member(id: u32) -> FamilyMember {
    FamilyMember {
        character_id: id,
        name: vec![0, 0xff, id as u8],
        role: if id == 1 { 1 } else { 7 },
        online: id == 1,
        title: vec![b't', id as u8],
    }
}
fn roster(id: u32, count: u32) -> FamilyRoster {
    FamilyRoster {
        family_id: id,
        members: (1..=count).map(member).collect(),
    }
}
fn roster_body(value: &FamilyRoster) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(value.family_id).i32(value.members.len() as i32);
    for m in &value.members {
        w.u32(m.character_id)
            .u16(m.name.len() as u16)
            .raw(&m.name)
            .u8(m.role)
            .bool(m.online)
            .u16(m.title.len() as u16)
            .raw(&m.title);
    }
    w.finish()
}
fn decoded(op: u16, body: &[u8]) -> FamilyEvent {
    let mut r = Reader::new(body);
    let value = FamilyEvent::read_lane(op, &mut r).unwrap();
    assert_eq!(r.remaining(), 0);
    value
}
#[test]
fn lane_snapshot_golden_keeps_raw_roles_strings_and_more_than_native_eight_rows() {
    let value = roster(0x10203040, 9);
    let bytes = roster_body(&value);
    assert_eq!(&bytes[..8], &[0x40, 0x30, 0x20, 0x10, 9, 0, 0, 0]);
    assert_eq!(&bytes[8..23], &[1, 0, 0, 0, 3, 0, 0, 255, 1, 1, 1, 2, 0, b't', 1]);
    assert_eq!(decoded(0x30, &bytes), FamilyEvent::Described(value.clone()));
    assert_eq!(decoded(0x2e, &bytes), FamilyEvent::Created(value.clone()));
    let mut added = bytes.clone();
    added.extend_from_slice(&(-3i32).to_le_bytes());
    assert_eq!(
        decoded(0x31, &added),
        FamilyEvent::MemberAdded {
            roster: value.clone(),
            added_index: -3
        }
    );
    for n in 0..bytes.len() {
        assert!(FamilyRoster::read_lane(&mut Reader::new(&bytes[..n])).is_err(), "cut {n}");
    }
    for count in [-1, i32::MAX] {
        let mut b = Writer::new();
        b.u32(1).i32(count);
        assert!(FamilyRoster::read_lane(&mut Reader::new(&b.finish())).is_err());
    }
    // A genuine DD04 following packet must begin after ALL nine lane rows.
    let mut raw = Writer::new();
    raw.u16(0).u16(0x30).raw(&bytes).u16(0).u16(crate::opcodes::sc::SCOnOffSnowPacket).bool(true);
    let raw = raw.finish();
    let n = raw.len() as u16;
    let mut frame = Writer::new();
    frame.raw(&[MAGIC, 4]).u16(2).u8(1).u16(n).u16(!n).raw(&raw);
    assert_eq!(
        ServerMsg::decode(&frame.finish()).unwrap(),
        ServerMsg::Batch(vec![ServerMsg::Family(FamilyEvent::Described(value)), ServerMsg::Snow(true)])
    );
}
#[test]
fn all_notice_layouts_and_every_truncation_are_bounded() {
    let cases: Vec<(u16, Vec<u8>, FamilyEvent)> = vec![
        (
            0x2d,
            vec![1, 0, 0, 0, 1, 0, 255, 2, 0, 0, 0, 1, 0, 0],
            FamilyEvent::Invitation(FamilyInvitation {
                inviter_id: 1,
                inviter_name: vec![255],
                family_id: 2,
                role: vec![0],
            }),
        ),
        (0x2f, vec![2, 0, 0, 0], FamilyEvent::Removed { family_id: 2 }),
        (
            0x32,
            vec![2, 0, 0, 0, 3, 0, 0, 0, 1],
            FamilyEvent::MemberRemoved {
                family_id: 2,
                character_id: 3,
                kicked: true,
            },
        ),
        (0x33, vec![2, 0, 0, 0, 3, 0, 0, 0], FamilyEvent::OwnerChanged { family_id: 2, character_id: 3 }),
        (
            0x34,
            vec![2, 0, 0, 0, 3, 0, 0, 0, 1, 0, 255],
            FamilyEvent::TitleChanged {
                family_id: 2,
                character_id: 3,
                title: vec![255],
            },
        ),
        (
            0x35,
            vec![2, 0, 0, 0, 3, 0, 0, 0, 1, 0, 255],
            FamilyEvent::NameChanged {
                family_id: 2,
                character_id: 3,
                name: vec![255],
            },
        ),
        (
            0x36,
            vec![3, 2, 1, 7, 1, 0, 255, 1, 0, 0],
            FamilyEvent::Title(FamilyTitle {
                unit_id: 0x010203,
                role: 7,
                owner: vec![255],
                title: vec![0],
            }),
        ),
        (
            0x37,
            vec![2, 0, 0, 0, 3, 0, 0, 0, 0],
            FamilyEvent::MemberOnline {
                family_id: 2,
                character_id: 3,
                online: false,
            },
        ),
    ];
    for (op, body, expected) in cases {
        assert_eq!(decoded(op, &body), expected);
        let mut frame = Writer::new();
        frame.raw(&[MAGIC, 1, 0, 1]).u16(op).raw(&body);
        assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(), ServerMsg::Family(expected));
        for n in 0..body.len() {
            assert!(FamilyEvent::read_lane(op, &mut Reader::new(&body[..n])).is_err(), "op{op:x} cut{n}");
        }
    }
    let mut malformed = Writer::new();
    malformed.u32(1).i32(1).u32(2).i16(-1);
    assert!(FamilyRoster::read_lane(&mut Reader::new(&malformed.finish())).is_err());
}
#[test]
fn native_keyed_cache_changes_only_known_rosters_members_and_actual_receipts() {
    let mut state = Families::default();
    let first = roster(1, 2);
    let second = roster(2, 1);
    assert!(state.apply(&Event::Family(FamilyEvent::Described(first.clone()))));
    assert!(!state.apply(&Event::Family(FamilyEvent::Described(first))));
    state.apply(&Event::Family(FamilyEvent::Created(second.clone())));
    assert!(!state.apply(&Event::Family(FamilyEvent::MemberOnline {
        family_id: 99,
        character_id: 1,
        online: false
    })));
    assert!(!state.apply(&Event::Family(FamilyEvent::TitleChanged {
        family_id: 1,
        character_id: 99,
        title: vec![9]
    })));
    assert!(state.apply(&Event::Family(FamilyEvent::OwnerChanged { family_id: 1, character_id: 2 })));
    assert_eq!(state.rosters[&1].members.iter().map(|m| m.role).collect::<Vec<_>>(), vec![0, 1]);
    assert!(state.rosters[&1].members[1].title.is_empty());
    assert_eq!(state.rosters[&2], second);
    // Native398a8860 clears all other roles even when the new owner's row is absent.
    assert!(state.apply(&Event::Family(FamilyEvent::OwnerChanged {
        family_id: 1,
        character_id: 99
    })));
    assert!(state.rosters[&1].members.iter().all(|m| m.role == 0));
    state.apply(&Event::Family(FamilyEvent::TitleChanged {
        family_id: 1,
        character_id: 2,
        title: vec![0xff],
    }));
    state.apply(&Event::Family(FamilyEvent::NameChanged {
        family_id: 1,
        character_id: 2,
        name: vec![0, 0xff],
    }));
    assert_eq!(state.rosters[&1].members[1].name, vec![0, 255]);
    state.apply(&Event::Family(FamilyEvent::MemberRemoved {
        family_id: 1,
        character_id: 1,
        kicked: true,
    }));
    assert_eq!(state.rosters[&1].members.len(), 1);
    state.apply(&Event::Family(FamilyEvent::Removed { family_id: 1 }));
    assert!(!state.rosters.contains_key(&1));
    assert_eq!(state.rosters[&2], second);
}
#[test]
fn lifecycle_clears_rosters_and_notice_history_without_invented_initial_family() {
    for reset in [
        Event::CharacterSelected(Box::default()),
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
        Event::LeftWorld { target: 0 },
        Event::Disconnected { message: String::new() },
    ] {
        let mut state = Families::default();
        assert!(state.rosters.is_empty());
        state.apply(&Event::Family(FamilyEvent::Described(roster(1, 1))));
        state.apply(&Event::Family(FamilyEvent::Invitation(FamilyInvitation {
            inviter_id: 1,
            inviter_name: vec![],
            family_id: 1,
            role: vec![],
        })));
        state.apply(&Event::Family(FamilyEvent::Title(FamilyTitle {
            unit_id: 1,
            role: 1,
            owner: vec![],
            title: vec![],
        })));
        assert!(state.apply(&reset));
        assert_eq!(state, Families::default());
        assert!(!state.apply(&reset));
    }
}
#[test]
fn world_state_folds_family_receipts_and_lifecycle() {
    let mut world = crate::state::WorldState::default();
    world.apply(&Event::Family(FamilyEvent::Described(roster(7, 9))));
    assert_eq!(world.families.rosters[&7].members.len(), 9);
    world.apply(&Event::Disconnected { message: String::new() });
    assert!(world.families.rosters.is_empty());
}
