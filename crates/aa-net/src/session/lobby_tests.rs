//! Exact offline writer fixtures through the decoder, session and existing Characters resource.
use super::*;
use crate::{AccountPayment, PremiumPoints, WorldState, state::Characters};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PAYMENT: &[u8] = include_bytes!("../../tests/fixtures/lobby_headers/lane-payment.frame");
const PREMIUM: &[u8] = include_bytes!("../../tests/fixtures/lobby_headers/lane-premium.frame");
const SLOTS: &[u8] = include_bytes!("../../tests/fixtures/lobby_headers/lane-extra-slots.frame");
const VARIED: &[u8] = include_bytes!("../../tests/fixtures/lobby_headers/varied-payment.frame");
const BATCH: &[u8] = include_bytes!("../../tests/fixtures/lobby_headers/adjacent-batch.frame");

fn payment() -> AccountPayment {
    AccountPayment {
        pay_method: 1,
        pay_location: 1,
        pay_start: 0,
        pay_end: 1_893_456_000,
        real_pay_time: 0,
    }
}

fn varied_payment() -> AccountPayment {
    AccountPayment {
        pay_method: -7,
        pay_location: i32::MAX,
        pay_start: i64::MIN,
        pay_end: i64::MAX,
        real_pay_time: 0x1122_3344_01e2_0000,
    }
}

fn points() -> PremiumPoints {
    PremiumPoints {
        point: 1,
        old_grade: 1,
        grade: 1,
    }
}

fn payment_message(value: AccountPayment) -> game::ServerMsg {
    game::ServerMsg::AccountInfo {
        pay_method: value.pay_method,
        pay_location: value.pay_location,
        pay_start: value.pay_start,
        pay_end: value.pay_end,
        real_pay_time: value.real_pay_time,
    }
}

