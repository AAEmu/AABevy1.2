//! The static public-farm tables the original farm windows read (`x2ui/commonfarm`: `info.alb`,
//! `my_farm_info.alb`, unchanged Lua). `X2.GetFarmGroups`, `GetFarmGorupInfo` (the client's typo),
//! `GetFarmGroupDoodadList` and `GetCommonFarmInfo` answer from `farm_groups`,
//! `farm_group_doodads` and `common_farms`; see `docs/aa-client-farm.md`.

use mlua::{Lua, Table, Value};

use super::api::Shared;
use super::data::GameData;

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let x2: Table = lua.globals().get("X2")?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGroups",
        lua.create_function(move |lua, _: mlua::MultiValue| {
            let out = lua.create_table()?;
            let data = b.borrow();
            if let Some(game) = data.game.get() {
                for (i, id) in game.housing.farm_group_ids.iter().enumerate() {
                    let Some(group) = game.housing.farm_groups.get(id) else {
                        continue;
                    };
                    // `FUN_394447C0`: each entry is `{ type = id, name = name }`.
                    let entry = lua.create_table()?;
                    entry.set("type", i64::from(*id))?;
                    entry.set("name", group.name.as_str())?;
                    out.raw_set(i + 1, entry)?;
                }
            }
            Ok(out)
        })?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGorupInfo",
        lua.create_function(move |lua, (_self, kind): (Value, u32)| -> mlua::Result<mlua::MultiValue> {
            let data = b.borrow();
            let Some(group) = data.game.get().and_then(|g| g.housing.farm_groups.get(&kind)) else {
                return Ok(mlua::MultiValue::new());
            };
            // `FUN_394445F0` -> `FUN_39441450`: name at +0xc, count at +4, description at +8.
            Ok(mlua::MultiValue::from_vec(vec![
                Value::String(lua.create_string(&group.name)?),
                Value::Integer(i64::from(group.count)),
                Value::String(lua.create_string(&group.description)?),
            ]))
        })?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGroupDoodadList",
        lua.create_function(move |lua, (_self, kind): (Value, u32)| {
            let data = b.borrow();
            let Some(game) = data.game.get() else { return Ok(Value::Nil) };
            if !game.housing.farm_groups.contains_key(&kind) {
                return Ok(Value::Nil);
            }
            let out = lua.create_table()?;
            let mut i = 0;
            for d in game.housing.farm_group_doodads.iter().filter(|d| d.farm_group_id == kind) {
                // `FUN_39445BA0` returns the doodad template's name, in row order.
                let name = game.doodad.name(d.doodad_id).unwrap_or_default();
                out.raw_set(i + 1, name)?;
                i += 1;
            }
            Ok(Value::Table(out))
        })?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetCommonFarmInfo",
        lua.create_function(move |lua, (_self, id): (Value, u32)| -> mlua::Result<mlua::MultiValue> {
            let data = b.borrow();
            let Some(farm) = data.game.get().and_then(|g| g.housing.common_farms.iter().find(|f| f.id == id)) else {
                return Ok(mlua::MultiValue::new());
            };
            // `FUN_394445B0` -> `FUN_39441370`: name at +0x10, guard_time at +0xc,
            // farm_group_id at +8.
            Ok(mlua::MultiValue::from_vec(vec![
                Value::String(lua.create_string(&farm.name)?),
                Value::Integer(i64::from(farm.guard_time)),
                Value::Integer(i64::from(farm.farm_group_id)),
            ]))
        })?,
    )?;

    Ok(())
}

pub fn plugin(_app: &mut bevy::prelude::App) {}

#[cfg(test)]
mod tests;
