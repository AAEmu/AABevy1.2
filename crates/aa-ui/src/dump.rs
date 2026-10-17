//! Tree dump in the format of `reference/<screen>/ui.json` (`widgets[]` with `paths`, `class`,
//! `offset`, `extent`, `visible`), so layouts can be compared with the real client by path.

use std::collections::HashMap;
use std::fmt::Write;

use mlua::{Lua, Table};

use crate::lua::{reg, ui};
use crate::tree::{Id, Kind};

/// Paths of every object table reachable from `_G` (`a.b[2].c`), breadth first, at most
/// `max_depth` fields deep. Like the reference capture, the shortest paths come first. Pass 0
/// walks `_G`; pass 1 the tables held in global functions' upvalues (`Func^upvalue`, the
/// reference's notation), so plain global paths win. The walk runs in Lua: holding tens of
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
  local getup = debug and debug.getupvalue
  local upv = {}
  local function keyOf(path, k)
    local tk = type(k)
    if tk == "string" then
      if path == "" then return k end
      return path .. "." .. k
    elseif tk == "number" then
      if k == math.floor(k) then return string.format("%s[%d]", path, k) end
      return path .. "[" .. tostring(k) .. "]"
    end
    return nil
  end
  local function walk(queue, seen, scanUpvalues)
    for _ = 1, maxDepth do
      local nextQ = {}
      for qi = 1, #queue do
        local t, path = queue[qi][1], queue[qi][2]
        for k, v in next, t do
          if scanUpvalues and path == "" and type(v) == "function" and type(k) == "string" then
            if getup then
              for i = 1, 255 do
                local name, uv = getup(v, i)
                if name == nil then break end
                if type(uv) == "table" then upv[#upv + 1] = { uv, k .. "^" .. name } end
              end
            end
          elseif type(v) == "table" then
            local key = keyOf(path, k)
            if key and not (path == "" and (key == "_G" or key == "package" or key == "FAKE")) then
              local id = ids[v]
              if id then add(id, key) end
              if not seen[v] then
                seen[v] = true
                nextQ[#nextQ + 1] = { v, key }
              end
            end
          end
        end
      end
      queue = nextQ
    end
  end
  walk({ { _G, "" } }, {}, true)
  local seen, queue = {}, {}
  for i = 1, #upv do
    local ut, p = upv[i][1], upv[i][2]
    local id = ids[ut]
    if id then add(id, p) end
    if not seen[ut] then
      seen[ut] = true
      queue[#queue + 1] = { ut, p }
    end
  end
  walk(queue, seen, false)
  return out
end
"#;

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
