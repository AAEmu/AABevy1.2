//! `slot` widgets (action bar, skill and bag slots): an icon bound to a game slot.
//!
//! The client binds a slot to game data natively (`EstablishSlot(slotType, slotIdx)`,
//! `EstablishSkill(skillType)`, `EstablishItem(itemType, grade)`), draws the bound icon and tells
//! the script through `OnContentUpdated(self, what, value)` (`"action_binded"`, `"can_use"`,
//! `"learned"`, `"stack_count_changed"`, `"skill_cooldown_start"` / `"skill_cooldown_done"`). Here
//! the game data comes from Lua hooks the app (or the example fakes) defines:
//!
//! ```lua
//! AAUI.SlotContent(slotType, slotIdx)  -- EstablishSlot
//! AAUI.SkillContent(skillType)         -- EstablishSkill (optional)
//! AAUI.ItemContent(itemType, grade)    -- EstablishItem (optional)
//! -- each returns nil (empty) or
//! -- { type = "skill" | "item" | "function" | ..., id = n, icon = "ui/icon/x.dds", stack = n,
//! --   grade = n (items: the grade frame ui/icon/item_grade_<n><name>.dds over the icon),
//! --   canUse = false (default true: dark and a red hotkey when false),
//! --   learned = false (default true), canLearn = bool, reqPoints = n, skillLevel = n  -- skill book
//! --   state of the skill window's slots (sent as OnContentUpdated "learned" / "canLearn" / ...),
//! --   cooldown = { remain = ms, total = ms },  -- optional, see below
//! --   tooltip = {...}, extraInfo = {...} }
//! AAUI.SlotHotKey(slotType, slotIdx)   -- "1", "SHIFT-1", "MINUS", ... or nil (unbound)
//! AAUI.UseSlot(slotType, slotIdx, button)  -- a click on a bound slot (`slot::clicked`)
//! AAUI.UseSkill(skillType, button), AAUI.UseItem(itemType, grade, button)
//! AAUI.PickupSlot(info), AAUI.DropSlot(info)  -- drag and drop of bound slots, see [`super::drag`]
//! ```
//!
//! A left click on a bound slot uses it; while the pointer holds something it drops it there instead
//! (`AAUI.DropSlot`). A drag from a bound, enabled slot picks its content
//! up (`AAUI.PickupSlot`), a drop on one that was bound with `EstablishSlot` goes to `AAUI.DropSlot`.
//!
//! # Cooldowns
//!
//! `cooldown = { remain = 8000, total = 12000 }` (both in milliseconds, `remain` <= `total`) says
//! the skill or item is cooling down right now. The native then draws the client's sweep over the
//! icon (a dark clockwise wipe from 12 o'clock, the dark part being the time left, in the colour
//! of `ui/icon/cooldown_mask.dds`: 46/255 grey at 78 % opacity) and the time left as text in
//! `slot.cooltime_style` (native single-precision seconds/minutes/hours, including a unit suffix;
//! totals below [`LABEL_MIN_TOTAL_MS`] skip slot drawing), counts it
//! down per frame in `update` on its own, and when it runs out removes the sweep and raises
//! `OnContentUpdated(self, "skill_cooldown_done")`. Binding a cooling slot raises
//! `OnContentUpdated(self, "skill_cooldown_start", total)`. The hook is asked again by
//! [`refresh_slots`], so the app can give the remaining time again (it restarts the count from
//! `remain`); a result without `cooldown` clears a running sweep.
//!
//! The cooldown buttons (`cooldownbutton`, `cooldownactionbutton`, `cooldownconstantbutton`,
//! `cooldowninventorybutton`: inventory, equipment and buff icons) get the same sweep from
//! `button:SetCoolDown(remainMs, totalMs)` (the scripts call it from `OnContentUpdated(self,
//! "item_cooldown" | "skill_cooldown", remain, total)`; `SetCoolDown(0, 0)` clears it);
//! `SetCoolDownMask` / `SetCoolDownMaskCoords` (always the stock mask) are accepted and ignored.
//! `cooldownconstantbutton:SetSkill(rawId)` observes `AAUI.SkillCooldown(rawId)`, which returns
//! `(remaining_ms, total_ms)` for an existing descriptor, or nil for an unknown descriptor.
//! This is independent of `SkillContent` and never replaces the item's icon or click action.
//! These four cooldown button classes draw a sweep without a native time caption. Slot captions
//! obey option 82 through [`set_cooldown_numbers`]. Native explicit global/own slot selection
//! (+311/+312) is not supplied by the current content hook; duration does not identify a global
//! cooldown. The app must not infer that state from `SkillCooldown`'s combined winning timer.
//!
//! [`refresh_slots`] asks again for every bound slot (the app calls it when bars change). The slot
//! creates its icon drawable itself (`slot.icon`, also `slot.gradeIcon`, inset 1 like the
//! client's 40x40 icon in a 42x42 slot) and a `cooltime_style` text style.

