use super::*;
use std::{cell::RefCell, rc::Rc};

fn send(state: &Shared, event: Event) {
    super::super::receive(state, &event);
}

fn objects(state: &Shared) {
    let own = crate::hud::expansion::fixture::spawn();
    let mut slave = own.unit.clone();
    slave.obj_id = 20;
    slave.kind = aa_net::model::UnitKind::Slave {
        id: 700,
        tl_id: 8,
        template_id: 14,
        summoner_id: 100,
    };
    for (index, slot) in slave.equipment.slots.iter_mut().enumerate() {
        // Typed receipt fixture, not a claim that Unit::read has parsed the
        // original kind2 full-item exception (its separate fix is held).
        *slot = Some(Item {
            template_id: 29689,
            id: 9_007_199_254_740_993 + index as u64,
            grade: 4,
            ..Default::default()
        });
    }
    send(state, Event::CharacterSelected(Box::new(own.state.clone())));
    send(state, Event::PlayerSpawn(Box::new(own)));
    send(
        state,
        Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
    );
    send(state, Event::UnitSpawned(Box::new(slave)));
    send(
        state,
        Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 20,
        }),
    );
}

fn state() -> Shared {
    let state = Rc::new(RefCell::new(HudData::default()));
    let mut game = crate::hud::data::GameData::default();
    game.mounted.rows.insert(
        14,
        super::super::Descriptor {
            name: "Clipper".into(),
            kind: "speedboat",
            customizing: Some(2),
        },
    );
    game.mounted.equipment.definitions.insert(2);
    game.mounted.equipment.names.insert((2, 13), "Lamp".into());
    game.mounted.equipment.slots.insert((14, 13));
    assert!(state.borrow().game.0.set(game).is_ok());
    objects(&state);
    state
}

fn config(bytes: Vec<u8>) -> Event {
    Event::InitialConfig(aa_net::InitialConfig {
        host: Vec::new(),
        feature_bytes: bytes,
        raw_candidate_count: 0,
        candidate_codes: Vec::new(),
        initial_lp: 0,
        service_flags: [0; 7],
        second_price_type: 0,
        second_password_max_fail_count: 0,
        idle_kick_time: 0,
    })
}

#[test]
fn all_28_slots_preserve_items_independently_of_configured_slot_membership() {
    let state = state();
    let data = state.borrow();
    assert_eq!(customizing_type(&data), None);
    for index in 0..28 {
        let value = selected_slot(&data, index).unwrap();
        assert_eq!(value.has, index == 13);
        assert_eq!(value.name, if index == 13 { "Lamp" } else { "" });
        assert_eq!(value.item.unwrap().id, 9_007_199_254_740_993 + index as u64);
    }
    assert!(selected_slot(&data, 28).is_none());
}

#[test]
fn gear_receipts_target_exact_objects_while_transaction_reports_do_not_settle_gear() {
    let state = state();
    let replacement = Item {
        template_id: 29690,
        id: 55,
        grade: 9,
        ..Default::default()
    };
    send(
        &state,
        Event::EquipmentChanged {
            obj_id: 21,
            items: vec![(13, None)],
        },
    );
    assert_eq!(
        selected_slot(&state.borrow(), 13)
            .unwrap()
            .item
            .unwrap()
            .grade,
        4
    );
    send(
        &state,
        Event::EquipmentChanged {
            obj_id: 20,
            items: vec![
                (13, None),
                (23, Some(replacement.clone())),
                (255, Some(replacement)),
            ],
        },
    );
    assert!(selected_slot(&state.borrow(), 13).unwrap().item.is_none());
    assert_eq!(
        selected_slot(&state.borrow(), 23)
            .unwrap()
            .item
            .unwrap()
            .grade,
        9
    );
    send(
        &state,
        Event::Slave(SlaveEvent::EquipmentChanged(aa_net::SlaveEquipmentChange {
            character_id: 47,
            slave_tl_id: 8,
            db_slave_id: 700,
            bts: false,
            first_item: None,
            second_item: None,
            first_location: aa_net::SlaveItemLocation {
                container: 0xf2,
                slot: 23,
            },
            second_location: aa_net::SlaveItemLocation {
                container: 2,
                slot: 0,
            },
            success: true,
        })),
    );
    assert_eq!(
        selected_slot(&state.borrow(), 23)
            .unwrap()
            .item
            .unwrap()
            .grade,
        9
    );
}

#[test]
fn feature_config_survives_character_world_changes_and_slot_query_has_no_feature_gate() {
    let state = state();
    let mut bytes = vec![0; 7];
    bytes[6] = 1;
    send(&state, config(bytes));
    assert_eq!(customizing_type(&state.borrow()), Some(2));
    send(&state, Event::LeftWorld { target: 0 });
    assert!(state.borrow().mounted.config.received.is_some());
    assert_eq!(customizing_type(&state.borrow()), None);
    objects(&state);
    assert_eq!(customizing_type(&state.borrow()), Some(2));
    send(&state, config(vec![0; 6]));
    assert_eq!(customizing_type(&state.borrow()), None);
    assert!(selected_slot(&state.borrow(), 13).unwrap().has);
    send(&state, Event::PhaseChanged(aa_net::Phase::Connecting));
    assert!(state.borrow().mounted.config.received.is_none());
    objects(&state);
    assert_eq!(customizing_type(&state.borrow()), None);
    assert!(selected_slot(&state.borrow(), 13).unwrap().has);
}

#[test]
fn notification_indices_keep_native_negative_and_high_slot_results() {
    for container in [1, 0xfc, 0xf2] {
        assert_eq!(notice_slot(container, 255), 255.0);
    }
    for container in [0, 2, 3, 0xfe] {
        assert_eq!(notice_slot(container, 13), -1.0);
    }
}
