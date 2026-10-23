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
//! --   tooltip = {...} | function() return {...} or nil end, extraInfo = {...} }
//! -- A tooltip provider runs only in GetTooltip(), with no arguments. Its table/nil
//! -- result is returned without caching; errors propagate to the Lua caller.
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
//! icon (eight clockwise perimeter sectors from 12 o'clock, sampling the original
//! `ui/icon/cooldown_mask.dds` as both image and mask) and the time left as text in
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
//! `SetCoolDownMask` loads its actual texture; `SetCoolDownMaskCoords` selects a pixel rectangle.
//! A generic button without a loaded mask does not draw a sweep. It uses its selected image/icon
//! background's rectangle, which is inset one pixel in the original equipped-item buttons.
//! `cooldownconstantbutton:SetSkill(rawId)` observes `AAUI.SkillCooldown(rawId)`, which returns
//! `(remaining_ms, total_ms)` for an existing descriptor, or nil for an unknown descriptor.
//! This is independent of `SkillContent` and never replaces the item's icon or click action.
//! These four cooldown button classes draw a sweep without a native time caption. Slot captions
//! obey option 82 through [`set_cooldown_numbers`]. Native explicit global/own slot selection
//! (+311/+312) is not supplied by the current content hook; duration does not identify a global
//! cooldown. The app must not infer that state from `SkillCooldown`'s combined winning timer.
//! [`set_native_cooldown_mode`] explicitly enables the native selector adapter. Once enabled,
//! the app supplies raw global counts, the native accessor kind and an on-demand query through
//! [`update_native_slot_cooldown`]; the legacy content cooldown/countdown is bypassed for slots.
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

mod completion;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotData {
    /// How the slot was bound, to ask again on refresh.
    pub source: Option<Source>,
    /// `GetBindedType()`: "none", "skill", "item", ...
    pub binded: Option<String>,
    pub icon: Option<Id>,
    /// Native slot loader's stock mask image (+314/+328); independent of bound icon artwork.
    pub cooldown_mask: Option<String>,
    /// Actual slot-frame selector flags +311/+312, present only for the explicit native adapter.
    pub native_cooldown: Option<NativeCooldownState>,
    completion: Option<completion::Completion>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeCooldownState {
    pub global: bool,
    pub accessor_active: bool,
}

/// Raw unsigned native manager/accessor result, before the UI's f32 narrowing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooldownSample {
    pub remaining_ms: u32,
    pub total_ms: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Slot(i64, i64),
    Skill(i64),
    Item(i64, i64),
}

/// A cooldown ([`crate::tree::Widget::cooldown`]): compatibility slots use the UI clock;
/// native slots and generic buttons retain the latest explicit sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cooldown {
    /// UI time the slot cooldown ends at; for generic buttons, time of the sample plus remaining.
    pub end_ms: f64,
    pub total_ms: f32,
    /// Generic buttons and native-adapter slots retain the supplied snapshot. Compatibility
    /// slots count down locally; button providers notify their original Lua consumers.
    pub sampled_ms: Option<f32>,
    /// Native slot +311 suppresses captions; duration does not identify this state.
    pub global: bool,
    /// What the last frame update saw (sweep step, label): the tree is touched when it changes.
    pub drawn: (i32, String),
}

const NATIVE_MODE: &str = "aa_ui_native_slot_cooldowns";
fn native_mode(lua: &Lua) -> bool {
    lua.named_registry_value(NATIVE_MODE).unwrap_or(false)
}

/// Opt in before original controls bind, only when the app can supply genuine native samples.
/// Changing adapter mode clears UI samples/flags without changing any game cooldown manager.
pub fn set_native_cooldown_mode(lua: &Lua, enabled: bool) {
    if native_mode(lua) == enabled {
        return;
    }
    let _ = lua.set_named_registry_value(NATIVE_MODE, enabled);
    let ids: Vec<_> = {
        let mut u = ui(lua);
        u.tree
            .objects
            .iter_mut()
            .enumerate()
            .filter_map(|(id, o)| {
                let w = o.widget_mut()?;
                let s = w.slot.as_mut()?;
                s.native_cooldown = enabled.then(NativeCooldownState::default);
                s.completion = None;
                w.cooldown = None;
                Some(id as Id)
            })
            .collect()
    };
    if let Ok(active) = active(lua) {
        for id in &ids {
            let _ = active.raw_set(*id, Value::Nil);
        }
    }
    for id in ids { completion::untrack(lua, id); }
    ui(lua).tree.touch();
}

