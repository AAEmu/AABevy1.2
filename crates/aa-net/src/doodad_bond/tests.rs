use super::*;
use crate::{game::{ClientMsg, ServerMsg}, model::{CharacterState, ModelParams}, session::PlayerSpawn, wire::{Pos, Writer}};

#[path = "../../tests/fixtures/doodad-bond/lane.rs"]
pub(crate) mod lane;

pub(crate) fn bond() -> DoodadBond {
    DoodadBond { owner_obj_id: 0x0a0b0c, attach_point: 2, kind: 2, space: 1, spot: 0 }
}

pub(crate) fn selected() -> CharacterState {
    let mut state = CharacterState::default();
    state.character.id = 42;
    state
}

pub(crate) fn player(value: Option<DoodadBond>) -> PlayerSpawn {
    let mut unit = crate::state::tests::unit(7);
    unit.kind = UnitKind::Character { char_id: 42 };
    unit.doodad_bond = value;
    PlayerSpawn { obj_id: 7, state: selected(), unit }
}

pub(crate) fn request() -> UnbondDoodadRequest {
    UnbondDoodadRequest { character_id: 42, actor_obj_id: 7, bond: bond() }
}

/// Independently executed native post-local-send SC139 payloads, in wire field types.
pub(crate) fn rebond_goldens() -> Vec<DoodadBond> {
    include_str!("../../tests/fixtures/doodad-bond/rebond-native.tsv").lines().map(|line| {
        let row: Vec<_> = line.split('\t').collect();
        DoodadBond { owner_obj_id: row[1].parse().unwrap(), attach_point: row[2].parse().unwrap(),
            kind: row[3].parse().unwrap(), space: row[4].parse().unwrap(), spot: row[5].parse().unwrap() }
    }).collect()
}

fn active(value: Option<DoodadBond>) -> DoodadBonds {
    let mut state = DoodadBonds::default();
    state.apply(&Event::CharacterSelected(Box::new(selected())));
    state.apply(&Event::PlayerSpawn(Box::new(player(value))));
    state
}

pub(crate) fn dd01(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(body);
    w.finish()
}

pub(crate) fn dd04(messages: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut raw = Writer::new();
    for (opcode, body) in messages { raw.u16(0).u16(*opcode).raw(body); }
    let raw = raw.finish();
    let mut w = Writer::new();
    let len = raw.len() as u16;
    w.raw(&[0xdd, 4]).u16(messages.len() as u16).u8(1).u16(len).u16(!len).raw(&raw);
    w.finish()
}

pub(crate) fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes().as_chunks::<2>().0.iter().map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
}

fn golden() -> Vec<Vec<&'static str>> {
    include_str!("../../tests/fixtures/doodad-bond/native.tsv").lines().map(|line| line.split('\t').collect()).collect()
}

#[test]
fn original_archives_and_actual_lane_writers_readers_agree() {
    use lane::*;
    use crate::music::tests::lane::{Marshal, PacketStream};
    let mut messages = Vec::new();
    let mut expected = Vec::new();
    for row in golden() {
        let (opcode, body) = (u16::from_str_radix(row[1], 16).unwrap(), bytes(row[9]));
        let parse = |i: usize| row[i].parse::<u32>().unwrap();
        let (actor, character, owner) = (parse(2), parse(3), parse(4));
        let b = DoodadBond { owner_obj_id: owner, attach_point: parse(5) as u8, kind: parse(6) as u8, space: row[7].parse().unwrap(), spot: row[8].parse().unwrap() };
        let mut lane_bytes = PacketStream::new();
        let receipt = match row[0] {
            "bond" => {
                SCBondDoodadPacket { unit_obj_id: actor, bond: BondDoodad { owner_obj_id: b.owner_obj_id, attach_point: b.attach_point, kind: b.kind, space: b.space, spot: b.spot } }.write(&mut lane_bytes);
                Some(DoodadBondEvent::Bonded { unit_obj_id: actor, bond: b })
            }
            "unbond" => {
                SCUnbondDoodadPacket { character_obj_id: actor, character_id: character, doodad_obj_id: owner }.write(&mut lane_bytes);
                Some(DoodadBondEvent::Unbonded { character_obj_id: actor, character_id: character, doodad_obj_id: owner })
            }
            "request" => {
                let request = UnbondDoodadRequest { character_id: character, actor_obj_id: actor, bond: b };
                let actual = ClientMsg::UnbondDoodad(request);
                assert_eq!(actual.opcode(), opcode);
                assert_eq!(actual.encode_body(), body);
                let mut stream = PacketStream::from_bytes(actual.encode_body());
                assert_eq!(CSUnbondDoodadPacket::read(&mut stream).unwrap(), CSUnbondDoodadPacket { character_obj_id: actor, doodad_obj_id: owner });
                assert_eq!(stream.remaining(), 0);
                for cut in 0..body.len() { assert!(CSUnbondDoodadPacket::read(&mut PacketStream::from_bytes(body[..cut].to_vec())).is_err()); }
                None
            }
            "absent" => Some(DoodadBondEvent::BondAbsent { unit_obj_id: actor }),
            _ => unreachable!(),
        };
        if let Some(receipt) = receipt {
            if row[0] != "absent" { assert_eq!(lane_bytes.into_bytes(), body); }
            let expected_receipt = ServerMsg::DoodadBond(receipt.clone());
            assert_eq!(ServerMsg::decode(&dd01(opcode, &body)).unwrap(), expected_receipt);
            assert_eq!(expected_receipt.name(), receipt.name());
            for cut in 0..body.len() {
                assert!(ServerMsg::decode(&dd01(opcode, &body[..cut])).is_err(), "{} prefix {cut}", row[0]);
            }
            messages.push((opcode, body));
            expected.push(expected_receipt);
        }
    }
    messages.push((sc::SCOnOffSnowPacket, vec![1]));
    expected.push(ServerMsg::Snow(true));
    assert_eq!(ServerMsg::decode(&dd04(&messages)).unwrap(), ServerMsg::Batch(expected));
}

