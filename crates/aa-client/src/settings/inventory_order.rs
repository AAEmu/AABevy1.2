//! Account-file storage for character/container-local physical slot views, independent of options.
use super::{Json, Store, json};
use mlua::{Lua, Table};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

// Application input budget; the original default-tab record contains130 physical slots.
const MAX_SLOTS: usize = 256;

pub(super) fn install(lua: &Lua, net: &Table, store: Arc<Mutex<Store>>) -> mlua::Result<()> {
    let state = store.clone();
    net.set(
        "inventory_order",
        lua.create_function(move |lua, (owner, container): (u32, u8)| {
            if owner == 0 || !matches!(container, 2 | 3) {
                return Ok(None);
            }
            let state = state.lock().unwrap_or_else(|e| e.into_inner());
            let key = format!("{owner}:{container}");
            let Some(entries) = state.data.get("inventory_order").and_then(|v| v.get(&key)).and_then(Json::as_array) else {
                return Ok(None);
            };
            if entries.is_empty() || entries.len() > MAX_SLOTS {
                return Ok(None);
            }
            let out = lua.create_table()?;
            for entry in entries {
                let (Some(slot), Some(uid)) = (entry.get("slot").and_then(Json::as_u64), entry.get("uid").and_then(Json::as_str)) else {
                    return Ok(None);
                };
                let value = lua.create_table()?;
                value.set("slot", slot)?;
                value.set("uid", uid)?;
                out.raw_push(value)?;
            }
            Ok(Some(out))
        })?,
    )?;
    net.set(
        "save_inventory_order",
        lua.create_function(move |_, (owner, container, entries): (u32, u8, Table)| {
            if owner == 0 || !matches!(container, 2 | 3) {
                return Ok(());
            }
            let len = entries.raw_len();
            if len == 0 || len > MAX_SLOTS {
                return Err(mlua::Error::runtime("invalid inventory view length"));
            }
            let mut seen = HashSet::new();
            let mut values = Vec::with_capacity(len);
            for value in entries.sequence_values::<Table>() {
                let value = value?;
                let slot: usize = value.get("slot")?;
                let uid: String = value.get("uid")?;
                if slot == 0 || slot > len || !seen.insert(slot) || uid.parse::<u64>().is_err() {
                    return Err(mlua::Error::runtime("invalid inventory view identity"));
                }
                values.push(json!({"slot":slot,"uid":uid}));
            }
            if values.len() != len {
                return Err(mlua::Error::runtime("incomplete inventory view"));
            }
            let mut state = store.lock().unwrap_or_else(|e| e.into_inner());
            let orders = state.data.entry("inventory_order").or_insert_with(|| json!({}));
            if !orders.is_object() {
                *orders = json!({});
            }
            let key = format!("{owner}:{container}");
            let value = Json::Array(values);
            if orders.get(&key) != Some(&value) {
                orders[key] = value;
                state.dirty = true;
            }
            if let Err(error) = state.save() {
                bevy::log::warn!("settings: inventory order save: {error:#}");
            }
            Ok(())
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_container_orders_keep_u64_identity_and_pending_options_separate() -> mlua::Result<()> {
        let path = std::env::temp_dir().join(format!("aa-inventory-order-{}.json", std::process::id()));
        let state = Arc::new(Mutex::new(Store {
            path: path.clone(),
            ..Default::default()
        }));
        let lua = Lua::new();
        let net = lua.create_table()?;
        super::super::install_store(&lua, &net, state.clone())?;
        lua.globals().set("NET", net)?;
        lua.load(
            r#"
            NET.option_set(35,0.1)
            NET.save_inventory_order(47,2,{{slot=2,uid='9007199254740993'},{slot=1,uid='0'}})
            NET.save_inventory_order(47,3,{{slot=1,uid='18446744073709551615'}})
            NET.save_inventory_order(48,2,{{slot=1,uid='7'}})
            assert(not pcall(NET.save_inventory_order,47,2,{{slot=1,uid='9'},{slot=1,uid='8'}}))
            assert(not pcall(NET.save_inventory_order,47,2,{{slot=1,uid='18446744073709551616'}}))
        "#,
        )
        .exec()?;
        let reopened = Arc::new(Mutex::new(Store::open(path.clone())));
        let net = lua.create_table()?;
        install(&lua, &net, reopened.clone())?;
        lua.globals().set("SAVED", net)?;
        lua.load(
            r#"
            local bag=SAVED.inventory_order(47,2)
            assert(bag[1].slot==2 and bag[1].uid=='9007199254740993')
            assert(SAVED.inventory_order(47,3)[1].uid=='18446744073709551615')
            assert(SAVED.inventory_order(48,2)[1].uid=='7')
            assert(SAVED.inventory_order(49,2)==nil)
        "#,
        )
        .exec()?;
        assert!(reopened.lock().unwrap().data.get("options").is_none(), "sorting cannot apply pending options");
        std::fs::remove_file(path).unwrap();
        Ok(())
    }
}
