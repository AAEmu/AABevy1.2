//! `slot` widgets (action bar, skill and bag slots): an icon bound to a game slot.
//!
//! The client binds a slot to game data natively (`EstablishSlot(slotType, slotIdx)`,
//! `EstablishSkill(skillType)`, `EstablishItem(itemType, grade)`), draws the bound icon and tells
//! the script through `OnContentUpdated(self, what, value)` (`"action_binded"`, `"can_use"`,
//! `"learned"`, `"stack_count_changed"`). Here the game data comes from Lua hooks the app (or the
//! example fakes) defines:
//!
//! ```lua
//! AAUI.SlotContent(slotType, slotIdx)  -- EstablishSlot
//! AAUI.SkillContent(skillType)         -- EstablishSkill (optional)
//! AAUI.ItemContent(itemType, grade)    -- EstablishItem (optional)
//! -- each returns nil (empty) or
//! -- { type = "skill" | "item" | "function" | ..., id = n, icon = "ui/icon/x.dds", stack = n,
//! --   grade = n (items: the grade frame ui/icon/item_grade_<n><name>.dds over the icon),
//! --   canUse = false (default true: dark and a red hotkey when false),
//! --   tooltip = {...}, extraInfo = {...} }
//! AAUI.SlotHotKey(slotType, slotIdx)   -- slot:GetHotKey: "1", "SHIFT-1", "MINUS", ... or nil
//! ```
//!
//! [`refresh_slots`] asks again for every bound slot (the app calls it when bars change). The slot
//! creates its icon drawable itself (`slot.icon`, also `slot.gradeIcon`, inset 1 like the
//! client's 40x40 icon in a 42x42 slot) and a `cooltime_style` text style.

use mlua::{Function, Lua, Table, Value};

use super::{method, new_object_table, none, ret, ui};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

#[derive(Clone, Debug, Default)]
pub struct SlotData {
    /// How the slot was bound, to ask again on refresh.
    pub source: Option<Source>,
    /// `GetBindedType()`: "none", "skill", "item", ...
    pub binded: Option<String>,
    pub icon: Option<Id>,
}

#[derive(Clone, Debug)]
pub enum Source {
    Slot(i64, i64),
    Skill(i64),
    Item(i64, i64),
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

/// Called when a slot widget is created: its icon drawable and `cooltime_style`.
pub(crate) fn init(lua: &Lua, id: Id, t: &Table) -> mlua::Result<()> {
    let icon = {
        let mut u = ui(lua);
        let d = u
            .tree
            .create_drawable(id, DrawKind::Image, "drawable:iconimage", DrawLayer::Artwork);
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
        }
        d
    };
    let it = new_object_table(lua, icon)?;
    t.raw_set("icon", &it)?;
    t.raw_set("gradeIcon", it)?;
    t.raw_set("cooltime_style", super::style_table(lua, id, 2)?)?;
    Ok(())
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
        Some(c) => Some(c.raw_get::<Option<String>>("type")?.unwrap_or_else(|| "skill".into())),
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
    let hotkey = match (&source, hook(lua, "SlotHotKey")) {
        (Source::Slot(ty, idx), Some(f)) => f.call::<Option<String>>((*ty, *idx))?.is_some(),
        _ => false,
    };
    let mut u = ui(lua);
    let icon = u.tree.widget(id).and_then(|w| w.slot.as_ref()).and_then(|s| s.icon);
    if let Some(s) = u.tree.widget_mut(id).and_then(|w| w.slot.as_mut()) {
        s.source = Some(source);
        s.binded = binded.clone();
    }
    if let Some(d) = icon {
        if let Some(dr) = u.tree.drawable_mut(d) {
            dr.texture = icon_path;
            dr.extra_textures = grade.and_then(grade_frame).into_iter().collect();
        }
    }
    u.tree.touch();
    let ev = |what: &str, v: Value| (id, "OnContentUpdated".to_string(), vec![Value::String(lua.create_string(what).unwrap()), v]);
    let bound = binded.is_some();
    u.pending.push(ev("action_binded", Value::Boolean(bound)));
    if bound {
        u.pending.push(ev("can_use", Value::Boolean(can_use)));
        u.pending.push(ev("learned", Value::Boolean(true)));
        u.pending.push(ev("hotkey_activated", Value::Boolean(hotkey)));
        if let Some(n) = stack {
            u.pending.push(ev("stack_count_changed", Value::Integer(n)));
        }
    }
    Ok(())
}

/// The client's item grade frame drawn over an item icon.
fn grade_frame(grade: i64) -> Option<String> {
    const NAMES: [&str; 12] = [
        "poor", "common", "uncommon", "rare", "ancient", "heroic", "unique", "artifact", "wonder",
        "epic", "legendary", "mythic",
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
        method(lua, |lua, id, _| ret(lua, content_field(lua, id, "tooltip")?))?,
    )?;
    def(
        "GetExtraInfo",
        method(lua, |lua, id, _| ret(lua, content_field(lua, id, "extraInfo")?))?,
    )?;
    def(
        "GetItemLevelRequirment",
        method(lua, |lua, id, _| ret(lua, content_field(lua, id, "levelRequirement")?))?,
    )?;
    def("ResetState", method(lua, |_, _, _| none())?)?;
    // (slotType, slotIdx): the key bound to the slot, from `AAUI.SlotHotKey`.
    def(
        "GetHotKey",
        method(lua, |lua, id, a| {
            let (ty, idx) = match a.opt_f(1) {
                Some(t) => (t as i64, a.f(2) as i64),
                None => match ui(lua).tree.widget(id).and_then(|w| w.slot.as_ref()).and_then(|s| s.source.clone()) {
                    Some(Source::Slot(t, i)) => (t, i),
                    _ => return none(),
                },
            };
            match hook(lua, "SlotHotKey") {
                Some(f) => ret(lua, f.call::<Value>((ty, idx))?),
                None => none(),
            }
        })?,
    )?;
    Ok(())
}
