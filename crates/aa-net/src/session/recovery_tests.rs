use super::*;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

async fn game_peer(s: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
    s.game = Some(connection.unwrap());
    accepted.unwrap().0
}

async fn payload(peer: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let len = peer.read_u16_le().await.unwrap();
        let mut bytes = vec![0; usize::from(len)];
        peer.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .expect("expected a request frame")
}

#[tokio::test]
async fn recovered_requests_obey_phase_identity_and_validation_guards() {
    use crate::{AccountCommand, ChatCommand, CofferCommand};
    let (evt, mut events) = unbounded_channel();
    let mut s = Session::new(evt, ClientOptions::default());
    let mut peer = game_peer(&mut s).await;
    let lobby = || Command::Account(AccountCommand::RefreshInCharacterList);
    let roll = || Command::ChatCommand(ChatCommand::RollDice { max: 100 });
    let open = || {
        Command::Coffer(CofferCommand::Interact {
            obj_id: 7,
            open: true,
        })
    };
    for phase in [Phase::Idle, Phase::EnteringWorld, Phase::Leaving] {
        s.phase = phase;
        s.self_obj = 7;
        s.command(lobby()).await;
        s.command(roll()).await;
        s.command(open()).await;
    }
    s.phase = Phase::CharacterSelect;
    s.command(roll()).await;
    s.command(open()).await;
    s.command(lobby()).await;
    assert_eq!(payload(&mut peer).await, [0xdd, 1, 0, 0, 0x21, 0]);
    s.phase = Phase::InWorld;
    s.self_obj = 0;
    s.command(roll()).await;
    s.command(open()).await;
    s.self_obj = 7;
    s.command(lobby()).await;
    s.command(Command::ChatCommand(ChatCommand::RollDice { max: 0 }))
        .await;
    s.command(Command::ChatCommand(ChatCommand::JoinChannel {
        name: " ".into(),
        password: String::new(),
        create: false,
    }))
    .await;
    s.command(Command::Coffer(CofferCommand::Interact {
        obj_id: 0x1000000,
        open: true,
    }))
    .await;
    s.command(Command::Coffer(CofferCommand::Split {
        mv: Default::default(),
        count: -1,
        container_id: 0,
    }))
    .await;
    s.command(roll()).await;
    assert_eq!(
        payload(&mut peer).await,
        [0xdd, 1, 0, 1, 0xbd, 0, 100, 0, 0, 0]
    );
    s.command(open()).await;
    assert_eq!(
        payload(&mut peer).await,
        [0xdd, 1, 0, 2, 0x6c, 0, 7, 0, 0, 1]
    );
    assert!(
        !s.received_inventory.borrow().coffer.is_open(),
        "requests do not fabricate contents"
    );
    assert!(events.try_recv().is_err(), "no synthetic success replies");
}

