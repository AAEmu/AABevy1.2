//! Exact original return shapes, numeric domains, authored ordering and local CRT calendars.
use super::*;

fn result(table: Table) -> MultiValue {
    MultiValue::from_iter([Value::Table(table)])
}
fn text(lua: &Lua, bytes: &[u8]) -> mlua::Result<mlua::LuaString> {
    lua.create_string(bytes.split(|&byte| byte == 0).next().unwrap_or_default())
}
fn local_date(lua: &Lua, raw: u64) -> Option<Table> {
    // Native ignores CRT failure and exposes uninitialized output. Refuse that undefined domain.
    if raw > 32_535_244_799 {
        return None;
    }
    let os: Table = lua.globals().get("os").ok()?;
    os.call_function::<Option<Table>>("date", ("*t", raw as i64)).ok().flatten()
}
fn date(lua: &Lua, raw: u64) -> mlua::Result<Option<Table>> {
    let Some(date) = local_date(lua, raw) else { return Ok(None) };
    let out = lua.create_table()?;
    for (key, source) in [
        ("year", "year"),
        ("month", "month"),
        ("day", "day"),
        ("hour", "hour"),
        ("minute", "min"),
        ("second", "sec"),
    ] {
        out.raw_set(key, signed(date.raw_get::<i32>(source)?))?;
    }
    Ok(Some(out))
}
pub(super) fn calendar(lua: &Lua, [year, month, day, hour, minute]: [i32; 5]) -> Option<u64> {
    if year == 0 || month == 0 || day == 0 {
        return Some(0);
    }
    let input = lua.create_table().ok()?;
    for (key, value) in [
        ("year", year.max(1900)),
        ("month", month.clamp(1, 12)),
        ("day", day.clamp(1, 31)),
        ("hour", hour.clamp(0, 23)),
        ("min", minute.clamp(0, 59)),
        ("sec", 0),
    ] {
        input.raw_set(key, value).ok()?;
    }
    input.raw_set("isdst", false).ok()?;
    let os: Table = lua.globals().get("os").ok()?;
    os.call_function::<i64>("time", input).ok().map(|value| value as u64)
}
pub(super) fn activated(lua: &Lua, projection: &Projection, tables: &Tables) -> bool {
    let Some(entry) = tables.get(projection.selected_rank) else {
        return false;
    };
    let Some(ack) = projection.received.ack_time_raw else {
        return false;
    };
    let Some(current) = local_date(lua, ack) else { return false };
    let Ok(day) = current.raw_get::<i32>("wday") else { return false };
    let Ok(hour) = current.raw_get::<i32>("hour") else { return false };
    if day == 2 && (5..7).contains(&hour) {
        return false;
    }
    let (Some(start), Some(end)) = (calendar(lua, entry.start), calendar(lua, entry.end)) else {
        return false;
    };
    (start == 0 || ack >= start)
        && (end == 0 || ack <= end)
        && (entry.weekday == 8 || entry.weekday == day)
        && (entry.start_hour == entry.end_hour || (hour >= entry.start_hour && hour < entry.end_hour))
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Rank")?;
    let s = state.clone();
    api.raw_set(
        "GetRankCount",
        lua.create_function(move |_, _: MultiValue| {
            // Original DB initialization precedes this constructor-time call, as for the skill book.
            Ok(unsigned(s.borrow().game.0.wait().ranking.rows.len() as u32))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetRankInfo",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(index) = integer(args.get(1))
                .and_then(|index| index.checked_sub(1))
                .filter(|index| *index >= 0)
            else {
                return Ok(MultiValue::new());
            };
            let data = s.borrow();
            let Some(entry) = data.game.0.wait().ranking.rows.get(index as usize) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            for (key, value) in [("name", &entry.name), ("v1", &entry.v1), ("v2", &entry.v2)] {
                out.raw_set(key, text(lua, value.as_bytes())?)?;
            }
            Ok(result(out))
        })?,
    )?;
    for (method, default) in [("GetDefaultTabIndex", true), ("GetSelectedTabIndex", false)] {
        let s = state.clone();
        api.raw_set(
            method,
            lua.create_function(move |_, _: MultiValue| {
                let data = s.borrow();
                let view = data.ranking.current();
                Ok(signed(if default { view.default_tab } else { view.selected_tab }))
            })?,
        )?;
    }
    let s = state.clone();
    api.raw_set(
        "IsOnGoingRank",
        lua.create_function(move |_, args: MultiValue| {
            let Some(index) = integer(args.get(1)) else {
                return Ok(MultiValue::new());
            };
            let data = s.borrow();
            let value = index
                .checked_sub(1)
                .filter(|index| *index >= 0)
                .and_then(|index| data.ranking.current().tabs.get(index as usize))
                .is_some_and(|&(present, id)| present && id != 0);
            Ok(MultiValue::from_iter([Value::Boolean(value)]))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "IsActivatedPeriod",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            Ok(data
                .game
                .get()
                .is_some_and(|game| activated(lua, data.ranking.current(), &game.ranking)))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetPlayerRecord",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = data.ranking.current();
            let Some(board) = view.received.by_id.get(&view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let Some(out) = date(lua, board.personal.recorded_raw)? else {
                return Ok(MultiValue::new());
            };
            out.raw_set("v1", signed(board.personal.v1))?;
            out.raw_set("v2", signed(board.personal.v2))?;
            Ok(result(out))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetSnapshot",
        lua.create_function(move |lua, _: MultiValue| {
            let entries = {
                let data = s.borrow();
                let view = data.ranking.current();
                let Some(board) = view.received.by_id.get(&view.selected_rank) else {
                    return Ok(MultiValue::new());
                };
                board.snapshot.as_ref().map_or_else(Vec::new, |snapshot| snapshot.entries.clone())
            };
            let out = lua.create_table()?;
            for entry in entries {
                let Some(row) = date(lua, entry.updated_raw)? else {
                    return Ok(MultiValue::new());
                };
                let mut data = s.borrow_mut();
                if let Some(name) = name(&mut data, entry.character_id) {
                    row.raw_set("name", text(lua, &name)?)?;
                }
                if let Some(world) = data.ranking.current().providers.worlds.get(&entry.world_id) {
                    row.raw_set("worldName", text(lua, world.as_bytes())?)?;
                }
                row.raw_set("v1", signed(entry.v1))?;
                row.raw_set("v2", signed(entry.v2))?;
                // Native integer table indexing preserves sparse/negative ranking keys and last-wins duplicates.
                out.raw_set(entry.ranking, row)?;
            }
            Ok(result(out))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetSnapshotScope",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = data.ranking.current();
            let Some(board) = view.received.by_id.get(&view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let Some(tables) = data.game.get().map(|game| &game.ranking) else {
                return Ok(MultiValue::new());
            };
            let Some(entry) = tables.get(view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            let snapshots = lua.create_table()?;
            let scopes = lua.create_table()?;
            if let Some(snapshot) = &board.snapshot {
                for received in &snapshot.scopes {
                    let row = lua.create_table()?;
                    for (key, value) in [("scope", received.scope_id), ("v1", received.v1), ("v2", received.v2)] {
                        row.raw_set(key, signed(value))?;
                    }
                    snapshots.raw_push(row)?;
                }
            }
            for (index, id) in entry.scope_ids.iter().enumerate() {
                if let Some(scope) = tables.scopes.get(id) {
                    let row = lua.create_table()?;
                    row.raw_set("name", text(lua, scope.name.as_bytes())?)?;
                    row.raw_set("from", signed(scope.from))?;
                    row.raw_set("to", signed(scope.to))?;
                    scopes.raw_set(index + 1, row)?;
                }
            }
            out.raw_set("snapshotScopes", snapshots)?;
            out.raw_set("rankScopes", scopes)?;
            Ok(result(out))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetSnapshotUpdatedTime",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = data.ranking.current();
            let Some(board) = view.received.by_id.get(&view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let raw = board.snapshot.as_ref().map_or(0, |snapshot| snapshot.updated_raw);
            let (Some(prev), Some(next)) = (
                date(lua, raw)?,
                date(lua, view.next_update.get(&view.selected_rank).copied().unwrap_or(0))?,
            ) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            out.raw_set("prev", prev)?;
            out.raw_set("next", next)?;
            Ok(result(out))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetResetPeriod",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = data.ranking.current();
            let Some(board) = view.received.by_id.get(&view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let Some(entry) = data.game.get().and_then(|game| game.ranking.get(view.selected_rank)) else {
                return Ok(MultiValue::new());
            };
            let reset = board.snapshot.as_ref().map_or(0, |snapshot| snapshot.reset_raw);
            let reward = reset.wrapping_add((i64::from(entry.reset_week) * 604800) as u64);
            let (Some(start), Some(end), Some(reward)) = (
                date(lua, reset.wrapping_add(3600))?,
                date(lua, reward.wrapping_sub(3600))?,
                date(lua, reward)?,
            ) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            out.raw_set("periodStart", start)?;
            out.raw_set("periodEnd", end)?;
            out.raw_set("periodReward", reward)?;
            Ok(result(out))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetReward",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let view = data.ranking.current();
            if !view.received.by_id.contains_key(&view.selected_rank) {
                return Ok(MultiValue::new());
            }
            let Some(game) = data.game.get() else {
                return Ok(MultiValue::new());
            };
            let tables = &game.ranking;
            let Some(entry) = tables.get(view.selected_rank) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            for &(scope, reward) in &entry.reward_links {
                let (Some(scope), Some(reward)) = (tables.scopes.get(&scope), tables.rewards.get(&reward)) else {
                    return Ok(MultiValue::new());
                };
                let row = lua.create_table()?;
                row.raw_set("scopeName", text(lua, scope.name.as_bytes())?)?;
                row.raw_set("scopeFrom", signed(scope.from))?;
                row.raw_set("scopeTo", signed(scope.to))?;
                row.raw_set("name", text(lua, reward.name.as_bytes())?)?;
                if let Some(title) = tables.appellations.get(&reward.appellation_id) {
                    let nested = lua.create_table()?;
                    nested.raw_set("name", text(lua, title.name.as_bytes())?)?;
                    nested.raw_set("buffType", unsigned(title.buff_id))?;
                    row.raw_set("appellation", nested)?;
                }
                if game.items.contains_key(&reward.item_id) {
                    let nested = lua.create_table()?;
                    nested.raw_set("type", unsigned(reward.item_id))?;
                    nested.raw_set("grade", unsigned(reward.item_grade.into()))?;
                    nested.raw_set("count", signed(reward.item_count))?;
                    row.raw_set("item", nested)?;
                }
                out.raw_push(row)?;
            }
            Ok(result(out))
        })?,
    )
}
