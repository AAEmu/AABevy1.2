//! Drag and drop, and what the pointer holds, as the client's native widgets do it.
//!
//! # Dragging
//!
//! `widget:RegisterForDrag("LeftButton")` marks a widget as a drag source (the slots of the action bars and the
//! skill window, the bag buttons, the equipment slots, the windows' title bars). With the left button down on it
//! (or on a child of it) the pointer starts a drag once it has moved [`DRAG_START_DISTANCE`] UI units
//! ([`crate::input`]); then
//!
//! 1. the source gets `OnDragStart(self, "LeftButton")`; a `slot` without that handler picks its content up
//!    natively through `AAUI.PickupSlot(info)` (below);
//! 2. on release the widget under the pointer (or the nearest ancestor with one) that takes drops gets
//!    `OnDragReceive(self, "LeftButton")` while something is held; a `slot` takes the drop natively through
//!    `AAUI.DropSlot(info)`;
//! 3. the source gets `OnDragStop(self, "LeftButton")`;
//! 4. if something is still held, `AAUI.DropOnNothing(overUi)` (`overUi`: the pointer was over a widget that
//!    did not take the drop; false: over the world).
//!
//! The click that would follow the release is not sent. Right click while holding something puts it back
//! (the held thing is cleared).
//!
//! # The held thing: `X2Cursor`
//!
//! What the pointer holds is a Lua table (the app's own: which bag slot, which skill ...) plus what the toolkit
//! needs to draw it next to the pointer ([`crate::render::held_quads`]):
//!
//! ```lua
//! X2Cursor:SetHeld({ kind = "bag", icon = "ui/icon/x.dds", frame = "ui/icon/item_grade_1common.dds", count = 3, ... })
//! X2Cursor:GetHeld()                 -- the same table, or nil
//! X2Cursor:GetCursorInfo()           -- native Item / Item_Id / Spell / Macro, or nil
//! X2Cursor:ClearCursor()
//! X2Cursor:SetCursorImage(path, x, y)  -- an image cursor (the move cursor of a window being dragged); kind "image"
//! X2Cursor:GetCursorPickedBagItemIndex() / GetCursorPickedBagItemAmount()  -- `slot` / `count` of a "bag" thing
//! ```
//!
//! `icon` / `frame` are pak paths, `count` is drawn in the corner when above 1. An `"image"` cursor is never
//! dropped anywhere.
//!
//! # Hooks the app fills in (`AAUI`)
//!
//! ```lua
//! AAUI.PickupSlot(info)          -- a drag started on a bound `slot` that has no OnDragStart of its own
//! AAUI.DropSlot(info)            -- something held was dropped on (or a left click landed on) a `slot` bound with EstablishSlot
//! AAUI.DropOnNothing(overUi)     -- a drag ended and the thing is still held
//! -- info = { widget, kind = "slot" | "skill" | "item", slotType, slotIdx | skillType | itemType, grade, content, empty }
//! ```

use std::collections::{HashMap, HashSet};

use mlua::{Function, Lua, MultiValue, Table, Value};

use super::{call_handler, has_handler, install_game_stub, ui};
use crate::tree::{Anchor, Id, Point, ROOT};

/// How far (UI units) the pointer must move with the button down before a drag starts.
pub const DRAG_START_DISTANCE: f32 = 4.0;

/// What the pointer holds, drawn next to it.
#[derive(Clone)]
pub struct Held {
    pub kind: String,
    pub icon: Option<String>,
    pub frame: Option<String>,
    pub count: i64,
    /// Where the image sits relative to the pointer (UI units); items are centred on it.
    pub offset: [f32; 2],
    /// The app's table (`X2Cursor:GetHeld()`).
    pub table: Table,
}

/// Drag registrations and the held thing; a field of [`Ui`].
#[derive(Default)]
pub struct DragState {
    pub(super) chat: Option<(Id, Id)>,
    /// Native per-window repeat clocks do not belong to tree repaint data.
    pub(super) chat_elapsed_ms: HashMap<Id, f32>,
    /// Widgets that did `RegisterForDrag`.
    pub registered: HashSet<Id>,
    pub held: Option<Held>,
    /// Counts every change of the held thing; the renderer rebuilds the pointer's icon when it moves on.
    pub held_gen: u64,
    /// Window moved by `StartMoving` (the original title bars call it on mouse down).
    moving: Option<Moving>,
    pub(crate) resize_hover: Option<Id>,
}

struct Moving {
    id: Id,
    /// Initial top-left and size, plus the pointer at the start, in UI units.
    rect: crate::tree::Rect,
    pointer: (f32, f32),
    sizing: Option<Point>,
}

