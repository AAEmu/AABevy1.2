//! Static native stack rules arrive before game packets whenever the background catalog is ready.
use aa_net::state::{Buffs, NativeBuffRule};
#[cfg(test)]
use aa_net::buff::BuffKind;
use bevy::prelude::*;
use rusqlite::Connection;
use std::collections::HashMap;
pub(super) mod cancel;
pub(super) mod query;

/// Native39421dcf: positive selected count only, converted through signed CVTSI2SS.
pub(super) fn stack_count(rule: u32, stacks: i32, charged: i32) -> Option<f64> {
    let count = match rule {
        2 | 3 => charged.wrapping_abs(),
        4 => stacks,
        _ => 1,
    };
    (count > 0).then(|| f64::from(count as f32))
}

pub(super) fn load(db: &Connection) -> rusqlite::Result<HashMap<u32, NativeBuffRule>> {
    crate::ingame::combat::skills::load_native_buff_rules(db)
}

pub(super) fn bind_catalog(mut bound: Local<bool>, bridge: NonSend<super::api::HudBridge>, buffs: Option<ResMut<Buffs>>) {
    if *bound {
        return;
    }
    let data = bridge.0.borrow();
    let Some(game) = data.game.get() else { return };
    let Some(mut buffs) = buffs else { return };
    buffs.native.set_catalog(game.native_buffs.clone());
    *bound = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_getter_count_goldens_preserve_rule_charge_omission_and_f32_rounding() {
        // Original39421dcf..e17 +390309f0, archived native-buff-stack-getter-oracle-20261018.json.
        for (rule, stacks, charged, expected) in [
            (0, -5, 0, Some(1.)),
            (1, 99, -20, Some(1.)),
            (5, 99, -20, Some(1.)),
            (6, 99, -20, Some(1.)),
            (2, 99, -5, Some(5.)),
            (2, 99, 0, None),
            (3, 99, i32::MIN, None),
            (3, 99, i32::MAX, Some(2_147_483_648.)),
            (4, -5, 99, None),
            (4, 0, 99, None),
            (4, 1, 99, Some(1.)),
            (4, i32::MAX, 99, Some(2_147_483_648.)),
        ] {
            assert_eq!(
                stack_count(rule, stacks, charged),
                expected,
                "rule={rule} stacks={stacks} charged={charged}"
            );
        }
    }
    #[test]
    fn loading_catalog_retains_raw_rule_zeroes_and_seeds_before_created_packets() -> Result<(), Box<dyn std::error::Error>> {
        let db = Connection::open_in_memory()?;
        db.execute_batch(
            "CREATE TABLE buffs(id,kind_id,per_unit_creation,stack_rule_id,max_stack,max_charge);INSERT INTO buffs VALUES(77,1,'T',4,3,0),(78,3,'F',6,0,0)",
        )?;
        let rules = load(&db)?;
        assert_eq!(rules[&77].kind, BuffKind::Good);
        assert!(rules[&77].per_unit_creation);
        assert_eq!(rules[&78].max_stack, 0);
        assert!(!rules[&78].per_unit_creation);
        let data = super::super::api::HudData::default();
        let mut game = super::super::data::GameData::default();
        game.native_buffs = rules;
        data.game.0.set(game).ok().unwrap();
        let mut app = App::new();
        app.init_resource::<Buffs>()
            .insert_non_send(super::super::api::HudBridge(std::rc::Rc::new(std::cell::RefCell::new(data))));
        app.add_systems(First, bind_catalog);
        app.update();
        let mut received = app.world_mut().resource_mut::<Buffs>();
        assert_eq!(received.native.rules[&77].stack_rule_id, 4);
        let mut spawn = super::super::expansion::fixture::spawn();
        spawn.unit.buffs.clear();
        received.apply(&aa_net::Event::PlayerSpawn(Box::new(spawn)));
        for index in [1, 2] {
            received.apply(&aa_net::Event::BuffCreated {
                obj_id: 100,
                buff: aa_net::buff::Buff {
                    index,
                    buff_id: 77,
                    stacks: 9,
                    charged: 9,
                    ..default()
                },
            });
        }
        assert_eq!(received.native.of(100).unwrap()[0].buff.stacks, 2);
        Ok(())
    }
}
