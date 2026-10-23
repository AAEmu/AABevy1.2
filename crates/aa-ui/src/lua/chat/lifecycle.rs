//! Application-driven local chat configuration; no additional native Lua API.
use super::*;

/// A configured tab. Optional style fields retain the legacy `AAUI.ChatTabs` defaults.
/// `filters=None` accepts all messages; `Some([])` accepts none. Native configuration
/// can contain filter IDs beyond the 2..59 subset exposed by its Lua getter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatTabInfo {
    pub id: i64,
    pub name: String,
    pub filters: Option<Vec<i64>>,
    pub font_size: Option<f32>,
    pub background: Option<[f32; 4]>,
    pub filter_colors: Vec<(i64, [f32; 4])>,
}

/// A window's persisted rows, selected numeric index and input method.
/// The application owns global ID allocation, name policy and persistence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatWindowInfo {
    pub window_id: i64,
    pub tabs: Vec<ChatTabInfo>,
    pub selected: i32,
    pub method: i64,
}

/// Native manager history is keyed by global raw ID and outlives a particular window.
#[derive(Default)]
struct History(std::collections::BTreeMap<i64, MessageData>);

/// Immediately retire every live chat window and clear shared tab history.
/// The application resets its configuration before invoking the original zero-argument
/// `OnResetAllChatWindow` callback. Native 39507410 destroys windows before 395072c0
/// clears history; ordinary tab removal deliberately retains that history instead.
pub fn reset_windows(lua: &Lua) -> usize {
    let windows = ui(lua).tree.of_class("chatwindow");
    for &window in &windows {
        super::super::widget::remove_subtree(lua, window);
    }
    if let Some(mut history) = lua.app_data_mut::<History>() {
        history.0.clear();
    }
    windows.len()
}

pub(super) fn retain_history(lua: &Lua, tab: &ChatTab) {
    let data = ui(lua)
        .tree
        .widget(tab.message)
        .and_then(|w| w.message.clone());
    if lua.app_data_ref::<History>().is_none() {
        lua.set_app_data(History::default());
    }
    if let Some(data) = data {
        lua.app_data_mut::<History>()
            .unwrap()
            .0
            .insert(tab.id, data);
    }
}

fn live_chat(lua: &Lua, widget: Id) -> Option<ChatData> {
    let u = ui(lua);
    u.tree
        .objects
        .get(widget as usize)
        .filter(|o| o.alive)?
        .widget()?
        .chat
        .clone()
}

fn find_tab(lua: &Lua, raw_id: i64) -> Option<(Id, ChatTab)> {
    let u = ui(lua);
    u.tree.of_class("chatwindow").into_iter().find_map(|id| {
        let c = u.tree.widget(id)?.chat.as_ref()?;
        Some((id, c.tabs.iter().find(|t| t.id == raw_id)?.clone()))
    })
}

fn apply_info(lua: &Lua, tab: &ChatTab, info: &ChatTabInfo) {
    super::super::auto_clip::geometry(&mut ui(lua), tab.button);
    if let Some(text) = ui(lua).tree.get_mut(tab.button).text_mut() {
        text.text = info.name.clone();
        text.lines.clear();
    }
    super::super::widget::auto_size(lua, tab.button);
    super::super::auto_clip::refresh(&mut ui(lua), tab.button);
    apply_message_info(lua, tab.message, info);
}

fn apply_message_info(lua: &Lua, message: Id, info: &ChatTabInfo) {
    let mut u = ui(lua);
    if let Some(size) = info.font_size
        && let Some(text) = u.tree.get_mut(message).text_mut()
    {
        text.style.size = size;
    }
    if let Some(data) = u.tree.widget_mut(message).and_then(|w| w.message.as_mut()) {
        data.filter_colors = info.filter_colors.clone();
        if info.background.is_some() {
            data.background = info.background;
        }
    }
    if let Some(color) = info.background {
        let drawables = u
            .tree
            .widget(message)
            .map(|w| w.drawables.clone())
            .unwrap_or_default();
        for id in drawables {
            if let Some(drawable) = u.tree.drawable_mut(id)
                && drawable.layer == DrawLayer::Background
            {
                // Native39512607 retains metadata alpha but applies an opaque RGB tint.
                // Global leave/over opacity is the independent owner-layer factor.
                drawable.color = [color[0], color[1], color[2], 1.];
            }
        }
    }
    u.tree.touch();
}

