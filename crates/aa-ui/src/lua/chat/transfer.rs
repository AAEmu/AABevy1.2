//! Native chat duplicate/detach/insert and whole-window tab-strip drop.
use super::*;

fn live(lua: &Lua, id: Id) -> Option<ChatData> {
    let u = ui(lua);
    u.tree
        .objects
        .get(id as usize)
        .filter(|o| o.alive)?
        .widget()?
        .chat
        .clone()
}

fn hook(lua: &Lua, name: &str) -> Option<Function> {
    lua.globals()
        .raw_get::<Option<Table>>("AAUI")
        .ok()
        .flatten()?
        .raw_get::<Option<Function>>(name)
        .ok()
        .flatten()
}

fn notify(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) {
    if let Some(f) = hook(lua, name) {
        if let Err(error) = f.call::<()>(args) {
            bevy::log::warn!("aa-ui: AAUI.{name}: {error}");
        }
        super::super::flush(lua);
    }
}

///3927a9f0 /392cb742: clone plain configuration before original OnCreateChatWindow.
/// Widget identities, handlers, rows, input state and native child references stay fresh.
pub(super) fn copy_configuration(lua: &Lua, source: Id, target: Id) {
    let mut u = ui(lua);
    let r = u.tree.rect(source);
    let object = u.tree.get(source).clone();
    let Some(w) = object.widget() else { return };
    let Some(c) = w.chat.clone() else { return };
    {
        let o = u.tree.get_mut(target);
        o.width = r.w;
        o.height = r.h;
        o.alpha = object.alpha;
        o.unanchored_origin = Some([r.x, r.y]);
    }
    let Some(wt) = u.tree.widget_mut(target) else {
        return;
    };
    wt.enabled = w.enabled;
    wt.layer = w.layer;
    wt.resizing = w.resizing.configuration();
    let Some(t) = wt.chat.as_mut() else { return };
    t.tab_width = c.tab_width;
    t.min_tab_width = c.min_tab_width;
    t.auto_resize_tabs = c.auto_resize_tabs;
    t.tab_offset = c.tab_offset;
    t.tab_gap = c.tab_gap;
    t.tab_button_alpha = c.tab_button_alpha;
    t.tab_area_height = c.tab_area_height;
    t.tab_inset = c.tab_inset;
    t.content_offset = c.content_offset;
    t.use_add_button = c.use_add_button;
    t.allow_tab_switch = c.allow_tab_switch;
    t.line_offset = c.line_offset;
    t.notify_blink_ms = c.notify_blink_ms;
    t.notify_max_ms = c.notify_max_ms;
    t.interaction.caret_offset = c.interaction.caret_offset;
    t.interaction.use_slides = c.interaction.use_slides;
    t.interaction.slide_ms = c.interaction.slide_ms;
    t.interaction.offset = c.interaction.offset;
}

fn reparent(lua: &Lua, id: Id, parent: Option<Id>) {
    let mut u = ui(lua);
    if let Some(old) = u.tree.get(id).parent {
        if let Some(w) = u.tree.widget_mut(old) {
            w.children.retain(|&c| c != id);
        }
    } else {
        u.tree.top.retain(|&c| c != id);
    }
    u.tree.get_mut(id).parent = parent;
    if let Some(parent) = parent {
        u.tree.widget_mut(parent).unwrap().children.push(id);
    } else {
        u.tree.top.push(id);
    }
}

fn detach(lua: &Lua, owner: Id, button: Id) -> Option<(ChatTab, usize, i64)> {
    let c = live(lua, owner)?;
    let index = c.tabs.iter().position(|t| t.button == button)?;
    let tab = c.tabs[index].clone();
    // Detached native objects outlive their old owner, including owner retirement in Lua.
    reparent(lua, tab.button, None);
    reparent(lua, tab.message, None);
    if let Some(c) = ui(lua).tree.widget_mut(owner).and_then(|w| w.chat.as_mut()) {
        c.tabs.remove(index);
        c.hovered = None;
    }
    layout(lua, owner);
    select_inner(lua, owner, index.saturating_sub(1), true);
    //39512830 ->39504f90 follows source selection, even if this was not selected.
    notify(lua, "ChatTabDetached", (c.window_id, tab.id));
    Some((tab, index, c.window_id))
}

