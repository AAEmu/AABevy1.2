use super::*;
use crate::{DoodadBond, DoodadBondEvent as E, WorldState};
use crate::doodad_bond::tests::{bond, character_body, dd01, dd04, player, rebond_goldens, request, selected};
use crate::opcodes::sc;
use tokio::{io::AsyncReadExt, net::{TcpListener, TcpStream}};

fn active(value: Option<DoodadBond>) -> (Session, UnboundedReceiver<Event>, WorldState) {
    let (tx, mut events) = unbounded_channel();
    let mut session = Session::new(tx, ClientOptions::default());
    session.phase = Phase::InWorld;
    session.selected = 42;
    session.self_obj = 7;
    session.selected_state = Some(selected());
    session.emit(Event::CharacterSelected(Box::new(selected())));
    session.emit(Event::PlayerSpawn(Box::new(player(value))));
    let mut world = WorldState::default();
    drain(&mut events, &mut world);
    (session, events, world)
}

fn drain(events: &mut UnboundedReceiver<Event>, world: &mut WorldState) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(event) = events.try_recv() { world.apply(&event); out.push(event); }
    out
}

async fn peer(session: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (conn, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    session.game = Some(conn.unwrap());
    accepted.unwrap().0
}

async fn frame(peer: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let len = peer.read_u16_le().await.unwrap();
        let mut bytes = vec![0; len as usize];
        peer.read_exact(&mut bytes).await.unwrap();
        bytes
    }).await.unwrap()
}

#[tokio::test]
async fn exact_unbond_write_is_only_a_request_until_actual_matching_receipt() {
    let (mut session, mut events, mut world) = active(Some(bond()));
    let mut socket = peer(&mut session).await;
    session.command(Command::UnbondDoodad(request())).await;
    let bytes = frame(&mut socket).await;
    assert_eq!(bytes, dd01(0xcd, &[7, 0, 0, 0x0c, 0x0b, 0x0a]));
    let out = drain(&mut events, &mut world);
    assert!(matches!(out.as_slice(), [Event::DoodadBond(E::UnbondRequested(actual))] if *actual == request()));
    assert_eq!(world.doodad_bonds.own(), Some(&bond()));
    assert!(session.received_doodad_bonds.borrow().matches_request(&request()));
    // A stale/foreign response cannot settle the captured request.
    for (actor, character, owner) in [(8, 42, bond().owner_obj_id), (7, 43, bond().owner_obj_id), (7, 42, 99)] {
        let mut body = crate::wire::Writer::new();
        body.bc(actor).u32(character).bc(owner);
        session.game_msg(Ok(game::ServerMsg::decode(&dd01(sc::SCUnbondDoodadPacket, &body.finish())).unwrap())).await;
        drain(&mut events, &mut world);
        assert_eq!(world.doodad_bonds.own(), Some(&bond()));
    }
    session.game_msg(Ok(game::ServerMsg::decode(&dd01(sc::SCUnbondDoodadPacket, &[7, 0, 0, 42, 0, 0, 0, 12, 11, 10])).unwrap())).await;
    assert!(matches!(drain(&mut events, &mut world).as_slice(), [Event::DoodadBond(E::Unbonded { character_obj_id:7, character_id:42, doodad_obj_id:0x0a0b0c })]));
    assert!(world.doodad_bonds.own_known()); assert!(world.doodad_bonds.own().is_none());
    session.command(Command::UnbondDoodad(request())).await;
    assert!(matches!(events.try_recv(), Ok(Event::DoodadBond(E::UnbondRejected { .. }))));
}

