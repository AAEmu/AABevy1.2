-- The in-game game API on real data (aa-net's state through `HUD.state`, game database lookups
-- through the `HUD.*` functions of api.rs). Installed after the FALLBACK fake of
-- aa-ui/examples/fake/fake_ingame.lua and overriding it function by function; whatever is not
-- overridden here still comes from the fake (search the log for "FALLBACK").

local function S() return HUD.state end
local U
local function isPlayer(unit) return U(unit) == S() end

-- There is currently no active cutscene player. Original level-up effects query this count.
-- Cutscene playback and its UI suppression remain an unimplemented feature.
Movie.GetNumOfPlayingCutscenes = function() return 0 end

-- Name policies, normalization and validation are installed natively by api.rs.

-- Received Dominion getters are installed by territory::install after this file.


-- Units --------------------------------------------------------------------------------------------------
-- The data of a unit the scripts ask about: the player (HUD.state) or the target (HUD.target).
-- HUD tables retain private decimal keys. Public unit strings are hexadecimal;
-- adapting here keeps persistent character/item IDs and saved state untouched.
local function privateUnit(unit)
  if unit == "player" or unit == S().unit_id then return S() end
  local pet = HUD.playerpet
  if pet ~= nil and (unit == "playerpet" or unit == pet.unit_id) then return pet end
  local t = HUD.target
  if t ~= nil and (unit == "target" or unit == t.unit_id) then return t end
  if HUD.team_units ~= nil then return HUD.team_units[unit] end
  return nil
end
local function unitToken(unit)
  if type(unit) ~= "string" then return nil end
  if unit == "player" or unit == "target" or unit == "playerpet" then return privateUnit(unit) end
  if string.match(unit, "^team%d+$") and HUD.team_units then return HUD.team_units[unit] end
end
U = function(unit)
  local actor = unitToken(unit)
  if actor ~= nil then return actor end
  local privateId = HUD.private_unit_id(unit)
  if privateId ~= nil then return privateUnit(privateId) end
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
-- Values of the unit frame tooltip's `|pN;` placeholders (UFT_*: name, hp, max hp, hp %, mp, max mp,
-- mp %, labor, max labor, pvp honor, hostile faction kills) for a unit token; aa-ui substitutes them
-- into the tooltip lines (SetTooltipData) and re-reads them while the tooltip is shown.
function AAUI_TooltipUnitValues(unit)
  local u = U(unit)
  if u == nil then return nil end
  local function pct(cur, max)
    if cur == nil or max == nil then return nil end
    if max <= 0 then return 0 end
    return math.floor(100 * cur / max)
  end
  local v = { u.name, u.hp, u.max_hp, pct(u.hp, u.max_hp), u.mp, u.max_mp, pct(u.mp, u.max_mp) }
  if u == S() then
    v[8], v[9] = S().labor, S().max_labor
  end
  v[10] = u.pvp_honor or 0
  v[11] = u.pvp_kills or 0
  return v
end
-- The player's Character Info numbers, computed from the database (tipcalc/charstats.rs): the key set and the units
-- are the original client's. The fake fixture's fixed values stay for any other unit.
do
  local info, modifiers, attr = X2Unit.UnitInfo, X2Unit.UnitModifierInfo, X2Unit.UnitAttr
  X2Unit.UnitInfo = function(self, unit)
    if isPlayer(unit) then
      local t = HUD.player_info()
      if t ~= nil then return t end
    end
    if info ~= nil then return info(self, unit) end
  end
  X2Unit.UnitModifierInfo = function(self, unit)
    if isPlayer(unit) then
      local t = HUD.player_modifiers()
      if t ~= nil then return t end
    end
    if modifiers ~= nil then return modifiers(self, unit) end
  end
  X2Unit.UnitAttr = function(self, unit, key)
    if isPlayer(unit) then
      local t = HUD.player_info()
      if t ~= nil then return t[key] or 0 end
    end
    if attr ~= nil then return attr(self, unit, key) end
    return 0
  end
end
-- The character window's update handler, which all the events it listens to call (it returns early while closed).
function HUD.refresh_character_info()
  if OnUpdatedCharacterInfo ~= nil then OnUpdatedCharacterInfo(false) end
