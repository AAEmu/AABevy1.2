-- The icons of the roadmap and of the big map (hud/markers.rs). HUD.map_icons is the list in game coordinates
-- ({ x, y, kind, tip, city }); this file keeps the "Show Icons" window's filters, places the icons on the zone or
-- city map the roadmap or the big map shows and hands them to the toolkit: AAUI.RoadMap().markers (the place on the
-- map image) and AAUI.WorldMap().markers (the place on the map `level`, `id`). Loaded after ingame.lua, whose
-- hooks it wraps.

-- The filters ------------------------------------------------------------------------------------------------
-- map/filter.lua tells the game what to show: X2Map:SetNpcFilter(iconType, on) for each icon type, and
-- SetNpcShowFilter / SetDoodadShowFilter / SetHouseShowFilter(on) for the three folders (Merchants, Infrastructure,
-- Housing Province: the lists of map/locale/ko.lua). An icon of a folder shows when the folder and its own check are on.
local FOLDERS = {
  npc = {
    "npc_store_weapon", "npc_store_defense", "npc_store_blacksmith", "npc_store_material", "npc_store_furniture",
    "npc_store_tree", "npc_store_plants", "npc_store_livestock", "npc_store_roamer", "npc_stabler", "npc_store_bank",
    "npc_store_auction_house", "npc_store_expedition", "npc_store_pirate_expedition", "npc_ability_changer",
    "npc_store_trade", "npc_store_delphinad", "npc_store_enlisting", "npc_store_gladiator",
  },
  doodad = {
    "doodad_mail", "doodad_inn", "doodad_cook", "doodad_craft", "doodad_archium", "doodad_alchemy", "doodad_metal",
    "doodad_timber", "doodad_fabric", "doodad_leather", "doodad_stone", "doodad_coatofarms", "doodad_woodwork",
    "doodad_paper", "doodad_print", "doodad_art_work", "doodad_fish_stand", "doodad_weapon", "doodad_light_armor",
    "doodad_leather_armor", "doodad_heavy_armor", "doodad_special_product", "doodad_portal",
    "doodad_portal_archemall", "doodad_portal_dungeon",
  },
  house = {
    "house_normal_house", "house_sea_farm_house", "house_high_house", "house_pumpkin_house", "house_farm_house",
    "house_benny_house", "house_bungalow_house",
  },
}
-- the checks that have no folder
local SINGLES = { "corpse_pos", "npc_station", "npc_graveyard", "common_farm", "housing_house", "housing_farm", "housing_fishfarm" }

local FOLDER_OF = {}
for folder, kinds in pairs(FOLDERS) do
  for _, kind in ipairs(kinds) do FOLDER_OF[kind] = folder end
end

local folderOn = { npc = true, doodad = true, house = true }
local checked = {} -- kind -> bool, as the window last said
local filterVersion = 0

-- icon type (the M_* numbers map/common.lua makes) -> kind; known once the map scripts have run
local kindOfType
local function kindOf(iconType)
  if kindOfType == nil then
    if M_NPC_STATION == nil then return nil end
    kindOfType = {}
    local function add(kind)
      local n = _G["M_" .. string.upper(kind)]
      if n ~= nil then kindOfType[n] = kind end
    end
    for kind in pairs(FOLDER_OF) do add(kind) end
    for _, kind in ipairs(SINGLES) do add(kind) end
  end
  return kindOfType[iconType]
end

X2Map.SetNpcFilter = function(self, iconType, on)
  local kind = kindOf(iconType)
  if kind ~= nil and checked[kind] ~= (on ~= false) then
    checked[kind] = on ~= false
    filterVersion = filterVersion + 1
  end
end
local function setFolder(folder, on)
  if folderOn[folder] ~= (on ~= false) then
    folderOn[folder] = on ~= false
    filterVersion = filterVersion + 1
  end
end
X2Map.SetNpcShowFilter = function(self, on) setFolder("npc", on) end
X2Map.SetDoodadShowFilter = function(self, on) setFolder("doodad", on) end
X2Map.SetHouseShowFilter = function(self, on) setFolder("house", on) end

local function shown(kind)
  local folder = FOLDER_OF[kind]
  if folder ~= nil and not folderOn[folder] then return false end
  return checked[kind] ~= false
end

-- The roadmap's "NPC" check: the dots of the units in view are only worth computing while it is on.
-- (The scripts keep the option in X2:SetRoadmapOptionUiData; the real client saves it, this one keeps it for the stage.)
local roadmapOption
X2.GetRoadmapOptionUiData = function(self) return roadmapOption end
X2.SetRoadmapOptionUiData = function(self, data)
  roadmapOption = data
  HUD.map_dots(data ~= nil and data.npcShow == true)