use mlua::{Function, Lua, Table, Value};

use super::{Ui, method, new_object_table, none, ret, ui};
use crate::render::{Quad, Tex};
use crate::tree::{
    Align, Anchor, DrawKind, DrawLayer, Id, Point, Rect, TextBlock, TextStyle, Widget,
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotData {
    /// How the slot was bound, to ask again on refresh.
    pub source: Option<Source>,
    /// `GetBindedType()`: "none", "skill", "item", ...
    pub binded: Option<String>,
    pub icon: Option<Id>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Slot(i64, i64),
    Skill(i64),
    Item(i64, i64),
}

/// A cooldown ([`crate::tree::Widget::cooldown`]): slots use `end_ms - Ui::time_ms`, while
/// generic buttons retain `sampled_ms` until their next explicit SetCoolDown call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cooldown {
    /// UI time the slot cooldown ends at; for generic buttons, time of the sample plus remaining.
    pub end_ms: f64,
    pub total_ms: f32,
    /// Generic buttons retain a draw snapshot until SetCoolDown is called again. Only slots
    /// count down locally; the derived button providers notify their original Lua consumers.
    pub sampled_ms: Option<f32>,
    /// What the last frame update saw (sweep step, label): the tree is touched when it changes.
    pub drawn: (i32, String),
}

/// Native constant-button skill identity and its one-shot ready notification (+5cc/+5d0).
#[derive(Clone, Debug, PartialEq)]
pub struct ConstantSkillCooldown {
    pub skill_type: u32,
    pub ready_notified: bool,
}

/// Widgets with a running cooldown, id -> true (so the per-frame update does not scan the whole
/// tree). Kept in the Lua registry: the app data container is locked while `ui(lua)` is held.
fn active(lua: &Lua) -> mlua::Result<Table> {
    const KEY: &str = "aa_ui_active_cooldowns";
    match lua.named_registry_value::<Option<Table>>(KEY)? {
        Some(t) => Ok(t),
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value(KEY, &t)?;
            Ok(t)
        }
    }
}

/// The dark colour of the sweep: the pixels of `ui/icon/cooldown_mask.dds` (46, 46, 46, 199).
const SWEEP_COLOR: [f32; 4] = [46.0 / 255.0, 46.0 / 255.0, 46.0 / 255.0, 199.0 / 255.0];

/// The sweep edge moves in steps of 2 degrees (180 per turn): about one pixel at the corners of a
/// slot, which keeps a long cooldown at ~15 mesh rebuilds a second.
const SWEEP_STEPS: f32 = 180.0;

/// Native CVar visibility and category 98 time templates, independent of cooldown identity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CaptionOptions {
    visible: bool,
    texts: [String; 3],
}

impl Default for CaptionOptions {
    fn default() -> Self {
        Self {
            // 392c50f0 permits captions when the CVar has not been installed.
            visible: true,
            texts: ["$1s".into(), "$1m".into(), "$1h".into()],
        }
    }
}

/// Supply the actual `OPTION_ITEM_SLOT_COOLDOWN_VISIBLE` (82) / `slot_cooldown_visible` value.
/// Changes existing captions immediately, without rebinding contents or restarting timers.
pub fn set_cooldown_numbers(lua: &Lua, visible: bool) {
    let mut u = ui(lua);
    if u.cooldown_captions.visible != visible {
        u.cooldown_captions.visible = visible;
        u.tree.touch();
    }
}

pub(crate) fn set_caption_texts(lua: &Lua, texts: &crate::uitext::UiTexts) {
    let mut u = ui(lua);
    for (i, key) in ["second_initial", "minute_initial", "hour_initial"].iter().enumerate() {
        if let Some(text) = texts.get(&(98, (*key).into())) {
            u.cooldown_captions.texts[i] = text.clone();
        }
    }
    u.tree.touch();
}

/// Bound contents (Lua tables) by slot id.
fn contents(lua: &Lua) -> mlua::Result<Table> {
    const KEY: &str = "aa_ui_slot_contents";
    match lua.named_registry_value::<Option<Table>>(KEY)? {
        Some(t) => Ok(t),
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value(KEY, &t)?;
            Ok(t)
        }
    }
}

fn hook(lua: &Lua, name: &str) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get(name).ok()
}

