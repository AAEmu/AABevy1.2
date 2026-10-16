-- reference/tools/lib.lua
-- Loaded into the UI Lua state of the real 1.2 client (aaemu-clientdriver `lua -t ui -`).
-- Defines the global table __ref with: a JSON encoder, a widget enumerator, class (metatable)
-- grouping, a call recorder (hooks) and the per-screen snapshot used for ui.json.
-- Nothing here changes game behaviour; the recorder only wraps method functions.

__ref = __ref or {}
local R = __ref
R.version = 4

------------------------------------------------------------------------------------------------
-- JSON (compact; keys sorted; arrays = 1..n sequences). Strings are bytes; UTF-8 passes through.
------------------------------------------------------------------------------------------------
local fmt, gsub, concat, sort = string.format, string.gsub, table.concat, table.sort
local function jstr(s)
  return '"' .. gsub(s, '[%z\1-\31\\"\127]', function(c)
    if c == '"' then return '\\"' elseif c == '\\' then return '\\\\' end
    return fmt('\\u%04x', string.byte(c))
  end) .. '"'
end
local function jnum(v)
  if v ~= v or v == math.huge or v == -math.huge then return 'null' end
  if v == math.floor(v) and v > -1e15 and v < 1e15 then return fmt('%d', v) end
  return fmt('%.9g', v)
end
local function isarray(t)
  local n = 0
  for k in pairs(t) do
    if type(k) ~= 'number' or k < 1 or k ~= math.floor(k) then return false end
    n = n + 1
  end
  for i = 1, n do if t[i] == nil then return false end end
  return true, n