impl DragState {
    pub(super) fn moving_window(&self) -> Option<Id> {
        self.moving.as_ref().filter(|m| m.sizing.is_none()).map(|m| m.id)
    }
    /// Native 392aa7e0 clears only input references to the destroyed widget.
    pub(super) fn removed(&mut self, ids: &std::collections::BTreeSet<Id>) {
        self.chat_elapsed_ms.retain(|id, _| !ids.contains(id));
        if self.moving.as_ref().is_some_and(|m| ids.contains(&m.id)) {
            self.moving = None;
            if self.held.as_ref().is_some_and(|h| h.kind == "image") {
                self.held = None;
                self.held_gen += 1;
            }
        }
        if self.resize_hover.is_some_and(|id| ids.contains(&id)) {
            self.resize_hover = None;
        }
    }
}

pub(crate) fn start_sizing(lua: &Lua, id: Id, point: Point) {
    let mut u = ui(lua);
    if u.tree.widget(id).is_none() {
        return;
    }
    let rect = u.tree.rect(id);
    super::resize::place(&mut u, id, rect);
    drop(u);
    super::anchors::changed(lua, id);
    let mut u = ui(lua);
    if !u.tree.get(id).alive { return; }
    u.drag.moving = Some(Moving {
        id,
        rect: u.tree.rect(id),
        pointer: u.cursor,
        sizing: Some(point),
    });
}

pub(crate) fn start_moving(lua: &Lua, id: Id) {
    let mut u = ui(lua);
    if u.tree.widget(id).is_some_and(|w| w.enabled) {
        let rect = u.tree.rect(id);
        super::resize::place(&mut u, id, rect);
        drop(u);
        // Native 39278a10 installs the absolute anchor before manager capture.
        super::anchors::changed(lua, id);
        let mut u = ui(lua);
        if !u.tree.get(id).alive { return; }
        u.drag.moving = Some(Moving {
            id,
            rect: u.tree.rect(id),
            pointer: u.cursor,
            sizing: None,
        });
    }
}

pub(crate) fn sizing(lua: &Lua) -> bool {
    ui(lua)
        .drag
        .moving
        .as_ref()
        .is_some_and(|m| m.sizing.is_some())
}

pub(crate) fn resize_capture(u: &super::Ui) -> Option<(Id, Point)> {
    let m = u.drag.moving.as_ref()?;
    m.sizing.map(|p| (m.id, p))
}

/// Move a captured window with the pointer before hit testing its title bar again.
pub(crate) fn move_to(lua: &Lua, pointer: (f32, f32)) {
    let mut u = ui(lua);
    let Some(m) = u.drag.moving.as_ref() else {
        return;
    };
    let (id, r, start, sizing) = (m.id, m.rect, m.pointer, m.sizing);
    if let Some(point) = sizing {
        if !u.tree.visible(id) {
            drop(u);
            stop_moving(lua);
            return;
        }
        if pointer == start { return; }
        u.drag.moving.as_mut().unwrap().pointer = pointer;
        super::resize::motion(&mut u, id, point, start, pointer);
        drop(u);
        super::anchors::changed(lua, id);
        return;
    }
    let (dx, dy) = (pointer.0 - start.0, pointer.1 - start.1);
    let current = u.tree.rect(id);
    let (x, y) = (r.x + dx, r.y + dy);
    if current.x == x && current.y == y {
        return;
    }
    // Capture already installed the absolute anchor. Motion updates that record,
    // retaining native dependency readiness rather than removing/recreating it.
    u.tree.add_anchor(
        id,
        Anchor {
            point: Point::TopLeft,
            target: ROOT,
            rel: Point::TopLeft,
            x,
            y,
        },
    );
    let object = u.tree.get_mut(id);
    object.width = r.w;
    object.height = r.h;
    drop(u);
    super::anchors::changed(lua, id);
}

/// Clear capture before invoking Lua, which may start another move or retire the window.
fn release_moving(lua: &Lua) -> Option<Id> {
    let mut u = ui(lua);
    let moving = u.drag.moving.take();
    if moving.is_some() && u.drag.held.as_ref().is_some_and(|h| h.kind == "image") {
        u.drag.held = None;
        u.drag.held_gen += 1;
    }
    moving.map(|m| m.id)
}

/// Pointer release ends only an active capture, including outside the OS window. Native
/// 39278ab0 calls virtual +138 even without a bounds change; scripts save through event 31.
pub(crate) fn stop_moving(lua: &Lua) {
    if let Some(id) = release_moving(lua) {
        call_handler(lua, id, "OnMovedPosition", vec![]);
    }
}

