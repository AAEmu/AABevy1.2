//! Tree dump in the format of `reference/<screen>/ui.json` (`widgets[]` with `paths`, `class`,
//! `offset`, `extent`, `visible`), so layouts can be compared with the real client by path.

use std::collections::HashMap;
use std::fmt::Write;

use mlua::{Lua, Table, Value};

use crate::lua::{reg, ui};
use crate::tree::{Id, Kind};

/// Paths of every object table reachable from `_G` (`a.b[2].c`), breadth first, at most
/// `max_depth` fields deep. Like the reference capture, the shortest paths come first.
fn object_paths(lua: &Lua, max_depth: usize) -> mlua::Result<HashMap<Id, Vec<String>>> {
    let r = reg(lua);
    let mut out: HashMap<Id, Vec<String>> = HashMap::new();
    let mut seen: HashMap<usize, ()> = HashMap::new();
    let mut queue: Vec<(Table, String)> = vec![(lua.globals(), String::new())];
    for _depth in 0..max_depth {
        let mut next = Vec::new();
        for (t, path) in queue {
            for pair in t.pairs::<Value, Value>() {
                let Ok((k, v)) = pair else { continue };
                let Value::Table(child) = v else { continue };
                let key = match &k {
                    Value::String(s) => {
                        let s = s.to_string_lossy();
                        if path.is_empty() { s.to_string() } else { format!("{path}.{s}") }
                    }
                    Value::Integer(i) => format!("{path}[{i}]"),
                    Value::Number(n) => format!("{path}[{n}]"),
                    _ => continue,
                };
                if path.is_empty() && matches!(key.as_str(), "_G" | "package" | "FAKE") {
                    continue;
                }
                if let Some(id) = r.ids.raw_get::<Option<i64>>(&child)? {
                    let paths = out.entry(id as Id).or_default();
                    if paths.len() < 4 {
                        paths.push(key.clone());
                    }
                }
                let addr = child.to_pointer() as usize;
                if seen.insert(addr, ()).is_none() {
                    next.push((child, key));
                }
            }
        }
        queue = next;
    }
    Ok(out)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// JSON with `widgets` (visible objects that have a script path, in paint order) and
/// `hidden_named` (hidden ones), like the reference capture.
pub fn dump_json(lua: &Lua) -> mlua::Result<String> {
    let paths = object_paths(lua, 14)?;
    let g = ui(lua);
    let tree = &g.tree;
    let order = tree.paint_order();
    let pos: HashMap<Id, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut ids: Vec<Id> = paths.keys().copied().filter(|&id| tree.get(id).alive).collect();
    ids.sort_by_key(|id| (pos.get(id).copied().unwrap_or(usize::MAX), *id));
    let mut vis = String::new();
    let mut hid = String::new();
    for id in ids {
        let o = tree.get(id);
        let (class, kind) = match &o.kind {
            Kind::Widget(w) => (w.class.clone(), "widget"),
            Kind::Drawable(d) => (d.class.to_string(), "drawable"),
        };
        let r = tree.rect(id);
        let visible = tree.visible(id)
            && match (o.parent, &o.kind) {
                (Some(owner), Kind::Drawable(_)) => {
                    crate::render::state_visible(&g, id, crate::render::button_state(&g, owner, None))
                }
                _ => true,
            };
        let mut ps: Vec<String> = paths[&id].clone();
        ps.sort_by_key(|p| (p.len(), p.clone()));
        let ps: Vec<String> = ps.iter().map(|p| format!("\"{}\"", esc(p))).collect();
        let owner = o.parent.map(|p| p as i64).unwrap_or(-1);
        let line = format!(
            "{{\"id\":{id},\"class\":\"{class}\",\"kind\":\"{kind}\",\"owner\":{owner},\"paths\":[{}],\"offset\":[{},{}],\"extent\":[{},{}],\"shown\":{},\"visible\":{visible}}}",
            ps.join(","),
            r.x,
            r.y,
            r.w,
            r.h,
            o.shown
        );
        let dst = if visible { &mut vis } else { &mut hid };
        if !dst.is_empty() {
            dst.push_str(",\n");
        }
        dst.push_str(&line);
    }
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"meta\":{{\"screen_width\":{},\"screen_height\":{},\"ui_scale\":{}}},\n\"widgets\":[\n{vis}\n],\n\"hidden_named\":[\n{hid}\n]}}\n",
        tree.screen_w, tree.screen_h, g.ui_scale
    );
    Ok(out)
}
