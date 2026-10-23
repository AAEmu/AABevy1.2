//! Synchronous native virtual +d8, event33 (OnBoundChanged).
//!
//! Size setters call this AFTER anchor propagation, including unchanged sizes.
//! Native 3927bd60 / 3927b5e0 visit the source then its live anchor dependents;
//! geometry comparisons, visibility, parenting and anchor readiness do not gate it.

use mlua::Lua;

use super::{call_handler, ui};
use crate::tree::Id;

pub(super) fn changed(lua: &Lua, id: Id) {
    {
        let u = ui(lua);
        // Native drawable +d8 (3928aab0) is a no-op, including its dependents.
        if !u.tree.get(id).alive || !u.tree.get(id).is_widget() {
            return;
        }
    }
    call_handler(lua, id, "OnBoundChanged", vec![]);
    let mut index = 0;
    loop {
        let next = {
            let u = ui(lua);
            if !u.tree.get(id).alive {
                return;
            }
            u.tree.anchor_dependent(id, index)
        };
        let Some(next) = next else { return };
        changed(lua, next);
        index += 1;
    }
}
