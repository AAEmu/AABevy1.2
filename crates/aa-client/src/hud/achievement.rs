//! Original achievement UI over received progress, static descriptors and six local tracing slots.
use super::api::{HudBridge, Shared};
use aa_net::{AchievementNotice, Achievements, Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
mod catalog;
#[allow(dead_code)]
pub(super) mod fixture;
mod persistence;
mod query;
mod roadmap;
pub(super) mod roadmap_fixture;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod roadmap_tests;
pub(super) use catalog::Tables;

#[derive(Default)]
pub(super) struct View {
    pub(super) cache: Achievements,
    catalog_bound: bool,
    recent: Vec<u32>,
    tracing: [u32; 6],
    persistence: persistence::State,
    roadmap_dirty: bool,
}

fn bind(data: &mut super::api::HudData) {
    if !data.achievement.catalog_bound {
        if let Some(game) = data.game.get() {
            data.achievement.cache.set_catalog(game.achievements.rules());
            data.achievement.catalog_bound = true;
        }
    }
}

pub(super) fn bind_catalog(mut bound: Local<bool>, bridge: NonSend<HudBridge>, received: Option<ResMut<Achievements>>) {
    bind(&mut bridge.0.borrow_mut());
    if *bound {
        return;
    }
    let data = bridge.0.borrow();
    let Some(game) = data.game.get() else { return };
    let Some(mut received) = received else { return };
    received.set_catalog(game.achievements.rules());
    *bound = true;
}

fn integer(value: &Value) -> Option<i32> {
    let value = match value {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    Some(if value.is_finite() && (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    })
}

fn numbers(args: &MultiValue, count: usize) -> Option<Vec<i32>> {
    (0..count).map(|i| args.get(i + 1).and_then(integer)).collect()
}

fn number(value: impl Into<f64>) -> Value {
    Value::Number(value.into())
}
fn unsigned(value: u32) -> Value {
    number(f64::from(value as f32))
}
fn signed(value: i32) -> Value {
    number(f64::from(value as f32))
}
fn result(table: Table) -> MultiValue {
    MultiValue::from_iter([Value::Table(table)])
}
fn updated(lua: &Lua) {
    aa_ui::lua::fire_ui_event(lua, "ACHIEVEMENT_UPDATE", vec![]);
}

fn remove_trace(view: &mut View, id: u32) -> bool {
    let Some(slot) = view.tracing.iter_mut().find(|slot| **slot == id) else {
        return false;
    };
    *slot = 0;
    view.persistence.record(&view.tracing);
    true
}

///394245c0 ignores _localtime64_s errors. Its six uninitialized-time sentinel fields are observable.
fn date(lua: &Lua, timestamp: i64) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let valid = if (0..=32_535_244_799).contains(&timestamp) {
        let os: Table = lua.globals().get("os")?;
        os.call_function::<Option<Table>>("date", ("*t", timestamp)).ok().flatten()
    } else {
        None
    };
    for (key, fallback) in [
        ("year", 1899),
        ("month", 0),
        ("day", -1),
        ("hour", -1),
        ("minute", -1),
        ("second", -1),
    ] {
        let source = match key {
            "minute" => "min",
            "second" => "sec",
            other => other,
        };
        let value = valid.as_ref().and_then(|date| date.get::<i32>(source).ok()).unwrap_or(fallback);
        out.set(key, signed(value))?;
    }
    Ok(out)
}

fn info(lua: &Lua, data: &super::api::HudData, id: u32) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let view = &data.achievement;
    if let Some(tables) = data.game.get().map(|game| &game.achievements) {
        if let Some(entry) = tables.get(id) {
            out.set("type", unsigned(entry.id))?;
            for (key, value) in [("name", &entry.name), ("summary", &entry.summary), ("desc", &entry.desc)] {
                out.set(key, value.split('\0').next().unwrap_or_default())?;
            }
            out.set("mainCategory", signed(entry.main))?;
            out.set("subCategory", signed(entry.sub))?;
            out.set("completeNum", signed(entry.rule.completed_amount()))?;
            out.set("isNew", view.recent.contains(&id))?;
            out.set("highRank", entry.parent == 0)?;
            out.set("reward", unsigned(entry.reward))?;
            let icon = if entry.parent != 0 && entry.icon_id == 0 {
                tables.get(entry.parent).map(|parent| parent.icon_id)
            } else {
                Some(entry.icon_id)
            };
            if let Some(icon_id) = icon {
                let suffix = tables
                    .icons
                    .get(&icon_id)
                    .filter(|name| !name.is_empty())
                    .map_or("_bad.dds", String::as_str);
                out.set("iconPath", format!("Game\\ui\\icon\\{suffix}"))?;
            }
            if !entry.objectives.is_empty() {
                let objectives = lua.create_table()?;
                for &id in &entry.objectives {
                    objectives.raw_push(signed(id))?;
                }
                out.set("objective", objectives)?;
            }
        }
    }
    let row = view.cache.get(id).copied().unwrap_or_default();
    out.set("current", signed(row.amount))?;
    out.set("complete", row.completed_at != 0)?;
    if row.completed_at != 0 {
        out.set("completeDate", date(lua, row.completed_at)?)?;
    }
    out.set("tracing", view.tracing.contains(&id))?;
    Ok(out)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    bind(&mut bridge.borrow_mut());
    roadmap::install(lua, bridge)?;
    let api: Table = lua.globals().get("X2Achievement")?;
    api.set("GetMainCategoryCount", lua.create_function(|_, _: MultiValue| Ok(5.0))?)?;
    api.set(
        "GetSubCategoryCount",
        lua.create_function(|_, args: MultiValue| {
            Ok(numbers(&args, 1).map_or_else(MultiValue::new, |v| MultiValue::from_iter([signed(query::sub_count(v[0]))])))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetAchievementInfo",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 1) else {
                return Ok(MultiValue::new());
            };
            Ok(result(info(lua, &state.borrow(), values[0] as u32)?))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetAchievementList",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 3) else {
                return Ok(MultiValue::new());
            };
            let data = state.borrow();
            let out = lua.create_table()?;
            if let Some(game) = data.game.get() {
                for id in query::list(&game.achievements, &data.achievement, values[0], values[1], values[2])? {
                    out.raw_push(unsigned(id))?;
                }
            }
            Ok(result(out))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetSubcategoryInfo",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 2) else {
                return Ok(MultiValue::new());
            };
            let data = state.borrow();
            let out = lua.create_table()?;
            let counts = if let Some(game) = data.game.get() {
                (
                    query::list(&game.achievements, &data.achievement, values[0], values[1], 2)?.len(),
                    query::list(&game.achievements, &data.achievement, values[0], values[1], 1)?.len(),
                )
            } else {
                (0, 0)
            };
            out.set("complete", unsigned(counts.0 as u32))?;
            out.set("total", unsigned(counts.1 as u32))?;
            Ok(result(out))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetTracingAchievementCount",
        lua.create_function(move |_, _: MultiValue| {
            Ok((
                f64::from(state.borrow().achievement.tracing.iter().filter(|&&id| id != 0).count() as f32),
                6.0,
            ))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "IsTracingAchievement",
        lua.create_function(move |_, args: MultiValue| {
            Ok(numbers(&args, 1).map_or_else(MultiValue::new, |v| {
                MultiValue::from_iter([Value::Boolean(state.borrow().achievement.tracing.contains(&(v[0] as u32)))])
            }))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "AddTracingAchievement",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 1) else {
                return Ok(MultiValue::new());
            };
            let id = values[0] as u32;
            let mut data = state.borrow_mut();
            let view = &mut data.achievement;
            let added = if view.tracing.contains(&id) {
                false
            } else if let Some(slot) = view.tracing.iter_mut().find(|slot| **slot == 0) {
                *slot = id;
                view.persistence.record(&view.tracing);
                true
            } else {
                false
            };
            drop(data);
            if added {
                updated(lua);
            }
            Ok(MultiValue::from_iter([Value::Boolean(added)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "RemoveTracingAchievement",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 1) else { return Ok(()) };
            let changed = remove_trace(&mut state.borrow_mut().achievement, values[0] as u32);
            if changed {
                updated(lua);
            }
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetNewCompleteAchievementCount",
        lua.create_function(move |_, _: MultiValue| Ok(f64::from(state.borrow().achievement.recent.len() as f32)))?,
    )?;
    let state = bridge.clone();
    api.set(
        "RemoveNewCompleteAchievement",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(values) = numbers(&args, 1) else { return Ok(()) };
            let mut data = state.borrow_mut();
            let recent = &mut data.achievement.recent;
            let index = recent.iter().position(|&id| id == values[0] as u32);
            if let Some(index) = index {
                recent.remove(index);
            }
            drop(data);
            if index.is_some() {
                updated(lua);
            }
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "RemoveAllNewCompleteAchievement",
        lua.create_function(move |lua, _: MultiValue| {
            state.borrow_mut().achievement.recent.clear();
            updated(lua);
            Ok(())
        })?,
    )?;
    Ok(())
}

fn dispatch(rt: Option<&UiRuntime>, bridge: &Shared, event: &Event) {
    let notices = {
        let mut data = bridge.borrow_mut();
        bind(&mut data);
        let (_, notices) = data.achievement.cache.apply_with_notices(event);
        let view = &mut data.achievement;
        let reset_owner = match event {
            Event::CharacterSelected(state) => Some(state.character.id),
            Event::PlayerSpawn(spawn) if view.persistence.owner != spawn.character().id => Some(spawn.character().id),
            Event::LoginSucceeded(_)
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::LeftWorld { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting) => Some(0),
            _ => None,
        };
        if let Some(owner) = reset_owner {
            view.recent.clear();
            view.tracing = [0; 6];
            view.persistence.reset(owner);
            view.roadmap_dirty = false;
        }
        if let Event::UiData(aa_net::UiDataEvent::Response(response)) = event {
            if let Some(slots) = view.persistence.receive(response) {
                view.tracing = slots;
                view.roadmap_dirty |= response.raw_length > 0;
            }
        }
        notices
    };
    for notice in notices {
        match notice {
            AchievementNotice::Completed(id) => {
                bridge.borrow_mut().achievement.recent.push(id);
                if let Some(rt) = rt {
                    rt.fire_event("COMPLETE_ACHIEVEMENT", f64::from(id as f32));
                }
                let removed = remove_trace(&mut bridge.borrow_mut().achievement, id);
                if removed {
                    if let Some(rt) = rt {
                        updated(&rt.lua);
                    }
                }
            }
            AchievementNotice::Updated => {
                if let Some(rt) = rt {
                    updated(&rt.lua);
                }
            }
            AchievementNotice::ItemSent(id) => {
                let name = bridge
                    .borrow()
                    .game
                    .get()
                    .and_then(|game| game.achievements.get(id))
                    .map(|entry| entry.name.clone());
                if let (Some(rt), Some(name)) = (rt, name) {
                    rt.fire_event("ACHIEVEMENT_ITEM_SENT", name);
                }
            }
        }
    }
    roadmap::flush(rt, bridge);
}

/// Native SaveAll precedes CSLeaveWorld. A socket write has no save acknowledgement.
pub(super) fn save_for_leave(data: &mut super::api::HudData, how: i64) -> Option<aa_net::UiDataCommand> {
    super::session::leave_target(how)?;
    if data.disconnected {
        return None;
    }
    data.achievement.persistence.save(data.character_id)
}

fn track(mut events: MessageReader<NetEvent>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>, bridge: NonSend<HudBridge>) {
    for NetEvent(event) in events.read() {
        dispatch(if built.up { rt.as_deref() } else { None }, &bridge.0, event);
    }
    roadmap::flush(if built.up { rt.as_deref() } else { None }, &bridge.0);
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(First, bind_catalog)
        .add_systems(Update, track.before(super::sync_hud).before(super::prebuild_hud));
}