#[tokio::test]
async fn recovered_replies_reach_events_in_order_without_changing_session_phase() {
    use crate::{
        AccountEvent, CashShopEvent, CofferContents, CofferEvent, DiceRolled, HousingEvent,
        NoticeEvent, UnitFxEvent,
    };
    let (evt, mut events) = unbounded_channel();
    let mut s = Session::new(evt, ClientOptions::default());
    s.phase = Phase::InWorld;
    s.game_msg(Ok(game::ServerMsg::Batch(vec![
        game::ServerMsg::Account(AccountEvent::LpManaged { character_id: 7 }),
        game::ServerMsg::Notice(NoticeEvent::ProcessingInstance { zone_id: 129 }),
        game::ServerMsg::UnitFx(UnitFxEvent::IdleStatus {
            obj_id: 7,
            idle: true,
        }),
        game::ServerMsg::CashShop(CashShopEvent::CheckTime),
        game::ServerMsg::Dice(DiceRolled {
            name: "Bob".into(),
            max: 100,
            value: 42,
        }),
        game::ServerMsg::Coffer(CofferEvent::Contents(CofferContents {
            obj_id: 8,
            container_id: 9,
            slots: vec![(0, None)],
        })),
        game::ServerMsg::Housing(HousingEvent::CharacterBrief {
            character_id: 7,
            name: "Bob".into(),
            race: 5,
        }),
        game::ServerMsg::ChatSpamDelay(crate::ChatSpamDelay::default()),
        game::ServerMsg::VegetationCutdown {
            unit_obj_id: 7,
            doodad_obj_id: 8,
        },
        game::ServerMsg::Snow(true),
    ])))
    .await;
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Account(AccountEvent::LpManaged { character_id: 7 }))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Notice(NoticeEvent::ProcessingInstance {
            zone_id: 129
        }))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::UnitFx(UnitFxEvent::IdleStatus {
            obj_id: 7,
            idle: true
        }))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::CashShop(CashShopEvent::CheckTime))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Dice(DiceRolled {
            max: 100,
            value: 42,
            ..
        }))
    ));
    assert!(matches!(events.try_recv(), Ok(Event::Coffer(_))));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::Housing(HousingEvent::CharacterBrief {
            character_id: 7,
            ..
        }))
    ));
    assert!(matches!(events.try_recv(), Ok(Event::ChatSpamDelay(_))));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::VegetationCutdown {
            unit_obj_id: 7,
            doodad_obj_id: 8
        })
    ));
    assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
    assert!(events.try_recv().is_err());
    assert_eq!(s.phase, Phase::InWorld);
    assert_eq!(s.received_inventory.borrow().coffer.container_id, 9);
}

#[tokio::test]
async fn dropped_game_stays_idle_and_an_explicit_new_login_can_spawn_again() {
    let (evt, mut events) = unbounded_channel();
    let mut s = Session::new(evt, ClientOptions::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    s.login_addr = listener.local_addr().unwrap().to_string();
    s.phase = Phase::InWorld;
    s.self_obj = 77;
    s.selected = 17;
    s.selected_state = Some(Default::default());
    s.next_ping = Some(Instant::now());
    s.wait_for("an old request");
    s.game_lost(NetError::Protocol("connection lost".into()));
    assert_eq!(s.phase, Phase::Idle);
    assert!(s.game.is_none() && s.stream.is_none());
    assert_eq!((s.self_obj, s.selected), (0, 0));
    assert!(s.selected_state.is_none() && s.waiting.is_none() && s.next_ping.is_none());
    assert!(matches!(
        events.try_recv(),
        Ok(Event::PhaseChanged(Phase::Idle))
    ));
    assert!(matches!(events.try_recv(), Ok(Event::Disconnected { .. })));
    s.keep_alive().await;
    s.check_timeout();
    assert!(events.try_recv().is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err(),
        "loss does not reconnect behind the UI"
    );

    // Also cover a fresh login made directly while an old character identity remains.
    s.self_obj = 88;
    s.selected = 18;
    s.selected_state = Some(Default::default());
    let addr = s.login_addr.clone();
    s.command(Command::Login {
        addr,
        user: "recovery".into(),
        password: "test".into(),
    })
    .await;
    let (mut login_peer, _) = listener.accept().await.unwrap();
    let auth = payload(&mut login_peer).await;
    assert_eq!(
        u16::from_le_bytes([auth[0], auth[1]]),
        crate::opcodes::ca::CARequestAuthTrionPacket
    );
    assert_eq!((s.self_obj, s.selected), (0, 0));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::PhaseChanged(Phase::Authenticating))
    ));
    s.phase = Phase::EnteringWorld;
    s.selected = 17;
    let mut selected = crate::model::CharacterState::default();
    selected.character.id = 17;
    s.selected_state = Some(selected);
    let mut unit = crate::state::tests::unit(99);
    unit.kind = UnitKind::Character { char_id: 17 };
    s.unit_state(unit).await;
    assert_eq!(s.phase, Phase::InWorld);
    assert_eq!(s.self_obj, 99);
    assert!(matches!(
        events.try_recv(),
        Ok(Event::PhaseChanged(Phase::InWorld))
    ));
    assert!(matches!(events.try_recv(), Ok(Event::PlayerSpawn(_))));
    assert!(matches!(events.try_recv(), Ok(Event::EnteredWorld)));
    s.close_all().await;
}
