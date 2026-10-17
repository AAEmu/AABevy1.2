-- The skill window's data (x2ui/skill: the combat tab's three ability columns, the basic and emote tabs): the `X2Ability`
-- calls the scripts make and the content of the window's `ISLOT_ABILITY_VIEW` slots, over the skill book of skillbook.rs
-- (HUD.book_*). Loaded after ingame.lua; it replaces the fake of aa-ui/examples/fake/fake_skill.lua, which has the same shapes.
--
-- Slot numbers are the scripts' kinds plus the position: ACTIVE_SKILL_n (10, 30, 50) + 1..15 and PASSIVE_SKILL_n (70, 80, 90)
-- + 1..8 for the three columns, ATTACK_SKILL (100), GENERAL_SKILL (110), JOB_SKILL (130) and EMOTION_SKILL (140) + 1.. for the
-- flat lists. A column shows the ability of its "view": the character's active abilities in order, or an ability the player is
-- looking at before learning it (SetAbilityToView).

local KEYS = { "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love" }
local ACTIVE_BASE = { ACTIVE_SKILL_1, ACTIVE_SKILL_2, ACTIVE_SKILL_3 }
local PASSIVE_BASE = { PASSIVE_SKILL_1, PASSIVE_SKILL_2, PASSIVE_SKILL_3 }
local MAX_ACTIVE, MAX_PASSIVE = 15, 8 -- slots per column (tab_combat.lua)
local FLAT = { ATTACK_SKILL, GENERAL_SKILL, JOB_SKILL, EMOTION_SKILL }

-- The skill window only opens when the client is not a demo.
X2Demo.GetDemoOptionCount = function(self) return 0 end

-- The ability view --------------------------------------------------------------------------------------------
local view
local function views()
  if view == nil then
    view = {}
    for i, a in ipairs(X2Ability:GetActiveAbility()) do view[i] = a.type end
  end
  return view
end
X2Ability.GetAbilityFromView = function(self, index) return views()[index] end
X2Ability.SetAbilityToView = function(self, index, ability)
  if ability == nil or ability >= ABILITY_MAX then
    views()[index] = nil
  else
    views()[index] = ability
  end
end
X2Ability.ResetAbilityView = function(self) view = nil end
X2Ability.IsActiveAbility = function(self, ability)
  for _, a in ipairs(X2Ability:GetActiveAbility()) do
    if a.type == ability then return true end
  end
  return false
end

-- Skill points: (total, spent); asked for an ability, (the character's total, spent in that ability's tree).
X2Ability.GetSkillPoint = function(self, ability) return HUD.book_points(ability) end

-- The trees ---------------------------------------------------------------------------------------------------
-- One table per ability, rebuilt when anything the window shows changed (HUD.book_version).
local EMPTY = { active = {}, passive = {} }
local cache, cacheVersion, tips = {}, nil, {}
local function tree(ability)
  local v = HUD.book_version()
  if v ~= cacheVersion then cache, cacheVersion, tips = {}, v, {} end
  local t = cache[ability]
  if t == nil then
    t = HUD.book_tree(ability)
    if t == nil then return EMPTY end -- the database is not read yet
    cache[ability] = t
  end
  return t
end

local flat = {}
local function basic(kind)
  if flat[kind] == nil then flat[kind] = HUD.book_basic(kind) end
  return flat[kind]
end

-- What a slot number is: "active" / "passive" (+ the row and the ability) or "basic" (+ the row).
local function entry(idx)
  local v = views()
  for col = 1, 3 do
    local ability = v[col]
    local base = ACTIVE_BASE[col]
    if idx > base and idx <= base + MAX_ACTIVE then
      if ability == nil then return nil end
      return "active", tree(ability).active[idx - base], ability
    end
    base = PASSIVE_BASE[col]
    if idx > base and idx <= base + MAX_PASSIVE then
      if ability == nil then return nil end
      return "passive", tree(ability).passive[idx - base], ability
    end
  end
  for _, kind in ipairs(FLAT) do
    if idx > kind and idx <= kind + #basic(kind) then return "basic", basic(kind)[idx - kind] end
  end
  return nil
end

-- Number of slots of a kind (the scripts show that many and hide the rest).
X2Ability.GetAbilitySlotCount = function(self, kind)
  local v = views()
  for col = 1, 3 do
    local ability = v[col]
    if kind == ACTIVE_BASE[col] then return ability and #tree(ability).active or 0 end
    if kind == PASSIVE_BASE[col] then return ability and #tree(ability).passive or 0 end
  end
  for _, k in ipairs(FLAT) do
    if kind == k then return #basic(kind) end
  end
  return 0
end
-- The skill's name next to its icon (the basic and emote tabs).
X2Ability.GetAbilitySlotName = function(self, idx)
  local _, e = entry(idx)
  if e == nil then return nil end
  return e.name
