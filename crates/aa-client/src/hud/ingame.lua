-- The in-game game API on real data (aa-net's state through `HUD.state`, game database lookups
-- through the `HUD.*` functions of api.rs). Installed after the FALLBACK fake of
-- aa-ui/examples/fake/fake_ingame.lua and overriding it function by function; whatever is not
-- overridden here still comes from the fake (search the log for "FALLBACK").

local function S() return HUD.state end
local function isPlayer(unit) return unit == "player" or unit == S().unit_id end

-- Names (guild, family, mail ... windows read the limits while they are built). FALLBACK: the limits
-- of the capture's "Enter 2-26 characters." until the server's name policy is on the wire.
X2Util.GetNamePolicyInfo = function(self, kind)
  return { local_min = 2, local_max = 26, eng_min = 2, eng_max = 26 }
end
X2Util.CheckAvailableName = function(self, text, composing)
  text = text or ""
  if composing then return text end
  return (string.gsub(text, "[^%a]", ""))
end
X2Util.IsValidName = function(self, text, kind)
  text = text or ""
  return #text >= 2 and #text <= 26 and string.find(text, "^%a+$") ~= nil
end

-- No peace tax on this server (the world map's bottom info reads it for the zone under the cursor).
X2Dominion.GetPeaceTaxMoney = function(self, zoneId) return "" end
X2Dominion.GetPeaceTaxAAPoint = function(self, zoneId) return "" end

-- The player ------------------------------------------------------------------------------------
X2Unit.UnitName = function(self, unit) if isPlayer(unit) then return S().name end return nil end
X2Unit.UnitLevel = function(self, unit) if isPlayer(unit) then return S().level end return nil end
X2Unit.UnitHealth = function(self, unit) if isPlayer(unit) then return S().hp end return nil end
X2Unit.UnitMaxHealth = function(self, unit) if isPlayer(unit) then return S().max_hp end return nil end
X2Unit.UnitMana = function(self, unit) if isPlayer(unit) then return S().mp end return nil end
X2Unit.UnitMaxMana = function(self, unit) if isPlayer(unit) then return S().max_mp end return nil end
X2Unit.UnitRace = function(self, unit) if isPlayer(unit) then return S().race end return nil end
X2Unit.UnitGender = function(self, unit) if isPlayer(unit) then return S().gender end return nil end
X2Unit.GetUnitId = function(self, unit) if isPlayer(unit) then return S().unit_id end return nil end
X2Unit.GetFactionName = function(self, unit) if isPlayer(unit) then return S().faction end return nil end

-- Experience: percent of the level, exp of the level, exp for the next level.
X2Player.GetExpInfo = function(self) return S().exp_percent, S().exp_cur, S().exp_need end
X2Player.GetLaborPower = function(self) return S().labor end
X2Player.GetMaxLaborPower = function(self) return S().max_labor end
X2Player.GetBmPoint = function(self) return S().bm_point end
X2Util.GetMyMoneyString = function(self) return S().money end

-- World clock and zone -------------------------------------------------------------------------------
-- Game time as (isAm, hour 1..12, minute).
X2Time.GetGameTime = function(self)
  local h = S().hour
  local isAm = h < 12
  h = h % 12
  if h == 0 then h = 12 end
  return isAm, h, S().minute
end
X2World.GetZoneText = function(self)
  if S().zone == "" then return nil end
  return S().zone
end
X2World.GetSubZoneText = function(self)
  if S().sub_zone == "" then return nil end
  return S().sub_zone
end
-- The faction that protects the zone (id), nil for a neutral zone.
X2World.GetZoneFaction = function(self)
  if S().zone_faction == 0 then return nil end
  return S().zone_faction
end
X2Faction.GetFactionInfo = function(self, id)
  local name
  if id == S().zone_faction then name = S().zone_faction_name else name = HUD.faction_name(id) end
  if name == nil or name == "" then return nil end
  return { name = name }
end

-- The zone map overlay (top right): the zone the player is in and where on its map image.
-- The image shows the zone rectangle (west, south, width, height in metres) at the scale of its
-- height; `FORM_ROAD_MAP_LIST` (map/form.lua) has the size the scripts lay the image out at.
AAUI.RoadMap = function()
  local s = S()
  if s.zone_group == 0 then return nil end
  local x, y = HUD.player_pos()
  local r = s.zone_rect
  local u, v = 0, 0
  if r[3] > 0 and r[4] > 0 then
    local form = FORM_ROAD_MAP_LIST and FORM_ROAD_MAP_LIST[string.upper(s.zone_key_name) .. "_100"]
    local scale = 1
    if form ~= nil and form[5] ~= nil and form[6] ~= nil then
      scale = (form[6] / r[4]) / (form[5] / r[3]) -- the image's own aspect against the rectangle's
    end
    u = (x - r[1]) / r[3] * scale
    v = 1 - (y - r[2]) / r[4]
  end
  return { level = 2, zone = s.zone_group, player = { u = u, v = v } }
end

-- Entering the world ------------------------------------------------------------------------------------
-- What the client raises when the character has spawned and the loading screen closed: the world is
-- entered, the zone chat channels are joined (the "Entering Chat" lines), the new character's
-- "Game Controls" tutorial opens.
HUD.enter_world = function()
  FAKE.fire_now("ENTERED_WORLD", true)
  FAKE.fire_now("LEFT_LOADING")
  -- FALLBACK: the chat channels are not on the wire yet; the zone and faction ones every character joins
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_ZONE, S().zone)
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_TRADE, S().zone)
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_FIND_PARTY, S().zone)
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_FACTION, S().faction)
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_RACE, S().faction)
  -- FALLBACK: the tutorial of a brand new character (id 2584, two pages)
  if S().level <= 1 then
    FAKE.fire_now("TUTORIAL_EVENT", 2584, {
      { { title = "" }, { "tutorial_contents_2584_2_1" } },
      { { title = "" }, { "tutorial_contents_2584_2_2" } },
    })
  end
end
