//! Native split/reparent notifications. Raw tab records survive the temporary no-owner phase.
use super::{Shared, View, ensure, model::Extra};
use mlua::{Lua, Table};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(super) struct Detached {
    source: i32,
    extra: Extra,
    locked: bool,
}
pub(super) type Pending = BTreeMap<i32, Detached>;

pub(super) fn extra_json(view: &View) -> Json {
    if view.transfers.is_empty() {
        return view.model.extra_json();
    }
    let mut extra = view.model.extra.clone();
    for (&raw, pending) in &view.transfers {
        if view.model.tabs.contains_key(&raw) && !view.model.windows.contains_key(&pending.source) {
            extra.entry(pending.source).or_insert(pending.extra).method = pending.extra.method;
        }
    }
    Json::Array(
        extra
            .into_iter()
            .map(|(id, e)| json!({"id":id,"selected":e.selected,"method":e.method}))
            .collect(),
    )
}

pub(super) fn saved_locks(view: &View) -> [bool; 8] {
    let mut locks = view.locks;
    for (&raw, pending) in &view.transfers {
        if view.model.tabs.contains_key(&raw) && !view.model.windows.contains_key(&pending.source) {
            locks[pending.source as usize] = pending.locked;
        }
    }
    locks
}

fn persist(lua: &Lua, state: &Shared) -> bool {
    let data = state.borrow();
    let view = &data.chat_config;
    let Some(key) = view.key() else { return true };
    crate::settings::chat_save_transfer(
        lua,
        &key,
        view.model.tabs_json(),
        extra_json(view),
        json!(saved_locks(view)),
    )
}

fn allocate(view: &mut View) -> Option<i32> {
    let id = (0..8).find(|id| !view.model.windows.contains_key(id))?;
    view.model.windows.insert(id, Vec::new());
    view.model.extra.insert(id, Extra::default());
    view.locks[id as usize] = false;
    Some(id)
}

fn release(view: &mut View, id: i32) {
    if !(0..8).contains(&id)
        || view
            .model
            .windows
            .get(&id)
            .is_some_and(|rows| !rows.is_empty())
    {
        return;
    }
    view.model.windows.remove(&id);
    view.model.extra.remove(&id);
    view.locks[id as usize] = false;
    // Pending raw records and their recovery extras are deliberately retained.
}

fn detached(view: &mut View, window: i32, raw: i32) {
    if view.model.current_window(raw) != Some(window) {
        return;
    }
    view.transfers.insert(
        raw,
        Detached {
            source: window,
            extra: view.model.extra.get(&window).copied().unwrap_or_default(),
            locked: view.locks[window as usize],
        },
    );
    let rows = view.model.windows.get_mut(&window).unwrap();
    rows.retain(|id| *id != raw);
    if rows.is_empty() {
        release(view, window);
    }
}

fn attached(view: &mut View, window: i32, raw: i32, index: usize) {
    if !(0..8).contains(&window) {
        return;
    }
    if !view.model.tabs.contains_key(&raw) {
        view.transfers.remove(&raw);
        return;
    }
    if view
        .model
        .current_window(raw)
        .is_some_and(|owner| owner != window)
        && !view.transfers.contains_key(&raw)
    {
        return;
    }
    let pending = view.transfers.remove(&raw);
    if !view.model.windows.contains_key(&window)
        && let Some(pending) = pending.filter(|p| p.source == window)
    {
        let selected = view
            .model
            .extra
            .get(&window)
            .map_or(pending.extra.selected, |e| e.selected);
        view.model.extra.insert(
            window,
            Extra {
                selected,
                ..pending.extra
            },
        );
        view.locks[window as usize] = pending.locked;
    }
    for rows in view.model.windows.values_mut() {
        rows.retain(|id| *id != raw);
    }
    let rows = view.model.windows.entry(window).or_default();
    rows.insert(index.min(rows.len()), raw);
    view.model.tabs.get_mut(&raw).unwrap().window = window;
    view.model.extra.entry(window).or_default();
}

#[cfg(test)]
mod tests;

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let hooks: Table = lua.globals().get("AAUI")?;
    let s = state.clone();
    hooks.raw_set(
        "ChatAllocateWindow",
        lua.create_function(move |lua, ()| {
            ensure(lua, &s)?;
            let (before, locks, id) = {
                let mut data = s.borrow_mut();
                let view = &mut data.chat_config;
                let before = view.model.clone();
                let locks = view.locks;
                (before, locks, allocate(view))
            };
            let Some(id) = id else { return Ok(None::<i32>) };
            if persist(lua, &s) {
                return Ok(Some(id));
            }
            let mut data = s.borrow_mut();
            data.chat_config.model = before;
            data.chat_config.locks = locks;
            Ok(None)
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatReleaseWindow",
        lua.create_function(move |lua, id: i32| {
            ensure(lua, &s)?;
            release(&mut s.borrow_mut().chat_config, id);
            persist(lua, &s);
            Ok(())
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatTabDetached",
        lua.create_function(move |lua, (window, raw): (i32, i32)| {
            ensure(lua, &s)?;
            detached(&mut s.borrow_mut().chat_config, window, raw);
            persist(lua, &s);
            Ok(())
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatTabAttached",
        lua.create_function(move |lua, (window, raw, index): (i32, i32, usize)| {
            ensure(lua, &s)?;
            attached(&mut s.borrow_mut().chat_config, window, raw, index);
            persist(lua, &s);
            Ok(())
        })?,
    )?;
    Ok(())
}
