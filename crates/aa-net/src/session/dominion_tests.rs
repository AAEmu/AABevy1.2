use super::*;
use crate::dominion::territory_tests::{cases, dd01, row};
use crate::{Dominion, DominionCommand as C, DominionEvent, NoticeEvent, WorldState};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn active() -> (Session, UnboundedReceiver<Event>) {
    let (tx, events) = unbounded_channel();
    let mut session = Session::new(tx, ClientOptions::default());
    session.phase = Phase::InWorld;
    session.selected = 42;
    // These requests carry no actor ID and require only the accepted persistent-header guard.
    let mut header = CharacterState::default();
    header.character.id = 42;
    session.selected_state = Some(header);
    (session, events)
}

async fn peer(session: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (connection, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    session.game = Some(connection.unwrap());
    accepted.unwrap().0
}

async fn frame(socket: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let length = socket.read_u16_le().await.unwrap();
        let mut payload = vec![0; usize::from(length)];
        socket.read_exact(&mut payload).await.unwrap();
        payload
    })
    .await
    .unwrap()
}

async fn silent(socket: &mut TcpStream) {
    assert!(
        tokio::time::timeout(Duration::from_millis(15), socket.read_u8())
            .await
            .is_err()
    );
}

fn rejected(events: &mut UnboundedReceiver<Event>, character_id: u32, command: C) {
    assert!(
        matches!(events.try_recv(), Ok(Event::DominionRequestRejected { character_id: id, command: actual, message })
        if id == character_id && actual == command && !message.is_empty())
    );
    assert!(events.try_recv().is_err());
}

async fn receive(session: &mut Session, socket: &mut TcpStream, payload: &[u8]) {
    socket.write_u16_le(payload.len() as u16).await.unwrap();
    socket.write_all(payload).await.unwrap();
    let message = tokio::time::timeout(
        Duration::from_secs(2),
        session.game.as_mut().unwrap().recv(),
    )
    .await
    .unwrap();
    session.game_msg(message).await;
}

fn world() -> WorldState {
    let mut world = WorldState::default();
    world.apply(&Event::Dominion(DominionEvent::Data {
        data: row(),
        newly_declared: 0,
        final_data_by_request: 1,
    }));
    world
}

#[tokio::test]
async fn territory_socket_requests_have_exact_native_frames_and_only_local_write_outcomes() {
    let (mut session, mut events) = active();
    let mut socket = peer(&mut session).await;
    let mut world = world();
    let before = world.dominion.clone();
    let expeditions = world.expeditions.clone();
    let names = world.houses.character_briefs.clone();
    let bag = session.received_inventory.borrow().bag.clone();
    let bank = session.received_inventory.borrow().bank.clone();
    let records = session.received_native_records.borrow().clone();
    let commands = [
        (
            C::ChangeTaxRate {
                zone_group_id: 0xffff,
                tax_rate: 0,
            },
            vec![0x12, 0, 0xff, 0xff, 0, 0, 0, 0],
        ),
        (
            C::ChangeTaxRate {
                zone_group_id: 0xffff,
                tax_rate: 50,
            },
            vec![0x12, 0, 0xff, 0xff, 50, 0, 0, 0],
        ),
        (
            C::ChangeNationalTaxRate {
                zone_group_id: 0xffff,
                tax_rate: 0,
            },
            vec![0x13, 0, 0xff, 0xff, 0, 0, 0, 0],
        ),
        (
            C::ChangeNationalTaxRate {
                zone_group_id: 0xffff,
                tax_rate: 1000,
            },
            vec![0x13, 0, 0xff, 0xff, 0xe8, 3, 0, 0],
        ),
        (
            C::RequestCharacterBrief {
                requested_id: 0xffffffff,
            },
            vec![0x2d, 0, 0xff, 0xff, 0xff, 0xff],
        ),
    ];
    for (counter, (command, body)) in commands.into_iter().enumerate() {
        // A persistent character ID is not a BC actor ID; test its full width too.
        session.selected = u32::MAX;
        session.selected_state.as_mut().unwrap().character.id = u32::MAX;
        session
            .command(Command::Dominion {
                character_id: u32::MAX,
                command,
            })
            .await;
        let mut expected = vec![0xdd, 1, 0, counter as u8];
        expected.extend(body);
        assert_eq!(frame(&mut socket).await, expected);
        let event = events.try_recv().unwrap();
        assert!(
            matches!(event, Event::DominionRequested { character_id: u32::MAX, command: actual } if actual == command)
        );
        world.apply(&event);
        assert!(events.try_recv().is_err());
        assert!(
            session.waiting.is_none(),
            "no invented transaction timeout or response wait"
        );
        session.check_timeout();
        assert!(events.try_recv().is_err());
    }
    assert_eq!(world.dominion, before);
    assert_eq!(world.expeditions, expeditions);
    assert_eq!(world.houses.character_briefs, names);
    assert_eq!(session.received_inventory.borrow().bag, bag);
    assert_eq!(session.received_inventory.borrow().bank, bank);
    assert_eq!(*session.received_native_records.borrow(), records);
    silent(&mut socket).await;
}