#[test]
fn lobby_headers_match_independent_lane_and_native_reader_fixtures() {
    use game::ServerMsg as M;
    for (frame, expected) in [
        (PAYMENT, payment_message(payment())),
        (VARIED, payment_message(varied_payment())),
        (
            PREMIUM,
            M::PremiumPoint {
                point: 1,
                old_grade: 1,
                grade: 1,
            },
        ),
        (SLOTS, M::SlotCount(2)),
    ] {
        assert_eq!(M::decode(frame).unwrap(), expected);
        // Including the formerly ignored realPayTime: no partial metadata receipt.
        for cut in 0..frame.len() {
            assert!(
                M::decode(&frame[..cut]).is_err(),
                "{} cut {cut}",
                expected.name()
            );
        }
    }
    assert_eq!(
        M::decode(BATCH).unwrap(),
        M::Batch(vec![
            payment_message(varied_payment()),
            M::SlotCount(2),
            M::PremiumPoint {
                point: 1,
                old_grade: 1,
                grade: 1
            },
            M::Snow(true),
        ]),
        "realPayTime must be consumed before the next compressed packet header",
    );
    let mut raw = PREMIUM.to_vec();
    raw[6..].copy_from_slice(&[0, 0, 0, 0x80, 0xff, 0xfe]);
    assert_eq!(
        M::decode(&raw).unwrap(),
        M::PremiumPoint {
            point: i32::MIN,
            old_grade: 255,
            grade: 254
        }
    );
    assert_eq!(
        M::decode(&[0xdd, 1, 0, 0, 0xe2, 1, 255]).unwrap(),
        M::SlotCount(255)
    );
    // Earlier assertions use independent lane-writer fixtures. These four rows were also
    // executed through retained original archives, with scalar/date leaves modeled.
    let goldens: Vec<_> = include_str!("../../tests/fixtures/lobby_headers/native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect();
    assert_eq!(goldens.len(), 4);
    for (line, frame) in goldens
        .into_iter()
        .zip([PAYMENT, VARIED, PREMIUM, raw.as_slice()])
    {
        let (_, hex) = line.split_once('\t').unwrap();
        let bytes: Vec<_> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        assert_eq!(bytes, frame[6..]);
    }
}

#[test]
fn lobby_header_cache_distinguishes_unreceived_zero_and_duplicate_receipts() {
    let mut characters = Characters::default();
    assert_eq!(
        (
            characters.extra_slots,
            characters.payment,
            characters.premium_points
        ),
        (None, None, None)
    );
    let zero = PremiumPoints {
        point: 0,
        old_grade: 0,
        grade: 0,
    };
    for event in [
        Event::CharacterSlotCount(0),
        Event::AccountPayment(payment()),
        Event::PremiumPoints(zero),
    ] {
        assert!(characters.apply(&event));
        assert!(!characters.apply(&event));
    }
    assert_eq!(characters.extra_slots, Some(0));
    assert_eq!(characters.payment, Some(payment()));
    assert_eq!(characters.premium_points, Some(zero));
    assert!(characters.apply(&Event::PremiumPoints(points())));
    assert_eq!(characters.premium_points, Some(points()));
}

#[test]
fn lobby_headers_survive_character_flow_but_reset_for_another_connection_or_failure() {
    let mut state = WorldState::default();
    let character = Character {
        id: 7,
        point: 400,
        prev_point: 300,
        ..Default::default()
    };
    state.apply(&Event::CharacterList(vec![character.clone()]));
    state.apply(&Event::CharacterSlotCount(2));
    state.apply(&Event::AccountPayment(payment()));
    state.apply(&Event::PremiumPoints(points()));
    for event in [
        Event::CharacterSelected(Box::default()),
        Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: Default::default(),
            unit: crate::state::tests::unit(7),
        })),
        Event::EnteredWorld,
        Event::LeftWorld { target: 1 },
        Event::PhaseChanged(Phase::Connecting),
        Event::LobbyReady,
        Event::CharacterList(vec![character.clone()]),
    ] {
        state.apply(&event);
        assert_eq!(state.characters.extra_slots, Some(2));
        assert_eq!(state.characters.payment, Some(payment()));
        assert_eq!(state.characters.premium_points, Some(points()));
        assert_eq!(
            state.characters.by_id(7),
            Some(&character),
            "an account receipt has no character ID"
        );
    }
    for event in [
        Event::PhaseChanged(Phase::Authenticating),
        Event::PhaseChanged(Phase::WorldSelect),
        Event::PhaseChanged(Phase::ReadyToConnect),
        Event::Disconnected {
            message: "closed".into(),
        },
        Event::LoginFailed {
            message: "denied".into(),
        },
        Event::Kicked {
            reason: 1,
            message: "kicked".into(),
        },
    ] {
        let mut characters = state.characters.clone();
        assert!(characters.apply(&event));
        assert_eq!(
            (
                characters.extra_slots,
                characters.payment,
                characters.premium_points
            ),
            (None, None, None)
        );
        if matches!(
            event,
            Event::PhaseChanged(Phase::ReadyToConnect)
                | Event::Kicked { .. }
                | Event::LoginFailed { .. }
        ) {
            assert_eq!(
                characters.list, state.characters.list,
                "new headers preserve existing list reset semantics"
            );
        }
    }
}

async fn peer(session: &mut Session) -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
    session.game = Some(connection.unwrap());
    accepted.unwrap().0
}

async fn send_frame(peer: &mut TcpStream, payload: &[u8]) {
    peer.write_u16_le(payload.len().try_into().unwrap())
        .await
        .unwrap();
    peer.write_all(payload).await.unwrap();
}

async fn receive(session: &mut Session) {
    let message = tokio::time::timeout(
        Duration::from_secs(2),
        session.game.as_mut().unwrap().recv(),
    )
    .await
    .unwrap();
    session.game_msg(message).await;
}

