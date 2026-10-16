-- reference/tools/snapshot.lua  (needs lib.lua loaded; run with `-t ui`)
-- Returns the JSON of every widget / drawable that is effectively visible right now, with the
-- values the API can read (rect, text, alpha, ...) plus, when install_hooks() was active while the
-- screen was built, the recorded construction calls (texture, coords, colour, font, anchors, ...).
local R = __ref
-- name the widget/drawable classes once per Lua state (the state is rebuilt on every stage change)
if not R.classes_probed then R.classes_probed = true R.probe_classes() end
local order, byaddr = R.enumerate(nil, 10)

-- owner of drawables: the nearest widget on the path (computed from creation log, else path prefix)
local owner_of, creator_of, class_hint = {}, {}, {}
for _, rec in ipairs(R.log) do
  if rec.r then
    owner_of[rec.r] = rec.a
    creator_of[rec.r] = rec
  end
end

local function isw(t) return type(t) == 'table' and type(rawget(t, '__this')) == 'userdata' end
-- walk global tables again to find, for each drawable/style object, the widget that holds it
local holder = {}
do
  local seen = {}
  local function walk(t, w, depth)
    if depth > 6 or seen[t] then return end
    seen[t] = true
    for k, v in pairs(t) do
      if type(v) == 'table' and k ~= '__this' and k ~= '__index' then
        if isw(v) then
          local a = R.addr(v)
          local m = getmetatable(v)
          local isdr = m and m.__index and m.__index.SetTexture ~= nil
          local iswidget = not isdr and m and m.__index and m.__index.CreateChildWidget ~= nil
          if iswidget then walk(v, v, depth + 1)
          else if w and not holder[a] then holder[a] = R.addr(w) end walk(v, w, depth + 1) end
        else walk(v, w, depth + 1) end
      end
    end
  end
  for _, rec in ipairs(order) do
    local m = getmetatable(rec.obj)
    if m and m.__index and m.__index.CreateChildWidget ~= nil then walk(rec.obj, rec.obj, 1) end
  end
end

local states = {}
for _, rec in ipairs(order) do
  local st = R.state(rec.obj)
  st.addr = rec.addr
  st.paths = rec.paths
  if st.kind ~= 'widget' then
    st.owner = owner_of[rec.addr] or holder[rec.addr]
  end
  local idn = st.id and tonumber(st.id, 16)
  st.order = idn
  states[rec.addr] = st
end

-- effective visibility: own flag and every ancestor (widget parent / drawable or style owner)
local shown_cache = {}
local function shown(a, depth)
  local c = shown_cache[a]
  if c ~= nil then return c end
  depth = depth or 0
  local st = states[a]
  local r
  if not st or depth > 40 then r = false
  elseif st.visible == false then r = false
  elseif st.kind == 'widget' then
    local up = st.parent
    if up == nil or up == 'UIParent' or up == '?' then r = true else r = shown(up, depth + 1) end
  else
    if st.owner == nil then r = false else r = shown(st.owner, depth + 1) end
  end
  shown_cache[a] = r
  return r
end

-- calls per object
local calls = {}
for _, rec in ipairs(R.log) do
  local a = rec.a
  if a then
    local t = calls[a]
    if not t then t = {} calls[a] = t end
    t[#t + 1] = rec
  end
end

local SKIP_ROOT = { tooltip = true, textTooltip = true, imageTooltip = true, unitFrameTooltip = true,
  summaryTooltip = true, fixedTooltip = true, mapTooltip = true, hotKeyOverrideTooltip = true,
  premiumServiceToolTip = true, candidateListWnd = true, systemConfigFrame = true, commonDropdown = true,
  gameExitFrame = true, staffWindow = true, section = true, loginStageBlackWnd = true,
  actionBar_dockingWindow = true, __ref = true }
local out, hidden = {}, {}
local nshown = 0
for _, rec in ipairs(order) do
  local a = rec.addr
  local st = states[a]
  local sh = shown(a)
  if R.include_hidden and #rec.paths > 0 then sh = true end
  st.shown = sh
  if sh then
    nshown = nshown + 1
    local c = calls[a]
    if c then
      local cc = {}
      for i, r in ipairs(c) do
        cc[i] = { s = r.s, m = r.m, args = r.args, r = r.r, rc = r.rc }
      end
      st.calls = cc
    end
    local cr = creator_of[a]
    if cr then st.created_by = { s = cr.s, m = cr.m, parent = cr.a, args = cr.args } end
    out[#out + 1] = st
  elseif #rec.paths > 0 and st.kind == 'widget' then
    local root = string.match(rec.paths[1], '^[%w_]+')
    if not SKIP_ROOT[root] then
      local e = st.extent
      hidden[#hidden + 1] = rec.paths[1] .. '|' .. tostring(st.class) .. '|' .. tostring(e and e[1]) .. 'x' .. tostring(e and e[2])
        .. (st.visible and '|selfvisible' or '')
    end
  end
end
table.sort(out, function(x, y) return (x.order or 0) < (y.order or 0) end)
table.sort(hidden)
local hidden_total = #hidden
while #hidden > 300 do hidden[#hidden] = nil end

local meta = {
  screen_width = UIParent:GetScreenWidth(), screen_height = UIParent:GetScreenHeight(),
  ui_scale = UIParent:GetUIScale(),
  stage = X2LoginCharacter and X2LoginCharacter:GetCurrentStage() or nil,
  objects_total = #order, objects_shown = nshown, hooks_active = (R.hooks_installed_at ~= nil),
  log_entries = #R.log,
  fps = UIParent:GetFrameRate(),
}
return R.json({ meta = meta, widgets = out, hidden_named_widgets = hidden, hidden_named_total = hidden_total })
