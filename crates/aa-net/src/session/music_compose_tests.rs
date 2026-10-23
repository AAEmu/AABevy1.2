use super::*;
use crate::music::compose_tests::{ACTOR, UID, fixture, source};
use crate::music::tests::dd01;
use crate::{MusicComposeRequest, item_effect::CapturedBagItem};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn active(captured: &CapturedBagItem) -> (Session, UnboundedReceiver<Event>) {
    let (evt, events) = unbounded_channel();
    let mut s = Session::new(evt, ClientOptions::default());
    let mut spawn = crate::native_item_records::tests::spawn();
    spawn.obj_id = ACTOR;
    spawn.unit.obj_id = ACTOR;
    spawn.unit.kind = crate::model::UnitKind::Character { char_id: 42 };
    spawn.state.character.id = 42;
    s.phase = Phase::InWorld;
    s.selected = 42;
    s.self_obj = ACTOR;
    s.selected_state = Some(spawn.state.clone());
    s.emit(Event::CharacterSelected(Box::new(spawn.state.clone())));
    s.emit(Event::PlayerSpawn(Box::new(spawn)));
    s.emit(Event::InventoryInit {
        bag_slots: 130,
        bank_slots: 10,
    });
    let mut items = vec![None; 10];
    items[usize::from(captured.slot % 10)] = Some(captured.item.clone());
    s.emit(Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: captured.slot / 10,
        items,
        truncated: false,
    }));
    (s, events)
}

fn drain(events: &mut UnboundedReceiver<Event>) {
    while events.try_recv().is_ok() {}
}

async fn peer(s: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (conn, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    s.game = Some(conn.unwrap());
    accepted.unwrap().0
}

async fn read_frame(socket: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let length = socket.read_u16_le().await.unwrap();
        let mut bytes = vec![0; usize::from(length)];
        socket.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .unwrap()
}

async fn silent(socket: &mut TcpStream) {
    assert!(
        tokio::time::timeout(Duration::from_millis(20), socket.read_u8())
            .await
            .is_err()
    );
}

fn command(request: &MusicComposeRequest) -> Command {
    Command::ComposeMusic {
        character_id: 42,
        request: request.clone(),
    }
}

fn rejected(events: &mut UnboundedReceiver<Event>, request: &MusicComposeRequest) {
    assert!(
        matches!(events.try_recv(), Ok(Event::MusicComposeRejected { character_id: 42, request: actual, .. }) if &actual == request)
    );
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn compose_uses_full_received_main_bag_record_and_physical_slot() {
    for slot in [0, 9, 129] {
        let captured = source(slot);
        let request = MusicComposeRequest::new(captured.clone(), 10).unwrap();
        let (mut s, mut events) = active(&captured);
        drain(&mut events);
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await;
        assert_eq!(
            read_frame(&mut socket).await,
            dd01(0x52, &fixture(ACTOR, 10))
        );
        assert!(
            matches!(events.try_recv(), Ok(Event::MusicComposeRequested { character_id: 42, request: actual }) if actual == request)
        );
        assert!(events.try_recv().is_err());
        silent(&mut socket).await;
        // The physical slot participates in rechecking even though CS052 carries no slot.
        let other = MusicComposeRequest::new(source(if slot == 0 { 1 } else { 0 }), 10).unwrap();
        s.command(command(&other)).await;
        rejected(&mut events, &other);
        silent(&mut socket).await;
    }
}

#[tokio::test]
async fn compose_rejects_every_stale_item_field_and_unknown_or_nonbag_slots() {
    let captured = source(9);
    for case in 0..13 {
        let mut actual = captured.clone();
        match case {
            0 => actual.item.id = UID - 1,
            1 => actual.item.template_id = 28051,
            2 => actual.item.count += 1,
            3 => actual.item.grade += 1,
            4 => actual.item.made_unit_id += 1,
            5 => actual.item.detail_payload.as_mut().unwrap().push(1),
            _ => {}
        }
        let (mut s, mut events) = active(&actual);
        if case == 6 {
            // A matching UID in bank does not qualify a main-bag capture.
            s.emit(Event::InventoryInit {
                bag_slots: 130,
                bank_slots: 10,
            });
            let mut items = vec![None; 10];
            items[9] = Some(captured.item.clone());
            s.emit(Event::InventoryChunk(InvenChunk {
                container: 3,
                chunks: 1,
                start_chunk: 0,
                items,
                truncated: false,
            }));
        } else if case == 7 {
            s.emit(Event::InventoryInit {
                bag_slots: 130,
                bank_slots: 10,
            });
        } else if case == 8 {
            s.emit(Event::InventoryChunk(InvenChunk {
                container: 2,
                chunks: 1,
                start_chunk: 0,
                items: vec![None; 10],
                truncated: false,
            }));
        } else if case == 9 {
            s.emit(Event::InventoryInit {
                bag_slots: 0,
                bank_slots: 10,
            });
        } else if case == 10 {
            s.emit(Event::InventoryChunk(InvenChunk {
                container: 2,
                chunks: 1,
                start_chunk: 0,
                items: vec![],
                truncated: true,
            }));
        }
        drain(&mut events);
        let request = MusicComposeRequest::new(
            source(match case {
                11 => 130,
                12 => 255,
                _ => 9,
            }),
            0,
        )
        .unwrap();
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
    }
}

#[tokio::test]
async fn compose_guards_phase_header_owner_and_actor_before_any_write() {
    let captured = source(9);
    let request = MusicComposeRequest::new(captured.clone(), 0).unwrap();
    for phase in [
        Phase::Idle,
        Phase::Authenticating,
        Phase::WorldSelect,
        Phase::Queued,
        Phase::ReadyToConnect,
        Phase::Connecting,
        Phase::CharacterSelect,
        Phase::EnteringWorld,
        Phase::Leaving,
    ] {
        let (mut s, mut events) = active(&captured);
        s.phase = phase;
        drain(&mut events);
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
    }
    for case in 0..8 {
        let (mut s, mut events) = active(&captured);
        match case {
            0 => s.selected = 0,
            1 => s.selected = 43,
            2 => s.self_obj = 0,
            3 => s.self_obj = 0x1000000,
            4 => s.selected_state = None,
            5 => s.selected_state.as_mut().unwrap().character.id = 43,
            6 => *s.received_native_records.borrow_mut() = Default::default(),
            7 => {
                let mut header = s.selected_state.clone().unwrap();
                header.character.id = 43;
                s.received_native_records
                    .borrow_mut()
                    .observe(&Event::CharacterSelected(Box::new(header)));
            }
            _ => unreachable!(),
        }
        drain(&mut events);
        let mut socket = peer(&mut s).await;
        s.command(command(&request)).await;
        rejected(&mut events, &request);
        silent(&mut socket).await;
    }
    let (mut s, mut events) = active(&captured);
    drain(&mut events);
    let mut socket = peer(&mut s).await;
    for character_id in [0, 43] {
        s.command(Command::ComposeMusic {
            character_id,
            request: request.clone(),
        })
        .await;
        assert!(
            matches!(events.try_recv(), Ok(Event::MusicComposeRejected { character_id: actual, request: captured, .. }) if actual == character_id && captured == request)
        );
        assert!(events.try_recv().is_err());
        silent(&mut socket).await;
    }
}

#[tokio::test]
async fn compose_missing_and_failed_writes_never_claim_cast_or_upload_success() {
    let captured = source(9);
    let request = MusicComposeRequest::new(captured.clone(), 3).unwrap();
    for closed in [false, true] {
        let (mut s, mut events) = active(&captured);
        drain(&mut events);
        if closed {
            let _socket = peer(&mut s).await;
            s.game.as_mut().unwrap().shutdown().await;
        }
        s.command(command(&request)).await;
        let mut saw_rejection = false;
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                Event::MusicComposeRequested { .. }
                    | Event::MusicRequested { .. }
                    | Event::Notice(_)
                    | Event::SkillStarted(_)
                    | Event::SkillFired(_)
            ));
            if let Event::MusicComposeRejected {
                request: actual, ..
            } = event
            {
                assert_eq!(actual, request);
                saw_rejection = true;
            }
        }
        assert!(saw_rejection);
    }
}

