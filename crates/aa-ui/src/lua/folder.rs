//! `folder` widgets: a title (open-state / close-state buttons) and a child widget shown below
//! it while open. The folder's height is the title height, plus the extend length when open
//! (checked against the character select list in `reference/character_select`).

use mlua::{Lua, Table, Value};

use super::widget::show;
use super::{method, none, ret, ui, value_id};
use crate::tree::{Anchor, Id, Point};

#[derive(Clone, Debug, Default)]
pub struct FolderData {
    pub open: bool,
    pub title_height: f32,
    pub extend: f32,
    /// Shown while open (`SetOpenStateButton`) / closed (`SetCloseStateButton`).
    pub open_button: Option<Id>,
    pub close_button: Option<Id>,
    pub child: Option<Id>,
    pub configured: bool,
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut FolderData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    let fd = w.folder.get_or_insert_with(FolderData::default);
    Some(f(fd))
}

/// Apply the state: height, child placement and which title button shows.
fn apply(lua: &Lua, id: Id) {
    let Some(fd) = with(lua, id, |f| f.clone()) else {
        return;
    };
    if !fd.configured {
        return;
    }
    {
        let mut u = ui(lua);
        let o = u.tree.get_mut(id);
        o.height = fd.title_height + if fd.open { fd.extend } else { 0.0 };
        if let Some(c) = fd.child {
            let co = u.tree.get_mut(c);
            co.anchors = vec![
                Anchor {
                    point: Point::TopLeft,
                    target: id,
                    rel: Point::TopLeft,
                    x: 3.0,
                    y: fd.title_height,
                },
                Anchor {
                    point: Point::BottomRight,
                    target: id,
                    rel: Point::BottomRight,
                    x: 0.0,
                    y: 0.0,
                },
            ];
        }
    }
    if let Some(b) = fd.open_button {
        show(lua, b, fd.open, None);
    }
    if let Some(b) = fd.close_button {
        show(lua, b, !fd.open, None);
    }
    if let Some(c) = fd.child {
        show(lua, c, fd.open, None);
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetOpenStateButton",
        method(lua, |lua, id, a| {
            let b = value_id(lua, &a.v(1));
            with(lua, id, |f| {
                f.open_button = b;
                f.configured = true;
            });
            apply(lua, id);
            none()
        })?,
    )?;
    def(
        "SetCloseStateButton",
        method(lua, |lua, id, a| {
            let b = value_id(lua, &a.v(1));
            with(lua, id, |f| {
                f.close_button = b;
                f.configured = true;
            });
            apply(lua, id);
            none()
        })?,
    )?;
    def(
        "SetChildWidget",
        method(lua, |lua, id, a| {
            let c = value_id(lua, &a.v(1));
            with(lua, id, |f| {
                f.child = c;
                f.configured = true;
            });
            apply(lua, id);
            none()
        })?,
    )?;
    def(
        "SetTitleHeight",
        method(lua, |lua, id, a| {
            let h = a.f(1);
            with(lua, id, |f| {
                f.title_height = h;
                f.configured = true;
            });
            apply(lua, id);
            none()
        })?,
    )?;
    def(
        "SetExtendLength",
        method(lua, |lua, id, a| {
            let h = a.f(1);
            with(lua, id, |f| f.extend = h);
            apply(lua, id);
            none()
        })?,
    )?;
    for (name, state) in [
        ("OpenFolder", Some(true)),
        ("CloseFolder", Some(false)),
        ("FixedCloseFolder", Some(false)),
        ("ToggleState", None),
    ] {
        def(
            name,
            method(lua, move |lua, id, _| {
                with(lua, id, |f| {
                    f.open = state.unwrap_or(!f.open);
                    f.configured = true;
                });
                apply(lua, id);
                none()
            })?,
        )?;
    }
    def(
        "GetState",
        method(lua, |lua, id, _| {
            let open = with(lua, id, |f| f.open).unwrap_or(false);
            ret(lua, if open { "open" } else { "close" })
        })?,
    )?;
    for name in [
        "UseAnimation",
        "SetAnimateStep",
        "EnableAutoToggle",
        "SetTitleButtonWidget",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    let _ = Value::Nil;
    Ok(())
}
