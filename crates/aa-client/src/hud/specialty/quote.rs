//! Known-bundle Refund over actual received NPC/equipment and complete static data.
use super::*;
use crate::hud::data::{GameData, any_int};
use aa_net::specialty::{KnownBundleQuote, quote_known_bundle};
use rusqlite::{Connection, OptionalExtension};

#[derive(Default)]
pub(in crate::hud) struct Tables {
    backpack_types: HashMap<u32, i32>,
    npc_coins: HashMap<u32, u32>,
    npc_bundles: HashMap<u32, u32>,
    profits: HashMap<(u32, u32), (i32, i32)>,
    grades: HashMap<u32, i32>,
    coin_divisor: Option<i32>,
}

impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut tables = Self::default();
        let mut query = conn.prepare("SELECT item_id,backpack_type_id FROM item_backpacks")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            tables.backpack_types.insert(any_int(row, 0)? as u32, any_int(row, 1)? as i32);
        }
        let mut query = conn.prepare("SELECT id,COALESCE(specialty_coin_id,0) FROM npcs")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            tables.npc_coins.insert(any_int(row, 0)? as u32, any_int(row, 1)? as u32);
        }
        let mut query = conn.prepare("SELECT npc_id,specialty_bundle_id FROM specialty_npcs")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            tables.npc_bundles.insert(any_int(row, 0)? as u32, any_int(row, 1)? as u32);
        }
        // Native vectors return the first matching item, not the last duplicate.
        let mut query = conn.prepare("SELECT id,item_id,profit,ratio,specialty_bundle_id FROM specialty_bundle_items")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let key = (any_int(row, 4)? as u32, any_int(row, 1)? as u32);
            tables
                .profits
                .entry(key)
                .or_insert((any_int(row, 2)? as i32, any_int(row, 3)? as i32));
        }
        let mut query = conn.prepare("SELECT id,refund_multiplier FROM item_grades")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            tables.grades.insert(any_int(row, 0)? as u32, any_int(row, 1)? as i32);
        }
        //395720e0 indexes ID, not kind_id (whose22 entry has a different value).
        tables.coin_divisor = conn
            .query_row("SELECT value FROM content_configs WHERE id=22", [], |row| {
                Ok(any_int(row, 0)? as i32)
            })
            .optional()?;
        Ok(tables)
    }

    fn quote(&self, game: &GameData, npc: u32, item: &aa_net::model::Item, ratio: i32) -> Result<Option<(i32, u32)>, i16> {
        let Some(&coin) = self.npc_coins.get(&npc) else { return Ok(None) };
        if self.backpack_types.get(&item.template_id) != Some(&3) {
            return Ok(None);
        }
        let Some(desc) = game.items.get(&item.template_id) else {
            return Ok(None);
        };
        if desc.price as i32 == 0 {
            return Ok(None);
        }
        let Some(&bundle) = self.npc_bundles.get(&npc) else {
            // Regional fallback/current-zone admission remains unprovided.
            return Ok(None);
        };
        let &(profit, profit_ratio) = self.profits.get(&(bundle, item.template_id)).ok_or(225i16)?;
        let Some(&refund_multiplier) = self.grades.get(&u32::from(item.grade)) else {
            return Ok(None);
        };
        let coin_divisor = if coin == 0 {
            None
        } else {
            let Some(divisor) = self.coin_divisor else { return Ok(None) };
            Some(divisor)
        };
        let quote = quote_known_bundle(KnownBundleQuote {
            refund: desc.refund as i32,
            refund_multiplier,
            profit,
            profit_ratio,
            received_ratio: ratio,
            // Captured original1.2 NPC factor map is empty; getter default100.
            // This is a qualified static input-copy, not a generic future default.
            npc_ratio: 100.0,
            coin_divisor,
        });
        let amount = if coin == 0 {
            quote.refund
        } else {
            let Some(amount) = quote.coin_amount else { return Ok(None) };
            amount
        };
        Ok(Some((amount, coin)))
    }
}

fn current(state: &Shared) -> Result<Option<(i32, u32)>, i16> {
    let data = state.borrow();
    // SC084 ->394270a0 writes this resolved local actor's Game+1aa0 target.
    // Native local setters also exist; this app subset needs a received target.
    let Some(actor) = data.specialty.target.obj_id else {
        return Ok(None);
    };
    let Some(&npc) = data.specialty.npc_templates.get(&actor) else {
        return Ok(None);
    };
    let Some(item) = crate::hud::enchant::equipment(&data, aa_net::model::slot::BACKPACK as u8) else {
        return Ok(None);
    };
    let Some(game) = data.game.get() else { return Ok(None) };
    game.specialties.quote(game, npc, item, data.specialty.ratio)
}

pub(super) fn install(lua: &Lua, store: &Table, state: &Shared) -> mlua::Result<()> {
    let state = state.clone();
    store.set(
        "GetSpecialtyRatioRefund",
        lua.create_function(move |lua, args: MultiValue| {
            //394af190 requires numeric ScriptAny4 (including NaN/Inf). Its converted
            // f32/CVTT integer is ignored by394ae8c0; no supplied refund enters price math.
            if !matches!(args.get(1), Some(Value::Integer(_) | Value::Number(_))) {
                return Ok(MultiValue::new());
            }
            let quote = current(&state);
            match quote {
                Ok(Some((amount, coin))) => {
                    let mut values = vec![Value::Boolean(true), Value::Number(f64::from(amount as f32))];
                    if coin != 0 {
                        values.push(Value::Number(f64::from(coin as f32)));
                    }
                    Ok(MultiValue::from_vec(values))
                }
                Err(code) => {
                    super::error(lua, code)?;
                    Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
                }
                Ok(None) => Ok(MultiValue::from_vec(vec![Value::Boolean(false)])),
            }
        })?,
    )
}

#[cfg(test)]
mod tests;