fn native_state(lua: &Lua, id: Id) -> Option<NativeCooldownState> {
    let u = ui(lua);
    let o = u.tree.objects.get(id as usize)?;
    o.alive.then_some(())?;
    o.widget()?.slot.as_ref()?.native_cooldown
}

fn native_event(lua: &Lua, id: Id, name: &str, duration: Option<u32>) {
    let mut args = vec![Value::String(lua.create_string(name).unwrap())];
    if let Some(ms) = duration {
        args.push(Value::Number(f64::from(ms as f32)));
    }
    super::call_handler(lua, id, "OnContentUpdated", args);
}

fn native_sample(lua: &Lua, id: Id, value: CooldownSample) {
    let Some(flags) = native_state(lua, id) else {
        return;
    };
    let mut u = ui(lua);
    if u.tree
        .widget(id)
        .and_then(|w| w.cooldown.as_ref())
        .is_some_and(|cd| {
            cd.sampled_ms == Some(value.remaining_ms as f32)
                && cd.total_ms == value.total_ms as f32
                && cd.global == flags.global
        })
    {
        return;
    }
    let now = u.time_ms;
    let sample = Cooldown {
        end_ms: now + f64::from(value.remaining_ms as f32),
        total_ms: value.total_ms as f32,
        sampled_ms: Some(value.remaining_ms as f32),
        global: flags.global,
        drawn: (-1, String::new()),
    };
    if let Some(w) = u.tree.widget_mut(id) {
        w.cooldown = Some(sample);
    }
}

/// Execute the source-closed cooldown selector of native392c7360. The caller must supply an
/// actual accessor kind and raw global sample; skip absent accessors. Query executes only on
/// the native accessor branch, after global_done (which can change the binding/data).
pub fn update_native_slot_cooldown(
    lua: &Lua,
    id: Id,
    accessor_kind: u32,
    global: CooldownSample,
    query: impl FnOnce(&Lua, Id) -> Option<CooldownSample>,
) {
    let Some(flags) = native_state(lua, id) else {
        return;
    };
    if !flags.accessor_active && global.remaining_ms > 0 && !matches!(accessor_kind, 16 | 17 | 19) {
        if !flags.global {
            if let Some(s) = ui(lua)
                .tree
                .widget_mut(id)
                .and_then(|w| w.slot.as_mut())
                .and_then(|s| s.native_cooldown.as_mut())
            {
                s.global = true;
            }
            native_event(lua, id, "global_cooldown_start", Some(global.total_ms));
        }
        native_sample(lua, id, global);
        return;
    }
    if flags.global {
        if let Some(s) = ui(lua)
            .tree
            .widget_mut(id)
            .and_then(|w| w.slot.as_mut())
            .and_then(|s| s.native_cooldown.as_mut())
        {
            s.global = false;
        }
        native_event(lua, id, "global_cooldown_done", None);
    }
    if native_state(lua, id).is_none() {
        return;
    }
    let value = query(lua, id);
    let Some(flags) = native_state(lua, id) else {
        return;
    };
    let Some(value) = value else {
        if let Some(s) = ui(lua)
            .tree
            .widget_mut(id)
            .and_then(|w| w.slot.as_mut())
            .and_then(|s| s.native_cooldown.as_mut())
        {
            *s = NativeCooldownState::default();
        }
        native_sample(lua, id, CooldownSample::default());
        return;
    };
    native_sample(lua, id, value);
    let running = value.remaining_ms > 0;
    if running != flags.accessor_active {
        if let Some(s) = ui(lua)
            .tree
            .widget_mut(id)
            .and_then(|w| w.slot.as_mut())
            .and_then(|s| s.native_cooldown.as_mut())
        {
            s.accessor_active = running;
        }
        native_event(
            lua,
            id,
            if running {
                "skill_cooldown_start"
            } else {
                "skill_cooldown_done"
            },
            running.then_some(value.total_ms),
        );
        if !running { completion::start(lua, id, value.total_ms as f32); }
    }
}

/// Native constant-button skill identity and its one-shot ready notification (+5cc/+5d0).
#[derive(Clone, Debug, PartialEq)]
pub struct ConstantSkillCooldown {
    pub skill_type: u32,
    pub ready_notified: bool,
}