fn attach(lua: &Lua, owner: Id, tab: &ChatTab, index: usize) -> bool {
    let Some(c) = live(lua, owner) else {
        return false;
    };
    if index > c.tabs.len() {
        return false;
    }
    if ![tab.button, tab.message]
        .iter()
        .all(|&id| ui(lua).tree.get(id).alive)
    {
        return false;
    }
    reparent(lua, tab.button, Some(owner));
    reparent(lua, tab.message, Some(owner));
    if let Some(c) = ui(lua).tree.widget_mut(owner).and_then(|w| w.chat.as_mut()) {
        c.tabs.insert(index, tab.clone());
        c.hovered = None;
    }
    layout_content(lua, owner);
    layout(lua, owner);
    //392cc2a0: selected/input binding and callbacks precede model insertion.
    select_inner(lua, owner, index, true);
    if live(lua, owner).is_none_or(|c| !c.tabs.iter().any(|t| t.button == tab.button)) {
        return false;
    }
    notify(lua, "ChatTabAttached", (c.window_id, tab.id, index as i32));
    true
}

fn rollback(lua: &Lua, source: Id, tab: &ChatTab, old_index: usize) {
    if let Some(c) = live(lua, source) {
        let index = old_index.min(c.tabs.len());
        if attach(lua, source, tab, index) {
            return;
        }
    }
    // Both owners retired: preserve manager scrollback; application retains detached metadata.
    lifecycle::retain_history(lua, tab);
    super::super::widget::remove_subtree(lua, tab.button);
    super::super::widget::remove_subtree(lua, tab.message);
}

fn release_empty(lua: &Lua, window: Id, window_id: i64) {
    if live(lua, window).is_none_or(|c| c.tabs.is_empty()) {
        super::super::widget::remove_subtree(lua, window);
        if window_id >= 0 {
            notify(lua, "ChatReleaseWindow", window_id);
        }
    }
}

pub(super) fn split(lua: &Lua, source: Id, button: Id) {
    if let Err(error) = split_inner(lua, source, button) {
        bevy::log::warn!("aa-ui: native chat split: {error}");
    }
}

fn split_inner(lua: &Lua, source: Id, button: Id) -> mlua::Result<()> {
    let Some(c) = live(lua, source) else {
        return Ok(());
    };
    if !c.tabs.iter().any(|t| t.button == button) {
        return Ok(());
    }
    let (parent, position) = {
        let u = ui(lua);
        let r = u.tree.rect(button);
        (u.tree.get(source).parent, (r.x, r.y))
    };
    // Native factory("chatwindow","dup",parent,source), not SetChatWindowId hydration.
    let target = ui(lua).tree.create_widget("chatwindow", "dup", parent);
    let table = match new_object_table(lua, target) {
        Ok(table) => table,
        Err(error) => {
            release_empty(lua, target, -1);
            return Err(error);
        }
    };
    if let Err(error) = init_window_from(lua, target, &table, Some(source)) {
        release_empty(lua, target, -1);
        return Err(error);
    }
    if live(lua, target).is_none() || live(lua, source).is_none() {
        release_empty(lua, target, -1);
        return Ok(());
    }
    set_allow_tab_switch(lua, target, true);
    let allocated = if let Some(f) = hook(lua, "ChatAllocateWindow") {
        match f.call::<Value>(()) {
            Ok(Value::Integer(n)) => Some(n),
            Ok(Value::Number(n)) if n.is_finite() && n.fract() == 0. => Some(n as i64),
            Ok(_) => None,
            Err(error) => {
                bevy::log::warn!("aa-ui: AAUI.ChatAllocateWindow: {error}");
                None
            }
        }
    } else {
        let u = ui(lua);
        (0..8).find(|&n| {
            !u.tree.of_class("chatwindow").iter().any(|&id| {
                u.tree
                    .widget(id)
                    .and_then(|w| w.chat.as_ref())
                    .is_some_and(|c| c.window_id == n)
            })
        })
    };
    let Some(window_id) = allocated.filter(|n| (0..8).contains(n)) else {
        release_empty(lua, target, -1);
        return Ok(());
    };
    if live(lua, target).is_none() {
        release_empty(lua, target, window_id);
        return Ok(());
    }
    if let Some(t) = ui(lua)
        .tree
        .widget_mut(target)
        .and_then(|w| w.chat.as_mut())
    {
        t.window_id = window_id;
        // Keep an empty duplicate alive through reentrant callback ticks until it has a row.
    }
    call_global(lua, "OnDuplicatedChatWindow", (table, window_id));
    let owner = {
        let u = ui(lua);
        tab_owner(&u, button).map(|p| p.0)
    };
    if live(lua, target).is_none() || owner != Some(source) {
        release_empty(lua, target, window_id);
        return Ok(());
    }
    let Some((row, index, _)) = detach(lua, source, button) else {
        release_empty(lua, target, window_id);
        return Ok(());
    };
    if !attach(lua, target, &row, 0) {
        rollback(lua, source, &row, index);
        release_empty(lua, target, window_id);
        return Ok(());
    }
    if live(lua, target).is_none() {
        return Ok(());
    }
    if let Some(t) = ui(lua)
        .tree
        .widget_mut(target)
        .and_then(|w| w.chat.as_mut())
    {
        t.hydrated = true;
    }
    // Native places the new window at the old tab top-left before movement capture.
    {
        let mut u = ui(lua);
        let extent = u.tree.rect(target);
        super::super::resize::place(
            &mut u,
            target,
            Rect {
                x: position.0,
                y: position.1,
                ..extent
            },
        );
    }
    super::super::anchors::changed(lua, target);
    super::super::widget::show(lua, target, true, None);
    super::super::drag::start_moving(lua, target);
    interaction::begin(lua, button, target);
    Ok(())
}