#[tokio::test]
async fn full_capture_identity_phase_socket_and_all_bond_fields_guard_writes() {
    for case in 0..20 {
        let (mut session, mut events, _) = active(Some(bond()));
        let mut socket = peer(&mut session).await;
        let mut captured = request();
        match case {
            0 => session.phase = Phase::Leaving,
            1 => captured.character_id = 43,
            2 => captured.actor_obj_id = 8,
            3 => captured.bond.owner_obj_id = 99,
            4 => captured.bond.attach_point = 3,
            5 => captured.bond.kind = 3,
            6 => captured.bond.space = -1,
            7 => captured.bond.spot = -1,
            8 => session.selected_state = None,
            9 => session.selected_state.as_mut().unwrap().character.id = 43,
            10 => captured.actor_obj_id = 0,
            11 => { captured.actor_obj_id = 0x1000000; session.self_obj = captured.actor_obj_id; }
            12 => captured.bond.owner_obj_id = 0,
            13 => captured.bond.owner_obj_id = 0x1000000,
            14 => session.emit(Event::GameDecodeFailed { message: "test loss".into() }),
            15 => session.emit(Event::UnhandledGamePacket { level:4, opcode:0x777, len:10 }),
            16 => session.emit(Event::UnitsRemoved(vec![7])),
            17 => session.emit(Event::DoodadsRemoved(vec![bond().owner_obj_id])),
            18 => session.emit(Event::CharacterSelected(Box::new(selected()))),
            19 => session.game = None,
            _ => unreachable!(),
        }
        while events.try_recv().is_ok() {}
        session.command(Command::UnbondDoodad(captured)).await;
        assert!(matches!(events.try_recv(), Ok(Event::DoodadBond(E::UnbondRejected { request: actual, .. })) if actual == captured), "case {case}");
        assert!(events.try_recv().is_err());
        let mut byte = [0];
        match tokio::time::timeout(Duration::from_millis(5), socket.read(&mut byte)).await {
            Err(_) | Ok(Ok(0)) => {},
            other => panic!("rejected case {case} wrote {other:?}"),
        }
    }
}

#[tokio::test]
async fn repeated_actual_own_snapshot_repairs_only_native_buff_and_bond_history() {
    let (mut session, mut events, mut world) = active(Some(bond()));
    let old_player = world.player.spawn.clone();
    session.game_msg(Ok(game::ServerMsg::DecodeFailed { message: "lost batch entry".into() })).await;
    drain(&mut events, &mut world);
    assert!(!world.doodad_bonds.own_known()); assert!(world.buffs.native.of(7).is_none());
    let unit = match game::ServerMsg::decode(&dd01(sc::SCUnitStatePacket, &character_body(None))).unwrap() {
        game::ServerMsg::UnitState(unit) => unit,
        _ => unreachable!(),
    };
    session.game_msg(Ok(game::ServerMsg::UnitState(unit.clone()))).await;
    assert!(matches!(drain(&mut events, &mut world).as_slice(), [Event::OwnUnitSnapshot(actual)] if actual == &unit));
    assert!(world.doodad_bonds.own_known()); assert!(world.doodad_bonds.own().is_none());
    assert_eq!(world.buffs.native.of(7).unwrap()[0].buff.buff_id, 5068);
    assert_eq!(world.player.spawn, old_player, "no player UI/transform/respawn rewrite");
    assert!(world.units.by_id.is_empty());
    assert!(world.buffs.of(7).is_empty(), "raw lane HUD list is unchanged");
    assert!(session.received_doodad_bonds.borrow().own_known());
    for case in 0..4 {
        let mut other = unit.clone();
        match case {
            0 => other.kind = UnitKind::Character { char_id:43 },
            1 => session.phase = Phase::Leaving,
            2 => session.selected_state.as_mut().unwrap().character.id = 43,
            3 => other.kind = UnitKind::Npc { template_id:1, owner_id:0 },
            _ => unreachable!(),
        }
        session.game_msg(Ok(game::ServerMsg::UnitState(other))).await;
        assert!(events.try_recv().is_err(), "invalid repeated-own receipt {case}");
        session.phase = Phase::InWorld;
        session.selected_state.as_mut().unwrap().character.id = 42;
    }
}

#[tokio::test]
async fn native_buff_and_bond_snapshot_recovery_reject_wrong_local_identity() {
    let (_, _, mut world) = active(Some(bond()));
    world.apply(&Event::GameDecodeFailed { message:"loss".into() });
    for (id, char_id) in [(8,42), (7,43)] {
        let mut unit = player(None).unit;
        unit.obj_id=id; unit.kind=UnitKind::Character {char_id};
        world.apply(&Event::OwnUnitSnapshot(Box::new(unit)));
        assert!(!world.doodad_bonds.own_known()); assert!(world.buffs.native.of(7).is_none());
    }
    world.apply(&Event::OwnUnitSnapshot(Box::new(player(None).unit)));
    assert!(world.doodad_bonds.own_known()); assert!(world.buffs.native.of(7).unwrap().is_empty());
    world.apply(&Event::LeftWorld {target:1});
    world.apply(&Event::OwnUnitSnapshot(Box::new(player(None).unit)));
    assert!(!world.doodad_bonds.own_known()); assert!(world.buffs.native.of(7).is_none());
}

