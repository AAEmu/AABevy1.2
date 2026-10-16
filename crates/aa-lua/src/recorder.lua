-- API recorder prelude (see recorder.rs). Every global the scripts read that no script defines,
-- and every member of such a value, resolves to a recording proxy that returns further proxies,
-- so the UI scripts run as far as possible while we log which native names they touch.
--
-- Inputs (set by Rust before this chunk runs):
--   __REC_SCRIPT_GLOBALS  set of global names some UI script assigns (those stay plain nil)
--   __REC_SCRIPT_FIELDS   set of field names scripts assign on tables (nil on widget proxies)
--   __REC_NATIVE_NAMES    set of function names the client registers natively (x2game.dll)
-- Output: __REC (members, handlers, events, callbacks, errors, hints), __REC_CTX (context).

local rawget, rawset, rawequal, type, select, tostring, pairs, ipairs, next, setmetatable, getmetatable, pcall, error, unpack =
  rawget, rawset, rawequal, type, select, tostring, pairs, ipairs, next, setmetatable, getmetatable, pcall, error, unpack
local sfind, ssub, slower, sformat, sgsub = string.find, string.sub, string.lower, string.format, string.gsub
local SCRIPT_GLOBALS = __REC_SCRIPT_GLOBALS or {}
local SCRIPT_FIELDS = __REC_SCRIPT_FIELDS or {}
local NATIVE_NAMES = __REC_NATIVE_NAMES or {}
-- Native non-function fields of widgets (the text style objects; see reference/api/widget_methods.json).
local NATIVE_FIELDS = { style = true, titleStyle = true, itemStyle = true, extraStyle = true, childStyle = true, prefixStyle = true, cooltime_style = true }
-- From the real client capture (reference/api): native constants/data tables, class method lists.
local VALUES = __REC_VALUES or {}
local CLASS_METHODS = __REC_CLASS_METHODS or {}

local REC = { members = {}, handlers = {}, events = {}, callbacks = {}, errors = {}, hints = {}, widgets = {} }
local CTX = { stage = "?", file = "?", phase = "load" }
__REC, __REC_CTX = REC, CTX

local INFO = setmetatable({}, { __mode = "k" }) -- proxy -> { class, name, parent, kids, widget, events }
local PMT = {}                                   -- metatable shared by every proxy
local NATIVE = {}                                -- global name -> proxy

local function is_proxy(v) return type(v) == "table" and INFO[v] ~= nil end
__REC_IS_PROXY = is_proxy

local function bump(t, k, n) t[k] = (t[k] or 0) + (n or 1) end

local function rec(class, member, kind)
  local key = class .. "\1" .. member
  local r = REC.members[key]
  if not r then
    r = { class = class, member = member, kind = kind, uses = 0, calls = 0, argc = {}, sigs = {}, strs = {}, stages = {}, files = {}, phases = {} }
    REC.members[key] = r
  end
  r.uses = r.uses + 1
  r.stages[CTX.stage] = true
  r.phases[CTX.phase] = true
  bump(r.files, CTX.file)
  return r
end

local function hint(class, what)
  local h = REC.hints[class]
  if not h then h = {}; REC.hints[class] = h end
  bump(h, what)
end

local function mkproxy(class, name, parent, widget)
  local p = setmetatable({}, PMT)
  INFO[p] = { class = class, name = name, parent = parent, kids = {}, widget = widget }
  return p
end

local function argtype(v)
  local t = type(v)
  if t == "table" then
    local i = INFO[v]
    if i then return i.widget and "widget" or "native" end
  end
  return t
end

