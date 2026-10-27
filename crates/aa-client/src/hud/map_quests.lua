-- Quest objective areas and the numbered notifier icons on the roadmap and the big map (hud/map_quests.rs).
-- quest_context_notifier.lua gives each tracked quest a decal index (0: a main quest, 1..9: the number in the
-- notifier list) when X2Decal:CanMakeGuidedDecal(qtype) holds, and tells the map with X2Map:AddNotifyQuestInfo /
-- UpdateNotifyQuestInfo / RemoveNotifyQuestInfo. The native map draws the quest's area (a tinted circle, the
-- colours and number cells of map/common.lua: notifyAreaColors, notifyAreaCoords) with the number on it.
-- HUD.quest_targets(qtype) is the list of areas of the quest's current step. Loaded after quest.lua and markers.lua,
-- whose hooks it wraps.

-- The decals: the quest has a place on the map when its step has an area. The path on the ground (the guided
-- decal itself) is not drawn: SetQuestGuidDecalByIndex stays a no-op.
X2Decal.GetMaxDecalCount = function(self) return 10 end
X2Decal.CanMakeGuidedDecal = function(self, qtype)
  local t = HUD.quest_targets(tonumber(qtype) or 0)
  return t ~= nil and #t > 0
end

-- The notifier's calls: qtype -> { index, show }
local notify = {}
local notifyVersion = 0
local function set(self, index, qtype, show)
  local n = notify[qtype]
  show = show ~= false
  if n == nil or n.index ~= index or n.show ~= show then
    notify[qtype] = { index = index, show = show }
    notifyVersion = notifyVersion + 1
  end
end
X2Map.AddNotifyQuestInfo = set
X2Map.UpdateNotifyQuestInfo = set
X2Map.RemoveNotifyQuestInfo = function(self, qtype)
  if notify[qtype] ~= nil then
    notify[qtype] = nil
    notifyVersion = notifyVersion + 1
  end
end
X2Map.ClearNotifyQuestInfo = function(self)
  notify = {}
  notifyVersion = notifyVersion + 1
end

-- notifyAreaColors of map/common.lua (r, g, b of 255; alpha AREA_COLOR_ALPHA), by index + 1
local COLORS = {
  { 40, 148, 63 }, { 194, 96, 107 }, { 211, 163, 60 }, { 138, 166, 40 }, { 210, 106, 29 },
  { 40, 148, 63 }, { 55, 95, 223 }, { 73, 167, 211 }, { 225, 150, 219 }, { 142, 76, 203 },
}
local ALPHA = 0.5
local SHEET = "ui/map/icon/npc_icon.dds"
local CIRCLE = "ui/map/territory_circle.dds"

-- the number's cell on the sheet (notifyAreaCoords: 16 x 16, the main quest's is the 24 x 24 diamond)
local function numberCoords(index)
  if index <= 0 then return { 480, 72, 24, 24 } end
  local i = index - 1
  return { 240 + 16 * (i % 3), 16 * math.floor(i / 3), 16, 16 }
end

local function titleOf(qtype)
  local ok, t = pcall(function() return X2Quest:GetActiveQuestTitle(qtype) end)
  return (ok and type(t) == "string" and t ~= "") and t or nil
end

-- The shown notifier quests as { qtype, index, targets } in the notifier's order (index 0 last: it is on top).
local function active()
  local out = {}
  for qtype, n in pairs(notify) do
    if n.show then
      local t = HUD.quest_targets(qtype)
      if t ~= nil and #t > 0 then out[#out + 1] = { qtype = qtype, index = n.index, targets = t } end
    end
  end
  table.sort(out, function(a, b) return a.index > b.index end)
  return out
end

-- The markers of one area: the circle (below everything) and the number (above), `place(t)` -> u, v, radius or nil.
local function build(list, place, tag)
  local below, above = {}, {}
  for _, q in ipairs(active()) do
    local c = COLORS[q.index + 1] or COLORS[1]
    local tip = titleOf(q.qtype)
    for _, t in ipairs(q.targets) do
      local u, v, r = place(t)
      if u ~= nil then
        local function mk(m)
          m.u, m.v = u, v
          for k, val in pairs(tag(t)) do m[k] = val end
          return m
        end
        if r ~= nil and r > 0 then
          below[#below + 1] = mk({ icon = CIRCLE, coords = { 0, 0, 128, 128 }, radius = r, color = { c[1] / 255, c[2] / 255, c[3] / 255, ALPHA } })
        end
        above[#above + 1] = mk({ icon = SHEET, coords = numberCoords(q.index), tooltip = tip })
      end
    end
  end
  local out = {}
  for _, m in ipairs(below) do out[#out + 1] = m end
  for _, m in ipairs(list or {}) do out[#out + 1] = m end
  for _, m in ipairs(above) do out[#out + 1] = m end
  return out
end

local function inside(u, v) return u >= 0 and u <= 1 and v >= 0 and v <= 1 end

-- the roadmap: only the player's zone map (level 2), placed as markers.lua places its icons
local road = { key = nil, list = {}, version = 0 }
local roadHook = AAUI.RoadMap
AAUI.RoadMap = function()
  local r = roadHook()
  if r == nil or r.level ~= 2 or next(notify) == nil then return r end
  local s = HUD.state
  local f, w, h = HUD.road_layout(r.level, r.zone, string.upper(s.zone_key_name))
  local key = table.concat({ r.markers_version or -1, notifyVersion, HUD.quest_version(), r.zone, s.zone_group,
    w or 0, h or 0, f and f[1] or 0, f and f[2] or 0, f and f[3] or 0, f and f[4] or 0 }, ":")
  if road.key ~= key then
    road.key, road.version = key, road.version + 1
    road.list = build(r.markers, function(t)
      if f == nil or t.group ~= s.zone_group then return nil end
      local u, v = HUD.road_image_point(f, w, h, t.u, t.v)
      if u == nil or not inside(u, v) then return nil end
      local u2 = HUD.road_image_point(f, w, h, t.u + t.ru, t.v)
      return u, v, u2 and math.abs(u2 - u) or nil
    end, function() return {} end)
  end
  r.markers, r.markers_version = road.list, road.version
  return r
end

-- the big map: every zone's map shows its quests' areas
local world = { key = nil, list = {}, version = 0 }
local worldHook = AAUI.WorldMap
AAUI.WorldMap = function()
  local r = worldHook()
  if r == nil or next(notify) == nil then return r end
  local key = table.concat({ r.markers_version or -1, notifyVersion, HUD.quest_version() }, ":")
  if world.key ~= key then
    world.key, world.version = key, world.version + 1
    world.list = build(r.markers, function(t) return t.u, t.v, t.ru end, function(t) return { level = 2, id = t.group } end)
  end
  r.markers, r.markers_version = world.list, world.version
  return r
end