#[tokio::test]
async fn bond_packets_in_mixed_batches_and_transport_loss_follow_lifecycle() {
    let (mut session, mut events, mut world) = active(None);
    let body = [7,0,0,2,12,11,10,2,1,0,0,0,0,0,0,0];
    let batch = game::ServerMsg::decode(&dd04(&[(sc::SCBondDoodadPacket, body.to_vec()), (sc::SCOnOffSnowPacket, vec![1])])).unwrap();
    session.game_msg(Ok(batch)).await;
    assert!(matches!(drain(&mut events, &mut world).as_slice(), [Event::DoodadBond(E::Bonded { .. }), Event::Snow(true)]));
    assert_eq!(world.doodad_bonds.own(),Some(&bond()));
    session.game_msg(Err(NetError::Wire(crate::wire::WireError::Invalid("truncated".into())))).await;
    drain(&mut events, &mut world);
    assert!(!world.doodad_bonds.own_known());
    session.game_msg(Ok(game::ServerMsg::DoodadBond(E::Bonded {unit_obj_id:7,bond:bond()}))).await;
    drain(&mut events, &mut world);
    assert!(world.doodad_bonds.own_known());
    session.left_world(1).await;
    drain(&mut events, &mut world);
    assert!(!world.doodad_bonds.own_known()); assert!(world.doodad_bonds.by_unit.is_empty());
    session.game_msg(Ok(game::ServerMsg::DoodadBond(E::Bonded {unit_obj_id:7,bond:bond()}))).await;
    assert!(events.try_recv().is_err(), "lobby packet cannot revive old seating");
}

#[tokio::test]
async fn kick_disconnect_and_reselect_clear_private_and_public_received_state() {
    for case in 0..3 {
        let (mut session, mut events, mut world) = active(Some(bond()));
        match case {
            0 => session.game_msg(Err(NetError::Closed)).await,
            1 => session.game_msg(Ok(game::ServerMsg::Kicked { reason:1, message:"test".into() })).await,
            2 => session.game_msg(Ok(game::ServerMsg::CharacterState(Box::new(selected())))).await,
            _ => unreachable!(),
        }
        drain(&mut events, &mut world);
        assert!(!world.doodad_bonds.own_known()); assert!(world.doodad_bonds.own().is_none());
        assert!(!session.received_doodad_bonds.borrow().matches_request(&request()));
        assert!(world.buffs.native.of(7).is_none());
        session.command(Command::UnbondDoodad(request())).await;
        assert!(matches!(events.try_recv(), Ok(Event::DoodadBond(E::UnbondRejected { .. }))));
    }
}

fn record_rebond(session: &Session, events: &mut UnboundedReceiver<Event>, world: &mut WorldState, history: &mut Vec<Event>) -> Vec<Event> {
    let emitted = drain(events, world);
    assert_eq!(*session.received_doodad_bonds.borrow(), world.doodad_bonds);
    history.extend(emitted.iter().cloned());
    emitted
}

fn rebond_body(value: DoodadBond) -> Vec<u8> {
    let mut writer = crate::wire::Writer::new();
    writer.bc(7).u8(value.attach_point).bc(value.owner_obj_id).u8(value.kind).i32(value.space).i32(value.spot);
    writer.finish()
}