fn hook(lua: &Lua, name: &str) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get(name).ok()
}

fn arg(lua: &Lua, s: &str) -> Value {
    Value::String(lua.create_string(s).expect("lua string"))
}

/// Something that can be dropped is held (an image cursor is not).
pub fn holding(lua: &Lua) -> bool {
    ui(lua)
        .drag
        .held
        .as_ref()
        .is_some_and(|h| h.kind != "image")
}

/// Whether anything at all is held (the pointer draws it).
pub fn held_kind(lua: &Lua) -> Option<String> {
    ui(lua).drag.held.as_ref().map(|h| h.kind.clone())
}

/// Native name table 39ac9974, queried by 394649d0. Internal hand kinds stay in GetHeld.
/// Apps with an item definition supply cursorInfo: stackable items use Item; unique items
/// use Item_Id. Count alone cannot distinguish a single remaining stackable item.
pub fn cursor_info(lua: &Lua) -> Option<String> {
    let held = ui(lua).drag.held.clone()?;
    if held.kind == "image" {
        return None;
    }
    if let Ok(Some(name)) = held.table.raw_get::<Option<String>>("cursorInfo") {
        return matches!(name.as_str(), "Item" | "Item_Id" | "Spell" | "Macro").then_some(name);
    }
    let name = match held.kind.as_str() {
        "bag" | "item" => "Item",
        "equip" => "Item_Id",
        "skill" => "Spell",
        "macro" => "Macro",
        "action" => match held
            .table
            .raw_get::<Option<String>>("what")
            .ok()
            .flatten()
            .as_deref()
        {
            Some("item") => "Item",
            Some("skill") => "Spell",
            Some("macro") => "Macro",
            _ => return None,
        },
        _ => return None,
    };
    Some(name.to_string())
}

/// Let go of what is held.
pub fn clear(lua: &Lua) {
    let mut g = ui(lua);
    if g.drag.held.take().is_some() {
        g.drag.held_gen += 1;
    }
}

/// Hold `table` (`kind`, `icon`, `frame`, `count` are read from it).
pub fn set_held(lua: &Lua, table: Table) -> mlua::Result<()> {
    let held = Held {
        kind: table
            .raw_get::<Option<String>>("kind")?
            .unwrap_or_else(|| "item".into()),
        icon: table.raw_get("icon")?,
        frame: table.raw_get("frame")?,
        count: table.raw_get::<Option<i64>>("count")?.unwrap_or(0),
        offset: [0.0; 2],
        table,
    };
    let mut g = ui(lua);
    g.drag.held = Some(held);
    g.drag.held_gen += 1;
    Ok(())
}

/// `id` or its nearest ancestor that was registered for dragging.
pub fn source_of(lua: &Lua, id: Id) -> Option<Id> {
    let u = ui(lua);
    let mut t = Some(id);
    while let Some(i) = t {
        if u.tree.widget(i).is_some_and(|w| !w.enabled) {
            return None;
        }
        if (u.drag.registered.contains(&i) || super::chat::tab_drag_source(&u, i))
            && u.tree.get(i).alive {
            return Some(i);
        }
        t = u.tree.get(i).parent;
    }
    None
}

/// The pointer started dragging `src`.
pub fn start(lua: &Lua, src: Id) {
    if super::widget::button_ignores_fading_input(lua, src) { return; }
    let name = ui(lua).tree.get(src).name.clone();
    bevy::log::debug!(target: "aa_ui::drag", "drag start on {name}");
    if has_handler(lua, src, "OnDragStart") {
        call_handler(lua, src, "OnDragStart", vec![arg(lua, "LeftButton")]);
    } else {
        super::slot::drag_start(lua, src);
    }
    super::chat::drag_started(lua, src);
}

/// The button was released after a drag of `src`, over `hover`.
pub fn finish(lua: &Lua, src: Id, hover: Option<Id>) {
    let names = {
        let u = ui(lua);
        (
            u.tree.get(src).name.clone(),
            hover.map(|h| u.tree.get(h).name.clone()),
        )
    };
    bevy::log::debug!(target: "aa_ui::drag", "drag of {} ends over {:?}, holding {}", names.0, names.1, holding(lua));
    if holding(lua) {
        let mut t = hover;
        while let Some(id) = t {
            // A disabled control blocks the drop rather than forwarding it to a parent.
            if ui(lua).tree.widget(id).is_some_and(|w| !w.enabled)
                || super::widget::button_ignores_fading_input(lua, id) {
                break;
            }
            if has_handler(lua, id, "OnDragReceive") {
                call_handler(lua, id, "OnDragReceive", vec![arg(lua, "LeftButton")]);
                break;
            }
            if super::slot::receive(lua, id) {
                break;
            }
            t = ui(lua).tree.get(id).parent;
        }
    }
    if !super::widget::button_ignores_fading_input(lua, src) {
        if has_handler(lua, src, "OnDragStop") {
            call_handler(lua, src, "OnDragStop", vec![arg(lua, "LeftButton")]);
        }
        super::chat::drag_stopped(lua, src);
    }
    if holding(lua) {
        drop_on_nothing(lua, hover.is_some());
    }
}