-- Native behaviour the scripts depend on, and plausible return values so common control flow
-- keeps going. Called as f(self, args...) (self = nil for global functions).
local CONTENTS = {}
-- Native widgets that create child widgets in array fields of themselves.
local function push_child(parent, field, class)
  if not parent then return end
  local t = rawget(parent, field)
  if type(t) ~= "table" or INFO[t] then t = {}; rawset(parent, field, t) end
  local c = mkproxy(class, class, nil, true)
  t[#t + 1] = c
  return c
end
local function add_tab(self)
  push_child(self, "window", "tab.window")
  push_child(self, "selectedButton", "tab.selectedButton")
  push_child(self, "unselectedButton", "tab.unselectedButton")
end
local OVERRIDES = {
  ["X2Locale:GetLocale"] = function() return "en_us" end,
  -- content windows: GetContent returns what RegisterContentWidget stored (nil before)
  ["ADDON:RegisterContentWidget"] = function(_, id, w) if id ~= nil then CONTENTS[id] = w end end,
  ["ADDON:GetContent"] = function(_, id) if id == nil then return nil end return CONTENTS[id] end,
  ["X2Util:GetGameProvider"] = function() return NATIVE.TRION or _G.TRION end,
  ["UIParent:GetScreenWidth"] = function() return 1920 end,
  ["UIParent:GetScreenHeight"] = function() return 1080 end,
  ["UIParent:GetUIScale"] = function() return 1 end,
  -- tab:AddSimpleTab/AddTab create tab.window[i], tab.selectedButton[i], tab.unselectedButton[i]
  ["tab:AddSimpleTab"] = add_tab,
  ["tab:AddTab"] = add_tab,
  -- listctrl:InsertColumn(width, type) creates listctrl.column[i]
  ["listctrl:InsertColumn"] = function(self) push_child(self, "column", "listctrl.column") end,
  -- listctrl:InsertRows(count, ...) creates listctrl.items[i] with items[i].subItems[column]
  ["listctrl:InsertRows"] = function(self, count)
    if not self or type(count) ~= "number" then return end
    local columns = rawget(self, "column")
    local ncol = type(columns) == "table" and #columns or 0
    for _ = 1, math.min(count, 200) do
      local item = push_child(self, "items", "listctrl.item")
      for _ = 1, ncol do push_child(item, "subItems", "listctrl.subItem") end
    end
  end,
}
local RET = __REC_RET or {}
local RET_M = __REC_RET_M or {}
local NEUTRAL = { number = 0, string = "", boolean = false }
local function guess_return(owner, member, widget)
  -- what the scripts do with the result (static scan): numbers, strings, booleans
  local kinds = RET[owner .. ":" .. member] or (widget and RET_M[member]) or nil
  if kinds then
    local out, n = {}, 0
    for i = 1, 4 do
      local k = kinds[i]
      if k then out[i] = NEUTRAL[k]; n = i end
    end
    if n > 0 then
      for i = 1, n do if out[i] == nil then out[i] = mkproxy(owner .. ":" .. member .. "()", member, nil, false) end end
      return out
    end
  end
  if sfind(member, "^Is[A-Z_]") or sfind(member, "^Has[A-Z]") or sfind(member, "^Can[A-Z]") then return { false } end
  if sfind(member, "Extent$") or sfind(member, "Offset$") then return { 0, 0 } end
  if sfind(member, "Count$") or sfind(member, "Num$") or sfind(member, "^GetNum") or sfind(member, "Width$")
    or sfind(member, "Height$") or sfind(member, "Level$") or sfind(member, "Index$") or sfind(member, "Size$") then
    return { 0 }
  end
  return nil
end

local WIDGET_CREATORS = { CreateWidget = true, CreateChildWidget = true, CreateChildWidgetByType = true }

local function do_call(p, ...)
  local info = INFO[p]
  local parent = info.parent
  local owner = parent and INFO[parent].class or "_G"
  local member = info.name
  local n = select("#", ...)
  local first = 1
  if parent and n > 0 and rawequal((...), parent) then first = 2 end -- method call: drop self
  local r = rec(owner, member, "call")
  r.calls = r.calls + 1
  local argc = n - first + 1
  bump(r.argc, argc)
  local sig, s1 = {}, nil
  for i = first, n do
    local v = select(i, ...)
    sig[#sig + 1] = argtype(v)
    local pos = i - first + 1
    if type(v) == "string" and pos <= 4 then
      local s = r.strs[pos]
      if not s then s = {}; r.strs[pos] = s end
      if s[v] or next(s) == nil or #s < 40 then
        if not s[v] then s[#s + 1] = v end
        bump(s, v)
      end
      if pos == 1 then s1 = v end
    elseif type(v) == "function" then
      REC.callbacks[#REC.callbacks + 1] = { fn = v, owner = owner, member = member, name = s1, self = parent, stage = CTX.stage }
    end
  end
  bump(r.sigs, table.concat(sig, ","))
  -- side tables for the report
  if member == "SetHandler" and s1 then
    local h = REC.handlers[s1]
    if not h then h = {}; REC.handlers[s1] = h end
    bump(h, owner)
  elseif member == "RegisterEvent" and s1 then
    local e = REC.events[s1]
    if not e then e = { count = 0, stages = {}, files = {} }; REC.events[s1] = e end
    e.count = e.count + 1
    e.stages[CTX.stage] = true
    bump(e.files, CTX.file)
    if parent then
      local pi = INFO[parent]
      pi.events = pi.events or {}
      pi.events[#pi.events + 1] = s1
    end
  end
  -- return value
  if WIDGET_CREATORS[member] and first <= n and type((select(first, ...))) == "string" then
    local wtype = slower((select(first, ...)))
    local w = REC.widgets[wtype]
    if not w then w = { count = 0, by = {}, stages = {} }; REC.widgets[wtype] = w end
    w.count = w.count + 1
    bump(w.by, member)
    w.stages[CTX.stage] = true
    local child = mkproxy(wtype, wtype, nil, true)
    -- parent:CreateChildWidget(type, name, index, true) also stores the child in the parent:
    -- parent[name] (index 0) or parent[name][index] (scripts read window.closeButton,
    -- window.button[1] without assigning them).
    local name, index, flag = select(first + 1, ...)
    if member ~= "CreateWidget" and parent and type(name) == "string" and flag then
      if index == nil or index == 0 then
        rawset(parent, name, child)
      else
        local t = rawget(parent, name)
        if type(t) ~= "table" or INFO[t] then t = {}; rawset(parent, name, t) end
        t[index] = child
      end
    end
    return child
  end
  if sfind(member, "^Create") and #member > 6 and parent and INFO[parent].widget then
    local cls = ssub(member, 7)
    local w = REC.widgets[cls]
    if not w then w = { count = 0, by = {}, stages = {}, object = true }; REC.widgets[cls] = w end
    w.count = w.count + 1
    bump(w.by, owner .. ":" .. member)
    w.stages[CTX.stage] = true
    return mkproxy(cls, cls, nil, true)
  end
  local o = OVERRIDES[owner .. ":" .. member]
  if o then return o(parent, select(first, ...)) end
  local g = guess_return(owner, member, parent ~= nil and (INFO[parent].widget or not NATIVE[owner]))
  if g then return unpack(g, 1, table.maxn(g)) end
  return mkproxy(owner .. ":" .. member .. "()", member, nil, false)
end

PMT.__index = function(p, k)
  local info = INFO[p]
  if info == nil then return nil end
  local kid = info.kids[k]
  if kid ~= nil then return kid end
  if type(k) ~= "string" then
    hint(info.class, "[" .. type(k) .. "]")
    return nil
  end
  -- Classes captured from the real client: exactly their native methods (plus the text style
  -- fields on widgets) exist; anything else is a script field not assigned yet.
  local cm = CLASS_METHODS[info.class]
    or (info.parent and (NATIVE_FIELDS[info.name] and CLASS_METHODS.style or CLASS_METHODS[info.name]))
  if cm then
    if not cm[k] and not (info.widget and NATIVE_FIELDS[k]) then
      hint(info.class, "missing:" .. k)
      return nil
    end
  -- Other widgets: names some script assigns are script fields (absent until assigned).
  elseif info.widget and SCRIPT_FIELDS[k] and not NATIVE_NAMES[k] and not NATIVE_FIELDS[k] then
    return nil
  end
  rec(info.class, k, "member")
  kid = mkproxy(info.class .. "." .. k, k, p, false)
  info.kids[k] = kid
  return kid
end
PMT.__newindex = function(p, k, v) rawset(p, k, v) end
PMT.__call = do_call
PMT.__tostring = function(p) local i = INFO[p]; return i and ("<" .. i.class .. ">") or "<proxy>" end
PMT.__concat = function(a, b)
  if is_proxy(a) then hint(INFO[a].class, "concat") end
  if is_proxy(b) then hint(INFO[b].class, "concat") end
  return tostring(a) .. tostring(b)
end
PMT.__len = function() return 0 end
PMT.__eq = function(a, b) return rawequal(a, b) end
local function cmp(a, b)
  if is_proxy(a) then hint(INFO[a].class, "compare") end
  if is_proxy(b) then hint(INFO[b].class, "compare") end
  return false
end
PMT.__lt, PMT.__le = cmp, cmp
local function arith(a, b)
  if is_proxy(a) then hint(INFO[a].class, "number") end
  if is_proxy(b) then hint(INFO[b].class, "number") end
  return 0
end
PMT.__add, PMT.__sub, PMT.__mul, PMT.__div, PMT.__mod, PMT.__pow, PMT.__unm = arith, arith, arith, arith, arith, arith, arith

-- `proxy < 3` only reaches __lt if numbers/strings carry the very same metamethod.
debug.setmetatable(0, { __lt = cmp, __le = cmp })
local smt = getmetatable("")
smt.__lt, smt.__le = cmp, cmp

-- Missing globals: plain nil if a script defines the name somewhere, a native proxy otherwise.
setmetatable(_G, {
  __index = function(_, k)
    if type(k) ~= "string" or SCRIPT_GLOBALS[k] then return nil end
    local v = VALUES[k]
    if v ~= nil then
      rec("_G", k, "global")
      return v
    end
    local p = NATIVE[k]
    if not p then
      p = mkproxy(k, k, nil, k == "UIParent")
      NATIVE[k] = p
    end
    rec("_G", k, "global")
    return p
  end,
})
-- A call on a global proxy (`PlayUISound(...)`) is recorded as a call of `_G.<name>`.
-- Give global proxies a parent-less call path: do_call uses owner "_G" when parent is nil.

-- Standard functions that would raise on a proxy argument: degrade to a neutral result instead.
local function tolerant(f, neutral)
  return function(...)
    local n = select("#", ...)
    for i = 1, n do
      if is_proxy((select(i, ...))) then
        local res = { pcall(f, ...) }
        if res[1] then return unpack(res, 2, table.maxn(res)) end
        return neutral
      end
    end
    return f(...)
  end
end
local _pairs, _ipairs, _unpack, _tonumber = pairs, ipairs, unpack, tonumber
_G.pairs = function(t) if is_proxy(t) then hint(INFO[t].class, "iterated"); return _pairs({}) end return _pairs(t) end
_G.ipairs = function(t) if is_proxy(t) then hint(INFO[t].class, "iterated"); return _ipairs({}) end return _ipairs(t) end
_G.unpack = function(t, ...) if is_proxy(t) then return end return _unpack(t, ...) end
_G.tonumber = function(v, ...) if is_proxy(v) then hint(INFO[v].class, "number"); return 0 end return _tonumber(v, ...) end
for _, lib in _ipairs({ string, math }) do
  for name, f in _pairs(lib) do
    if type(f) == "function" then
      local neutral = ""
      if lib == math or name == "len" or name == "byte" then neutral = 0 elseif name == "find" or name == "match" then neutral = nil end
      lib[name] = tolerant(f, neutral)
    end
  end
end
local gmatch = string.gmatch
string.format = function(fmt, ...)
  if is_proxy(fmt) then return "" end
  local n = select("#", ...)
  local args = { ... }
  local i = 0
  for spec in gmatch(fmt, "%%[-+ #0]*%d*%.?%d*([%a%%])") do
    if spec ~= "%" then
      i = i + 1
      if is_proxy(args[i]) then args[i] = (spec == "s" or spec == "q") and tostring(args[i]) or 0 end
    end
  end
  for j = 1, n do if is_proxy(args[j]) then args[j] = tostring(args[j]) end end
  return sformat(fmt, _unpack(args, 1, n))
end
for _, name in _ipairs({ "insert", "remove", "sort", "concat", "getn" }) do
  local f = table[name]
  table[name] = function(t, ...)
    if is_proxy(t) then
      if name == "concat" then return "" elseif name == "getn" then return 0 end
      return nil
    end
    return f(t, ...)
  end
end

-- Record CryEngine `Script.*` use too.
if type(Script) == "table" then
  for name, f in _pairs(Script) do
    if type(f) == "function" then
      Script[name] = function(...)
        local r = rec("Script", name, "call")
        r.calls = r.calls + 1
        bump(r.argc, select("#", ...))
        local a = ...
        if type(a) == "string" then
          r.strs[1] = r.strs[1] or {}
          if not r.strs[1][a] then r.strs[1][#r.strs[1] + 1] = a end
          bump(r.strs[1], a)
        end
        return f(...)
      end
    end
  end
end

-- Call every collected callback (SetHandler handlers, functions passed to natives) with proxy
-- arguments, so code that only runs on events is explored too. OnEvent handlers get each event
-- their widget registered.
function __REC_FUZZ(budget, max_calls, from)
  local done = {}
  local calls, failures = 0, 0
  local i = from or 1
  while i <= #REC.callbacks and calls < max_calls do
    local cb = REC.callbacks[i]
    i = i + 1
    if not done[cb.fn] then
      done[cb.fn] = true
      local src = debug.getinfo(cb.fn, "S").source or "?"
      CTX.file = ssub(src, 1, 1) == "@" and ssub(src, 2) or src
      CTX.phase = "handler"
      local runs = {}
      local selfp = cb.self
      if cb.member == "SetHandler" and cb.name == "OnEvent" and selfp and INFO[selfp] and INFO[selfp].events then
        for _, ev in _ipairs(INFO[selfp].events) do
          runs[#runs + 1] = { selfp, ev, mkproxy("event " .. ev .. " arg1", "a1"), mkproxy("event " .. ev .. " arg2", "a2"), mkproxy("event " .. ev .. " arg3", "a3") }
        end
      elseif cb.member == "SetHandler" then
        local hn = cb.name or "?"
        runs[1] = { selfp, mkproxy("handler " .. hn .. " arg1", "a1"), mkproxy("handler " .. hn .. " arg2", "a2"), mkproxy("handler " .. hn .. " arg3", "a3") }
      else
        local cn = cb.owner .. ":" .. cb.member
        runs[1] = { mkproxy("callback " .. cn .. " arg1", "a1"), mkproxy("callback " .. cn .. " arg2", "a2"), mkproxy("callback " .. cn .. " arg3", "a3") }
      end
      for _, args in _ipairs(runs) do
        calls = calls + 1
        debug.sethook(function() error("recorder: instruction budget exhausted", 0) end, "", budget)
        local ok, err = pcall(cb.fn, unpack(args, 1, 5))
        debug.sethook()
        if not ok then
          failures = failures + 1
          REC.errors[#REC.errors + 1] = { stage = CTX.stage, file = CTX.file, phase = "handler", msg = tostring(err) }
        end
      end
    end
  end
  return calls, failures, i
end
