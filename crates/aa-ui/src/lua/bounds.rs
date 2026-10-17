//! `OnBoundChanged`: the handler the client calls when a widget's bound (offset or extent as laid
//! out) changed.
//!
//! The scripts use it to place their children once the widget has its real size, e.g. the skill
//! window's scroll windows (`contentWnd.OnBoundChanged` puts the labels in columns by the width the
//! window got), the raid frames and `ui_management`'s movable windows (`SaveBound`). The client
//! raises it from its layout pass; here [`update`] runs every tick (after the scripts changed the
//! tree): for each widget that has the handler it compares the resolved rectangle with the one it
//! saw last, and calls the handler when it differs. The first time a widget is seen counts as a
//! change (it had no bound before), and hidden widgets are laid out like shown ones, so a tab that
//! is not selected yet is already placed when it is shown. A handler that changes anchors runs
//! its dependants in the same tick (at most a few passes).

use std::collections::HashMap;

use mlua::Lua;

use super::{call_handler, ui};
use crate::tree::Id;

/// What the last pass saw.
#[derive(Default)]
struct Seen {
    /// Tree generation of the last scan.
    generation: u64,
    /// Content version of the last scan ([`crate::tree::Tree::content`]).
    content: u64,
    /// Rectangle (x, y, w, h) per widget with the handler.
    rects: HashMap<Id, [f32; 4]>,
}

/// Most passes per tick (a handler can move another widget that has the handler too).
const MAX_PASSES: usize = 4;

/// Call `OnBoundChanged` on the widgets whose rectangle changed since the last tick.
pub(crate) fn update(lua: &Lua) {
    if lua.app_data_ref::<Seen>().is_none() {
        lua.set_app_data(Seen::default());
    }
    {
        // Only writes of the same values since the last scan (the HUD's per-frame updates): no
        // bound moved.
        let u = ui(lua);
        let seen = lua.app_data_ref::<Seen>().expect("aa-ui: bounds state");
        if seen.generation != 0 && seen.content == u.tree.content() && !u.tree.changed() {
            return;
        }
    }
    for _ in 0..MAX_PASSES {
        let changed: Vec<Id> = {
            let mut u = ui(lua);
            let mut seen = lua.app_data_mut::<Seen>().expect("aa-ui: bounds state");
            if seen.generation == u.tree.generation {
                return;
            }
            u.tree.checkpoint();
            seen.generation = u.tree.generation;
            seen.content = u.tree.content();
            let mut ids: Vec<Id> = u
                .handlers
                .iter()
                .filter(|(_, hs)| hs.iter().any(|h| h == "OnBoundChanged"))
                .map(|(id, _)| *id)
                .filter(|id| u.tree.objects.get(*id as usize).is_some_and(|o| o.alive))
                .collect();
            ids.sort_unstable();
            let mut changed = Vec::new();
            for id in ids {
                let r = u.tree.rect(id);
                let key = [r.x, r.y, r.w, r.h];
                if seen.rects.insert(id, key) != Some(key) {
                    changed.push(id);
                }
            }
            changed
        };
        if changed.is_empty() {
            return;
        }
        for id in changed {
            call_handler(lua, id, "OnBoundChanged", vec![]);
        }
    }
}
