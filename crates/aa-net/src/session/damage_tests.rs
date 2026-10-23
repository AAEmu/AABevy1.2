use super::*;
use crate::skill::damage_tests::{lane_fixtures, mixed_batch, plain};

#[tokio::test]
async fn unit_damage_session_preserves_plain_and_dd04_contexts_without_state_side_effects() {
    let (evt, mut events) = unbounded_channel();
    let mut session = Session::new(evt, ClientOptions::default());
    session.phase = Phase::InWorld;
    let mut world = crate::state::WorldState::default();
    let first = lane_fixtures().remove(1);
    session
        .game_msg(Ok(game::ServerMsg::decode(&plain(&first.body)).unwrap()))
        .await;
    let event = events.try_recv().unwrap();
    assert!(matches!(&event, Event::UnitDamaged(actual) if *actual == first.expected));
    world.apply(&event);
    assert!(events.try_recv().is_err());
    let (frame, expected) = mixed_batch();
    session
        .game_msg(Ok(game::ServerMsg::decode(&frame).unwrap()))
        .await;
    assert!(matches!(events.try_recv().unwrap(), Event::Snow(false)));
    let mut retained = Vec::new();
    for damage in &expected {
        let event = events.try_recv().unwrap();
        let Event::UnitDamaged(actual) = &event else {
            panic!("missing damage: {event:?}")
        };
        assert_eq!(actual, damage);
        assert_eq!(actual.pierce_selector(), (damage.raw_hit >> 12) as u8);
        retained.push(actual.clone());
        world.apply(&event);
    }
    assert_eq!(
        retained, expected,
        "later receipts must not overwrite earlier contexts"
    );
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::SkillEnded { tl: 0x9876 }
    ));
    assert!(matches!(events.try_recv().unwrap(), Event::Snow(true)));
    assert!(events.try_recv().is_err());
    assert!(
        world.units.by_id.is_empty(),
        "damage receipts do not fabricate units or HP snapshots"
    );
}
