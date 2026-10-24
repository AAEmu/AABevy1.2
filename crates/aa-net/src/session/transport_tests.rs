use super::*;
use crate::transport::tests::{ADJACENT, expected_adjacent};

#[tokio::test]
async fn transport_session_and_bevy_forward_once_in_order_without_state_updates() {
    let (evt, events) = unbounded_channel();
    let mut session = Session::new(evt, ClientOptions::default());
    session.phase = Phase::InWorld;
    session.selected = 7;
    session.self_obj = 0x112233;
    session
        .game_msg(game::ServerMsg::decode(&ADJACENT[2..]).map_err(Into::into))
        .await;

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
    assert_eq!(forwarded.len(), 11);
    let expected = expected_adjacent();
    let mut state = crate::WorldState::default();
    let unit = crate::state::tests::unit(0x112233);
    state.units.by_id.insert(unit.obj_id, unit.clone());
    for (event, expected) in forwarded[..10].iter().zip(expected) {
        assert!(matches!(event, Event::Transport(actual) if *actual == expected));
        state.apply(event);
        assert_eq!(state.units.get(unit.obj_id), Some(&unit));
        assert_eq!(state.units.by_id.len(), 1);
        assert!(state.player.spawn.is_none());
        assert!(state.doodads.by_id.is_empty());
        assert!(state.slaves.created.is_empty());
        assert!(state.doodad_bonds.by_unit.is_empty());
    }
    assert!(matches!(
        forwarded[10],
        Event::UnitPoints {
            obj_id: 0xaabbcc,
            hp: 123,
            mp: -456
        }
    ));
    assert_eq!(session.phase, Phase::InWorld);
    assert_eq!(session.selected, 7);
    assert_eq!(session.self_obj, 0x112233);
    assert!(session.selected_state.is_none());
    assert!(session.game.is_none());
    assert!(session.received_doodad_bonds.borrow().by_unit.is_empty());
}

#[tokio::test]
async fn transport_session_truncations_report_failure_without_partial_receipts() {
    let (evt, mut events) = unbounded_channel();
    let mut session = Session::new(evt, ClientOptions::default());
    for (op, body) in [
        (
            0x06du16,
            vec![0x33, 0x22, 0x11, 0, 0, 0, 0x60, 0xc0, 2, 255],
        ),
        (0x06d, vec![0x33, 0x22, 0x11, 255]),
        (0x07a, vec![0x33, 0x22, 0x11, 255]),
        (0x137, vec![0x33, 0x22, 0x11, 0xcc, 0xbb, 0xaa]),
        (
            0x138,
            vec![0x33, 0x22, 0x11, 0xcc, 0xbb, 0xaa, 0xff, 0xff, 0xff, 0xff],
        ),
    ] {
        for cut in 0..body.len() {
            let mut frame = vec![0xdd, 1, 0, 0];
            frame.extend_from_slice(&op.to_le_bytes());
            frame.extend_from_slice(&body[..cut]);
            let decoded = game::ServerMsg::decode(&frame);
            assert!(decoded.is_err());
            session.game_msg(decoded.map_err(Into::into)).await;
            assert!(matches!(
                events.try_recv(),
                Ok(Event::GameDecodeFailed { .. })
            ));
            assert!(events.try_recv().is_err());
            assert_eq!(session.phase, Phase::Idle);
            assert_eq!(session.self_obj, 0);
        }
    }
}