#[tokio::test]
async fn fresh_rebond_before_ack_keeps_session_world_and_bevy_consistent() {
    for next in rebond_goldens() {
        let (mut session, mut events, mut world) = active(Some(bond()));
        let mut history = vec![Event::CharacterSelected(Box::new(selected())), Event::PlayerSpawn(Box::new(player(Some(bond()))))];
        let mut socket = peer(&mut session).await;
        let old_history = world.doodad_bonds.by_unit.clone();
        session.command(Command::UnbondDoodad(request())).await;
        assert_eq!(frame(&mut socket).await, dd01(0xcd, &[7, 0, 0, 0x0c, 0x0b, 0x0a]));
        let emitted = record_rebond(&session, &mut events, &mut world, &mut history);
        assert!(matches!(emitted.as_slice(), [Event::DoodadBond(E::UnbondRequested(actual))] if *actual == request()));
        assert_eq!(world.doodad_bonds.by_unit, old_history);
        assert_eq!(world.doodad_bonds.own(), Some(&bond())); assert!(world.doodad_bonds.own_known());

        // Raw sentinel and unmatched ACKs are published without consuming eligibility.
        let mut batch = vec![(sc::SCBondDoodadPacket, vec![7, 0, 0, 255])];
        for (actor, character, owner) in [(8, 42, bond().owner_obj_id), (7, 43, bond().owner_obj_id), (7, 42, 999)] {
            let mut writer = crate::wire::Writer::new();
            writer.bc(actor).u32(character).bc(owner);
            batch.push((sc::SCUnbondDoodadPacket, writer.finish()));
        }
        session.game_msg(Ok(game::ServerMsg::decode(&dd04(&batch)).unwrap())).await;
        assert_eq!(record_rebond(&session, &mut events, &mut world, &mut history).len(), 4);
        assert_eq!(world.doodad_bonds.by_unit, old_history);

        // Both raw SC139s are delivered, but only the first can replace received history.
        let conflict = DoodadBond { owner_obj_id: 444, ..next };
        let batch = dd04(&[(sc::SCBondDoodadPacket, rebond_body(next)),
            (sc::SCBondDoodadPacket, rebond_body(conflict)), (sc::SCOnOffSnowPacket, vec![1])]);
        session.game_msg(Ok(game::ServerMsg::decode(&batch).unwrap())).await;
        assert_eq!(record_rebond(&session, &mut events, &mut world, &mut history).len(), 3);
        assert_eq!(world.doodad_bonds.own(), Some(&next)); assert!(world.doodad_bonds.own_known());
        if next != bond() {
            session.command(Command::UnbondDoodad(request())).await;
            assert!(matches!(record_rebond(&session, &mut events, &mut world, &mut history).as_slice(),
                [Event::DoodadBond(E::UnbondRejected { .. })]));
        }
        let capture = crate::UnbondDoodadRequest { bond: next, ..request() };
        assert!(session.received_doodad_bonds.borrow().matches_request(&capture));
        session.command(Command::UnbondDoodad(capture)).await;
        let mut body = crate::wire::Writer::new(); body.bc(7).bc(next.owner_obj_id);
        let mut expected = dd01(0xcd, &body.finish()); expected[3] = 1;
        assert_eq!(frame(&mut socket).await, expected, "stale rejection wrote no extra packet");
        assert!(matches!(record_rebond(&session, &mut events, &mut world, &mut history).as_slice(),
            [Event::DoodadBond(E::UnbondRequested(actual))] if *actual == capture));
        assert_eq!(world.doodad_bonds.own(), Some(&next));

        // Old B's ACK rejects only when the owner changed. Same-owner epochs are absent on wire.
        session.game_msg(Ok(game::ServerMsg::DoodadBond(E::Unbonded {
            character_obj_id: 7, character_id: 42, doodad_obj_id: bond().owner_obj_id,
        }))).await;
        record_rebond(&session, &mut events, &mut world, &mut history);
        assert_eq!(world.doodad_bonds.own(), if next.owner_obj_id == bond().owner_obj_id { None } else { Some(&next) });
        session.game_msg(Ok(game::ServerMsg::DoodadBond(E::Unbonded {
            character_obj_id: 7, character_id: 42, doodad_obj_id: next.owner_obj_id,
        }))).await;
        record_rebond(&session, &mut events, &mut world, &mut history);
        assert!(world.doodad_bonds.own().is_none()); assert!(world.doodad_bonds.own_known());
        #[cfg(feature = "bevy")]
        crate::plugin::doodad_bond_plugin_tests::assert_rebond_event_history(&history, &world.doodad_bonds);
    }
}

#[tokio::test]
async fn actual_own_snapshot_supersedes_send_eligibility_in_every_fold() {
    for snapshot in [None, Some(bond()), Some(DoodadBond { owner_obj_id: 999, ..bond() })] {
        let (mut session, mut events, mut world) = active(Some(bond()));
        let mut history = vec![Event::CharacterSelected(Box::new(selected())), Event::PlayerSpawn(Box::new(player(Some(bond()))))];
        let mut socket = peer(&mut session).await;
        session.command(Command::UnbondDoodad(request())).await;
        frame(&mut socket).await;
        record_rebond(&session, &mut events, &mut world, &mut history);
        session.game_msg(Ok(game::ServerMsg::decode(&dd01(sc::SCUnitStatePacket, &character_body(snapshot))).unwrap())).await;
        assert!(matches!(record_rebond(&session, &mut events, &mut world, &mut history).as_slice(), [Event::OwnUnitSnapshot(_)]));
        assert_eq!(world.doodad_bonds.own(), snapshot.as_ref()); assert!(world.doodad_bonds.own_known());
        let next = DoodadBond { owner_obj_id: 444, ..bond() };
        session.game_msg(Ok(game::ServerMsg::decode(&dd01(sc::SCBondDoodadPacket, &rebond_body(next))).unwrap())).await;
        record_rebond(&session, &mut events, &mut world, &mut history);
        assert_eq!(world.doodad_bonds.own(), snapshot.as_ref().or(Some(&next)));
        #[cfg(feature = "bevy")]
        crate::plugin::doodad_bond_plugin_tests::assert_rebond_event_history(&history, &world.doodad_bonds);
    }
}