/// Exact lane character-state field order, with full native-format buff lists at the end.
pub(crate) fn character_body(value: Option<DoodadBond>) -> Vec<u8> {
    let mut w = Writer::new();
    w.bc(7).string("Seated").u8(0).u32(42).i64(0).string("");
    w.position(Pos::new(1.0, 2.0, 3.0)).f32(1.0).u8(30).u32(101);
    w.raw(&[0; 112]);
    ModelParams::default_face().write(&mut w);
    w.bc(0).i32(10000).i32(5000).u8(255);
    if let Some(b) = value { w.u8(b.attach_point).bc(b.owner_obj_id).u8(b.kind).i32(b.space).i32(b.spot); }
    else { w.u8(255); }
    w.u8(0).bool(false).u8(0).u8(0).i32(0).raw(&[0, 0, 32]).u8(1);
    w.pisc(&[0, 0, 0, 0]).pisc(&[101, 0, 0, 0]).pisc(&[0, 0]).u16(0);
    for _ in 0..10 { w.i32(0).u8(255); }
    w.u8(0).bc(0).raw(&[0; 10]).i32(0).raw(&[0; 24]);
    w.u8(1);
    crate::buff::tests::net_buff(&mut w, 3, 5068);
    w.u8(0).u8(0);
    w.finish()
}

#[test]
fn full_character_snapshot_retains_bond_and_buff_tail_and_rejects_every_prefix() {
    for value in [None, Some(bond()), Some(DoodadBond { space: i32::MIN, spot: i32::MAX, ..bond() })] {
        let body = character_body(value);
        let mut reader = Reader::new(&body);
        let unit = Unit::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        assert_eq!(unit.doodad_bond, value);
        assert_eq!((unit.buffs[0].buff_id, unit.buffs[0].index), (5068, 3));
        assert!(unit.attached_to.is_none());
        for cut in 0..body.len() { assert!(Unit::read(&mut Reader::new(&body[..cut])).is_err(), "character prefix {cut}/{}", body.len()); }
        let expected = ServerMsg::UnitState(Box::new(unit));
        assert_eq!(ServerMsg::decode(&dd01(sc::SCUnitStatePacket, &body)).unwrap(), expected);
        assert_eq!(ServerMsg::decode(&dd04(&[(sc::SCUnitStatePacket, body), (sc::SCOnOffSnowPacket, vec![1])])).unwrap(), ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)]));
    }
}