pub(super) fn tint_dividers(lua: &Lua, window: Id, message: Id) {
    let mut u = ui(lua);
    let color = u
        .tree
        .widget(message)
        .and_then(|w| w.message.as_ref())
        .and_then(|m| m.background);
    let Some(color) = color else { return };
    let Some(c) = u.tree.widget(window).and_then(|w| w.chat.clone()) else {
        return;
    };
    for id in [c.left_line, c.right_line] {
        if let Some(drawable) = u.tree.drawable_mut(id) {
            drawable.color = [1.0, 1.0, 1.0, color[3]];
        }
    }
}

fn insert(lua: &Lua, widget: Id, info: &ChatTabInfo, select_new: bool) -> mlua::Result<bool> {
    let Some(c) = live_chat(lua, widget) else {
        return Ok(false);
    };
    let index = c.tabs.len();
    // Native 39513680 (append) and 39514120 (hydrate): message callback precedes button.
    let (message, table) = create_tab_message(lua, widget, index)?;
    call_global(lua, "OnCreateChatMessage", table);
    apply_chat_options(lua, message);
    if let Some(mut data) = lua
        .app_data_ref::<History>()
        .and_then(|h| h.0.get(&info.id).cloned())
    {
        // Native history is shared, but scroll/time-visible/max-lines belong to the frame.
        if let Some(frame) = ui(lua).tree.widget(message).and_then(|w| w.message.clone()) {
            data.scroll = frame.scroll;
            data.time_visible = frame.time_visible;
            data.max_lines = frame.max_lines;
            data.controls = frame.controls;
        }
        ui(lua).tree.widget_mut(message).unwrap().message = Some(data);
    }
    if select_new {
        apply_message_info(lua, message, info);
    }
    let button = create_child(
        lua,
        Some(widget),
        "button",
        &format!("chatTab{index}"),
        0,
        false,
    )?;
    let bid = value_id(lua, &Value::Table(button.clone())).unwrap();
    if let Some(text) = ui(lua).tree.get_mut(bid).text_mut() {
        text.text = info.name.clone();
    }
    call_global(lua, "OnCreateChatTabButton", button.clone());
    let tab = ChatTab {
        id: info.id,
        button: bid,
        message,
        filters: info.filters.clone(),
    };
    apply_info(lua, &tab, info);
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(widget)
        .and_then(|w| w.chat.as_mut())
    {
        c.tabs.push(tab);
        c.hovered = None;
    }
    layout_content(lua, widget);
    layout(lua, widget);
    if select_new {
        select_inner(lua, widget, index, true);
    }
    // Append selects before binding the raw ID to the original Lua button handlers.
    call_global(lua, "OnSetChatTabId", (button, info.id));
    Ok(true)
}

/// Replace rows when assigning a different window ID, then restore stored selection/method.
/// Native 39514120 returns immediately for the same ID, including after local row mutations.
/// Invalid stored selection leaves the reset no-selection state and fires no callback.
pub fn hydrate_window(lua: &Lua, widget: Id, info: &ChatWindowInfo) -> mlua::Result<bool> {
    let Some(c) = live_chat(lua, widget) else {
        return Ok(false);
    };
    if c.window_id == info.window_id {
        return Ok(false);
    }
    for tab in &c.tabs {
        retain_history(lua, tab);
        super::super::widget::remove_subtree(lua, tab.button);
        super::super::widget::remove_subtree(lua, tab.message);
    }
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(widget)
        .and_then(|w| w.chat.as_mut())
    {
        c.window_id = info.window_id;
        c.tabs.clear();
        c.selected = usize::MAX;
        c.hydrated = true;
        c.hovered = None;
    }
    for tab in &info.tabs {
        insert(lua, widget, tab, false)?;
    }
    // 395141b4 returns before selection/method restoration for an empty record set.
    if info.tabs.is_empty() {
        return Ok(true);
    }
    if let Ok(index) = usize::try_from(info.selected) {
        select_inner(lua, widget, index, true);
    }
    super::super::chat_input::restore_method(lua, widget, info.method);
    Ok(true)
}

/// Append one row, retaining all other tab objects/history. Select before `OnSetChatTabId`.
pub fn append_tab(lua: &Lua, widget: Id, info: &ChatTabInfo) -> mlua::Result<bool> {
    if live_chat(lua, widget).is_none() {
        return Ok(false);
    }
    // A new configuration allocation starts with empty native history, including ID reuse.
    if let Some(mut history) = lua.app_data_mut::<History>() {
        history.0.remove(&info.id);
    }
    insert(lua, widget, info, true)
}