/// A click on a slot uses what it holds, as the client does natively (the action bar scripts'
/// own `OnClick` only hides the tooltip): `AAUI.UseSlot(slotType, slotIdx, button)` for
/// `EstablishSlot`, `AAUI.UseSkill(skillType, button)` for `EstablishSkill`,
/// `AAUI.UseItem(itemType, grade, button)` for `EstablishItem` (`button` = `"LeftButton"` /
/// `"RightButton"`). The app runs the same path as the slot's hotkey. Disabled or empty slots
/// do nothing. Returns whether a hook ran.
pub(crate) fn clicked(lua: &Lua, target: Id, button: &str) -> bool {
    let (slot, found) = {
        let u = ui(lua);
        let mut t = Some(target);
        let mut found = None;
        let mut slot = None;
        while let Some(id) = t {
            if let Some(w) = u.tree.widget(id) {
                if let Some(s) = w.slot.as_ref() {
                    slot = Some(id);
                    if w.enabled && s.binded.as_deref().is_some_and(|b| b != "none") {
                        found = s.source.clone();
                    }
                    break;
                }
                if w.button.is_some() {
                    break;
                }
            }
            t = u.tree.get(id).parent;
        }
        (slot, found)
    };
    // A left click while the pointer holds something drops it here instead of using the slot.
    if button == "LeftButton"
        && let Some(slot) = slot
        && super::drag::holding(lua)
        && receive(lua, slot)
    {
        return true;
    }
    let Some(source) = found else { return false };
    let (name, args): (&str, Vec<Value>) = match source {
        Source::Slot(t, i) => ("UseSlot", vec![Value::Integer(t), Value::Integer(i)]),
        Source::Skill(s) => ("UseSkill", vec![Value::Integer(s)]),
        Source::Item(it, g) => ("UseItem", vec![Value::Integer(it), Value::Integer(g)]),
    };
    let Some(f) = hook(lua, name) else {
        return false;
    };
    let mut args = args;
    match lua.create_string(button) {
        Ok(s) => args.push(Value::String(s)),
        Err(_) => return false,
    }
    if let Err(e) = f.call::<()>(mlua::MultiValue::from_iter(args)) {
        bevy::log::warn!("aa-ui: AAUI.{name}: {e}");
    }
    true
}

/// The table `AAUI.PickupSlot` / `AAUI.DropSlot` get for a slot: what it is bound to (`kind` `"slot"` with `slotType` /
/// `slotIdx`, `"skill"` with `skillType`, `"item"` with `itemType` / `grade`), its `widget`, the `content` the hook
/// returned when it was bound (nil when `empty`).
fn info_table(lua: &Lua, id: Id) -> mlua::Result<Option<Table>> {
    let (source, bound, enabled) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id) else { return Ok(None) };
        let Some(s) = w.slot.as_ref() else { return Ok(None) };
        (s.source.clone(), s.binded.as_deref().is_some_and(|b| b != "none"), w.enabled)
    };
    let Some(source) = source else { return Ok(None) };
    let t = lua.create_table()?;
    match source {
        Source::Slot(ty, idx) => {
            t.raw_set("kind", "slot")?;
            t.raw_set("slotType", ty)?;
            t.raw_set("slotIdx", idx)?;
        }
        Source::Skill(s) => {
            t.raw_set("kind", "skill")?;
            t.raw_set("skillType", s)?;
        }
        Source::Item(it, g) => {
            t.raw_set("kind", "item")?;
            t.raw_set("itemType", it)?;
            t.raw_set("grade", g)?;
        }
    }
    t.raw_set("empty", !bound)?;
    t.raw_set("enabled", enabled)?;
    if let Some(w) = super::object_table(lua, id)? {
        t.raw_set("widget", w)?;
    }
    if let Some(c) = contents(lua)?.raw_get::<Option<Table>>(id)? {
        t.raw_set("content", c)?;
    }
    Ok(Some(t))
}

/// A drag started on slot `id` (it has no `OnDragStart` of its own): a bound, enabled slot lets the app pick its
/// content up (`AAUI.PickupSlot(info)`). Returns whether the hook ran.
pub(crate) fn drag_start(lua: &Lua, id: Id) -> bool {
    let Some(f) = hook(lua, "PickupSlot") else { return false };
    let info = match info_table(lua, id) {
        Ok(Some(t)) => t,
        _ => return false,
    };
    if info.raw_get::<bool>("empty").unwrap_or(true) || !info.raw_get::<bool>("enabled").unwrap_or(false) {
        return false;
    }
    if let Err(e) = f.call::<()>(info) {
        bevy::log::warn!("aa-ui: AAUI.PickupSlot: {e}");
    }
    super::flush(lua);
    true
}

/// The held thing was dropped on (or a click landed on) widget `id`: a `slot` bound with `EstablishSlot` (the action
/// bars) takes it through `AAUI.DropSlot(info)`, empty or not. False for anything else.
pub(crate) fn receive(lua: &Lua, id: Id) -> bool {
    let is_slot_source = {
        let u = ui(lua);
        u.tree.visible(id)
            && u.tree.widget(id).is_some_and(|w| {
                w.enabled
                    && w.slot.as_ref().is_some_and(|s| matches!(s.source, Some(Source::Slot(..))))
            })
    };
    if !is_slot_source {
        return false;
    }
    let Some(f) = hook(lua, "DropSlot") else { return false };
    let Ok(Some(info)) = info_table(lua, id) else { return false };
    if let Err(e) = f.call::<()>(info) {
        bevy::log::warn!("aa-ui: AAUI.DropSlot: {e}");
    }
    super::flush(lua);
    true
}

