//! Synchronous native 39276350 / virtual +d4 (event 32, OnChangedAnchor).

use mlua::{Lua, Value};

use super::{Args, call_handler, ui};
use crate::tree::Id;

/// Size bindings read their notification control at parameter 2, including SetExtent
/// (39261840 reads its height, not a third flag). Failed bool conversions default to true.
fn notify_size(args: &Args) -> bool {
    match args.v(2) {
        Value::Boolean(v) => v,
        Value::Integer(v) => v != 0,
        Value::Number(v) => v != 0.0,
        _ => true,
    }
}

pub(super) fn size_changed(lua: &Lua, id: Id, args: &Args) {
    let notify = notify_size(args);
    propagate(lua, id, notify);
    // Native 392763c0/39276410/39276460 call +d8 even if +d4's readiness
    // check skipped propagation. Reentry/retirement can change the live receiver.
    if notify {
        super::bounds::changed(lua, id);
    }
}

/// Native propagation is depth first over the live dependency vector, then the source.
/// It does not compare rectangles or filter visibility. Release the UI borrow before Lua:
/// callbacks may re-anchor, replace handlers, retire a widget or reenter this setter.
pub(super) fn changed(lua: &Lua, id: Id) {
    propagate(lua, id, true);
}

fn propagate(lua: &Lua, id: Id, notify: bool) {
    {
        let mut u = ui(lua);
        if !u.tree.get(id).alive || !u.tree.begin_anchor_update(id) {
            return;
        }
    }
    let mut index = 0;
    loop {
        let next = {
            let mut u = ui(lua);
            if !u.tree.get(id).alive {
                break;
            }
            let next = u.tree.anchor_dependent(id, index);
            if let Some(next) = next {
                u.tree.validate_anchor_target(next, id);
            }
            next
        };
        let Some(next) = next else { break };
        propagate(lua, next, notify);
        index += 1;
    }
    if notify {
        call_handler(lua, id, "OnChangedAnchor", vec![]);
    }
    ui(lua).tree.end_anchor_update();
}

/// Native 3927b2f0 recalculates each owned child's scale recursively, propagating
/// that child's anchors, before 3927d120 propagates the source and sends OnScale.
pub(super) fn scaled(lua: &Lua, id: Id) {
    ui(lua).tree.prepare_anchor_change(id);
    ui(lua).tree.anchor_scaling(id, true);
    scale_children(lua, id);
    changed(lua, id);
    ui(lua).tree.anchor_scaling(id, false);
}

fn scale_children(lua: &Lua, id: Id) {
    let mut index = 0;
    loop {
        let child = {
            let u = ui(lua);
            if !u.tree.get(id).alive {
                return;
            }
            u.tree
                .widget(id)
                .and_then(|w| w.children.get(index))
                .copied()
        };
        let Some(child) = child else { break };
        scale_children(lua, child);
        changed(lua, child);
        index += 1;
    }
}
