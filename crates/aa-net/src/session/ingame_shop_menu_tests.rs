use super::*;
use crate::{
    CashShopEvent, WorldState,
    cash_shop::{
        menu_tests::{config, dd01, dd04, fixture},
        op,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn active() -> (Session, UnboundedReceiver<Event>) {
    let (tx, events) = unbounded_channel();
    let mut s = Session::new(tx, ClientOptions::default());
    s.phase = Phase::InWorld;
    s.selected = 42;
    s.self_obj = 7;
    let mut header = CharacterState::default();
    header.character.id = 42;
    s.selected_state = Some(header);
    (s, events)
}

async fn peer(s: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (connection, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
    s.game = Some(connection.unwrap());
    accepted.unwrap().0
}

async fn frame(socket: &mut TcpStream) -> Vec<u8> {
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
        tokio::time::timeout(Duration::from_millis(15), socket.read_u8())
            .await
            .is_err()
    );
}

fn rejected(events: &mut UnboundedReceiver<Event>, character_id: u32) {
    assert!(
        matches!(events.try_recv(), Ok(Event::InGameShopMenuRequestRejected { character_id: actual, message }) if actual == character_id && !message.is_empty())
    );
    assert!(events.try_recv().is_err());
}

async fn receive(s: &mut Session, socket: &mut TcpStream, payload: &[u8]) {
    socket.write_u16_le(payload.len() as u16).await.unwrap();
    socket.write_all(payload).await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(2), s.game.as_mut().unwrap().recv())
        .await
        .unwrap();
    s.game_msg(message).await;
}

#[tokio::test]
async fn ingame_shop_menu_send_outcome_has_empty_body_without_receipt_retry_or_timeout() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    let (bag, bank) = {
        let inventory = s.received_inventory.borrow();
        (inventory.bag.clone(), inventory.bank.clone())
    };
    let records = s.received_native_records.borrow().clone();
    for counter in [0, 1] {
        s.command(Command::RequestInGameShopMenu { character_id: 42 })
            .await;
        assert_eq!(frame(&mut socket).await, [0xdd, 1, 0, counter, 0x1b, 1]);
        assert!(matches!(
            events.try_recv(),
            Ok(Event::InGameShopMenuRequested { character_id: 42 })
        ));
        assert!(
            events.try_recv().is_err(),
            "transport submission cannot fabricate a CashShop receipt"
        );
        assert!(
            s.waiting.is_none(),
            "native shop waiting belongs to root, not the session login timeout"
        );
        s.check_timeout();
        assert!(events.try_recv().is_err());
        silent(&mut socket).await;
    }
    assert_eq!((s.phase, s.selected, s.self_obj), (Phase::InWorld, 42, 7));
    assert_eq!(s.received_inventory.borrow().bag, bag);
    assert_eq!(s.received_inventory.borrow().bank, bank);
    assert_eq!(*s.received_native_records.borrow(), records);
}