end
local function enc(v, depth, out)
  local t = type(v)
  if t == 'nil' then out[#out + 1] = 'null'
  elseif t == 'boolean' then out[#out + 1] = tostring(v)
  elseif t == 'number' then out[#out + 1] = jnum(v)
  elseif t == 'string' then out[#out + 1] = jstr(v)
  elseif t == 'table' then
    if depth > 12 then out[#out + 1] = '"<depth>"' return end
    local arr, n = isarray(v)
    if arr and n > 0 then
      out[#out + 1] = '['
      for i = 1, n do if i > 1 then out[#out + 1] = ',' end enc(v[i], depth + 1, out) end
      out[#out + 1] = ']'
    elseif arr then out[#out + 1] = '[]'
    else
      local keys = {}
      for k in pairs(v) do keys[#keys + 1] = k end
      sort(keys, function(a, b) return tostring(a) < tostring(b) end)
      out[#out + 1] = '{'
      for i, k in ipairs(keys) do
        if i > 1 then out[#out + 1] = ',' end
        out[#out + 1] = jstr(tostring(k)) out[#out + 1] = ':'
        enc(v[k], depth + 1, out)
      end
      out[#out + 1] = '}'
    end
  else out[#out + 1] = jstr('<' .. t .. '>') end
end
function R.json(v) local out = {} enc(v, 0, out) return concat(out) end

------------------------------------------------------------------------------------------------
-- Widgets: a widget / drawable is a Lua table with a userdata field `__this`; all objects of one
-- kind share one metatable whose __index is the method table ("class").
------------------------------------------------------------------------------------------------
local function addr(w)
  local u = rawget(w, '__this')
  if u == nil then return nil end
  return (string.gsub(tostring(u), '^userdata: ', ''))
end
R.addr = addr
local function isw(t) return type(t) == 'table' and type(rawget(t, '__this')) == 'userdata' end
R.isw = isw
local function methods(w)
  local m = getmetatable(w)
  return m and type(m.__index) == 'table' and m.__index or nil
end

-- Class naming. Widget classes are named by creating one probe per candidate type name.
R.widget_type_candidates = {
  'emptywidget', 'window', 'button', 'label', 'textbox', 'editbox', 'editboxmultiline', 'listbox',
  'listctrl', 'slot', 'statusbar', 'slider', 'scroll', 'tab', 'checkbutton', 'radiobutton',
  'folder', 'grid', 'gametooltip', 'webbrowser', 'chatwindow', 'cooldownactionbutton',
  'cooldowninventorybutton', 'cooldownconstantbutton', 'circlediagram', 'modelview', 'message',
  'damagedisplay', 'megaphonechatedit', 'roadmap', 'minimap', 'colorpickbutton',
  'pagecontrol', 'cooldownbutton', 'itemlistctrl', 'combobox', 'dropdown', 'scrollbar',
  'tabwindow', 'imagebutton', 'progressbar', 'pallet', 'listview', 'tree', 'canvas', 'cursor',
  'baselist', 'hotkey', 'tooltip', 'icon', 'image', 'text', 'frame', 'panel', 'dialog',
}
R.drawable_makers = {
  { 'image', 'CreateImageDrawable', function() return LOGIN_STAGE_TEXTURE_PATH and LOGIN_STAGE_TEXTURE_PATH.IMG_TEXT or 'ui/login_stage/img_text.dds', 'background' end },
  { 'color', 'CreateColorDrawable', function() return 0, 0, 0, 0.5, 'background' end },
  { 'text', 'CreateTextDrawable', function() return FONT_PATH.DEFAULT, FONT_SIZE.LARGE, 'background' end },
  { 'ninepart', 'CreateNinePartDrawable', function() return 'ui/login_stage/img_text.dds', 'background' end },
  { 'threepart', 'CreateThreePartDrawable', function() return 'ui/login_stage/img_text.dds', 'background' end },
  { 'iconimage', 'CreateIconImageDrawable', function() return 'ui/login_stage/img_text.dds', 'background' end },
  { 'effect', 'CreateEffectDrawable', function() return 'ui/login_stage/img_text.dds', 'background' end },
}

local function sorted_methods(m)
  local n = {}
  for k, v in pairs(m) do if k ~= '__index' then n[#n + 1] = tostring(k) end end
  sort(n)
  return n
end

-- name the classes by probing; returns {className = {methods}} and fills R.class_of (index table -> name)
function R.probe_classes()
  R.class_of = R.class_of or setmetatable({}, { __mode = 'k' })
  local out = { widgets = {}, drawables = {}, widget_failed = {} }
  local first
  for _, ty in ipairs(R.widget_type_candidates) do
    local ok, w = pcall(function() return UIParent:CreateWidget(ty, '__ref_probe_' .. ty, 'UIParent') end)
    if ok and isw(w) then
      pcall(w.Show, w, false)
      local m = methods(w)
      local nm = R.class_of[m]
      if nm and nm ~= ty then out.widgets[ty] = { same_as = nm }
      else
        R.class_of[m] = ty
        out.widgets[ty] = sorted_methods(m)
        R.probe_widgets = R.probe_widgets or {}
        R.probe_widgets[ty] = w
        first = first or w
      end
    else out.widget_failed[#out.widget_failed + 1] = ty end
  end
  local host = R.probe_widgets and R.probe_widgets.emptywidget or first
  if host then
    for _, d in ipairs(R.drawable_makers) do
      local ok, dr = pcall(function() return host[d[2]](host, d[3]()) end)
      if ok and isw(dr) then
        local m = methods(dr)
        local nm = R.class_of[m]
        if nm and nm ~= 'drawable:' .. d[1] then out.drawables[d[1]] = { same_as = nm }
        else
          R.class_of[m] = 'drawable:' .. d[1]
          out.drawables[d[1]] = sorted_methods(m)
        end
      else out.drawables[d[1]] = { failed = tostring(dr) } end
    end
  end
  -- the text style object that labels / buttons / textboxes expose as `.style`
  for ty, w in pairs(R.probe_widgets or {}) do
    local st = rawget(w, 'style')
    if isw(st) then
      local m = methods(st)
      if m and not R.class_of[m] then
        R.class_of[m] = 'textstyle'
        out.textstyle = sorted_methods(m)
      end
    end
  end
  return out
end

------------------------------------------------------------------------------------------------
-- Enumeration: every widget/drawable that exists (registry) plus the first Lua path that reaches it
------------------------------------------------------------------------------------------------
local SKIP = { _G = true, package = true, string = true, table = true, math = true, os = true,
               io = true, debug = true, coroutine = true, __ref = true, _LOADED = true }

function R.enumerate(roots, maxdepth, use_registry)
  maxdepth = maxdepth or 10
  local byaddr, order = {}, {}
  local function reg(w, path)
    local a = addr(w)
    local rec = byaddr[a]
    if not rec then
      rec = { addr = a, obj = w, paths = {} }
      byaddr[a] = rec
      order[#order + 1] = rec
    end
    if path and #rec.paths < 4 then rec.paths[#rec.paths + 1] = path end
    return rec
  end
  -- breadth-first over global tables, and over the upvalues of Lua closures stored in them (scripts keep
  -- many widgets in locals that only handlers see), so the recorded path is the shortest one.
  -- Only objects the scripts still reference are touched: the registry also holds widgets that were
  -- destroyed natively, and calling a getter on one can fault the client.
  local seen = {}
  local queue, qi = {}, 1
  local function visit(v, kp, depth)
    if isw(v) then reg(v, kp) end
    if seen[v] then return end
    seen[v] = true
    queue[#queue + 1] = { v, kp, depth }
  end
  roots = roots or _G
  for k, v in pairs(roots) do
    if type(k) == 'string' and not SKIP[k] and (type(v) == 'table' or type(v) == 'function') then visit(v, k, 1) end
  end
  while qi <= #queue do
    local item = queue[qi] qi = qi + 1
    local t, path, depth = item[1], item[2], item[3]
    if depth < maxdepth then
      if type(t) == 'function' then
        local info = debug.getinfo(t, 'S')
        if info and info.what == 'Lua' then
          local i = 1
          while true do
            local n, v = debug.getupvalue(t, i)
            if not n then break end
            if type(v) == 'table' and not SKIP[n] and v ~= roots then visit(v, path .. '^' .. n, depth + 1) end
            i = i + 1
          end
        end
      else
        for k, v in pairs(t) do
          local tv = type(v)
          if (tv == 'table' or tv == 'function') and k ~= '__this' and k ~= '__index' then
            local kp
            if type(k) == 'number' then kp = path .. '[' .. k .. ']'
            elseif type(k) == 'string' then kp = path .. '.' .. k
            else kp = path .. '.<' .. type(k) .. '>' end
            visit(v, kp, depth + 1)
          end
        end
      end
    end
  end
  if use_registry then
    local r = debug.getregistry()
    for k, v in pairs(r) do
      if type(k) == 'number' and isw(v) then reg(v, nil) end
    end
  end
  return order, byaddr
end

------------------------------------------------------------------------------------------------
-- Per object state via the getters the API exposes
------------------------------------------------------------------------------------------------
local function pack(...) return { n = select('#', ...), ... } end
local function call(w, name, ...)
  local m = methods(w)
  if not m or type(m[name]) ~= 'function' then return nil, false end
  local ok, a, b, c, d = pcall(m[name], w, ...)
  if not ok then return nil, false end
  return { a, b, c, d }, true
end
local function v1(w, name) local r, ok = call(w, name) if ok and r then return r[1] end end
local function v2(w, name)
  local r, ok = call(w, name)
  if ok and r and (r[1] ~= nil or r[2] ~= nil) then return { r[1], r[2] } end
end
local function v4(w, name)
  local r, ok = call(w, name)
  if ok and r and r[1] ~= nil then return { r[1], r[2], r[3], r[4] } end
end

local function kind_of(w)
  local m = methods(w)
  if not m then return 'unknown' end
  if m.CreateChildWidget ~= nil then return 'widget' end
  if m.SetFont ~= nil then return 'style' end
  return 'drawable'
end
R.kind_of = kind_of
local function class_name(w)
  local m = methods(w)
  return m and R.class_of and R.class_of[m] or nil
end

function R.state(w)
  local s = {}
  local kind = kind_of(w)
  local dr = kind ~= 'widget'
  s.kind = kind
  s.class = class_name(w)
  local m = methods(w)
  -- method-set fingerprint so unnamed classes can still be told apart / matched to widget_methods.json
  local cnt = 0
  for _ in pairs(m or {}) do cnt = cnt + 1 end
  s.method_count = cnt
  s.id = v1(w, 'GetId')
  s.visible = v1(w, 'IsVisible')
  s.enabled = v1(w, 'IsEnabled')
  s.alpha = v1(w, 'GetAlpha')
  s.offset = v2(w, 'GetOffset')
  s.extent = v2(w, 'GetExtent')
  s.eoffset = v2(w, 'GetEffectiveOffset')
  s.eextent = v2(w, 'GetEffectiveExtent')
  s.content_extent = v2(w, 'GetContentExtent')
  s.rotation = v1(w, 'GetRotation')
  if kind == 'widget' then
    s.line_count = v1(w, 'GetLineCount')
    s.longest_line_width = v1(w, 'GetLongestLineWidth')
    s.text_height = v1(w, 'GetTextHeight')
    s.text_length = v1(w, 'GetTextLength')
    s.inset = v4(w, 'GetInset')
  elseif kind == 'style' then
    s.line_height = v1(w, 'GetLineHeight')
  end
  if kind ~= 'widget' then s.id = nil end
  s.text = v1(w, 'GetText')
  if not dr then
    local p = v1(w, 'GetParent')
    if type(p) == 'table' then
      local pa = addr(p)
      -- top level windows report themselves as their parent
      if pa == addr(w) then s.parent = 'UIParent' else s.parent = pa or (p == UIParent and 'UIParent' or '?') end
    end
    s.mouse_over = v1(w, 'IsMouseOver')
    s.button_state = v1(w, 'GetButtonState')
    -- button skins are drawables
    for _, g in ipairs { 'GetNormalBackground', 'GetHighlightBackground', 'GetPushedBackground', 'GetDisabledBackground' } do
      local b = v1(w, g)
      if type(b) == 'table' and addr(b) then s[g:gsub('^Get', ''):lower()] = addr(b) end
    end
    for _, g in ipairs { 'GetNormalColor', 'GetHighlightColor', 'GetPushedColor', 'GetDisabledColor' } do
      local c = v4(w, g)
      if c then s[g:gsub('^Get', ''):lower()] = c end
    end
  end
  for k, v in pairs(s) do if v == nil then s[k] = nil end end
  return s
end


------------------------------------------------------------------------------------------------
-- Call recorder. The widget API has no getters for textures, texture coords, colours, fonts,
-- anchors, layers or the widget name, so the only way to learn them is to watch the calls the
-- original scripts make. install_hooks() wraps the method tables of every widget / drawable /
-- text-style class (they are shared per class) plus UIParent.CreateWidget. Install it before the
-- screen under test is built.
------------------------------------------------------------------------------------------------
R.log = R.log or {}
R.seq = R.seq or 0
R.last = R.last or {}
R.hooked = R.hooked or {}
local MAXLOG = 600000

local function ser(a, depth)
  local t = type(a)
  if t == 'nil' then return '<nil>'
  elseif t == 'number' or t == 'boolean' or t == 'string' then return a
  elseif t == 'function' then return '<fn>'
  elseif t == 'userdata' then return '<ud>'
  elseif t == 'table' then
    if isw(a) then return 'w:' .. addr(a) end
    if a == UIParent then return 'UIParent' end
    if (depth or 0) >= 2 then return '<table>' end
    local o, n = {}, 0
    for k, v in pairs(a) do
      n = n + 1
      if n > 12 then o['...'] = true break end
      o[tostring(k)] = ser(v, (depth or 0) + 1)
    end
    return o
  end
  return '<' .. t .. '>'
end

local function argstr(...)
  local n = select('#', ...)
  local o = {}
  for i = 1, n do o[i] = ser((select(i, ...))) end
  o.n = nil
  return o, n
end

local getter = { Get = true, Is = true, Has = true, Find = true, Check = true }
local function is_getter(name)
  return getter[string.match(name, '^%u%l+')] and true or false
end

local function record(rec)
  R.seq = R.seq + 1
  rec.s = R.seq
  local n = #R.log
  if n < MAXLOG then R.log[n + 1] = rec end
end

local function wrap_method(m, name, orig, clsname)
  if is_getter(name) or name == '__index' then return end
  local creator = string.sub(name, 1, 6) == 'Create'
  m[name] = function(self, ...)
    local ok, a = pcall(addr, self)
    local args, n = argstr(...)
    if creator then
      local r = orig(self, ...)
      local rec = { a = ok and a or nil, m = name, args = args }
      if isw(r) then
        rec.r = addr(r)
        local rm = methods(r)
        if rm then
          R.class_of = R.class_of or setmetatable({}, { __mode = 'k' })
          if not R.class_of[rm] then
            local base = string.gsub(name, '^Create', '')
            base = string.gsub(base, 'Drawable$', '')
            if name == 'CreateChildWidget' or name == 'CreateWidget' or name == 'CreateChildWidgetByType' then
              base = type((...)) == 'string' and (...) or nil
              if base then R.class_of[rm] = base end
            else R.class_of[rm] = 'drawable:' .. string.lower(base) end
          end
          rec.rc = R.class_of[rm]
        end
      end
      record(rec)
      return r
    end
    local key = (ok and a or '?') .. name
    local sig = R.json(args)
    if R.last[key] ~= sig then
      R.last[key] = sig
      record({ a = ok and a or nil, m = name, args = args })
    end
    return orig(self, ...)
  end
end

local function hook_table(m, clsname)
  if R.hooked[m] then return 0 end
  R.hooked[m] = clsname or true
  local names = {}
  for k, v in pairs(m) do if type(v) == 'function' then names[#names + 1] = k end end
  for _, k in ipairs(names) do wrap_method(m, k, m[k], clsname) end
  return #names
end

function R.install_hooks()
  local info = R.probe_classes()
  local n = 0
  for ty, w in pairs(R.probe_widgets or {}) do n = n + hook_table(methods(w), ty) end
  -- drawable + style classes: recreate (probe_classes keeps no handles) through a probe host
  local host = R.probe_widgets and R.probe_widgets.emptywidget
  if host then
    for _, d in ipairs(R.drawable_makers) do
      local ok, dr = pcall(function() return host[d[2]](host, d[3]()) end)
      if ok and isw(dr) then n = n + hook_table(methods(dr), 'drawable:' .. d[1]) end
    end
  end
  for ty, w in pairs(R.probe_widgets or {}) do
    local st = rawget(w, 'style')
    if isw(st) then n = n + hook_table(methods(st), 'textstyle') end
  end
  -- creation of top level widgets goes through UIParent:CreateWidget(type, name, parent, ...)
  for _, tbl in ipairs { UIParent, UI } do
    if type(tbl) == 'table' and not R.hooked[tbl] and type(tbl.CreateWidget) == 'function' then
      R.hooked[tbl] = true
      local orig = tbl.CreateWidget
      tbl.CreateWidget = function(self, ...)
        local args = argstr(...)
        local r = orig(self, ...)
        local rec = { a = 'UIParent', m = 'CreateWidget', args = args }
        if isw(r) then
          rec.r = addr(r)
          local rm = methods(r)
          local ty = (...)
          if rm and type(ty) == 'string' and R.class_of then R.class_of[rm] = R.class_of[rm] or ty end
          rec.rc = rm and R.class_of and R.class_of[rm]
        end
        record(rec)
        return r
      end
      n = n + 1
    end
  end
  R.hooks_installed_at = os.clock()
  return n
end

function R.log_summary()
  local c = {}
  for _, r in ipairs(R.log) do c[r.m] = (c[r.m] or 0) + 1 end
  return { entries = #R.log, seq = R.seq, by_method = c }
end

return 'lib ok v' .. R.version