end

-- Slot contents -----------------------------------------------------------------------------------------------
-- The tooltips of the original's skill window carry what is needed to learn: for a skill not learned yet the ability level and
-- the points (`firstLearnLevel`, `skillPoints`, shown with `showReq`), for a passive buff the level and the points the tree
-- must have spent (`passive_buff_learn_level`, `reqPoints`).
local function activeTip(e)
  local key = "a" .. e.id
  if tips[key] == nil then
    local tip = HUD.skill_tip(e.id)
    if tip ~= nil then
      tip.skillLevel = e.learned and e.rank or 0
      if not e.learned then
        tip.showReq = true
        tip.firstLearnLevel = e.level
        tip.skillPoints = e.points
      end
    end
    tips[key] = tip or false
  end
  return tips[key] or nil
end

local function passiveTip(e, ability)
  local key = "p" .. e.id
  if tips[key] == nil then
    tips[key] = {
      tipType = "passive", category = "passive", name = e.name, path = e.icon, ability = KEYS[ability],
      description = HUD.buff_text(e.buff, 0) or e.desc, passive_buff_learn_level = e.level, reqPoints = e.req,
      skillPoints = 1, learn = e.learned and "master" or nil,
    }
  end
  return tips[key]
end

local function viewContent(idx)
  local kind, e, ability = entry(idx)
  if e == nil then return nil end
  if kind == "active" then
    return {
      type = "skill", id = e.id, icon = e.icon, canUse = e.learned, learned = e.learned, canLearn = e.can,
      skillLevel = e.learned and e.rank or 0, tooltip = activeTip(e),
    }
  elseif kind == "passive" then
    return {
      type = "buff", id = e.id, icon = e.icon, canUse = false, learned = e.learned, canLearn = e.can, reqPoints = e.req,
      tooltip = passiveTip(e, ability),
    }
  end
  return { type = "skill", id = e.id, icon = e.icon, canUse = true, tooltip = HUD.skill_tip(e.id) }
end

local baseSlotContent = AAUI.SlotContent
AAUI.SlotContent = function(slotType, slotIdx)
  if slotType == ISLOT_ABILITY_VIEW then return viewContent(slotIdx) end
  return baseSlotContent(slotType, slotIdx)
end

-- Learning ------------------------------------------------------------------------------------------------------
-- A left click on a skill or passive buff that can be learned asks first (DLG_TASK_LEARN_SKILL / DLG_TASK_LEARN_BUFF:
-- "Learn [name] of [ability]? Points: 1"); OK sends the learn.
local function askLearn(idx)
  local kind, e = entry(idx)
  if e == nil or e.learned or not e.can then return end
  local wnd
  if kind == "active" then
    wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_LEARN_SKILL, { skillType = e.id }, "skillWnd")
    if wnd ~= nil then wnd.OkProc = function() HUD.request("learn_skill", e.id) end end
  elseif kind == "passive" then
    wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_LEARN_BUFF, { passiveBuffType = e.id }, "skillWnd")
    if wnd ~= nil then wnd.OkProc = function() HUD.request("learn_buff", e.id) end end
  end
end

local baseUseSlot = AAUI.UseSlot
AAUI.UseSlot = function(slotType, slotIdx, button)
  if slotType == ISLOT_ABILITY_VIEW then
    if button == "LeftButton" then askLearn(slotIdx) end
    return
  end
  baseUseSlot(slotType, slotIdx, button)
end

-- What the dialogs ask the game.
X2Skill.Info = function(self, skillType)
  local name, ability, points = HUD.book_skill(skillType)
  return { type = skillType, name = name or "", abilityName = ability or "fight", skillPoints = points or 1 }
end
X2Ability.GetPassiveBuffInfo = function(self, passiveType)
  local buff, ability = HUD.book_passive(passiveType)
  return { abilityName = ability or "fight", buffType = buff or 0 }
end
X2Ability.GetBuffTooltip = function(self, buffType, level)
  local name, icon, desc = HUD.book_buff(buffType)
  return { tipType = "passive", category = "passive", name = name or "", path = icon or "", description = HUD.buff_text(buffType, level or 0) or desc or "" }
end

-- Choosing a new ability for an empty column: the server swaps "no ability" (ABILITY_MAX) for it. Resetting a tree
-- asks first (DLG_TASK_RESET_SKILLS); the server gives the points back for nothing.
X2Ability.LearnAbility = function(self, ability) HUD.request("swap_ability", ABILITY_MAX, ability) end
X2Ability.AskResetSkills = function(self, index)
  local ability = X2Ability:GetAbilityFromView(index)
  if ability == nil then return end
  local wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_RESET_SKILLS, { cost = X2Ability:GetResetSkillsCost() }, "skillWnd")
  if wnd ~= nil then wnd.OkProc = function() HUD.request("reset_skills", ability) end end
end
