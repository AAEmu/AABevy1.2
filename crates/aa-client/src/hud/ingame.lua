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

-- Units --------------------------------------------------------------------------------------------------
-- The data of a unit the scripts ask about: the player (HUD.state) or the target (HUD.target).
local function U(unit)
  if isPlayer(unit) then return S() end
  local t = HUD.target
  if t ~= nil and (unit == "target" or unit == t.unit_id) then return t end
  return nil
end
local function field(key, default)
  return function(self, unit)
    local u = U(unit)
    if u == nil then return default end
    return u[key]
  end
end
X2Unit.UnitName = field("name", nil)
X2Unit.UnitLevel = field("level", nil)
X2Unit.UnitHealth = field("hp", nil)
X2Unit.UnitMaxHealth = field("max_hp", nil)
X2Unit.UnitMana = field("mp", nil)
X2Unit.UnitMaxMana = field("max_mp", nil)
X2Unit.UnitRace = field("race", nil)
X2Unit.UnitGender = field("gender", nil)
X2Unit.GetUnitId = function(self, unit)
  local u = U(unit)
  if u == nil then return nil end
  return u.unit_id
end
X2Unit.GetFactionName = field("faction", nil)
X2Unit.UnitIsDead = field("dead", false)
X2Unit.UnitCombatState = field("in_combat", false)

-- What kind of unit: ("character" | "npc", detail "character" | "npc" | "slave" | "housing" ...).
X2Unit.GetUnitTypeString = function(self, unit)
  if isPlayer(unit) then return "character", "character" end
  local t = U(unit)
  if t == nil then return nil end
  return t.unit_type, t.detail
end
X2Unit.GetUnitType = X2Unit.GetUnitTypeString
X2Unit.GetUnitGradeById = function(self, id)
  local t = HUD.target
  if t ~= nil and id == t.unit_id then return t.grade end
  return "normal"
end
X2Unit.GetCombatRelationshipStr = function(self, unit)
  if isPlayer(unit) then return "friendly" end
  local t = U(unit)
  if t == nil then return "neutral" end
  return t.relation
end
-- Metres to the target.
X2Unit.UnitDistance = function(self, unit)
  if unit == "target" and HUD.target ~= nil then return HUD.target_distance() end
  return nil
