//! `tab` widgets: `AddSimpleTab(text)` creates `tab.window[i]`, `tab.selectedButton[i]` and
//! `tab.unselectedButton[i]`; one tab is selected (its window and selected button show, the
//! others show their unselected button). Buttons line up from the tab's corner (`SetCorner`,
//! `SetVertical`, `SetGap`, `SetOffset`); windows fill the rest of the tab. Checked against the
//! appearance tab of `reference/character_create/step2_*`.
//!
//! `AddNewTab(text, selectedButton, unselectedButton, window)` is the variant of the bag, bank and
//! coffer: the script made the two buttons and the content window itself (many tabs may share one
//! window) and the tab only lays them out and switches them (`TabData::borrowed`: removing such a
//! tab forgets the widgets instead of destroying them). The window is anchored below the button
//! row, where the script's divider drawables expect it (`AddAnchor("TOPLEFT", tabWindow, x, -24)`).

use mlua::{Lua, Table, Value};

use super::widget::{create_child, show};
use super::{call_handler, method, none, ret, ui, value_id};
use crate::tree::{Anchor, Id, Point};

#[derive(Clone, Debug, Default)]
pub struct TabData {
    /// (window, selected button, unselected button) per tab.
    pub tabs: Vec<(Id, Id, Id)>,
    /// Parallel to `tabs`: the widgets belong to the script (`AddNewTab` with its own buttons).
    pub borrowed: Vec<bool>,
    /// 1-based, 0 = none.
    pub selected: usize,
    pub vertical: bool,
    pub corner_right: bool,
    pub gap: f32,
    pub offset: f32,
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut TabData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    let td = w.tab.get_or_insert_with(TabData::default);
    Some(f(td))
}

/// Show the selected tab's window and button, the other tabs' unselected buttons, tab by tab:
/// a window shared by several tabs (`AddNewTab`) ends up as the last of them left it, so the
/// bag's `commonInvenTab.tabWindow` (one window for the "All" tab and the four custom tabs, the
/// first selected) is hidden, as in `reference/ingame/bag`; only its anchors matter.
fn apply_selection(lua: &Lua, id: Id) {
    let Some(td) = with(lua, id, |t| t.clone()) else { return };
    for (i, &(win, sel, unsel)) in td.tabs.iter().enumerate() {
        let on = i + 1 == td.selected;
        show(lua, win, on, None);
        show(lua, sel, on, None);
        show(lua, unsel, !on, None);
    }
}

pub(crate) fn select(lua: &Lua, id: Id, index: usize, fire: bool) {
    // Out of range (`SelectTab(0)`, `SelectTab(X2Bag:TabCount())` past the tabs) keeps the
    // selection: the inventory's `OnTabChanged` indexes per-tab tables by the new index.
    let changed = with(lua, id, |t| {
        if index < 1 || index > t.tabs.len() {
            return false;
        }
        let changed = index != t.selected;
        t.selected = index;
        changed
    })
    .unwrap_or(false);
    apply_selection(lua, id);
    if fire && changed {
        call_handler(lua, id, "OnTabChanged", vec![Value::Integer(index as i64)]);
    }
}

/// Native click on a tab button: an unselected button selects its tab. Returns true if `id`
/// was a tab button.
pub(crate) fn button_clicked(lua: &Lua, id: Id) -> bool {
    let found = {
        let u = ui(lua);
        let Some(parent) = u.tree.get(id).parent else { return false };
        u.tree
            .widget(parent)
            .and_then(|w| w.tab.as_ref())
            .and_then(|t| t.tabs.iter().position(|&(_, s, un)| s == id || un == id))
            .map(|i| (parent, i + 1))
    };
    let Some((tab, index)) = found else { return false };
    select(lua, tab, index, true);
    true
}