#[test]
fn own_history_requires_received_identity_and_exact_current_bond() {
    let mut state = active(None);
    assert!(state.own_known());
    assert!(state.own().is_none());
    assert!(!state.matches_request(&request()));
    let attach = Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: 7, bond: bond() });
    assert!(state.apply(&attach));
    assert!(state.matches_request(&request()));
    assert!(!state.apply(&attach), "duplicate is idempotent");
    assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: 7, bond: DoodadBond { spot: 3, ..bond() } })), "native already-bonded gate");
    for (actor, character, owner) in [(8, 42, bond().owner_obj_id), (7, 43, bond().owner_obj_id), (7, 42, 10)] {
        assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: actor, character_id: character, doodad_obj_id: owner })));
        assert!(state.matches_request(&request()));
    }
    let mut stale = request();
    stale.bond.space = -1;
    assert!(!state.matches_request(&stale));
    assert!(state.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRequested(request()))));
    assert!(state.own_known()); assert_eq!(state.own(), Some(&bond()));
    for event in [Event::DoodadBond(DoodadBondEvent::UnbondRequested(request())), Event::DoodadBond(DoodadBondEvent::UnbondRejected { request: request(), message: "test".into() }), Event::DoodadBond(DoodadBondEvent::BondAbsent { unit_obj_id: 7 })] {
        assert!(!state.apply(&event)); assert!(state.matches_request(&request()));
    }
    assert!(state.apply(&Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: 7, character_id: 42, doodad_obj_id: bond().owner_obj_id })));
    assert!(state.own_known()); assert!(state.own().is_none());
    assert!(!state.matches_request(&request()));
    assert!(!state.apply(&Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: 8, bond: bond() })), "no phantom actor");
}

#[path = "rebond_tests.rs"]
mod rebond_tests;

#[test]
fn loss_is_unknown_and_only_actual_own_receipts_repair_it() {
    for gap in [Event::GameDecodeFailed { message: "truncated".into() }, Event::UnhandledGamePacket { level: 4, opcode: 0x777, len: 12 }] {
        for value in [None, Some(bond())] {
            let mut state = active(value);
            assert!(state.apply(&gap));
            assert!(!state.own_known()); assert!(state.own().is_none());
            state.apply(&Event::DoodadBond(DoodadBondEvent::Unbonded { character_obj_id: 7, character_id: 42, doodad_obj_id: bond().owner_obj_id }));
            state.apply(&Event::DoodadBond(DoodadBondEvent::BondAbsent { unit_obj_id: 7 }));
            state.apply(&Event::DoodadBond(DoodadBondEvent::UnbondRequested(request())));
            let mut wrong = player(None).unit;
            wrong.kind = UnitKind::Character { char_id: 43 };
            state.apply(&Event::OwnUnitSnapshot(Box::new(wrong)));
            assert!(!state.own_known());
            state.apply(&Event::OwnUnitSnapshot(Box::new(player(value).unit)));
            assert!(state.own_known()); assert_eq!(state.own(), value.as_ref());
            state.apply(&gap);
            state.apply(&Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: 7, bond: bond() }));
            assert!(state.own_known()); assert!(state.matches_request(&request()));
        }
    }
    for no_gap in [Event::UnhandledGamePacket { level: 1, opcode: 0x777, len: 12 }, Event::UnhandledGamePacket { level: 4, opcode: 0x777, len: 0 }] {
        let mut state = active(Some(bond()));
        assert!(!state.apply(&no_gap)); assert!(state.matches_request(&request()));
    }
}

#[test]
fn removal_reuse_and_all_session_teardowns_prevent_stale_unbond() {
    for event in [Event::UnitsRemoved(vec![7]), Event::DoodadsRemoved(vec![bond().owner_obj_id]), Event::LeftWorld { target: 1 }, Event::Disconnected { message: "test".into() }, Event::Kicked { reason: 1, message: "test".into() }, Event::CharacterSelected(Box::new(selected())), Event::PhaseChanged(Phase::CharacterSelect)] {
        let mut state = active(Some(bond()));
        state.apply(&event);
        assert!(!state.own_known(), "{event:?}"); assert!(!state.matches_request(&request()));
        assert!(state.own().is_none());
    }
    let mut state = active(Some(bond()));
    state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(7))));
    assert!(!state.own_known()); assert!(!state.matches_request(&request()));
    let mut state = active(Some(bond()));
    let mut remote = player(Some(bond())).unit;
    remote.obj_id = 8;
    remote.kind = UnitKind::Character { char_id: 43 };
    state.apply(&Event::UnitSpawned(Box::new(remote)));
    assert_eq!(state.by_unit.len(), 2);
    state.apply(&Event::DoodadsRemoved(vec![bond().owner_obj_id]));
    assert!(state.by_unit.is_empty()); assert!(!state.own_known());
    state.apply(&Event::LeftWorld { target: 1 });
    state.apply(&Event::UnitSpawned(Box::new(player(Some(bond())).unit)));
    state.apply(&Event::DoodadBond(DoodadBondEvent::Bonded { unit_obj_id: 7, bond: bond() }));
    assert!(state.by_unit.is_empty());
}