/// The text style a slot's remaining time is drawn in (`slot.cooltime_style`).
fn cooltime_block() -> TextBlock {
    TextBlock {
        style: TextStyle {
            size: 16.0,
            align: Align::Center,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Called when a slot widget is created: its icon drawable and `cooltime_style`.
pub(crate) fn init(lua: &Lua, id: Id, t: &Table) -> mlua::Result<()> {
    let icon = {
        let mut u = ui(lua);
        let d = u.tree.create_drawable(
            id,
            DrawKind::Image,
            "drawable:iconimage",
            DrawLayer::Artwork,
        );
        for (point, x, y) in [(Point::TopLeft, 1.0, 1.0), (Point::BottomRight, -1.0, -1.0)] {
            u.tree.add_anchor(
                d,
                Anchor {
                    point,
                    target: id,
                    rel: point,
                    x,
                    y,
                },
            );
        }
        // Always shown (the reference lists it on empty slots too); no texture, no quad.
        if let Some(w) = u.tree.widget_mut(id) {
            w.slot = Some(SlotData {
                icon: Some(d),
                ..Default::default()
            });
            w.aux_style = Some(cooltime_block());
        }
        d
    };
    let it = new_object_table(lua, icon)?;
    t.raw_set("icon", &it)?;
    t.raw_set("gradeIcon", it)?;
    t.raw_set("cooltime_style", super::style_table(lua, id, 2)?)?;
    Ok(())
}

/// `cooldown = { remain, total }` of a content table, in milliseconds.
fn content_cooldown(c: &Table) -> mlua::Result<Option<(f32, f32)>> {
    let Some(t) = c.raw_get::<Option<Table>>("cooldown")? else {
        return Ok(None);
    };
    let remain = t.raw_get::<Option<f64>>("remain")?.unwrap_or(0.0) as f32;
    let total = t.raw_get::<Option<f64>>("total")?.unwrap_or(remain as f64) as f32;
    Ok((remain > 0.0).then_some((remain, total.max(remain))))
}

/// Bind a slot: ask the hook, show the icon, tell the script.
fn bind(lua: &Lua, id: Id, source: Source) -> mlua::Result<()> {
    let content = match &source {
        Source::Slot(ty, idx) => match hook(lua, "SlotContent") {
            Some(f) => f.call::<Value>((*ty, *idx))?,
            None => Value::Nil,
        },
        Source::Skill(skill) => match hook(lua, "SkillContent") {
            Some(f) => f.call::<Value>(*skill)?,
            None => {
                let t = lua.create_table()?;
                t.raw_set("type", "skill")?;
                t.raw_set("id", *skill)?;
                Value::Table(t)
            }
        },
        Source::Item(item, grade) => match hook(lua, "ItemContent") {
            Some(f) => f.call::<Value>((*item, *grade))?,
            None => {
                let t = lua.create_table()?;
                t.raw_set("type", "item")?;
                t.raw_set("id", *item)?;
                Value::Table(t)
            }
        },
    };
    let content = match content {
        Value::Table(t) => Some(t),
        _ => None,
    };
    contents(lua)?.raw_set(id, content.clone())?;
    let binded: Option<String> = match &content {
        Some(c) => Some(
            c.raw_get::<Option<String>>("type")?
                .unwrap_or_else(|| "skill".into()),
        ),
        None => None,
    };
    let icon_path: Option<String> = match &content {
        Some(c) => c.raw_get("icon")?,
        None => None,
    };
    let stack: Option<i64> = match &content {
        Some(c) => c.raw_get("stack")?,
        None => None,
    };
    let grade: Option<i64> = match &content {
        Some(c) if binded.as_deref() == Some("item") => c.raw_get("grade")?,
        _ => None,
    };
    let can_use = match &content {
        Some(c) => c.raw_get::<Option<bool>>("canUse")?.unwrap_or(true),
        None => false,
    };
    let cooldown = match &content {
        Some(c) => content_cooldown(c)?,
        None => None,
    };
    let hotkey = match (&source, hook(lua, "SlotHotKey")) {
        (Source::Slot(ty, idx), Some(f)) => f.call::<Option<String>>((*ty, *idx))?.is_some(),
        _ => false,
    };
    let mut u = ui(lua);
    let icon = u
        .tree
        .widget(id)
        .and_then(|w| w.slot.as_ref())
        .and_then(|s| s.icon);
    if let Some(s) = u.tree.widget_mut(id).and_then(|w| w.slot.as_mut()) {
        s.source = Some(source);
        s.binded = binded.clone();
    }
    if let Some(d) = icon
        && let Some(dr) = u.tree.drawable_mut(d)
    {
        dr.texture = icon_path;
        dr.extra_textures = grade.and_then(grade_frame).into_iter().collect();
    }
    set_cooldown(lua, &mut u, id, cooldown);
    u.tree.touch();
    let ev = |what: &str, v: Value| {
        (
            id,
            "OnContentUpdated".to_string(),
            vec![Value::String(lua.create_string(what).unwrap()), v],
        )
    };
    let bound = binded.is_some();
    u.pending.push(ev("action_binded", Value::Boolean(bound)));
    if bound {
        u.pending.push(ev("can_use", Value::Boolean(can_use)));
        let (learned, extras) = skill_view_state(content.as_ref());
        u.pending.push(ev("learned", Value::Boolean(learned)));
        for (what, v) in extras {
            u.pending.push(ev(what, v));
        }
        u.pending
            .push(ev("hotkey_activated", Value::Boolean(hotkey)));
        if let Some(n) = stack {
            u.pending.push(ev("stack_count_changed", Value::Integer(n)));
        }
        if let Some((_, total)) = cooldown {
            u.pending
                .push(ev("skill_cooldown_start", Value::Number(total as f64)));
        }
    }
    Ok(())
}

/// Start (`Some((remain, total))` in ms) or clear a widget's cooldown sweep.
fn set_cooldown(lua: &Lua, u: &mut Ui, id: Id, cd: Option<(f32, f32)>) {
    let now = u.time_ms;
    let clocked = u.tree.widget(id).is_some_and(|w| w.slot.is_some());
    let value = cd.map(|(remain, total)| Cooldown {
        end_ms: now + remain as f64,
        total_ms: total,
        sampled_ms: (!clocked).then_some(remain),
        drawn: (-1, String::new()),
    });
    if let Ok(a) = active(lua) {
        let _ = a.raw_set(id, if value.is_some() && clocked { Value::Boolean(true) } else { Value::Nil });
    }
    if let Some(w) = u.tree.widget_mut(id) {
        w.cooldown = value;
    }
}

/// Optional skill-book state of a content table (the skill window's ability view slots):
/// `learned = false` (default true) and the events `canLearn` (bool), `reqPoints` (n) and
/// `skillLevel` (n) the action slot script reads (`slot.learned`, `.canLearn`, `.reqPoints`,
/// `.skillLevel`: red / cyan tint of unlearned skills, the required points over a passive buff,
/// the level badge over a skill above level 1).
fn skill_view_state(content: Option<&Table>) -> (bool, Vec<(&'static str, Value)>) {
    let Some(c) = content else {
        return (true, Vec::new());
    };
    let learned = c
        .raw_get::<Option<bool>>("learned")
        .ok()
        .flatten()
        .unwrap_or(true);
    let mut extras = Vec::new();
    if let Some(b) = c.raw_get::<Option<bool>>("canLearn").ok().flatten() {
        extras.push(("canLearn", Value::Boolean(b)));
    }
    for key in ["reqPoints", "skillLevel"] {
        if let Some(n) = c.raw_get::<Option<i64>>(key).ok().flatten() {
            extras.push((key, Value::Integer(n)));
        }
    }
    (learned, extras)
}

/// The client's item grade frame drawn over an item icon.
fn grade_frame(grade: i64) -> Option<String> {
    const NAMES: [&str; 12] = [
        "poor",
        "common",
        "uncommon",
        "rare",
        "ancient",
        "heroic",
        "unique",
        "artifact",
        "wonder",
        "epic",
        "legendary",
        "mythic",
    ];
    let name = NAMES.get(usize::try_from(grade).ok()?)?;
    Some(format!("ui/icon/item_grade_{grade}{name}.dds"))
}

/// Ask the hooks again for every bound slot (the app's bars or bags changed).
pub fn refresh_slots(lua: &Lua) -> mlua::Result<()> {
    let slots: Vec<(Id, Source)> = {
        let u = ui(lua);
        u.tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.alive)
            .filter_map(|(i, o)| {
                let s = o.widget()?.slot.as_ref()?.source.clone()?;
                Some((i as Id, s))
            })
            .collect()
    };
    for (id, s) in slots {
        bind(lua, id, s)?;
    }
    super::flush(lua);
    Ok(())
}

fn content_field(lua: &Lua, id: Id, key: &str) -> mlua::Result<Value> {
    match contents(lua)?.raw_get::<Option<Table>>(id)? {
        Some(t) => t.raw_get(key),
        None => Ok(Value::Nil),
    }
}

/// The native en_us time caption. Uses the same f32 operations and truncation as 392c50f0,
/// so exactly 1000 ms is `2s`, 60000 ms is `2m`, and durations above a day stay in hours.
pub fn time_label(remain_ms: f64) -> String {
    caption(remain_ms as f32, &CaptionOptions::default().texts)
}

fn cvtt(value: f32) -> i32 {
    // CVTTSS2SI returns the indefinite integer on NaN/overflow; Rust's cast saturates.
    if !value.is_finite() || !(-2147483648.0..2147483648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

fn caption(remain_ms: f32, texts: &[String; 3]) -> String {
    let seconds = remain_ms * 0.001_f32;
    if seconds <= 0.0 {
        return String::new();
    }
    let (value, unit) = if seconds > 3600.0 {
        (1_i32.wrapping_sub(cvtt(seconds * -0.00027777778450399637_f32)), 2)
    } else if seconds > 60.0 {
        (1_i32.wrapping_sub(cvtt(seconds * -0.01666666753590107_f32)), 1)
    } else {
        (cvtt(seconds).wrapping_add(1), 0)
    };
    let text = texts[unit].replace("$1", &value.to_string());
    // Native destination is eight bytes including NUL. Its subsequent XlMultiToWide
    // drops an incomplete final UTF-8 character (executed through 3922b830).
    let mut end = text.len().min(7);
    while !text.is_char_boundary(end) { end -= 1; }
    text[..end].to_owned()
}

/// Native 392c67e0 skips the slot's mask and caption below this total; equality is included.
pub const LABEL_MIN_TOTAL_MS: f32 = 200.0;

fn label_of(cd: &Cooldown, remain: f64, options: &CaptionOptions) -> Option<String> {
    (cd.total_ms >= LABEL_MIN_TOTAL_MS && options.visible)
        .then(|| caption(remain as f32, &options.texts))
}

/// What the screen shows of a cooldown with `remain` ms left: (sweep step, label).
fn drawn_state(cd: &Cooldown, remain: f64, options: &CaptionOptions, slot: bool) -> (i32, String) {
    let frac = (remain as f32 / cd.total_ms.max(1.0)).clamp(0.0, 1.0);
    (
        (frac * SWEEP_STEPS).floor() as i32,
        if slot { label_of(cd, remain, options).unwrap_or_default() } else { String::new() },
    )
}

/// Per frame, before the clock advances by `dt_ms`: finish the cooldowns that ran out (the sweep
/// goes, scripts get `OnContentUpdated(self, "skill_cooldown_done")`) and touch the tree when a
/// sweep or a label changed on screen.
pub(crate) fn update(lua: &Lua, dt_ms: f32) {
    let Ok(act) = active(lua) else { return };
    let ids: Vec<Id> = act
        .pairs::<Id, bool>()
        .filter_map(|kv| kv.ok().map(|(k, _)| k))
        .collect();
    if ids.is_empty() {
        return;
    }
    let mut finished: Vec<Id> = Vec::new();
    {
        let mut g = ui(lua);
        let u = &mut *g;
        let now = u.time_ms + dt_ms as f64;
        for id in ids {
            let cd = u
                .tree
                .objects
                .get(id as usize)
                .and_then(|o| o.widget())
                .and_then(|w| w.cooldown.clone());
            let Some(cd) = cd else {
                finished.push(id);
                continue;
            };
            if cd.sampled_ms.is_some() {
                finished.push(id);
                continue;
            }
            let remain = cd.end_ms - now;
            if remain <= 0.0 {
                if let Some(w) = u.tree.widget_mut(id) {
                    w.cooldown = None;
                }
                finished.push(id);
                let slot = u.tree.widget(id).is_some_and(|w| w.slot.is_some());
                if slot {
                    u.pending.push((
                        id,
                        "OnContentUpdated".into(),
                        vec![Value::String(
                            lua.create_string("skill_cooldown_done").unwrap(),
                        )],
                    ));
                }
                continue;
            }
            let slot = u.tree.widget(id).is_some_and(|w| w.slot.is_some());
            let state = drawn_state(&cd, remain, &u.cooldown_captions, slot);
            if state != cd.drawn
                && let Some(c) = u.tree.widget_mut(id).and_then(|w| w.cooldown.as_mut())
            {
                c.drawn = state;
            }
        }
    }
    if !finished.is_empty() {
        for id in finished {
            let _ = act.raw_set(id, Value::Nil);
        }
        super::flush(lua);
    }
}

/// The dark wipe of a cooldown with `frac` of its time left over `r` (a clockwise sweep from
/// 12 o'clock: the part already elapsed is clear, the rest dark), as triangles from the centre:
/// pixel positions, ready for [`Quad::corners`].
pub fn sweep_triangles(r: [f32; 4], frac: f32) -> Vec<[[f32; 2]; 3]> {
    let [x0, y0, x1, y1] = r;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (hw, hh) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    if hw <= 0.0 || hh <= 0.0 || frac <= 0.0 {
        return Vec::new();
    }
    let tau = std::f32::consts::TAU;
    let start = tau * (1.0 - frac.min(1.0));
    // Where a ray from the centre at `a` (clockwise from up) leaves the rectangle.
    let edge = |a: f32| -> [f32; 2] {
        let (dx, dy) = (a.sin(), -a.cos());
        let tx = if dx.abs() > 1e-6 {
            hw / dx.abs()
        } else {
            f32::MAX
        };
        let ty = if dy.abs() > 1e-6 {
            hh / dy.abs()
        } else {
            f32::MAX
        };
        let t = tx.min(ty);
        [cx + dx * t, cy + dy * t]
    };
    // Corner angles, clockwise from the top right one.
    let c1 = hw.atan2(hh);
    let mut angles = vec![start];
    angles.extend(
        [
            c1,
            std::f32::consts::PI - c1,
            std::f32::consts::PI + c1,
            tau - c1,
        ]
        .into_iter()
        .filter(|&a| a > start + 1e-4 && a < tau - 1e-4),
    );
    angles.push(tau);
    let mut out = Vec::new();
    for w in angles.windows(2) {
        let (a, b) = (edge(w[0]), edge(w[1]));
        let c = [cx, cy];
        let cross = (a[0] - c[0]) * (b[1] - c[1]) - (a[1] - c[1]) * (b[0] - c[0]);
        if cross.abs() < 1e-4 {
            continue;
        }
        // The same winding as the rectangles (clockwise on screen).
        out.push(if cross < 0.0 { [c, b, a] } else { [c, a, b] });
    }
    out
}

/// The quads of a widget's cooldown: the dark sweep over the icon (a slot's icon drawable, else
/// the whole widget) and the time left in `cooltime_style`. Drawn by `render::build_quads` after
/// the widget's background and artwork, before its overlay.
pub(crate) fn cooldown_quads(
    u: &mut Ui,
    id: Id,
    w: &Widget,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let Some(cd) = w.cooldown.as_ref() else {
        return;
    };
    let remain = cd.sampled_ms.map(f64::from).unwrap_or(cd.end_ms - u.time_ms);
    if remain <= 0.0 || cd.total_ms <= 0.0 {
        return;
    }
    if w.slot.is_some() && cd.total_ms < LABEL_MIN_TOTAL_MS {
        return;
    }
    let frac = (remain as f32 / cd.total_ms).clamp(0.0, 1.0);
    let widget_rect = u.tree.rect(id);
    let area = w
        .slot
        .as_ref()
        .and_then(|s| s.icon)
        .map(|d| u.tree.rect(d))
        .filter(|r| r.w > 0.0 && r.h > 0.0)
        .unwrap_or(widget_rect);
    let px = [
        (area.x * scale).round(),
        (area.y * scale).round(),
        (area.right() * scale).round(),
        (area.bottom() * scale).round(),
    ];
    let color = [
        SWEEP_COLOR[0],
        SWEEP_COLOR[1],
        SWEEP_COLOR[2],
        SWEEP_COLOR[3] * alpha,
    ];
    for [a, b, c] in sweep_triangles(px, frac) {
        out.push(Quad {
            tex: Tex::Atlas,
            pos: [0.0; 4],
            uv: crate::render::white_uv(),
            color,
            corners: Some([a, b, c, c]),
            uvs: None,
            vcolors: None,
        });
    }
    if w.slot.is_none() {
        return;
    }
    let Some(label) = label_of(cd, remain, &u.cooldown_captions) else {
        return;
    };
    let mut tb = w.aux_style.clone().unwrap_or_else(cooltime_block);
    tb.text = label;
    let rect: Rect = widget_rect;
    crate::render::text_quads(u, &tb, rect, tb.style.color, alpha, scale, out);
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "EstablishSlot",
        method(lua, |lua, id, a| {
            bind(lua, id, Source::Slot(a.f(1) as i64, a.f(2) as i64))?;
            none()
        })?,
    )?;
    def(
        "EstablishSkill",
        method(lua, |lua, id, a| {
            bind(lua, id, Source::Skill(a.f(1) as i64))?;
            none()
        })?,
    )?;
    def(
        "EstablishItem",
        method(lua, |lua, id, a| {
            bind(lua, id, Source::Item(a.f(1) as i64, a.f(2) as i64))?;
            none()
        })?,
    )?;
    def(
        "GetBindedType",
        method(lua, |lua, id, _| {
            let t = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.slot.as_ref())
                .and_then(|s| s.binded.clone())
                .unwrap_or_else(|| "none".into());
            ret(lua, t)
        })?,
    )?;
    def(
        "IsEmpty",
        method(lua, |lua, id, _| {
            let empty = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.slot.as_ref())
                .is_none_or(|s| s.binded.is_none());
            ret(lua, empty)
        })?,
    )?;
    def(
        "GetTooltip",
        method(lua, |lua, id, _| {
            ret(lua, content_field(lua, id, "tooltip")?)
        })?,
    )?;
    def(
        "GetExtraInfo",
        method(lua, |lua, id, _| {
            ret(lua, content_field(lua, id, "extraInfo")?)
        })?,
    )?;
    def(
        "GetItemLevelRequirment",
        method(lua, |lua, id, _| {
            ret(lua, content_field(lua, id, "levelRequirement")?)
        })?,
    )?;
    def("ResetState", method(lua, |_, _, _| none())?)?;
    // (slotType, slotIdx): the key bound to the slot, from `AAUI.SlotHotKey`.
    def(
        "GetHotKey",
        method(lua, |lua, id, a| {
            let (ty, idx) = match a.opt_f(1) {
                Some(t) => (t as i64, a.f(2) as i64),
                None => match ui(lua)
                    .tree
                    .widget(id)
                    .and_then(|w| w.slot.as_ref())
                    .and_then(|s| s.source.clone())
                {
                    Some(Source::Slot(t, i)) => (t, i),
                    _ => return ret(lua, ""),
                },
            };
            match hook(lua, "SlotHotKey") {
                Some(f) => {
                    let key = f.call::<Value>((ty, idx))?;
                    if key.is_nil() { ret(lua, "") } else { ret(lua, key) }
                }
                // Native 39256500 returns the slot's string at +0x340; its constructor
                // initializes it empty. Original pet bars abbreviate it without a nil guard.
                None => ret(lua, ""),
            }
        })?,
    )?;
    Ok(())
}

/// The cooldown buttons' natives: `SetCoolDown(remainMs, totalMs)` starts the sweep
/// (`SetCoolDown(0, 0)` clears it); the mask setters take the stock mask and do nothing.
pub(crate) fn cooldown_button_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetCoolDown",
        method(lua, |lua, id, a| {
            let remain = a.opt_f(1).unwrap_or(0.0);
            let total = a.opt_f(2).unwrap_or(remain).max(remain);
            let cd = (remain > 0.0).then_some((remain, total));
            let mut u = ui(lua);
            set_cooldown(lua, &mut u, id, cd);
            none()
        })?,
    )?;
    for name in ["SetCoolDownMask", "SetCoolDownMaskCoords"] {
        m.raw_set(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}

pub(crate) fn constant_cooldown_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetSkill",
        method(lua, |lua, id, a| {
            // 39280c20 only changes the cooldown identity and clears the ready flag.
            // EstablishSkill would also replace the equipment icon and native click accessor.
            if let Some(button) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                button.cooldown_skill = Some(ConstantSkillCooldown {
                    skill_type: match a.v(1) {
                        Value::Integer(n) => n as u32,
                        Value::Number(n) => n as u32,
                        _ => 0,
                    },
                    ready_notified: false,
                });
            }
            none()
        })?,
    )
}