end
-- The id of what a unit targets (the player: the target frame's unit).
X2Unit.GetTargetUnitId = function(self, unit)
  if (unit == nil or isPlayer(unit)) and HUD.target ~= nil then return HUD.target.unit_id end
  return nil
end

-- Buffs: the lists of HUD.buffs / HUD.target.buffs; timeLeft counts down on the HUD clock.
local function buffList(unit, good)
  local u = U(unit)
  if u == nil then return nil end
  local lists = (u == S()) and HUD.buffs or u.buffs
  if lists == nil then return nil end
  if good then return lists.good end
  return lists.bad
end
local function buffAt(unit, index, good)
  local list = buffList(unit, good)
  local b = list and list[index]
  if b == nil then return nil end
  local info = {
    buff_id = b.buff_id, name = b.name, path = b.path, description = b.description, stack = b.stack,
  }
  if b.ends ~= nil then
    info.timeLeft = math.max(0, b.ends - HUD.now_ms())
  end
  return info
end
X2Unit.UnitBuffCount = function(self, unit) local l = buffList(unit, true); return l and #l or 0 end
X2Unit.UnitDeBuffCount = function(self, unit) local l = buffList(unit, false); return l and #l or 0 end
X2Unit.UnitBuff = function(self, unit, index, flags) return buffAt(unit, index, true) end
X2Unit.UnitDeBuff = function(self, unit, index, flags) return buffAt(unit, index, false) end

-- Experience: percent of the level, exp of the level, exp for the next level.
X2Player.GetExpInfo = function(self) return S().exp_percent, S().exp_cur, S().exp_need end
-- Exp a resurrection at a shrine wins back: the amount and its percent of the level.
X2Player.GetRecoverableExp = function(self)
  local need = tonumber(S().exp_need) or 0
  local pct = 0
  if need > 0 then pct = 100 * S().recoverable_exp / need end
  return tostring(S().recoverable_exp), pct
end
X2Player.GetGamePoints = function(self)
  return { honorPointStr = tostring(S().honor), livingPointStr = tostring(S().vocation), crimePoint = 0, crimeRecord = 0 }
end
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
  local x, y, _, yaw = HUD.player_pos()
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
  -- the heading: radians, 0 north, clockwise (the engine's yaw turns left)
  return { level = 2, zone = s.zone_group, player = { u = u, v = v, angle = -yaw } }
end

-- Action bars and bag -----------------------------------------------------------------------------------
-- Slot widgets ask these hooks (AAUI.SlotContent ...); the lists come from aa-net. FALLBACK to the fake
-- of the capture while the server has not sent the action bars / the bag yet.
local fakeSlotContent = AAUI.SlotContent
local function slotContent(s)
  local c = { type = s.kind, id = s.id, icon = s.icon, grade = s.grade }
  if s.kind == "item" then
    if s.stack then c.stack = s.count end
    c.canUse = s.count > 0
  end
  if s.cd_end ~= nil then
    local remain = s.cd_end - HUD.now_ms()
    if remain > 0 then c.cooldown = { remain = remain, total = math.max(s.cd_total, remain) } end
  end
  return c
end
AAUI.SlotContent = function(slotType, slotIdx)
  if slotType == ISLOT_ACTION then
    if not HUD.actions_known then return fakeSlotContent(slotType, slotIdx) end
    local s = HUD.actions[slotIdx]
    if s == nil then return nil end
    return slotContent(s)
  elseif slotType == ISLOT_BAG then
    if HUD.bag_slots == 0 then return fakeSlotContent(slotType, slotIdx) end
    local s = HUD.bag[slotIdx]
    if s == nil then return nil end
    return slotContent(s)
  end
  return nil
end
local fakeSkillContent, fakeItemContent = AAUI.SkillContent, AAUI.ItemContent
AAUI.SkillContent = function(skillType)
  local name, icon = HUD.skill_info(skillType)
  if icon == nil then return fakeSkillContent(skillType) end
  return { type = "skill", id = skillType, icon = icon }
end
AAUI.ItemContent = function(itemType, grade)
  local name, icon, maxStack, fixedGrade = HUD.item_info(itemType)
  if icon == nil then return fakeItemContent(itemType, grade) end
  return { type = "item", id = itemType, icon = icon, grade = grade or fixedGrade }
end

local function bagCount()
  local n = 0
  for _ in pairs(HUD.bag) do n = n + 1 end
  return n
end
local fakeCapacity, fakeEmpty, fakeCount = X2Bag.Capacity, X2Bag.CountEmptyBagSlots, X2Bag.CountItems
X2Bag.Capacity = function(self) if HUD.bag_slots == 0 then return fakeCapacity(self) end return HUD.bag_slots end
X2Bag.CountItems = function(self) if HUD.bag_slots == 0 then return fakeCount(self) end return bagCount() end
X2Bag.CountEmptyBagSlots = function(self)
  if HUD.bag_slots == 0 then return fakeEmpty(self) end
  return HUD.bag_slots - bagCount()
end
X2Bag.IsInvalidSlot = function(self, virtualSlotIdx)
  local n = HUD.bag_slots
  if n == 0 then n = fakeCapacity(self) end
  return virtualSlotIdx < 1 or virtualSlotIdx > n
end

-- Leaving ------------------------------------------------------------------------------------------------
-- The system menu and the disconnect dialog leave the game through X2World:LeaveWorld(EXIT_*): the server
-- counts down (LEAVING_WORLD_STARTED opens the frame) and lets go; with the connection lost the same call
-- returns to the server list.
X2World.LeaveWorld = function(self, how)
  HUD.request("leave", how)
  return true
end
X2World.CancelLeaveWorld = function(self) HUD.request("leave_cancel") end
-- The frame's OK button after the countdown: the server lets go on its own.
X2World.LeaveGame = function(self) end

-- Entering the world ------------------------------------------------------------------------------------
-- What the client raises when the character has spawned and the loading screen closed: the world is
-- entered (the chat channels and lines follow from aa-net's state, chat.rs), the new character's
-- "Game Controls" tutorial opens.
HUD.enter_world = function()
  FAKE.fire_now("ENTERED_WORLD", true)
  FAKE.fire_now("LEFT_LOADING")
  if S().sub_zone ~= "" then FAKE.fire_now("ENTERED_SUBZONE", 0, S().sub_zone) end
  -- FALLBACK: the tutorial of a brand new character (id 2584, two pages)
  if S().level <= 1 then
    FAKE.fire_now("TUTORIAL_EVENT", 2584, {
      { { title = "" }, { "tutorial_contents_2584_2_1" } },
      { { title = "" }, { "tutorial_contents_2584_2_2" } },
    })
  end
  HUD.setup_chat()
end

-- Chat input ----------------------------------------------------------------------------------------------
-- Enter (or "/") opens the chat window's own edit box; Enter again sends the line to the channel of the
-- last slash command (say at first); Escape closes it. "/w name text" whispers, "/1" shouts, "/p" party.
local chat = { channel = 0, target = "" }
local SLASH = {
  s = "SAY", say = "SAY", ["1"] = "ZONE", y = "ZONE", shout = "ZONE", ["2"] = "TRADE", trade = "TRADE",
  ["3"] = "FIND_PARTY", findparty = "FIND_PARTY", p = "PARTY", party = "PARTY", r = "RAID", raid = "RAID",
  n = "FACTION", nation = "FACTION", faction = "FACTION", e = "EXPEDITION", ex = "EXPEDITION",
  f = "FAMILY", family = "FAMILY", a = "RACE", ally = "RACE", alliance = "RACE",
}
local CHANNEL_LABEL = {
  SAY = "Say", ZONE = "Shout", TRADE = "Trade", FIND_PARTY = "Party Search", PARTY = "Party", RAID = "Raid",
  FACTION = "Nation", EXPEDITION = "Guild", FAMILY = "Family", RACE = "Alliance",
}

local function chatEdit()
  local w = HUD.chat_window()
  if w == nil then return nil end
  return w:GetChatEdit()
end

local function channelValue(name) return _G["CHAT_" .. name] end

local function showChannel(edit)
  local label = "Say"
  for name, text in pairs(CHANNEL_LABEL) do
    if channelValue(name) == chat.channel then label = text end
  end
  if chat.target ~= "" then label = "To " .. chat.target end
  local w = HUD.chat_window()
  local selector = w and w:GetChatMethodSelector()
  if selector ~= nil and selector.SetText ~= nil then selector:SetText(label) end
end

HUD.chat_open = function(prefill)
  local edit = chatEdit()
  if edit == nil then return end
  edit:Show(true)
  edit:SetText(prefill or "")
  showChannel(edit)
  edit:SetFocus()
end

HUD.chat_close = function()
  local edit = chatEdit()
  if edit == nil then return end
  edit:SetText("")
  edit:ClearFocus()
  edit:Show(false)
end

-- Send what was typed: a slash command picks the channel (and keeps it for the next line).
HUD.chat_submit = function(text)
  text = text or ""
  local cmd, rest = string.match(text, "^/(%S+)%s*(.*)$")
  if cmd ~= nil then
    cmd = string.lower(cmd)
    if cmd == "w" or cmd == "whisper" or cmd == "t" or cmd == "tell" then
      local name, msg = string.match(rest, "^(%S+)%s*(.*)$")
      if name ~= nil then
        chat.channel, chat.target = CHAT_WHISPER, name
        text = msg
      else
        text = ""
      end
    elseif SLASH[cmd] ~= nil then
      chat.channel, chat.target = channelValue(SLASH[cmd]), ""
      text = rest
    end
  end
  text = string.gsub(text, "^%s+", "")
  if text ~= "" then
    HUD.request("chat", chat.channel, text, chat.target)
  end
  HUD.chat_close()
end

HUD.setup_chat = function()
  chat.channel = CHAT_SAY
  local edit = chatEdit()
  if edit == nil then return end
  edit:SetHandler("OnEnterPressed", function(self) HUD.chat_submit(self:GetText()) end)
  edit:SetHandler("OnEscapePressed", function(self) HUD.chat_close() end)
  edit:Show(false)
end
