//! Static native stack rules arrive before game packets whenever the background catalog is ready.
use aa_net::{
    buff::BuffKind,
    state::{Buffs, NativeBuffRule},
};
use bevy::prelude::*;
use rusqlite::Connection;
use std::collections::HashMap;

pub(super) fn load(db: &Connection) -> rusqlite::Result<HashMap<u32, NativeBuffRule>> {
    let mut out = HashMap::new();
    let mut query = db.prepare("SELECT id,kind_id,per_unit_creation,stack_rule_id,max_stack,max_charge FROM buffs")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let id = super::data::any_int(row, 0)? as u32;
        let kind = match super::data::any_int(row, 1)? {
            1 => BuffKind::Good,
            2 => BuffKind::Bad,
            3 => BuffKind::Hidden,
            _ => BuffKind::Unknown,
        };
        out.insert(
            id,
            NativeBuffRule {
                kind,
                per_unit_creation: super::data::any_flag(row, 2)?,
                stack_rule_id: super::data::any_int(row, 3)? as u32,
                max_stack: super::data::any_int(row, 4)? as i32,
                max_charge: super::data::any_int(row, 5)? as i32,
            },
        );
    }
    Ok(out)
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
