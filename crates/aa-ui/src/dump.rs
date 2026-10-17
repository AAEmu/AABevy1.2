//! Tree dump in the format of `reference/<screen>/ui.json` (`widgets[]` with `paths`, `class`,
//! `offset`, `extent`, `visible`), so layouts can be compared with the real client by path.

use std::collections::HashMap;
use std::fmt::Write;

use mlua::{Lua, Table};

use crate::lua::{reg, ui};
use crate::tree::{Id, Kind};

/// Paths of every object table reachable from `_G` (`a.b[2].c`, `Func^upvalue.x`), breadth
/// first over tables and the upvalues of Lua functions, at most `max_depth` steps deep: the walk
/// of the reference capture (`reference/tools/lib.lua`, `R.enumerate`), so the recorded paths
/// are the same shortest ones (up to 4 per object). The walk runs in Lua: holding tens of
/// thousands of table handles from Rust overflows Lua 5.1's C stack (`out of Lua stack`).
fn object_paths(lua: &Lua, max_depth: usize) -> mlua::Result<HashMap<Id, Vec<String>>> {
    let r = reg(lua);
    let f: mlua::Function = lua.load(OBJECT_PATHS_LUA).set_name("@aa-ui/dump").eval()?;
    let found: Table = f.call((r.ids.clone(), max_depth))?;
    let mut out: HashMap<Id, Vec<String>> = HashMap::new();
    for pair in found.pairs::<i64, Table>() {
        let (id, paths) = pair?;
        let ps: Vec<String> = paths.sequence_values::<String>().filter_map(Result::ok).collect();
        out.insert(id as Id, ps);
    }
    Ok(out)
}

const OBJECT_PATHS_LUA: &str = r#"
return function(ids, maxDepth)
  local out = {}
  local function add(id, p)
    local t = out[id]
    if not t then t = {}; out[id] = t end
    if #t < 4 then t[#t + 1] = p end
  end
  local SKIP = { _G = true, package = true, string = true, table = true, math = true, os = true,
                 io = true, debug = true, coroutine = true, __ref = true, _LOADED = true, FAKE = true }
  local getup = debug and debug.getupvalue
  local getinfo = debug and debug.getinfo
  local seen, queue, qi = {}, {}, 1
  local function visit(v, kp, depth)
    local id = ids[v]
    if id then add(id, kp) end
    if seen[v] then return end
    seen[v] = true
    queue[#queue + 1] = { v, kp, depth }
  end
  for k, v in next, _G do
    local tv = type(v)
    if type(k) == "string" and not SKIP[k] and (tv == "table" or tv == "function") then visit(v, k, 1) end
  end
  while qi <= #queue do
    local item = queue[qi]
    qi = qi + 1
    local t, path, depth = item[1], item[2], item[3]
    if depth < maxDepth then
      if type(t) == "function" then
        local info = getinfo and getinfo(t, "S")
        if getup and info and info.what ~= "C" then
          local i = 1
          while true do
            local n, v = getup(t, i)
            if not n then break end
            if type(v) == "table" and not SKIP[n] and v ~= _G then visit(v, path .. "^" .. n, depth + 1) end
            i = i + 1
          end
        end
      else
        for k, v in next, t do
          local tv = type(v)
          if (tv == "table" or tv == "function") and k ~= "__this" and k ~= "__index" then
            local kp
            local tk = type(k)
            if tk == "number" then
              if k == math.floor(k) then kp = string.format("%s[%d]", path, k) else kp = path .. "[" .. tostring(k) .. "]" end
            elseif tk == "string" then kp = path .. "." .. k
            else kp = path .. ".<" .. tk .. ">" end
            visit(v, kp, depth + 1)
          end
        end
      end
    end
  end
  return out
end
"#;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// JSON with `widgets` (visible objects that have a script path, in paint order) and
/// `hidden_named` (hidden ones), like the reference capture.
pub fn dump_json(lua: &Lua) -> mlua::Result<String> {
    // The reference capture walks 10 steps deep (`R.enumerate(nil, 10)`).
    let paths = object_paths(lua, 10)?;
    let g = ui(lua);
    let tree = &g.tree;
    let order = tree.paint_order();
    let pos: HashMap<Id, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let mut ids: Vec<Id> = paths
        .keys()
        .copied()
        .filter(|&id| tree.get(id).alive)
        .collect();
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
                (_, Kind::Drawable(d)) if d.effect == Some(false) => false,
                (Some(owner), Kind::Drawable(_)) => crate::render::state_visible(
                    &g,
                    id,
                    crate::render::button_state(&g, owner, None),
                ),
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