///392ccb30 ->392cc780: paired-vector reorder uses the stored caret side only
/// for its adjacent no-op guards. The insertion index is deliberately unadjusted.
pub(super) fn receive(lua: &Lua, button: Id, hovered: Id) {
    let owners = {
        let u = ui(lua);
        (tab_owner(&u, button), tab_owner(&u, hovered))
    };
    let (Some((source, index)), Some((target, at))) = owners else { return };
    let Some(c) = live(lua, target).filter(|c| c.allow_tab_switch) else { return };
    if source == target {
        let before = c.interaction.before;
        if index == at || (before && at.checked_sub(1) == Some(index))
            || (!before && at.checked_add(1) == Some(index)) { return; }
        let row = c.tabs[index].clone();
        if let Some(c) = ui(lua).tree.widget_mut(target).and_then(|w| w.chat.as_mut()) {
            c.tabs.remove(index);
            c.tabs.insert(at, row.clone());
        }
        layout(lua, target);
        // App-only persistence of the native vector mutation; preserve selection/input.
        notify(lua, "ChatTabAttached", (c.window_id, row.id, at as i32));
        return;
    }
    let Some(source_data) = live(lua, source) else { return };
    if source_data.tabs[index].id == 0 { return; }
    let pointer = ui(lua).cursor;
    let Some(at) = insertion(lua, target, pointer) else { return };
    let Some((row, old_index, _)) = detach(lua, source, button) else { return };
    if !attach(lua, target, &row, at) { rollback(lua, source, &row, old_index); }
}

fn insertion(lua: &Lua, window: Id, pointer: (f32, f32)) -> Option<usize> {
    let c = live(lua, window)?;
    let u = ui(lua);
    let (nearest, left) = interaction::nearest(&u, &c, pointer)?;
    let index = nearest + usize::from(!left);
    //39513080 permits append/outside rows but rejects insertion before raw tab zero.
    c.tabs.get(index).is_none_or(|t| t.id != 0).then_some(index)
}

pub(super) fn drop_window(lua: &Lua, button: Id) {
    let owner = {
        let u = ui(lua);
        tab_owner(&u, button)
    };
    let Some((source, 0)) = owner else { return };
    let Some(c) = live(lua, source).filter(|c| c.allow_tab_switch) else {
        return;
    };
    let pointer = ui(lua).cursor;
    let windows = ui(lua).tree.of_class("chatwindow");
    //392cca60 scans the native creation list, independent of generic hit/hover identity.
    let target = windows.into_iter().find(|&w| {
        let Some(c) = live(lua, w).filter(|c| c.allow_tab_switch) else {
            return false;
        };
        let u = ui(lua);
        let r = u.animations.widget_rect(&u.tree, w);
        pointer.0 >= r.x
            && pointer.0 <= r.right()
            && pointer.1 >= r.y + c.tab_clip_vertical[0]
            && pointer.1 <= r.y + c.tab_clip_vertical[1]
    });
    let Some(target) = target.filter(|&t| t != source) else {
        return;
    };
    if c.tabs.iter().any(|t| t.id == 0) {
        return;
    }
    let Some(index) = insertion(lua, target, pointer) else {
        return;
    };
    let mut detached = Vec::new();
    //392cbef0 detaches every row before inserting any; it does not select the source.
    for (old_index, tab) in c.tabs.iter().enumerate() {
        if ![tab.button, tab.message]
            .iter()
            .all(|&id| ui(lua).tree.get(id).alive)
        {
            continue;
        }
        reparent(lua, tab.button, None);
        reparent(lua, tab.message, None);
        notify(lua, "ChatTabDetached", (c.window_id, tab.id));
        detached.push((tab.clone(), old_index));
    }
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(source)
        .and_then(|w| w.chat.as_mut())
    {
        c.tabs
            .retain(|t| !detached.iter().any(|(row, _)| row.button == t.button));
        c.selected = usize::MAX;
        c.hovered = None;
    }
    if live(lua, source).is_some() {
        layout(lua, source);
        if let Some(c) = live(lua, source) {
            layout_lines(&mut ui(lua), source, &c);
        }
    }
    for (offset, (row, old_index)) in detached.into_iter().enumerate() {
        if !attach(lua, target, &row, index + offset) {
            rollback(lua, source, &row, old_index);
        }
    }
}