async fn payload(peer: &mut TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let len = peer.read_u16_le().await.unwrap();
        let mut bytes = vec![0; usize::from(len)];
        peer.read_exact(&mut bytes).await.unwrap();
        bytes
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn lobby_header_loopback_keeps_handshake_and_reports_later_premium_updates_once() {
    let (evt, mut events) = unbounded_channel();
    let mut session = Session::new(evt, ClientOptions::default());
    session.phase = Phase::Connecting;
    let mut peer = peer(&mut session).await;
    // No account/cookie exists in this isolated session, so no stream connection can start.
    send_frame(&mut peer, PREMIUM).await;
    receive(&mut session).await;
    assert!(matches!(events.try_recv(), Ok(Event::PremiumPoints(value)) if value == points()));
    assert!(matches!(
        events.try_recv(),
        Ok(Event::PhaseChanged(Phase::CharacterSelect))
    ));
    assert!(matches!(events.try_recv(), Ok(Event::LobbyReady)));
    assert!(events.try_recv().is_err());
    assert_eq!(session.phase, Phase::CharacterSelect);
    let request = payload(&mut peer).await;
    assert_eq!(
        u16::from_le_bytes([request[4], request[5]]),
        crate::opcodes::cs::CSListCharacterPacket
    );

    let mut world = WorldState::default();
    for phase in [Phase::CharacterSelect, Phase::InWorld] {
        session.phase = phase;
        for frame in [PAYMENT, SLOTS, PREMIUM] {
            send_frame(&mut peer, frame).await;
            receive(&mut session).await;
            let event = events.try_recv().unwrap();
            match event {
                Event::AccountPayment(value) => assert_eq!(value, payment()),
                Event::CharacterSlotCount(value) => assert_eq!(value, 2),
                Event::PremiumPoints(value) => assert_eq!(value, points()),
                ref other => panic!("unexpected header event: {other:?}"),
            }
            world.apply(&event);
            assert!(events.try_recv().is_err());
            assert_eq!(session.phase, phase);
        }
    }
    assert_eq!(world.characters.extra_slots, Some(2));
    assert_eq!(world.characters.payment, Some(payment()));
    assert_eq!(world.characters.premium_points, Some(points()));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), peer.read_u8())
            .await
            .is_err(),
        "updates do not restart the lobby/list request"
    );
}

#[tokio::test]
async fn truncated_payment_keeps_previous_header_then_valid_batch_reaches_the_bridge() {
    let (evt, events) = unbounded_channel();
    let mut session = Session::new(evt, ClientOptions::default());
    session.phase = Phase::CharacterSelect;
    let mut peer = peer(&mut session).await;
    send_frame(&mut peer, PAYMENT).await;
    receive(&mut session).await;
    for cut in 6..PAYMENT.len() {
        send_frame(&mut peer, &PAYMENT[..cut]).await;
        receive(&mut session).await;
    }
    send_frame(&mut peer, BATCH).await;
    receive(&mut session).await;

    #[cfg(feature = "bevy")]
    let forwarded = {
        let (commands, _commands_rx) = unbounded_channel();
        crate::plugin::transport_tests::forward_through_bridge(Client { commands, events })
    };
    #[cfg(not(feature = "bevy"))]
    let forwarded: Vec<_> = {
        let mut events = events;
        std::iter::from_fn(|| events.try_recv().ok()).collect()
    };
    let mut world = WorldState::default();
    let bad_count = PAYMENT.len() - 6;
    assert_eq!(
        forwarded.len(),
        5 + bad_count,
        "bad frames report failures without a partial/default header"
    );
    assert!(matches!(forwarded[0], Event::AccountPayment(value) if value == payment()));
    for event in &forwarded[..1 + bad_count] {
        world.apply(event);
        assert_eq!(world.characters.payment, Some(payment()));
        assert_eq!(world.characters.extra_slots, None);
        assert_eq!(world.characters.premium_points, None);
    }
    assert!(
        forwarded[1..1 + bad_count]
            .iter()
            .all(|event| matches!(event, Event::GameDecodeFailed { .. }))
    );
    let tail = &forwarded[1 + bad_count..];
    assert!(matches!(tail[0], Event::AccountPayment(value) if value == varied_payment()));
    assert!(matches!(tail[1], Event::CharacterSlotCount(2)));
    assert!(matches!(tail[2], Event::PremiumPoints(value) if value == points()));
    assert!(matches!(tail[3], Event::Snow(true)));
    for event in tail {
        world.apply(event);
    }
    assert_eq!(world.characters.payment, Some(varied_payment()));
    assert_eq!(world.characters.extra_slots, Some(2));
    assert_eq!(world.characters.premium_points, Some(points()));
    assert_eq!(session.phase, Phase::CharacterSelect);
}