#[tokio::test]
async fn territory_socket_rejects_inactive_stale_missing_headers_and_invalid_wire_values() {
    let (mut session, mut events) = active();
    let mut socket = peer(&mut session).await;
    let command = C::ChangeTaxRate {
        zone_group_id: 33,
        tax_rate: 50,
    };
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
        session.phase = phase;
        session
            .command(Command::Dominion {
                character_id: 42,
                command,
            })
            .await;
        rejected(&mut events, 42, command);
    }
    session.phase = Phase::InWorld;
    for character_id in [0, 41] {
        session
            .command(Command::Dominion {
                character_id,
                command,
            })
            .await;
        rejected(&mut events, character_id, command);
    }
    for selected in [0, 43] {
        session.selected = selected;
        session
            .command(Command::Dominion {
                character_id: 42,
                command,
            })
            .await;
        rejected(&mut events, 42, command);
    }
    session.selected = 42;
    session.selected_state.as_mut().unwrap().character.id = 43;
    session
        .command(Command::Dominion {
            character_id: 42,
            command,
        })
        .await;
    rejected(&mut events, 42, command);
    let mut header = session.selected_state.take().unwrap();
    session
        .command(Command::Dominion {
            character_id: 42,
            command,
        })
        .await;
    rejected(&mut events, 42, command);
    header.character.id = 42;
    session.selected_state = Some(header);
    for command in [
        C::ChangeTaxRate {
            zone_group_id: 0,
            tax_rate: 25,
        },
        C::ChangeTaxRate {
            zone_group_id: 33,
            tax_rate: -1,
        },
        C::ChangeTaxRate {
            zone_group_id: 33,
            tax_rate: 51,
        },
        C::ChangeTaxRate {
            zone_group_id: 33,
            tax_rate: i32::MIN,
        },
        C::ChangeTaxRate {
            zone_group_id: 33,
            tax_rate: i32::MAX,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 0,
            tax_rate: 500,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: -1,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: 1001,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: i32::MIN,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: i32::MAX,
        },
        C::RequestCharacterBrief { requested_id: 0 },
    ] {
        session
            .command(Command::Dominion {
                character_id: 42,
                command,
            })
            .await;
        rejected(&mut events, 42, command);
    }
    silent(&mut socket).await;
    let (mut missing, mut missing_events) = active();
    for command in [
        command,
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: 500,
        },
        C::RequestCharacterBrief { requested_id: 7 },
    ] {
        missing
            .command(Command::Dominion {
                character_id: 42,
                command,
            })
            .await;
        rejected(&mut missing_events, 42, command);
    }
}

#[tokio::test]
async fn territory_socket_failed_write_rejects_once_without_a_requested_or_received_event() {
    for command in [
        C::ChangeTaxRate {
            zone_group_id: 33,
            tax_rate: 50,
        },
        C::ChangeNationalTaxRate {
            zone_group_id: 33,
            tax_rate: 500,
        },
        C::RequestCharacterBrief { requested_id: 7 },
    ] {
        let (mut session, mut events) = active();
        let _socket = peer(&mut session).await;
        session.game.as_mut().unwrap().shutdown().await;
        session
            .command(Command::Dominion {
                character_id: 42,
                command,
            })
            .await;
        let mut rejects = 0;
        let mut disconnected = false;
        while let Ok(event) = events.try_recv() {
            match event {
                Event::DominionRequestRejected {
                    character_id: 42,
                    command: actual,
                    message,
                } => {
                    assert_eq!(actual, command);
                    assert!(!message.is_empty());
                    rejects += 1;
                }
                Event::Disconnected { .. } => disconnected = true,
                Event::PhaseChanged(Phase::Idle) => {}
                unexpected => panic!("failed write emitted {unexpected:?}"),
            }
        }
        assert_eq!(rejects, 1);
        assert!(disconnected);
        assert!(session.game.is_none());
    }
}