/// Refresh metadata on an existing raw ID without creation/selection callbacks.
/// Retained messages use the new palette when composed, including their inline links/colours.
pub fn update_tab(lua: &Lua, raw_id: i64, info: &ChatTabInfo) -> mlua::Result<bool> {
    if raw_id != info.id {
        return Ok(false);
    }
    let Some((window, tab)) = find_tab(lua, raw_id) else {
        return Ok(false);
    };
    apply_info(lua, &tab, info);
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(window)
        .and_then(|w| w.chat.as_mut())
    {
        if let Some(tab) = c.tabs.iter_mut().find(|t| t.id == raw_id) {
            tab.filters = info.filters.clone();
        }
        c.hovered = None;
    }
    tint_dividers(lua, window, tab.message);
    layout(lua, window);
    Ok(true)
}

/// Remove only the live widget row; application configuration deletion is separate.
/// Raw ID0 is accepted here: native Delete removes its widget before rejecting model ID0.
/// Preserve numeric selection, clamp to the last row, and emit no selection callback.
/// An emptied configured window is destroyed at the next toolkit update, as at 395130e0.
pub fn remove_tab(lua: &Lua, raw_id: i64) -> mlua::Result<bool> {
    let Some((window, tab)) = find_tab(lua, raw_id) else {
        return Ok(false);
    };
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(window)
        .and_then(|w| w.chat.as_mut())
    {
        c.tabs.retain(|t| t.id != raw_id);
        if c.selected != usize::MAX && c.selected >= c.tabs.len() {
            c.selected = c.tabs.len().checked_sub(1).unwrap_or(usize::MAX);
        }
        c.hovered = None;
    }
    // Widget removal precedes model deletion, which can fail (notably for raw ID0).
    // Keep history available to later hydration; a new append allocation clears it.
    retain_history(lua, &tab);
    super::super::widget::remove_subtree(lua, tab.button);
    super::super::widget::remove_subtree(lua, tab.message);
    layout(lua, window);
    // Empty layout intentionally returns early; visibility still clears the native dividers.
    if let Some(c) = chat(lua, window) {
        layout_lines(&mut ui(lua), window, &c);
    }
    Ok(true)
}

fn rgba(table: &Table) -> mlua::Result<[f32; 4]> {
    Ok([
        table.raw_get("r")?,
        table.raw_get("g")?,
        table.raw_get("b")?,
        table.raw_get("a")?,
    ])
}

pub(super) fn info_from_hook(lua: &Lua, window_id: i64) -> mlua::Result<ChatWindowInfo> {
    let mut info = ChatWindowInfo {
        window_id,
        ..Default::default()
    };
    let hook = lua
        .globals()
        .raw_get::<Option<Table>>("AAUI")?
        .and_then(|t| t.raw_get::<Option<Function>>("ChatTabs").ok().flatten());
    if let Some(hook) = hook
        && let Value::Table(rows) = hook.call::<Value>(window_id)?
    {
        info.selected = rows.raw_get::<Option<i32>>("selected")?.unwrap_or(0);
        info.method = rows.raw_get::<Option<i64>>("method")?.unwrap_or(0);
        for row in rows.sequence_values::<Table>() {
            let row = row?;
            let filters = row
                .raw_get::<Option<Table>>("filters")?
                .map(|table| -> mlua::Result<Vec<i64>> {
                    let values = table
                        .clone()
                        .pairs::<i64, Value>()
                        .collect::<mlua::Result<Vec<_>>>()?;
                    if values.iter().any(|(_, v)| matches!(v, Value::Boolean(_))) {
                        Ok(values
                            .into_iter()
                            .filter_map(|(key, value)| {
                                matches!(value, Value::Boolean(true)).then_some(key)
                            })
                            .collect())
                    } else {
                        Ok(table
                            .sequence_values::<i64>()
                            .filter_map(Result::ok)
                            .collect())
                    }
                })
                .transpose()?;
            let mut filter_colors = Vec::new();
            if let Some(colors) = row.raw_get::<Option<Table>>("filterColors")? {
                for pair in colors.pairs::<i64, Table>() {
                    let (filter, color) = pair?;
                    filter_colors.push((filter, rgba(&color)?));
                }
                filter_colors.sort_by_key(|(filter, _)| *filter);
            }
            info.tabs.push(ChatTabInfo {
                id: row
                    .raw_get::<Option<i64>>("id")?
                    .unwrap_or(info.tabs.len() as i64),
                name: row.raw_get::<Option<String>>("name")?.unwrap_or_default(),
                filters,
                font_size: row.raw_get("fontSize")?,
                background: row
                    .raw_get::<Option<Table>>("bgColor")?
                    .map(|table| rgba(&table))
                    .transpose()?,
                filter_colors,
            });
        }
        return Ok(info);
    }
    info.tabs.push(ChatTabInfo {
        id: 0,
        name: "All".into(),
        ..Default::default()
    });
    Ok(info)
}
