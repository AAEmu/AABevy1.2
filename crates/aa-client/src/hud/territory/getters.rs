use super::*;

/// Host Lua's localtime conversion supplies the selected client's local timezone.
/// Only the six native fields escape; unsupported CRT dates remain unavailable.
pub(super) fn date(
    lua: &Lua,
    local_date: &mlua::Function,
    timestamp: i64,
) -> mlua::Result<MultiValue> {
    // _localtime64_s's supported nonnegative range, through year3000. All these
    // seconds are exactly representable by Lua5.1's double.
    if !(0..=32_535_215_999).contains(&timestamp) {
        return Ok(MultiValue::new());
    }
    let Ok(source) = local_date.call::<Table>(("*t", timestamp)) else {
        return Ok(MultiValue::new());
    };
    let out = lua.create_table()?;
    for (target, source_key) in [
        ("year", "year"),
        ("month", "month"),
        ("day", "day"),
        ("hour", "hour"),
        ("minute", "min"),
        ("second", "sec"),
    ] {
        out.raw_set(target, source.raw_get::<i32>(source_key)?)?;
    }
    Ok(MultiValue::from_iter([Value::Table(out)]))
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Dominion")?;
    for (name, leader) in [
        ("GetOwnerExpeditionName", false),
        ("GetOwnerPlayerName", true),
    ] {
        let state = state.clone();
        api.set(
            name,
            lua.create_function(move |lua, (_, key): (Value, Value)| {
                let data = state.borrow();
                let faction =
                    row(&data, key).and_then(|row| data.expedition.faction(row.expedition_id));
                text(
                    lua,
                    faction.map_or(&[], |f| if leader { &f.owner_name } else { &f.name }),
                )
            })?,
        )?;
    }
    for (name, aa_point) in [("GetPeaceTaxMoney", false), ("GetPeaceTaxAAPoint", true)] {
        let state = state.clone();
        api.set(
            name,
            lua.create_function(move |lua, (_, key): (Value, Value)| {
                let data = state.borrow();
                let value = row(&data, key).map(|r| {
                    if aa_point {
                        r.peace_tax_aa_point
                    } else {
                        r.peace_tax_money
                    }
                });
                lua.create_string(value.map(|n| n.to_string()).unwrap_or_default())
            })?,
        )?;
    }
    let state_date = state.clone();
    let os: Table = lua.globals().get("os")?;
    let local_date: mlua::Function = os.get("date")?;
    api.set(
        "GetReignStartDate",
        lua.create_function(move |lua, (_, key): (Value, Value)| {
            let timestamp = row(&state_date.borrow(), key).map(|r| r.reign_start_time);
            match timestamp {
                Some(timestamp) => date(lua, &local_date, timestamp),
                None => Ok(MultiValue::new()),
            }
        })?,
    )?;
    let state_rate = state.clone();
    api.set(
        "GetTaxRate",
        lua.create_function(move |_, (_, key): (Value, Value)| {
            Ok(
                row(&state_rate.borrow(), key).map_or_else(MultiValue::new, |r| {
                    MultiValue::from_iter([Value::Number(f64::from(r.tax_rate as f32))])
                }),
            )
        })?,
    )?;
    let state_tax = state.clone();
    api.set(
        "IsTaxationWithoutMyExpedition",
        lua.create_function(move |_, (_, key): (Value, Value)| {
            Ok(if row(&state_tax.borrow(), key).is_some() {
                MultiValue::from_iter([Value::Boolean(true)])
            } else {
                MultiValue::new()
            })
        })?,
    )?;
    let state_owner = state.clone();
    api.set(
        "IsDominionOwner",
        lua.create_function(move |_, (_, key): (Value, Value)| {
            let data = state_owner.borrow();
            Ok(row(&data, key).is_some_and(|r| owner(&data, r)))
        })?,
    )?;
    let state_zone = state.clone();
    api.set(
        "GetZoneGroupName",
        lua.create_function(move |lua, (_, key): (Value, Value)| {
            let data = state_zone.borrow();
            let name = integer(key)
                .and_then(|key| data.game.get()?.zone_groups.get(&(u32::from(key as u16))));
            lua.create_string(name.map_or("", |r| r.text.as_str()))
        })?,
    )?;
    let house: Table = lua.globals().get("X2House")?;
    let state = state.clone();
    house.set(
        "GetTaxations",
        lua.create_function(move |lua, (_, rate): (Value, Value)| {
            let Some(rate) = integer(rate) else {
                return Ok(MultiValue::new());
            };
            let data = state.borrow();
            let Some(game) = data.game.get() else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            for tax in &game.territory.taxes {
                let value = lua.create_table()?;
                value.raw_set("desc", tax.desc.as_str())?;
                value.raw_set("taxString", catalog::preview(tax.amount, rate).to_string())?;
                out.raw_push(value)?;
            }
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    Ok(())
}