#[tokio::test]
async fn territory_socket_repeated_dd04_receipts_forward_once_in_wire_order_without_automatic_sends()
 {
    let (mut session, mut events) = active();
    let mut socket = peer(&mut session).await;
    let mut world = world();
    receive(
        &mut session,
        &mut socket,
        include_bytes!("../../tests/fixtures/dominion/repeated-deltas.dd04.bin"),
    )
    .await;
    for _ in 0..2 {
        let event = events.try_recv().unwrap();
        assert!(
            matches!(event, Event::Notice(NoticeEvent::DominionTaxRate { zone_group: 0x1234, tax_rate }) if tax_rate == 0x89abcdefu32 as i32)
        );
        world.apply(&event);
    }
    for (_, _, expected) in cases() {
        for _ in 0..2 {
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::Dominion(actual) if *actual == expected));
            world.apply(&event);
        }
    }
    assert!(
        matches!(events.try_recv(), Ok(Event::Expedition(crate::ExpeditionEvent::Policies(rows))) if rows.is_empty())
    );
    assert!(events.try_recv().is_err());
    assert_eq!(world.dominion.rows[&0x1234].tax_rate, 0x89abcdefu32 as i32);
    assert_eq!(world.dominion.rows[&0x1234].non_pvp_end_time, 0);
    assert_eq!(
        world.dominion.last_receipt,
        Some(DominionEvent::SiegeReinforced { id: 0x1234 })
    );
    assert!(
        !world.dominion.coverage_gap,
        "newly decoded 024/025 do not report an unknown-packet gap"
    );
    silent(&mut socket).await;
}

#[tokio::test]
async fn territory_socket_short_prefixes_emit_only_decode_failure_and_recover_on_next_frame() {
    let (mut session, mut events) = active();
    let mut socket = peer(&mut session).await;
    let mut world = world();
    for (opcode, body, expected) in cases() {
        for end in 0..body.len() {
            let before = world.dominion.rows.clone();
            let last = world.dominion.last_receipt.clone();
            receive(&mut session, &mut socket, &dd01(opcode, &body[..end])).await;
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::GameDecodeFailed { message } if !message.is_empty()));
            world.apply(&event);
            assert_eq!(world.dominion.rows, before);
            assert_eq!(world.dominion.last_receipt, last);
            assert!(events.try_recv().is_err());
        }
        receive(&mut session, &mut socket, &dd01(opcode, body)).await;
        let event = events.try_recv().unwrap();
        assert!(matches!(&event, Event::Dominion(actual) if *actual == expected));
        world.apply(&event);
        assert!(world.dominion.coverage_gap);
        assert!(events.try_recv().is_err());
    }
    assert_eq!(session.phase, Phase::InWorld);
    silent(&mut socket).await;
}

#[tokio::test]
async fn territory_socket_canonical_member_notices_fold_once_without_duplicate_dominion_receipts() {
    let (mut session, mut events) = active();
    let mut socket = peer(&mut session).await;
    let mut world = world();
    // remove11 -> [22,22,33], add22 -> [22,22,33,22], then full-limit add is ignored.
    for added in [false, true, true] {
        let character_id = if added { 22u32 } else { 11u32 };
        let mut body = vec![0x34, 0x12, 77, 0, 0, 0];
        body.extend(character_id.to_le_bytes());
        body.push(u8::from(added));
        receive(&mut session, &mut socket, &dd01(0xec, &body)).await;
        let event = events.try_recv().unwrap();
        assert!(
            matches!(&event, Event::Notice(NoticeEvent::SiegeMember { zone_group: 0x1234, expedition_id: 77, character_id: id, added: actual }) if *id == character_id && *actual == added)
        );
        world.apply(&event);
        assert!(events.try_recv().is_err());
    }
    assert_eq!(
        world.dominion.rows[&0x1234].siege_timers.defender.members,
        [22, 22, 33, 22]
    );
    assert!(matches!(
        world.dominion.last_receipt,
        Some(DominionEvent::Data { .. })
    ));
    assert!(!world.dominion.coverage_gap);
    silent(&mut socket).await;
    world.apply(&Event::Disconnected {
        message: String::new(),
    });
    assert_eq!(world.dominion, Dominion::default());
}
