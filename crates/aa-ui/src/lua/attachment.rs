//! Native widget attachment: a lifetime link, separate from parentage and anchors.

use mlua::{Lua, Table};

use super::{flush, method, new_object_table, none, ret, ui, value_id, widget};
use crate::tree::Id;

pub(super) fn methods(lua: &Lua, methods: &Table) -> mlua::Result<()> {
    methods.raw_set(
        "AttachWidget",
        method(lua, |lua, owner, args| {
            let Some(target) = value_id(lua, &args.v(1)).filter(|&id| {
                let u = ui(lua);
                u.tree.get(id).alive && u.tree.widget(id).is_some()
            }) else {
                return none();
            };
            let old = ui(lua).attachments.get(&owner).copied();
            if old == Some(target) {
                return none();
            }
            // Native 3927fd00 hides the old shown widget before assigning the new +0xc4 link.
            // Its OnHide may call owner:DetachWidget(), so dispatch that before replacement.
            if let Some(old) = old.filter(|&id| {
                let u = ui(lua);
                u.tree.get(id).alive && u.tree.get(id).shown
            }) {
                widget::show(lua, old, false, None);
                flush(lua);
            }
            ui(lua).attachments.insert(owner, target);
            none()
        })?,
    )?;
    methods.raw_set(
        "DetachWidget",
        method(lua, |lua, owner, _| {
            // Native 392774a0 clears the link; it does not hide the attached widget.
            ui(lua).attachments.remove(&owner);
            none()
        })?,
    )?;
    methods.raw_set(
        "GetAttachedWidget",
        method(lua, |lua, owner, _| {
            let target = {
                let u = ui(lua);
                u.attachments
                    .get(&owner)
                    .copied()
                    .filter(|&id| u.tree.get(id).alive)
            };
            match target {
                Some(id) => ret(lua, new_object_table(lua, id)?),
                None => none(),
            }
        })?,
    )?;
    Ok(())
}

/// Native OnHide 3927ff20 dispatches the owner's handler, then 3927f9b0 hides/clears
/// its attachment and repeats through ordinary descendants, even when they are not shown.
pub(super) fn hidden(lua: &Lua, owner: Id) {
    if ui(lua).attachments.is_empty() {
        return;
    }
    let target = ui(lua).attachments.get(&owner).copied();
    if let Some(target) = target {
        let shown = {
            let u = ui(lua);
            u.tree.get(target).alive && u.tree.get(target).shown
        };
        if shown {
            widget::show(lua, target, false, None);
            flush(lua);
        }
        ui(lua).attachments.remove(&owner);
    }
    let children = ui(lua)
        .tree
        .widget(owner)
        .map(|w| w.children.clone())
        .unwrap_or_default();
    for child in children {
        hidden(lua, child);
    }
}