end

X2Unit.GetUnitId = function(self, unit)
  local u = unitToken(unit)
  if u == nil then return nil end
  return HUD.script_unit_id(u.unit_id)
end
-- A unit's name by its id (the player's own, the target's): the death message prints the name of whoever died.
X2Unit.GetUnitNameById = function(self, id)
  local privateId = HUD.private_unit_id(id)
  if privateId == nil then return nil end
  local t = privateUnit(privateId)
  if t ~= nil then return t.name end
  -- a resurrection caster outside the player's target or party (`PLAYER_RESURRECTION` carries its unit id)
  return HUD.unit_name(id)
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
X2Unit.GetUnitType = function(self, unit)
  local kind, detail = self:GetUnitTypeString(unit)
  return detail or kind
end
-- Native394c0f40 requires raw kind1 (Npc), not the broader UI "npc" category.
X2Unit.GetNpcTypeIndex = function(self, unit)
  local t = U(unit)
  if t ~= nil and t.detail == "npc" and t.npc_type ~= nil then return t.npc_type end
end
X2Unit.GetUnitGradeById = function(self, id)
  local privateId = HUD.private_unit_id(id)
  if privateId == nil then return nil end
  local t = HUD.target
  if t ~= nil and t.detail == "npc" and privateId == t.unit_id then return t.grade end
end
X2Unit.GetCombatRelationshipStr = function(self, unit)
  if isPlayer(unit) then return "friendly" end
  local t = U(unit)
  if t == nil then return nil end
  if t.relation == "hostile" or t.relation == "neutral" or t.relation == "friendly" then return t.relation end
  return 0
end
-- Native39442960 /394bb977: the original target popup excludes hostile1;
-- neutral2 and friendly3 remain eligible. A missing target returns neutral2.
X2.GetTargetFactionRelationship = function(self)
  local t = HUD.target
  if t == nil then return 2 end
  return ({hostile=1,neutral=2,friendly=3})[t.relation] or 0
end
-- Metres to the target.
X2Unit.UnitDistance = function(self, unit)
  if unit == "target" and HUD.target ~= nil then return HUD.target_distance() end
  local t = U(unit)
  if t and t.x and t.online then
    local x, y, z = HUD.player_pos()
    return math.sqrt((t.x-x)^2 + (t.y-y)^2 + (t.z-z)^2)
  end
  return nil