/// Constant-button virtual update 39281df0. The app resolves the descriptor and native
/// global/tag/skill winner; this layer delivers the original Lua callback lifecycle.
pub(crate) fn update_constant_cooldowns(lua: &Lua) {
    let ids = ui(lua).tree.of_class("cooldownconstantbutton");
    for id in ids {
        let skill_type = {
            let u = ui(lua);
            if !u.tree.visible(id) {
                continue;
            }
            let Some(state) = u
                .tree
                .widget(id)
                .and_then(|w| w.button.as_ref())
                .and_then(|b| b.cooldown_skill.as_ref())
            else {
                continue;
            };
            state.skill_type
        };
        let result = match hook(lua, "SkillCooldown") {
            Some(f) => match f.call::<(Option<u32>, Option<u32>)>(skill_type) {
                Ok((Some(remain), Some(total))) => Some((remain, total)),
                Ok(_) => None,
                Err(e) => {
                    bevy::log::warn!("aa-ui: AAUI.SkillCooldown: {e}");
                    continue;
                }
            },
            None => None,
        };
        let notify = {
            let mut u = ui(lua);
            if !u.tree.visible(id) {
                continue;
            }
            let Some(state) = u
                .tree
                .widget_mut(id)
                .and_then(|w| w.button.as_mut())
                .and_then(|b| b.cooldown_skill.as_mut())
            else {
                continue;
            };
            if state.skill_type != skill_type {
                continue;
            }
            match result {
                // An absent descriptor emits zeros every update without setting +5d0.
                None => (!state.ready_notified).then_some((0, 0)),
                Some((0, total)) if !state.ready_notified => {
                    state.ready_notified = true;
                    Some((0, total))
                }
                Some((0, _)) => None,
                Some(pair) => {
                    state.ready_notified = false;
                    Some(pair)
                }
            }
        };
        if let Some((remain, total)) = notify {
            super::call_handler(
                lua,
                id,
                "OnContentUpdated",
                vec![
                    Value::String(lua.create_string("skill_cooldown").unwrap()),
                    Value::Number((remain as f32) as f64),
                    Value::Number((total as f32) as f64),
                ],
            );
        }
    }
}