#[tokio::test]
async fn compose_native_sequence_waits_for_received_fire_then_explicit_staged_upload() {
    let captured = source(9);
    let request = MusicComposeRequest::new(captured.clone(), 3).unwrap();
    let (mut s, mut events) = active(&captured);
    drain(&mut events);
    let mut socket = peer(&mut s).await;
    s.command(command(&request)).await;
    let body = fixture(ACTOR, 3);
    assert_eq!(read_frame(&mut socket).await, dd01(0x52, &body));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::MusicComposeRequested { .. })
    ));
    assert!(events.try_recv().is_err());
    silent(&mut socket).await;
    let mut identity = body[..4].to_vec();
    identity.extend_from_slice(&17u16.to_le_bytes());
    identity.extend_from_slice(&body[4..]);
    for fired in [false, true] {
        let mut receipt = identity.clone();
        if fired {
            receipt.extend_from_slice(&[10, 0, 10, 0, 0, 0, 0, 0, 0, 0]);
        } else {
            receipt.extend_from_slice(&[0; 6]);
        }
        let frame = dd01(if fired { 0xa2 } else { 0xa1 }, &receipt);
        socket.write_u16_le(frame.len() as u16).await.unwrap();
        socket.write_all(&frame).await.unwrap();
        let message = s.game.as_mut().unwrap().recv().await;
        s.game_msg(message).await;
        let event = events.try_recv().unwrap();
        if fired {
            assert!(
                matches!(event, Event::SkillFired(ref skill) if skill.skill_id == 22215 && skill.tl == 17
                && matches!(skill.raw_caster, crate::skill::SkillCasterData::Item { obj_id: ACTOR, item_id: UID, template_id: 27902, .. }))
            );
        } else {
            assert!(
                matches!(event, Event::SkillStarted(ref skill) if skill.skill_id == 22215 && skill.tl == 17)
            );
        }
        assert!(events.try_recv().is_err());
        silent(&mut socket).await; // no automatic upload on write, started or fired
    }
    // App owns this received-action handoff; net only writes the explicit command.
    let upload = crate::MusicCommand::UploadNotes {
        item_id: UID,
        slot_type: 2,
        slot: 9,
        title: "score".into(),
        notes: "MML@C;".into(),
    };
    s.command(Command::Music {
        character_id: 42,
        command: upload.clone(),
    })
    .await;
    let mut expected = dd01(0x122, &game::ClientMsg::Music(upload.clone()).encode_body());
    expected[3] = 1;
    assert_eq!(read_frame(&mut socket).await, expected);
    assert!(
        matches!(events.try_recv(), Ok(Event::MusicRequested { character_id: 42, command }) if command == upload)
    );
    assert!(events.try_recv().is_err());
    assert_eq!(
        s.received_native_records.borrow().item(2, 9),
        Some(&captured.item)
    );
    silent(&mut socket).await;
}