/// Place buttons and windows from the current button sizes. Anchors are only rewritten when
/// they change, so a static screen does not rebuild.
pub(crate) fn relayout_all(lua: &Lua) {
    let tabs: Vec<(Id, TabData)> = {
        let u = ui(lua);
        u.tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.alive)
            .filter_map(|(i, o)| o.widget().and_then(|w| w.tab.clone()).map(|t| (i as Id, t)))
            .collect()
    };
    for (id, td) in tabs {
        if td.tabs.is_empty() {
            continue;
        }
        let mut u = ui(lua);
        let sizes: Vec<(f32, f32)> = td.tabs.iter().map(|&(_, s, _)| (u.tree.get(s).width, u.tree.get(s).height)).collect();
        let max_w = sizes.iter().map(|s| s.0).fold(0.0, f32::max);
        let max_h = sizes.iter().map(|s| s.1).fold(0.0, f32::max);
        let corner = if td.corner_right { Point::TopRight } else { Point::TopLeft };
        let sign = if td.corner_right { -1.0 } else { 1.0 };
        let mut along = td.offset;
        for (i, &(win, sel, unsel)) in td.tabs.iter().enumerate() {
            let (bx, by) = if td.vertical { (0.0, along) } else { (sign * along, 0.0) };
            let a = vec![Anchor { point: corner, target: id, rel: corner, x: bx, y: by }];
            for b in [sel, unsel] {
                if u.tree.get(b).anchors != a {
                    u.tree.get_mut(b).anchors = a.clone();
                }
            }
            along += if td.vertical { sizes[i].1 } else { sizes[i].0 } + td.gap;
            // The window fills the tab beside / below the buttons.
            let (l, t, r) = match (td.vertical, td.corner_right) {
                (true, true) => (0.0, 0.0, -max_w),
                (true, false) => (max_w, 0.0, 0.0),
                (false, _) => (0.0, max_h, 0.0),
            };
            let wa = vec![
                Anchor { point: Point::TopLeft, target: id, rel: Point::TopLeft, x: l, y: t },
                Anchor { point: Point::BottomRight, target: id, rel: Point::BottomRight, x: r, y: 0.0 },
            ];
            if u.tree.get(win).anchors != wa {
                u.tree.get_mut(win).anchors = wa;
            }
        }
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    for name in ["AddSimpleTab", "AddNewTab"] {
        def(
            name,
            method(lua, |lua, id, a| {
                let text = a.s(1).unwrap_or_default();
                // `AddNewTab(text, selectedButton, unselectedButton, window)`: the script's own widgets.
                if let (Some(sel), Some(unsel), Some(win)) =
                    (value_id(lua, &a.v(2)), value_id(lua, &a.v(3)), value_id(lua, &a.v(4)))
                {
                    {
                        let mut u = ui(lua);
                        for b in [sel, unsel] {
                            if let Some(t) = u.tree.get_mut(b).text_mut() {
                                t.text = text.clone();
                            }
                        }
                    }
                    with(lua, id, |t| {
                        t.tabs.push((win, sel, unsel));
                        t.borrowed.resize(t.tabs.len() - 1, false);
                        t.borrowed.push(true);
                        if t.selected == 0 {
                            t.selected = 1;
                        }
                    });
                    apply_selection(lua, id);
                    relayout_all(lua);
                    return ret(lua, a.v(4));
                }
                let i = with(lua, id, |t| t.tabs.len() + 1).unwrap_or(1) as i64;
                let win = create_child(lua, Some(id), "window", "window", i, true)?;
                let sel = create_child(lua, Some(id), "button", "selectedButton", i, true)?;
                let unsel = create_child(lua, Some(id), "button", "unselectedButton", i, true)?;
                let ids = [&win, &sel, &unsel].map(|t| value_id(lua, &Value::Table((*t).clone())).unwrap());
                {
                    let mut u = ui(lua);
                    for b in [ids[1], ids[2]] {
                        if let Some(t) = u.tree.get_mut(b).text_mut() {
                            t.text = text.clone();
                        }
                    }
                }
                let first = with(lua, id, |t| {
                    t.tabs.push((ids[0], ids[1], ids[2]));
                    t.selected == 0
                })
                .unwrap_or(false);
                if first {
                    with(lua, id, |t| t.selected = 1);
                }
                apply_selection(lua, id);
                relayout_all(lua);
                ret(lua, win)
            })?,
        )?;
    }
    def(
        "SelectTab",
        method(lua, |lua, id, a| {
            let i = a.f(1).max(0.0) as usize;
            select(lua, id, i, true);
            none()
        })?,
    )?;
    def("GetSelectedTab", method(lua, |lua, id, _| ret(lua, with(lua, id, |t| t.selected as i64).unwrap_or(0)))?)?;
    def("GetActivateTabCount", method(lua, |lua, id, _| ret(lua, with(lua, id, |t| t.tabs.len() as i64).unwrap_or(0)))?)?;
    def(
        "SetVertical",
        method(lua, |lua, id, a| {
            let v = a.b(1, true);
            with(lua, id, |t| t.vertical = v);
            relayout_all(lua);
            none()
        })?,
    )?;
    def(
        "SetCorner",
        method(lua, |lua, id, a| {
            let right = a.s(1).is_some_and(|s| s.to_ascii_uppercase().ends_with("RIGHT"));
            with(lua, id, |t| t.corner_right = right);
            relayout_all(lua);
            none()
        })?,
    )?;
    def(
        "SetGap",
        method(lua, |lua, id, a| {
            let g = a.f(1);
            with(lua, id, |t| t.gap = g);
            relayout_all(lua);
            none()
        })?,
    )?;
    def(
        "SetOffset",
        method(lua, |lua, id, a| {
            let o = a.f(1);
            with(lua, id, |t| t.offset = o);
            relayout_all(lua);
            none()
        })?,
    )?;
    def(
        "RemoveAllTabs",
        method(lua, |lua, id, _| {
            let (tabs, borrowed) = with(lua, id, |t| {
                t.selected = 0;
                (std::mem::take(&mut t.tabs), std::mem::take(&mut t.borrowed))
            })
            .unwrap_or_default();
            {
                let mut u = ui(lua);
                for (i, (w, s, un)) in tabs.into_iter().enumerate() {
                    // The script's own widgets (`AddNewTab`) live on; it hides them itself.
                    if borrowed.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    for x in [w, s, un] {
                        u.tree.get_mut(x).alive = false;
                    }
                }
            }
            if let Some(this) = super::object_table(lua, id)? {
                for f in ["window", "selectedButton", "unselectedButton"] {
                    this.raw_set(f, lua.create_table()?)?;
                }
            }
            none()
        })?,
    )?;
    for name in ["SetActivateTabCount", "RemoveTab"] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}