end
-- The id of what a unit targets (the player: the target frame's unit).
X2Unit.GetTargetUnitId = function(self, unit)
  if (unit == nil or isPlayer(unit)) and HUD.target ~= nil then return HUD.script_unit_id(HUD.target.unit_id) end
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
local function buffAt(unit, index, good, ...)
  if type(unit) ~= "string" then return end
  local n, all, description, mine, duration, timeLeft, stack = HUD.buff_fields(index, ...)
  local list = buffList(unit, good)
  local b = n and list and list[n]
  if b == nil or not b.descriptor then return end
  -- Native always formats the description; flags only select exposed keys.
  local text = HUD.buff_text(b.buff_id, b.ab_level) or b.description
  local info = {}
  if description then info.description = text end
  if all then
    local tipType = b.buff_kind == 1 and "buff" or "debuff"
    info.buff_id, info.name, info.path = b.buff_id, b.name, b.path
    info.tipType = tipType
    info.category = X2Locale:LocalizeUiText(33, tipType)
  end
  if mine then info.mine = HUD.buff_mine(b.caster_character_id) end
  if duration then info.duration = b.duration end
  if timeLeft and b.ends ~= nil then info.timeLeft = HUD.buff_float(math.max(0, b.ends - HUD.now_ms())) end
  if stack then info.stack = b.stack end
  return info
end
X2Unit.UnitBuffCount = function(self, unit) local l = buffList(unit, true); return l and #l or 0 end
X2Unit.UnitDeBuffCount = function(self, unit) local l = buffList(unit, false); return l and #l or 0 end
X2Unit.UnitBuff = function(self, unit, index, ...) return buffAt(unit, index, true, ...) end
X2Unit.UnitDeBuff = function(self, unit, index, ...) return buffAt(unit, index, false, ...) end

-- Experience: percent of the level, exp of the level, exp for the next level.
X2Player.GetExpInfo = function(self) return S().exp_percent, S().exp_cur, S().exp_need end
-- Abilities (archetype ids 1..10: fight illusion adamant will death wild magic vocation romance love): the
-- character's own three active ones in slot order, as the server sent them (HUD.state.abilities: the rows
-- GetActiveAbility lists; ability_levels: every ability's level). The character window's title ("Archery Novice")
-- is GetCombinedAbilityName over the names GetTargetAbilityTemplates answers, the same ids the character list shows.
local ABILITY_KEYS = { "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love" }
local function activeAbilities() return S().abilities or {} end
X2Ability.GetActiveAbility = function(self) return activeAbilities() end
X2Ability.NumActiveAbility = function(self) return #activeAbilities() end
X2Ability.IndexFromAbility = function(self, ability)
  for i, a in ipairs(activeAbilities()) do
    if a.type == ability then return i end
  end
  return nil
end
X2Ability.GetAbilityLevel = function(self, ability)
  local levels = S().ability_levels
  return levels and levels[ability] or 0
end
X2Ability.GetAllAbility = function(self)
  local all = {}
  for i = 1, 10 do all[i] = { type = i, level = X2Ability:GetAbilityLevel(i) } end
  return all
end
-- (another character's: the ids its unit state carried, HUD.target.abilities; nothing for NPCs, which hides the job icon)
X2Unit.GetTargetAbilityTemplates = function(self, unit)
  local list = {}
  if isPlayer(unit) then
    for i, a in ipairs(activeAbilities()) do list[i] = { name = ABILITY_KEYS[a.type] } end
    return list
  end
  local t = U(unit)
  if t == nil or t.unit_type ~= "character" or t.abilities == nil or #t.abilities == 0 then return nil end
  for i, id in ipairs(t.abilities) do list[i] = { name = ABILITY_KEYS[id] } end
  return list
end
-- Native GetResurrectionCountdown: server permission for in-place revival, five-minute automatic temple deadline,
-- and the separate short wait until the buttons enable. An offer does not restart the death timer.
X2Player.GetResurrectionCountdown = function(self)
  local left = HUD.death_wait()
  return HUD.can_resurrect_here(), HUD.death_timeout(), left
end
X2Player.Resurrect = function(self, inPlace) HUD.request("resurrect", inPlace and 1 or 0) end
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
-- The cast bars (player, target) ask what a unit is casting: spell, total ms, ms so far (cast.rs).
X2Unit.UnitCastingInfo = function(self, unit) return HUD.casting_info(unit) end

-- The loot window (x2ui/loot): the rows are the items left in the corpse the player opened (loot.rs).
X2Loot.GetNumLootItems = function(self) return HUD.loot_count() end
X2Loot.GetLootingBagItemInfo = function(self, k)
  local id, count, grade = HUD.loot_item(k)
  if id == nil then return nil end
  if id == 500 then return { money = count, itemType = 500, itemGrade = 0 } end
  return { name = HUD.item_info(id), itemType = id, itemGrade = grade, stackSize = count }
end
X2Loot.GetLootingBagItemTooltipText = function(self, k)
  local id, count, grade = HUD.loot_item(k)
  if id == nil then return {} end
  if id == 500 then return { isMoney = true } end
  return HUD.item_tip(id, grade, count) or {}
end
X2Loot.LootItem = function(self, k) HUD.loot_take(k) end
X2Loot.CloseLoot = function(self) HUD.loot_close() end
-- Numbers as text: "1234567" -> "1,234,567" (the chat prints the XP gained so); decimal strings add as numbers
-- (the gains of the floating "XP +N" text pile up while it shows).
X2Util.MakeMarkedCiphersStr = function(self, text, ciphers)
  text = tostring(text or "")
  local sign, int, rest = string.match(text, "^([%-+]?)(%d+)(.*)$")
  if not int then return text end
  local marked = string.reverse((string.gsub(string.reverse(int), "(%d%d%d)", "%1,")))
  return sign .. (string.gsub(marked, "^,", "")) .. rest
end
X2Util.StrNumericAdd = function(self, a, b)
  return string.format("%d", (tonumber(a) or 0) + (tonumber(b) or 0))
end

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

-- The zone map overlay (top right): the zone (or city) the player is in and where on its map image.
-- The map rectangle (zone group / sub zone `x, y, w, h`, y up) covers the current roadmap extent;
-- it; the images of map/road_map_list.lua sit in it at their own offset and size (HUD_ROAD_FRAMES, road_frames.lua,
-- or the script's FORM_ROAD_MAP_LIST for the zones whose numbers come from there). aa-ui wants the place on the
-- image: (current Map pixel - selected offset) / selected extent. At L the Nuian spawn is at window pixel
-- (703.2, 147.7) of the 926 x 556 window, where the real client puts the cursor.
local function onRoadImage(level, id, formKey, uw, vw)
  local f, w, h = HUD.road_layout(level, id, formKey)
  return HUD.road_image_point(f, w, h, uw, vw)
end

AAUI.RoadMap = function()
  local s = S()
  if s.zone_group == 0 then return nil end
  local x, y, _, yaw = HUD.player_pos()
  -- the heading: radians, 0 north, clockwise (the engine's yaw turns left)
  local angle = -yaw
  -- in a city (Ezna ...) the overlay shows the city's own map: level 3, the sub zone's id, its rectangle
  local c = s.sub_map_rect
  if s.sub_map ~= 0 and HUD_ROAD_FRAMES[3][s.sub_map] ~= nil then
    local u, v = onRoadImage(3, s.sub_map, "", (x - c[1]) / c[3], 1 - (y - c[2]) / c[4])
    return { level = 3, zone = s.sub_map, player = { u = u, v = v, angle = angle } }
  end
  local r = s.zone_rect
  local u, v = 0, 0
  if r[3] > 0 and r[4] > 0 then
    u, v = onRoadImage(2, s.zone_group, string.upper(s.zone_key_name), (x - r[1]) / r[3], 1 - (y - r[2]) / r[4])
  end
  return { level = 2, zone = s.zone_group, player = { u = u, v = v, angle = angle } }
end

-- The big map (M): the whole map image covers the rectangle of the map the player is on (zone group or sub zone
-- `x, y, w, h`, y up), so the place is the plain fraction of it; the maps above (the zone, the continent, the
-- world) get theirs from their rectangles (world_groups). Checked against reference/ingame/worldmap_*: the Nuian
-- spawn is u 0.7594 v 0.2657 on Solzreed, 0.6952 / 0.3418 on the western continent, 0.4688 / 0.6054 on the world.
local CONTINENT = { w = 3, e = 4, o = 5 } -- the zone group key's first letter -> world_groups.id

AAUI.WorldMap = function()
  local s = S()
  if s.zone_group == 0 then return nil end
  local x, y, _, yaw = HUD.player_pos()
  local function on(r)
    if r == nil or r[3] == nil or r[3] <= 0 then return nil end
    return (x - r[1]) / r[3], 1 - (y - r[2]) / r[4]
  end
  local zr = s.zone_rect
  local out = { level = 2, zone = s.zone_group, player = { angle = -yaw }, maps = {} }
  out.player.u, out.player.v = on(zr)
  if out.player.u == nil then return nil end
  local c = s.sub_map_rect
  if s.sub_map ~= 0 and HUD_ROAD_FRAMES[3][s.sub_map] ~= nil then
    -- in a city: its own map first, the zone's map is one of the maps above
    out.level, out.zone = 3, s.sub_map
    out.player.u, out.player.v = on(c)
    local zu, zv = on(zr)
    table.insert(out.maps, { level = 2, id = s.zone_group, u = zu, v = zv })
  end
  local continent = CONTINENT[string.sub(s.zone_key_name, 1, 1)]
  local cu, cv = on(continent and HUD.world_rect(continent))
  if cu ~= nil then table.insert(out.maps, { level = 1, id = continent, u = cu, v = cv }) end
  local wu, wv = on(HUD.world_rect(2))
  if wu ~= nil then table.insert(out.maps, { level = 0, id = 2, u = wu, v = wv }) end
  return out
end

-- Action bars and bag -----------------------------------------------------------------------------------
-- Slot widgets ask these hooks (AAUI.SlotContent ...); the lists come from aa-net. FALLBACK to the fake
-- of the capture while the server has not sent the action bars / the bag yet.
-- The 1-based page API: the main 12 slots follow the page (1..12, 49..60, 61..72, 73..84), the two
-- 6-slot sub bars stay at 13..24 (`FUN_392c5a50`, `(page + 3) * 12`).
local function actionSlotIndex(slotIdx)
  return HUD.action_slot(slotIdx) or slotIdx
end
X2Action.GetActionBarPage = function(self) return HUD.get_action_bar_page() end
X2Action.SetActionBarPage = function(self, page) return HUD.set_action_bar_page(page) end
local fakeSlotContent = AAUI.SlotContent
local function slotContent(s)
  local c = { type = s.kind, id = s.id, icon = s.icon, grade = s.grade }
  if s.kind == "item" then
    if s.stack then c.stack = s.count end
    c.canUse = s.count > 0 and s.blocked == nil
    c.tooltip = function() return HUD.item_tip(s.id, s.grade, s.count, s) end
  elseif s.kind == "skill" then
    -- dark and a red hotkey while the skill cannot be used now: no mana, too far, no target, dead ... (usable.rs)
    c.canUse = s.blocked == nil
    c.learned = HUD.book_learned(s.id)
    c.tooltip = HUD.skill_tip(s.id)
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
    local s = HUD.actions[actionSlotIndex(slotIdx)]
    if s == nil then return nil end
    return slotContent(s)
  elseif slotType == ISLOT_BAG then
    if HUD.bag_slots == 0 then return fakeSlotContent(slotType, slotIdx) end
    local s = HUD.bag[slotIdx]
    if s == nil then return nil end
    return slotContent(s)
  elseif slotType == ISLOT_MODE_ACTION then
    local s = HUD.mode_slot(slotIdx)
    if s == nil then return nil end
    local c = slotContent(s)
    c.learned = true
    return c
  end
  return nil
end
local fakeSkillContent, fakeItemContent = AAUI.SkillContent, AAUI.ItemContent
AAUI.SkillContent = function(skillType)
  local name, icon = HUD.skill_info(skillType)
  if icon == nil then return fakeSkillContent(skillType) end
  return { type = "skill", id = skillType, icon = icon, tooltip = HUD.skill_tip(skillType) }
end
AAUI.ItemContent = function(itemType, grade)
  local name, icon, maxStack, fixedGrade = HUD.item_info(itemType)
  if icon == nil then return fakeItemContent(itemType, grade) end
  return { type = "item", id = itemType, icon = icon, grade = grade or fixedGrade, tooltip = HUD.item_tip(itemType, grade) }
end

-- The description of an item's use skill ("Use ..." in the item tooltip) and of an action bar skill.
local fakeSkillTooltip = X2Skill.GetSkillTooltip
X2Skill.GetSkillTooltip = function(self, skillType, itemType, flags)
  local t = HUD.skill_tip(skillType)
  if t ~= nil then return t end
  return fakeSkillTooltip(self, skillType, itemType, flags)
end

-- Items for the bag and character windows -----------------------------------------------------------------
-- The windows read item data through X2Bag / X2Equipment / X2Item rather than the slot hooks. Real when the
-- server has sent the bag (HUD.bag_slots > 0), the fake of fake_bag.lua / fake_character.lua until then.
local GRADE_NAMES = { [0] = "poor", "common", "uncommon", "rare", "ancient", "heroic", "unique", "artifact",
                      "wonder", "epic", "legendary", "mythic" }
local function gradeFrame(grade)
  local name = GRADE_NAMES[grade]
  if name == nil then return nil end
  return string.format("ui/icon/item_grade_%d%s.dds", grade, name)
end
local function realInfo(s, flags)
  local tip = HUD.item_tip(s.id, s.grade, s.count, s, flags == nil and -1 or flags)
  if tip ~= nil then return tip end
  return {
    itemType = s.id, lookType = s.id, itemGrade = s.grade, name = s.name, description = s.desc, stack = s.count,
    level_requirement = s.level, price = 0, refund = 0, sellable = false,
  }
end
local function bagKnown() return HUD.bag_slots ~= 0 end
local function bagCount()
  local n = 0
  for _ in pairs(HUD.bag) do n = n + 1 end
  return n
end
local fake = {}
for _, name in ipairs({ "Capacity", "CountEmptyBagSlots", "CountItems", "IsInvalidSlot", "SlotByIdx", "GetBagItemInfo",
                        "ItemIdentifier", "ItemStack", "GetBagNumSlots", "Slots", "CurrentTabIdx", "TabInfo" }) do
  fake[name] = X2Bag[name]
end
X2Bag.Capacity = function(self) if not bagKnown() then return fake.Capacity(self) end return HUD.bag_slots end
X2Bag.GetBagNumSlots = function(self, id) if not bagKnown() then return fake.GetBagNumSlots(self, id) end return HUD.bag_slots end
X2Bag.Slots = function(self) if not bagKnown() then return fake.Slots(self) end return HUD.bag_slots end
X2Bag.CountItems = function(self) if not bagKnown() then return fake.CountItems(self) end return bagCount() end
X2Bag.CountEmptyBagSlots = function(self)
  if not bagKnown() then return fake.CountEmptyBagSlots(self) end
  return HUD.bag_slots - bagCount()
end
X2Bag.IsInvalidSlot = function(self, v)
  if not bagKnown() then return fake.IsInvalidSlot(self, v) end
  return v < 1 or v > HUD.bag_slots
end
-- the All tab shows the slots as they are; the others (default_inventory_tabs, or the player's own) show the
-- items of their item groups one after the other, in bag order
local function tabSlots(groups)
  local want = {}
  for _, g in ipairs(groups or {}) do want[g] = true end
  local list = {}
  for slot = 1, HUD.bag_slots do
    local s = HUD.bag[slot]
    if s ~= nil and want[s.group] then list[#list + 1] = slot end
  end
  return list
end
local tabCache = { bag = false, groups = false, list = {} }
X2Bag.SlotByIdx = function(self, v)
  if not bagKnown() then return fake.SlotByIdx(self, v) end
  local tab = fake.CurrentTabIdx(self)
  if tab == 1 or fake.TabInfo == nil then
    if HUD.bag[v] ~= nil then return v end
    return nil
  end
  -- one list per tab and bag change (the scripts ask for every slot in turn)
  local _, _, groups = fake.TabInfo(X2Bag, tab)
  if tabCache.bag ~= HUD.bag or tabCache.groups ~= groups then
    tabCache = { bag = HUD.bag, groups = groups, list = tabSlots(groups) }
  end
  return tabCache.list[v]
end
X2Bag.GetBagItemInfo = function(self, bagId, slot, ...)
  if not bagKnown() then return fake.GetBagItemInfo(self, bagId, slot, ...) end
  local s = HUD.bag[slot]
  if s == nil then return nil end
  local flags = -1
  if select('#', ...) ~= 0 then
    local value = (...)
    flags = type(value) == 'number' and value or 0
  end
  return realInfo(s, flags)
end
X2Bag.ItemIdentifier = function(self, slot)
  if not bagKnown() then return fake.ItemIdentifier(self, slot) end
  local s = HUD.bag[slot]
  if s == nil then return 0, 0 end
  return s.id, s.grade
end
X2Bag.ItemStack = function(self, slot)
  if not bagKnown() then return fake.ItemStack(self, slot) end
  local s = HUD.bag[slot]
  return s and s.count or 0
end

local fakeIconSet, fakeStackable, fakeLevel = X2Item.GetItemIconSet, X2Item.IsStackable, X2Item.GetLevelRequirement
local fakeItemName = X2Item.Name
X2Item.Name = function(self, itemType)
  local name = HUD.item_info(itemType)
  if name == nil then return fakeItemName(self, itemType) end
  return name
end
X2Item.GetItemIconSet = function(self, itemType, grade)
  local name, icon, maxStack, fixedGrade = HUD.item_info(itemType)
  if icon == nil or icon == "" then return fakeIconSet(self, itemType, grade) end
  return { itemIcon = icon, frameIcon = gradeFrame(grade or fixedGrade) }
end
X2Item.IsStackable = function(self, itemType)
  local name, icon, maxStack = HUD.item_info(itemType)
  if maxStack == nil then return fakeStackable(self, itemType) end
  return maxStack > 1
end
X2Item.GetLevelRequirement = function(self, itemType)
  local name, icon, maxStack, fixedGrade, level = HUD.item_info(itemType)
  if level == nil then return fakeLevel(self, itemType) end
  return level
end
local fakeItemInfoByType = X2Item.GetItemInfoByType
X2Item.GetItemInfoByType = function(self, itemType, grade)
  return HUD.item_tip(itemType, grade) or fakeItemInfoByType(self, itemType, grade)
end

-- Worn items: HUD.equipment[ES_*].
local fakeEquipText, fakeEquipInfo, fakeEquipId = X2Equipment.GetEquippedItemTooltipText, X2Equipment.GetEquippedItemTooltipInfo,
  X2Equipment.ItemIdentifier
local function equippedInfo(slot)
  local s = HUD.equipment[slot]
  if s == nil then return { itemType = 0, lookType = 0, itemGrade = 0 } end
  local info = realInfo(s)
  info.equip = true
  return info
end
X2Equipment.GetEquippedItemTooltipText = function(self, unit, slot)
  if not bagKnown() then return fakeEquipText(self, unit, slot) end
  return equippedInfo(slot)
end
X2Equipment.GetEquippedItemTooltipInfo = function(self, slot, flags)
  if not bagKnown() then return fakeEquipInfo(self, slot, flags) end
  -- Native comparisons omit empty slots; Text keeps its separate item-zero record.
  if HUD.equipment[slot] == nil then return end
  return equippedInfo(slot)
end
X2Equipment.ItemIdentifier = function(self, slot)
  if not bagKnown() then return fakeEquipId(self, slot) end
  local s = HUD.equipment[slot]
  if s == nil then return 0, 0 end
  return s.id, s.grade
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
-- Enter (or "/") opens the chat window's own edit box; the toolkit parses the slash commands natively
-- (`/s`, `/p`, `/1`, `/w Name ...`), keeps the channel for the next line and, on Enter, hands the line to
-- AAUI.ChatSend, which asks the app to send it. Escape closes the box.
local function chatEdit()
  local w = HUD.chat_window()
  if w == nil then return nil end
  return w:GetChatEdit()
end

HUD.chat_open = function(prefill)
  local window = HUD.chat_window()
  if window == nil then return end
  local edit = window:GetChatEdit()
  if edit == nil then return end
  AAUI.PrepareChatInput(window)
  edit:Show(true)
  edit:SetText(prefill or "")
  edit:SetFocus()
end

HUD.chat_close = function()
  local edit = chatEdit()
  if edit == nil then return end
  edit:SetText("")
  edit:ClearFocus()
  edit:Show(false)
end

AAUI.ChatSend = function(channel, text, target)
  text = string.gsub(text or "", "^%s+", "")
  if text ~= "" and not (HUD.slash and HUD.slash(text)) then HUD.request("chat", channel, text, target or "") end
end

-- A click on a bound slot (after the script's own OnClick): the action bar slot is used like its number
-- key; a right click on a bag item uses it (the bag window passes the real slot through X2Bag:SlotByIdx).
AAUI.UseSlot = function(slotType, slotIdx, button)
  if slotType == ISLOT_ACTION then
    HUD.request("use_action", slotIdx)
  elseif slotType == ISLOT_MODE_ACTION then
    HUD.request("use_mode_action", slotIdx)
  elseif slotType == ISLOT_PET_ACTION then
    HUD.request("use_pet_action", slotIdx)
  elseif slotType == ISLOT_BAG and button == "RightButton" then
    HUD.request("use_bag", slotIdx)
  end
end

HUD.setup_chat = function()
  local edit = chatEdit()
  if edit == nil then return end
  edit:SetHandler("OnEscapePressed", function(self) HUD.chat_close() end)
  edit:Show(false)
end