/// Nothing took the held thing: tell the app (`AAUI.DropOnNothing`), or let go of it without the hook.
pub fn drop_on_nothing(lua: &Lua, over_ui: bool) {
    let Some(f) = hook(lua, "DropOnNothing") else {
        clear(lua);
        return;
    };
    if let Err(e) = f.call::<()>(over_ui) {
        bevy::log::warn!("aa-ui: AAUI.DropOnNothing: {e}");
        clear(lua);
    }
    super::flush(lua);
}

/// `X2Cursor` and `RegisterForDrag`'s storage.
pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let cursor = install_game_stub(lua, "X2Cursor")?;
    cursor.raw_set(
        "ClearCursor",
        lua.create_function(|lua, _: MultiValue| {
            clear(lua);
            Ok(())
        })?,
    )?;
    cursor.raw_set(
        "SetCursorImage",
        lua.create_function(
            |lua, (_this, path, x, y): (Value, Option<String>, Option<f32>, Option<f32>)| {
                let Some(path) = path else {
                    clear(lua);
                    return Ok(());
                };
                let t = lua.create_table()?;
                t.raw_set("kind", "image")?;
                t.raw_set("icon", path.as_str())?;
                let mut g = ui(lua);
                g.drag.held = Some(Held {
                    kind: "image".into(),
                    icon: Some(path),
                    frame: None,
                    count: 0,
                    offset: [x.unwrap_or(0.0), y.unwrap_or(0.0)],
                    table: t,
                });
                g.drag.held_gen += 1;
                Ok(())
            },
        )?,
    )?;
    cursor.raw_set(
        "SetHeld",
        lua.create_function(|lua, (_this, t): (Value, Option<Table>)| match t {
            Some(t) => set_held(lua, t),
            None => {
                clear(lua);
                Ok(())
            }
        })?,
    )?;
    cursor.raw_set(
        "GetHeld",
        lua.create_function(|lua, _: MultiValue| {
            Ok(ui(lua).drag.held.as_ref().map(|h| h.table.clone()))
        })?,
    )?;
    cursor.raw_set(
        "GetCursorInfo",
        lua.create_function(|lua, _: MultiValue| Ok(cursor_info(lua)))?,
    )?;
    cursor.raw_set(
        "GetCursorPickedBagItemIndex",
        lua.create_function(|lua, _: MultiValue| {
            let h = ui(lua).drag.held.clone();
            match h.filter(|h| h.kind == "bag") {
                Some(h) => h.table.raw_get::<Option<i64>>("slot"),
                None => Ok(None),
            }
        })?,
    )?;
    cursor.raw_set(
        "GetCursorPickedBagItemAmount",
        lua.create_function(|lua, _: MultiValue| {
            Ok(ui(lua)
                .drag
                .held
                .as_ref()
                .filter(|h| h.kind == "bag")
                .map(|h| h.count))
        })?,
    )?;
    Ok(())
}

/// The widget natives: `RegisterForDrag`.
pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "StartMoving",
        super::method(lua, |lua, id, _| {
            start_moving(lua, id);
            super::none()
        })?,
    )?;
    m.raw_set(
        "StopMovingOrSizing",
        super::method(lua, |lua, id, _| {
            //392651e0 -> 39278ab0: clear the manager capture, then notify the receiver,
            // even if it was not captured, did not move, or is hidden.
            release_moving(lua);
            call_handler(lua, id, "OnMovedPosition", vec![]);
            super::none()
        })?,
    )?;
    m.raw_set(
        "RegisterForDrag",
        super::method(lua, |lua, id, a| {
            // `RegisterForDrag("LeftButton")`; the toolkit only starts drags with the left button
            let button = a.s(1).unwrap_or_else(|| "LeftButton".into());
            if button == "LeftButton" {
                ui(lua).drag.registered.insert(id);
            }
            super::none()
        })?,
    )
}