#[tokio::test]
async fn ingame_shop_menu_rejects_every_inactive_phase_identity_header_actor_and_missing_socket() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
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
        s.phase = phase;
        s.command(Command::RequestInGameShopMenu { character_id: 42 })
            .await;
        rejected(&mut events, 42);
        silent(&mut socket).await;
    }
    s.phase = Phase::InWorld;
    for character_id in [0, 41] {
        s.command(Command::RequestInGameShopMenu { character_id })
            .await;
        rejected(&mut events, character_id);
        silent(&mut socket).await;
    }
    for bad in 0..6 {
        match bad {
            0 => s.selected = 0,
            1 => s.selected = 43,
            2 => s.self_obj = 0,
            3 => s.self_obj = 0x1000000,
            4 => s.selected_state = None,
            _ => s.selected_state.as_mut().unwrap().character.id = 43,
        }
        s.command(Command::RequestInGameShopMenu { character_id: 42 })
            .await;
        rejected(&mut events, 42);
        silent(&mut socket).await;
        s.selected = 42;
        s.self_obj = 7;
        let mut header = CharacterState::default();
        header.character.id = 42;
        s.selected_state = Some(header);
    }
    let (mut missing, mut missing_events) = active();
    missing
        .command(Command::RequestInGameShopMenu { character_id: 42 })
        .await;
    rejected(&mut missing_events, 42);
    // Captured persistent identity is u32, independently of the24-bit own object guard.
    s.selected = u32::MAX;
    s.selected_state.as_mut().unwrap().character.id = u32::MAX;
    s.self_obj = 0xffffff;
    s.command(Command::RequestInGameShopMenu {
        character_id: u32::MAX,
    })
    .await;
    assert_eq!(frame(&mut socket).await, [0xdd, 1, 0, 0, 0x1b, 1]);
    assert!(matches!(
        events.try_recv(),
        Ok(Event::InGameShopMenuRequested {
            character_id: u32::MAX
        })
    ));
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn ingame_shop_menu_failed_socket_write_never_claims_requested_or_menu_success() {
    let (mut s, mut events) = active();
    let _socket = peer(&mut s).await;
    s.game.as_mut().unwrap().shutdown().await;
    s.command(Command::RequestInGameShopMenu { character_id: 42 })
        .await;
    let mut rejected_count = 0;
    let mut disconnected = false;
    while let Ok(event) = events.try_recv() {
        match event {
            Event::InGameShopMenuRequestRejected {
                character_id: 42,
                message,
            } => {
                assert!(!message.is_empty());
                rejected_count += 1;
            }
            Event::Disconnected { .. } => disconnected = true,
            Event::PhaseChanged(Phase::Idle) => {}
            unexpected => panic!("failed write emitted {unexpected:?}"),
        }
    }
    assert_eq!(rejected_count, 1);
    assert!(disconnected);
    assert!(s.game.is_none());
}

#[tokio::test]
async fn ingame_shop_menu_receipts_forward_once_in_order_without_feature_or_catalog_side_effects() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    let mut world = WorldState::default();
    world.apply(&Event::InitialConfig(config(vec![0, 0, 0, 0, 0x40])));
    let feature_config = world.game_config.clone();
    for result in [0, 1, 2, 255] {
        let (body, expected) = fixture(result);
        receive(
            &mut s,
            &mut socket,
            &dd04(&[(op::MENU_LIST, body), (0xb4, vec![1])]),
        )
        .await;
        let event = events.try_recv().unwrap();
        assert!(
            matches!(&event, Event::CashShop(CashShopEvent::NativeMenuList(actual)) if *actual == expected)
        );
        world.apply(&event);
        assert_eq!(world.game_config, feature_config);
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(
            events.try_recv().is_err(),
            "no duplicate legacy menu event, wallet, goods or waiting event"
        );
        assert_eq!((s.phase, s.selected, s.self_obj), (Phase::InWorld, 42, 7));
        silent(&mut socket).await;
    }
    // Root, not the transport decoder, must fence an old untagged reply after owner change.
    s.selected = 43;
    s.selected_state.as_mut().unwrap().character.id = 43;
    s.command(Command::RequestInGameShopMenu { character_id: 42 })
        .await;
    rejected(&mut events, 42);
    let (body, expected) = fixture(255);
    receive(&mut s, &mut socket, &dd01(op::MENU_LIST, &body)).await;
    assert!(
        matches!(events.try_recv(), Ok(Event::CashShop(CashShopEvent::NativeMenuList(actual))) if actual == expected)
    );
    assert!(events.try_recv().is_err());
    silent(&mut socket).await;
}

#[tokio::test]
async fn ingame_shop_menu_truncated_receipt_reports_decode_failure_without_partial_event() {
    let (mut s, mut events) = active();
    let mut socket = peer(&mut s).await;
    let (body, _) = fixture(0);
    for prefix in 0..49 {
        receive(&mut s, &mut socket, &dd01(op::MENU_LIST, &body[..prefix])).await;
        assert!(
            matches!(events.try_recv(), Ok(Event::GameDecodeFailed { message }) if !message.is_empty())
        );
        assert!(events.try_recv().is_err());
    }
    assert_eq!((s.phase, s.selected, s.self_obj), (Phase::InWorld, 42, 7));
    silent(&mut socket).await;
}