/// Generic cooldown mask (+5b0/+5b4), retained independently of the drawing sample.
#[derive(Clone, Debug, PartialEq)]
pub struct CooldownMask {
    pub texture: String,
    pub uv: [f32; 4],
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

/// Native ordinary slots sample this stock texture as both image and mask.
const SLOT_MASK: &str = "ui/icon/cooldown_mask.dds";

/// Compatibility countdowns invalidate at180 equal progress steps per turn.
/// Actual native-adapter samples retain their separately supplied frame timing.
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
        let cooldown_mask = u.textures
            .resolve(SLOT_MASK)
            .filter(|path| u.textures.size(path).is_some());
        if let Some(w) = u.tree.widget_mut(id) {
            w.slot = Some(SlotData {
                icon: Some(d),
                cooldown_mask,
                native_cooldown: native_mode(lua).then(NativeCooldownState::default),
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
    let cooldown = if native_mode(lua) {
        None
    } else {
        match &content {
            Some(c) => content_cooldown(c)?,
            None => None,
        }
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
        global: false,
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
    (!cd.global && cd.total_ms >= LABEL_MIN_TOTAL_MS && options.visible)
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
    update_clocks(lua, dt_ms);
    completion::invalidate(lua);
}

fn update_clocks(lua: &Lua, dt_ms: f32) {
    let Ok(act) = active(lua) else { return };
    let ids: Vec<Id> = act
        .pairs::<Id, bool>()
        .filter_map(|kv| kv.ok().map(|(k, _)| k))
        .collect();
    if ids.is_empty() {
        return;
    }
    let mut finished: Vec<Id> = Vec::new();
    let mut flashes = Vec::new();
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
                    flashes.push((id, cd.total_ms));
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
        for (id, total) in flashes { completion::start(lua, id, total); }
    }
}

pub(crate) fn prepare_cooldown_draw(u: &mut Ui, id: Id) { completion::draw_frame(u, id); }
pub(crate) fn has_completion(w: &Widget) -> bool { completion::has(w) }

/// Native perimeter fan with `frac` of its time left, mapped to `r` without pixel snapping.
/// The slot's separate progress gate precedes this geometry routine.
pub fn sweep_triangles(r: [f32; 4], frac: f32) -> Vec<[[f32; 2]; 3]> {
    let [x0, y0, x1, y1] = r;
    if x1 <= x0 || y1 <= y0 || frac <= 0.0 {
        return Vec::new();
    }
    mask_triangles(1.0 - frac.min(1.0))
        .into_iter()
        .map(|triangle| triangle.map(|[x, y]| [x0 + (x1 - x0) * x, y0 + (y1 - y0) * y]))
        .collect()
}

/// Native3928ace0: eight equal time sectors along the rectangle perimeter.
/// Unit positions also supply the two independent affine UV streams.
pub fn mask_triangles(progress: f32) -> Vec<[[f32; 2]; 3]> {
    let edge = [
        [0.5, 0.],
        [1., 0.],
        [1., 0.5],
        [1., 1.],
        [0.5, 1.],
        [0., 1.],
        [0., 0.5],
        [0., 0.],
        [0.5, 0.],
    ];
    let mut out = Vec::new();
    let mut start = [0.5, 0.5];
    for i in 0..8 {
        if progress >= (i + 1) as f32 * 0.125 {
            continue;
        }
        // Native3928aed2 skips interpolation for negative/unordered progress,
        // retaining the previous endpoint (initially the center) instead of clamping.
        if progress >= i as f32 * 0.125 {
            let f = (progress - i as f32 * 0.125) * 8.;
            start = [
                edge[i][0] + (edge[i + 1][0] - edge[i][0]) * f,
                edge[i][1] + (edge[i + 1][1] - edge[i][1]) * f,
            ];
        }
        out.push([[0.5, 0.5], edge[i + 1], start]);
        start = edge[i + 1];
    }
    out
}

/// Shared native3928ace0/39220920 fan and submission for both button and slot images.
fn masked_image_quads(image: Quad, mask: &CooldownMask, progress: f32, out: &mut Vec<Quad>) {
    let Tex::Image(path) = image.tex else { return };
    let [x0, y0, x1, y1] = image.pos;
    let corners = image
        .corners
        .unwrap_or([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
    for [a, b, c] in mask_triangles(progress) {
        // Snap after interpolation, retaining both original fractional UV streams.
        let point = |[x, y]: [f32; 2]| {
            [
                corners[0][0]
                    + (corners[1][0] - corners[0][0]) * x
                    + (corners[3][0] - corners[0][0]) * y,
                corners[0][1]
                    + (corners[1][1] - corners[0][1]) * x
                    + (corners[3][1] - corners[0][1]) * y,
            ]
            .map(|p| (p + 0.5).trunc())
        };
        out.push(Quad {
            tex: Tex::MaskedImage {
                image: path.clone(),
                mask: mask.texture.clone(),
                image_uv: image.uv.map(f32::to_bits),
                mask_uv: mask.uv.map(f32::to_bits),
            },
            pos: [0.; 4],
            uv: [0., 0., 1., 1.],
            color: crate::render::rgba8(image.color),
            corners: Some([point(a), point(b), point(c), point(c)]),
            uvs: Some([a, b, c, c]),
            vcolors: None,
        });
    }
}

/// Native slot+32c: a fixed 5x22 image on a ten-pixel circle, restarted at the
/// current sweep angle on every draw. This is independent of the Lua motion API.
fn cooldown_edge(
    u: &mut Ui,
    rect: Rect,
    progress: f32,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    const HUD: &str = "ui/common/hud.dds";
    let Some(path) = u.textures.resolve(HUD) else {
        return;
    };
    let Some((width, height)) = u.textures.size(HUD) else {
        return;
    };
    let angle = (progress * 360.).to_radians();
    let (sin, cos) = angle.sin_cos();
    let center = [
        (rect.x + rect.w * 0.5) * scale,
        (rect.y + rect.h * 0.5) * scale,
    ];
    let radius = 10. * scale;
    let position = [center[0] + sin * radius, center[1] - cos * radius];
    // Keep the native matrix's f32 operation order. Reassociating this around
    // local coordinates changes add0.5/CVTT results at half-pixel boundaries.
    let translation = [
        (-center[0] * cos + center[1] * sin) + position[0],
        (-center[0] * sin - center[1] * cos) + position[1],
    ];
    let corners = [[-2.5, -11.], [2.5, -11.], [2.5, 11.], [-2.5, 11.]].map(|[x, y]| {
        let (x, y) = (center[0] + x * scale, center[1] + y * scale);
        // Image39220590 snaps AFTER the effect's rotation and translation.
        [
            (x * cos + y * -sin) + translation[0],
            (y * cos + x * sin) + translation[1],
        ]
        .map(|p| (p + 0.5).trunc())
    });
    out.push(Quad {
        tex: Tex::Image(path),
        pos: [corners[0][0], corners[0][1], corners[2][0], corners[2][1]],
        uv: [
            748. / width as f32,
            166. / height as f32,
            753. / width as f32,
            188. / height as f32,
        ],
        color: [
            1.,
            1.,
            1.,
            ((f64::from(alpha.clamp(0., 1.)) * 255.).trunc() / 255.) as f32,
        ],
        corners: Some(corners),
        uvs: None,
        vcolors: None,
    });
}

/// The quads of a widget's cooldown: the native texture fan (a slot's independent one-pixel
/// inset image, or a generic button's selected art) and slot time in `cooltime_style`. Drawn after
/// the widget's background and artwork, before its overlay.
pub(crate) fn cooldown_quads(
    u: &mut Ui,
    id: Id,
    w: &Widget,
    alpha: f32,
    scale: f32,
    background: Option<(Id, crate::tree::ButtonState)>,
    out: &mut Vec<Quad>,
) {
    completion::quads(u, id, w, alpha, scale, out);
    let Some(cd) = w.cooldown.as_ref() else {
        return;
    };
    let remain = cd
        .sampled_ms
        .map(f64::from)
        .unwrap_or(cd.end_ms - u.time_ms);
    if remain <= 0.0 || (w.slot.is_some() && cd.total_ms <= 0.0) {
        return;
    }
    if w.slot.is_some() && cd.total_ms < LABEL_MIN_TOTAL_MS {
        return;
    }
    let widget_rect = u.tree.rect(id);
    if w.slot.is_none() {
        let Some(mask) = w
            .button
            .as_ref()
            .and_then(|b| b.cooldown_mask.as_ref())
            .cloned()
        else {
            return;
        };
        let Some((background, state)) = background else {
            return;
        };
        let Some(drawable) = u.tree.get(background).drawable() else {
            return;
        };
        if !matches!(drawable.kind, DrawKind::Image | DrawKind::Icon) {
            return;
        }
        let icon = drawable.class == "drawable:iconimage";
        let mut art = Vec::new();
        crate::render::cooldown_background_quads(u, background, alpha, state, scale, &mut art);
        // Icon3928ed10 reuses object+d0 for every retained texture layer.
        let icon_uv = if icon {
            art.first().map(|q| q.uv)
        } else {
            None
        };
        let progress = (cd.total_ms - remain as f32) / cd.total_ms;
        for mut image in art {
            image.uv = icon_uv.unwrap_or(image.uv);
            masked_image_quads(image, &mask, progress, out);
        }
        return;
    }
    let slot = w.slot.as_ref().unwrap();
    let progress = (cd.total_ms - remain as f32) / cd.total_ms;
    if let Some(texture) = &slot.cooldown_mask {
        if progress > 0. && progress <= 1. {
            masked_image_quads(
                Quad {
                    tex: Tex::Image(texture.clone()),
                    pos: [
                        (widget_rect.x + 1.) * scale,
                        (widget_rect.y + 1.) * scale,
                        (widget_rect.right() - 1.) * scale,
                        (widget_rect.bottom() - 1.) * scale,
                    ],
                    uv: [0., 0., 1., 1.],
                    color: [1., 1., 1., alpha],
                    corners: None,
                    uvs: None,
                    vcolors: None,
                },
                &CooldownMask {
                    texture: texture.clone(),
                    uv: [0., 0., 1., 1.],
                },
                progress,
                out,
            );
            cooldown_edge(u, widget_rect, progress, alpha, scale, out);
        } else if slot.native_cooldown.is_some() {
            // Native392c3960 clears draw fields before caption delivery, without an event.
            // Compatibility's separate estimated clock remains until its next tick/rebind.
            if let Some(cd) = u.tree.widget_mut(id).and_then(|w| w.cooldown.as_mut()) {
                cd.total_ms = 0.;
                cd.sampled_ms = Some(0.);
            }
            return;
        }
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
            // Resolve the stored value before entering Lua: the provider may rebind or
            // retire this slot. No UI/tree/app-data borrow survives the callback.
            let tooltip = content_field(lua, id, "tooltip")?;
            match tooltip {
                Value::Function(provider) => ret(lua, provider.call::<Option<Table>>(())?),
                value => ret(lua, value),
            }
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
    def(
        "ResetState",
        method(lua, |lua, id, _| {
            if let Some(w) = ui(lua).tree.widget_mut(id)
                && let Some(flags) = w.slot.as_mut().and_then(|s| s.native_cooldown.as_mut())
            {
                *flags = NativeCooldownState::default();
                if let Some(cd) = w.cooldown.as_mut() {
                    cd.global = false;
                }
            }
            none()
        })?,
    )?;
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

/// The cooldown buttons' native setter stores two raw f32 numbers; nonpositive remaining
/// suppresses drawing without discarding the sample. Mask state survives timer replacement.
pub(crate) fn cooldown_button_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetCoolDown",
        method(lua, |lua, id, a| {
            // Native392328b0/3922f380 requires both Lua number values. Strings,
            // booleans and missing arguments leave the prior raw sample intact.
            let number = |index| match a.v(index) {
                Value::Integer(n) => Some(n as f32),
                Value::Number(n) => Some(n as f32),
                _ => None,
            };
            if let (Some(remain), Some(total)) = (number(1), number(2)) {
                let mut u = ui(lua);
                set_cooldown(lua, &mut u, id, Some((remain, total)));
            }
            none()
        })?,
    )?;
    m.raw_set("SetCoolDownMask", method(lua, |lua, id, a| {
        let mut u = ui(lua);
        if let Some(path) = a.s(1)
            && let Some(texture) = u.textures.resolve(&path)
            && u.textures.size(&texture).is_some()
            && let Some(button) = u.tree.widget_mut(id).and_then(|w| w.button.as_mut())
        {
            button.cooldown_mask = Some(CooldownMask { texture, uv: [0., 0., 1., 1.] });
        }
        none()
    })?)?;
    m.raw_set("SetCoolDownMaskCoords", method(lua, |lua, id, a| {
        let mut u = ui(lua);
        let mask = u.tree.widget(id).and_then(|w| w.button.as_ref()).and_then(|b| b.cooldown_mask.clone());
        if let Some(mask) = mask
            && let Some((width, height)) = u.textures.size(&mask.texture)
        {
            let rw = 1.0_f32 / width as f32;
            let rh = 1.0_f32 / height as f32;
            let (x, y, w, h) = (a.f(1), a.f(2), a.f(3), a.f(4));
            let uv = [x * rw, y * rh,
                if w == 0. { 1. } else { (x + w) * rw },
                if h == 0. { 1. } else { (y + h) * rh }];
            if let Some(mask) = u.tree.widget_mut(id).and_then(|w| w.button.as_mut()).and_then(|b| b.cooldown_mask.as_mut()) {
                mask.uv = uv;
            }
        }
        none()
    })?)?;
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