end

-- The places ---------------------------------------------------------------------------------------------------
-- The place on a road map image: the roadmap window (926 x 556) is the map rectangle, the image sits in it at its
-- own frame (see ingame.lua, AAUI.RoadMap).
local function onRoadImage(level, id, formKey, uw, vw)
  local f = HUD_ROAD_FRAMES[level] and HUD_ROAD_FRAMES[level][id]
  local form = FORM_ROAD_MAP_LIST and FORM_ROAD_MAP_LIST[formKey]
  if f == nil and form ~= nil then f = { form[1], form[2], form[5], form[6] } end
  if f == nil then return uw, vw end
  return (uw * 926 - f[1]) / f[3], (vw * 556 - f[2]) / f[4]
end

-- the tooltip of an icon: the unit's or doodad's name, else the symbol's text (the filter window's label)
local symbolTexts = {}
local function tipOf(ic)
  if ic.tip ~= "" then return ic.tip end
  local text = symbolTexts[ic.kind]
  if text == nil then
    local ok, t = pcall(function() return locale.map.getSymbolText(X2Map:GetMapIconText(_G["M_" .. string.upper(ic.kind)])) end)
    text = (ok and type(t) == "string") and t or false
    symbolTexts[ic.kind] = text
  end
  return text or nil
end

local function inside(u, v)
  return u >= 0 and u <= 1 and v >= 0 and v <= 1
end

-- the markers of the roadmap's map (level 2 the zone, 3 a city) and of the big map, rebuilt when the list, the
-- filters or the map change
local road = { key = nil, list = {}, version = 0 }
local world = { key = nil, list = {}, version = 0 }

local function roadMarkers(r)
  local s = HUD.state
  local key = table.concat({ HUD.map_icons_version or -1, filterVersion, r.level, r.zone, s.zone_group }, ":")
  if road.key == key then return road.list, road.version end
  road.key, road.version, road.list = key, road.version + 1, {}
  local zr, cr = s.zone_rect, s.sub_map_rect
  for _, ic in ipairs(HUD.map_icons or {}) do
    if shown(ic.kind) then
      local u, v
      if r.level == 3 then
        if ic.city == r.zone and cr[3] > 0 then
          u, v = onRoadImage(3, r.zone, "", (ic.x - cr[1]) / cr[3], 1 - (ic.y - cr[2]) / cr[4])
        end
      elseif zr[3] > 0 then
        u, v = onRoadImage(2, r.zone, string.upper(s.zone_key_name) .. "_100", (ic.x - zr[1]) / zr[3], 1 - (ic.y - zr[2]) / zr[4])
      end
      if u ~= nil and inside(u, v) then
        road.list[#road.list + 1] = { u = u, v = v, kind = ic.kind, tooltip = tipOf(ic) }
      end
    end
  end
  return road.list, road.version
end

local function worldMarkers()
  local s = HUD.state
  local key = table.concat({ HUD.map_icons_version or -1, filterVersion, s.zone_group }, ":")
  if world.key == key then return world.list, world.version end
  world.key, world.version, world.list = key, world.version + 1, {}
  local zr, cities = s.zone_rect, HUD.map_city_rects or {}
  for _, ic in ipairs(HUD.map_icons or {}) do
    if shown(ic.kind) then
      -- the whole map image covers the map rectangle: the place is the plain fraction of it
      if zr[3] > 0 then
        local u, v = (ic.x - zr[1]) / zr[3], 1 - (ic.y - zr[2]) / zr[4]
        if inside(u, v) then
          world.list[#world.list + 1] = { level = 2, id = s.zone_group, u = u, v = v, kind = ic.kind, tooltip = tipOf(ic) }
        end
      end
      local c = ic.city and cities[ic.city]
      if c ~= nil and c[3] > 0 then
        local u, v = (ic.x - c[1]) / c[3], 1 - (ic.y - c[2]) / c[4]
        if inside(u, v) then
          world.list[#world.list + 1] = { level = 3, id = ic.city, u = u, v = v, kind = ic.kind, tooltip = tipOf(ic) }
        end
      end
    end
  end
  return world.list, world.version
end

local roadMapHook, worldMapHook = AAUI.RoadMap, AAUI.WorldMap
AAUI.RoadMap = function()
  local r = roadMapHook()
  if r ~= nil then r.markers, r.markers_version = roadMarkers(r) end
  return r
end
AAUI.WorldMap = function()
  local r = worldMapHook()
  if r ~= nil then r.markers, r.markers_version = worldMarkers() end
  return r
end
