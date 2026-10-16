-- The game API of the character-create stages (loginstage/character_create + customizing):
-- X2LoginCharacter, X2Customizer and the customizing unit, over the Rust state in `CC`
-- (api.rs). Step changes only record the request; the screen (mod.rs) moves the 3D camera and
-- raises the SHOW_CHARACTER_*_WINDOW events when the camera is on its way.

local raceStr = {
  [RACE_NUIAN] = "nuian", [RACE_FAIRY] = "fairy", [RACE_DWARF] = "dwarf", [RACE_ELF] = "elf",
  [RACE_HARIHARAN] = "hariharan", [RACE_FERRE] = "ferre", [RACE_RETURNED] = "returned",
  [RACE_WARBORN] = "warborn",
}
local genderStr = { [GENDER_MALE] = "male", [GENDER_FEMALE] = "female" }
local raceNum, genderNum = {}, {}
for k, v in pairs(raceStr) do raceNum[v] = k end
for k, v in pairs(genderStr) do genderNum[v] = k end

-- The client's feature flags the customizing screens read (as in the reference capture):
-- `beautyshopBypass` -> the "Save / Load Appearance" window (custom_saveload.alb) and no old
-- "Temp. Save" window.
X2Player.GetFeatureSet = function(self)
  return { dwarfWarborn = false, customsaveload = true, beautyshopBypass = true }
end

-- The race / gender being edited, as RACE_* / GENDER_* numbers.
local function editRG()
  return raceNum[CC.edit_race()], genderNum[CC.edit_gender()]
end

-- Race advantages shown under the race description (the passive buffs of the race).
-- (the scripts pass the race and gender names, `characterCreate.race`; numbers work too)
local function rname(r) return type(r) == "number" and raceStr[r] or r end
local function gname(g) return type(g) == "number" and genderStr[g] or g end
X2Ability.GetRaceBuffCount = function(self, race, gender)
  return #CC.race_buffs(rname(race), gname(gender))
end
X2Ability.GetRaceBuffType = function(self, race, gender, i)
  return CC.race_buffs(rname(race), gname(gender))[i]
end
X2Ability.GetBuffTooltip = function(self, buffType, level)
  local name, path, description = CC.buff_info(buffType)
  return { tipType = "passive", name = name or "", path = path or "", description = description or "" }
end

-- Name field: letters and digits (the server's rule), at most 18 of them; the displayed policy
-- is the client's own ("Enter 2-26 characters.").
X2Util.GetNamePolicyInfo = function(self, kind)
  return { local_min = 2, local_max = 26, eng_min = 2, eng_max = 26 }
end
X2Util.CheckAvailableName = function(self, text, composing)
  text = text or ""
  if composing then return text end
  return string.sub((string.gsub(text, "[^%w]", "")), 1, 18)
end
X2Util.IsValidName = function(self, text, kind)
  text = text or ""
  return #text >= 2 and #text <= 18 and string.find(text, "^%w+$") ~= nil
end

-- Step 1 --------------------------------------------------------------------------------------
X2LoginCharacter.GetEditUnitRace = function(self) return CC.edit_race() end
X2LoginCharacter.GetEditUnitGender = function(self) return CC.edit_gender() end
X2LoginCharacter.GetEditUnitName = function(self) return nil end
X2LoginCharacter.GetEditUnitAbility = function(self) return 0 end
X2LoginCharacter.ShowPreviewRaceGender = function(self, race, gender) CC.show_preview(race, gender) end
X2LoginCharacter.HidePreviewCharacterUnits = function(self) end
X2LoginCharacter.GotoCharacterSelect = function(self) CC.goto_select() end
X2LoginCharacter.EndCharacterCreate = function(self, gotoNext)
  if gotoNext then CC.end_step(1, true) else CC.goto_select() end
end
X2LoginCharacter.EndCharacterCustomize = function(self, gotoNext) CC.end_step(2, gotoNext) end
X2LoginCharacter.EndCharacterAbility = function(self, gotoNext) CC.end_step(3, gotoNext) end

-- Step 2: how many entries each list has. The icon sheets the scripts register in
-- cosmetic_list.lua (AddCustomizingStyle) have one cell per entry; the list is never longer than
-- the sheet.
local function styleCount(style, key)
  local race, gender = editRG()
  local sheet = GetCustomizingStyleCount(race, gender, style) or 0
  local n = CC.count(key)
  if style == HAIR_COLOR then return math.min(n, sheet) end
  return math.min(n, sheet > 0 and sheet or n)
end
X2Customizer.GetNumCustomHair = function(self) return styleCount(HAIR_TYPE, "hair") end
X2Customizer.GetNumCustomHairColor = function(self) return styleCount(HAIR_COLOR, "hair_color") end
X2Customizer.GetNumCustomizingEyebrow = function(self) return styleCount(EYEBROW_TYPE, "eyebrow") end
X2Customizer.GetNumCustomizingMakeUp = function(self) return styleCount(EYE_MAKEUP, "makeup") end
-- "Deco" is the cheek (female) or mustache (male) style, whichever the race / gender has.
X2Customizer.GetNumCustomizingDeco = function(self)
  local race, gender = editRG()
  local sheet = math.max(GetCustomizingStyleCount(race, gender, CHEEK_TYPE) or 0,
                         GetCustomizingStyleCount(race, gender, MUSTACHE_TYPE) or 0)
  return math.min(CC.count("deco"), sheet)
end
X2Customizer.GetNumCustomizingFaceDiffuse = function(self) return 0 end
X2Customizer.GetNumCustomSkinColor = function(self) return styleCount(SKIN_COLOR, "skin") end
X2Customizer.GetNumCustomizingFaceNormal = function(self) return styleCount(WRINKLE_TYPE, "wrinkle") end
X2Customizer.GetNumCustomizingTattoo = function(self) return styleCount(TATTOO_TYPE, "tattoo") end
X2Customizer.GetNumCustomizingScar = function(self) return styleCount(SCAR_TYPE, "scar") end
-- Premade faces / eyes / nose / mouth / head: 9 per kind and race (custom_face_presets).
X2Customizer.GetCustomPresetCount = function(self, kind) return 9 end
X2Customizer.IsCosmeticFix = function(self) return CC.fix() end
X2Customizer.FixCosmetic = function(self, fixed) CC.set_fix(fixed and true or false) end
-- Hair icon of the item list: the icon file of the hair style's item.
X2Customizer.GetCustomHairIcon = function(self, index) return CC.hair_icon(index) end
X2Customizer.SaveCurrentParam = function(self, slot) CC.save_slot(slot or 0) end
X2Customizer.LoadSavedParam = function(self, slot) CC.load_slot(slot or 0) end
X2Customizer.SetMixedParam = function(self, index) CC.set_mixed(index) end
X2Customizer.MixParam = function(self, kind, value, random) CC.mix(kind - 1, value) end
X2Customizer.InitSubParam = function(self, kind, page) end
X2Customizer.GetNumFaceTargets = function(self, part) return CC.num_targets(part) end
X2Customizer.GetFaceTargetIndex = function(self, part, order) return CC.target_index(part, order) end
X2Customizer.GetFaceTargetName = function(self, index) return CC.target_name(index) end
X2Customizer.GetFaceTargetMinValue = function(self, index) local mn = CC.target_range(index); return mn end
X2Customizer.GetFaceTargetMaxValue = function(self, index) local _, mx = CC.target_range(index); return mx end

-- Saved looks (the "Save / Load Appearance" window): see saveload below.
X2Customizer.GetSavedCustomDataCount = function(self) return CC.saved_count() end
X2Customizer.GetCustomDataInfoByIndex = function(self, index)
  local name, path = CC.saved_info(index)
  return { name = name or "", fullname = name or "", filepath = path or "" }
end
X2Customizer.LoadCustomFile = function(self) CC.saved_reload() end
X2Customizer.FindCustomSameNameFile = function(self, name)
  if CC.saved_exists(name) then return CDR_ERROR_AS_SAME_FILE end
  return CDR_INVALID_RESULT
end
X2Customizer.SaveCustomData = function(self, name) return CC.saved_save(name) end
X2Customizer.OverWriteCustomData = function(self, index) return CC.saved_overwrite(index) end
X2Customizer.LoadCustomData = function(self, index) return CC.saved_load(index) end
X2Customizer.DeleteCustomData = function(self, index) return CC.saved_delete(index) end
X2Customizer.HasTemporaryCustomData = function(self) return false end
X2Customizer.LoadTemporaryCustomData = function(self) end
X2Customizer.DeleteTemporaryCustomData = function(self) end

-- The unit being customized (the 3D preview character). The scripts take its getters/setters as
-- fields (`unit.GetCustomHair`) and call them with the unit as the first argument.
local STYLES = {
  { "Hair", "hair" }, { "HairColor", "hair_color" }, { "Eyebrow", "eyebrow" },
  { "EyebrowColor", "eyebrow_color", "rgb" }, { "PupilColor", "pupil_color", "rgb" },
  { "MakeUp", "makeup", "weight" }, { "LipColor", "lip_color", "rgb" }, { "Deco", "deco", "weight" },
  { "DecoColor", "deco_color", "rgb" }, { "FaceDiffuse", "diffuse" }, { "SkinColor", "skin" },
  { "FaceNormal", "wrinkle", "weight" }, { "Tattoo", "tattoo" }, { "Scar", "scar" },
}

local function newCustomUnit()
  local u = {}
  for _, s in ipairs(STYLES) do
    local name, key, kind = s[1], s[2], s[3]
    -- GetCustomHair(unit) -> index [, weight]; GetCustomEyebrowColor(unit) -> r, g, b
    -- (GetCustomizingDecoColor is the one getter spelled with "Customizing").
    local getter = (name == "DecoColor") and "GetCustomizingDecoColor" or ("GetCustom" .. name)
    u[getter] = function(self)
      if kind == "rgb" then return CC.get_rgb(key) end
      return CC.get(key)
    end
    -- SetCustomizingHair(unit, index [, weight]); SetCustomizingEyebrowColor(unit, r, g, b)
    u["SetCustomizing" .. name] = function(self, a, b, c, d, e)
      if kind == "rgb" then
        CC.set_rgb(key, a, b, c)
      elseif key == "scar" then
        CC.set_scar(a, b, c, d, e)
      else
        -- (unit, index, 0, 0, 1, 0) for compound styles, (unit, index, weight) for sliders
        CC.set(key, a, kind == "weight" and b or nil)
      end
    end
  end
  -- GetCustomMakeUp and the other weighted styles return (index, weight)
  u.GetScarStatus = function(self) return CC.scar() end
  u.ModifyFaceParamValue = function(self, index, value) CC.set_slider(index, value) end
  u.GetFaceTargetCurValue = function(self, index) return CC.slider(index) end
  u.SetSmile = function(self, smile) CC.set_smile(smile) end
  u.IsSmile = function(self) return CC.smile() end
  -- SetCustomizingParam(part, index, fixed): premade face (part 0), eyes, nose, mouth, head
  u.SetCustomizingParam = function(self, part, index, fixed) CC.apply_preset(part, index, fixed) end
  u.InitCustomizerControl = function(self) end
  u.ZoomInCustomizeCamera = function(self) CC.zoom(1) end
  u.ZoomOutCustomizeCamera = function(self) CC.zoom(-1) end
  return setmetatable(u, { __index = function(_, k) return function() end end })
end

local customUnit
X2LoginCharacter.GetCustomizingUnit = function(self)
  if customUnit == nil then customUnit = newCustomUnit() end
  return customUnit
end
X2LoginCharacter.RemoveUnit = function(self, unit)
  customUnit = nil
  CC.remove_unit()
end

-- Step 3 ---------------------------------------------------------------------------------------
-- The three starter skills per skillset (ABILITY_SKILL in loginstage/common.lua) as the client's
-- skill tooltip tables: { name, icon file in ui/icon, mana, cooldown ms, casting ms, range m },
-- from the game database.
local skillInfo = {
  [18132] = { "Triple Slash", "icon_skill_fight02", 30, 0, 0, 4 },
  [10455] = { "Frenzy", "icon_skill_fight10", 0, 90000, 0, 0 },
  [13282] = { "Whirlwind Slash", "icon_skill_fight09", 0, 12000, 0, 4 },
  [10752] = { "Flamebolt", "icon_skill_magic01", 16, 0, 1000, 20 },
  [11967] = { "Chain Lightning", "icon_skill_magic19", 0, 36000, 0, 20 },
  [10664] = { "Meteor Strike", "icon_skill_magic13", 0, 45000, 5000, 25 },
  [13564] = { "Piercing Shot", "icon_skill_vocation01", 0, 12000, 0, 20 },
  [13281] = { "Missile Rain", "icon_skill_wild10", 0, 36000, 2000, 50 },
  [10708] = { "Intensity", "icon_skill_wild09", 0, 60000, 0, 0 },
  [10534] = { "Antithesis", "icon_skill_love01", 0, 5000, 2000, 25 },
  [10546] = { "Revive", "icon_skill_love06", 0, 0, 7000, 25 },
  [10721] = { "Twilight", "icon_skill_love13", 0, 120000, 0, 3 },
  [10201] = { "Crippling Mire", "icon_skill_death11", 0, 18000, 0, 20 },
  [11441] = { "Absorb Lifeforce", "icon_skill_death05", 0, 36000, 0, 20 },
  [10434] = { "Summon Wraith", "icon_skill_death13", 0, 60000, 0, 3 },
  [10481] = { "Toxic Shot", "icon_skill_wild01", 0, 9000, 0, 20 },
  [12029] = { "Wallop", "icon_skill_vocation27", 0, 12000, 0, 4 },
  [12075] = { "Shadow Step", "icon_skill_vocation16", 0, 30000, 0, 20 },
}
X2LoginCharacter.GetSkillTooltip = function(self, skillId, level)
  local s = skillInfo[skillId] or { "Skill " .. tostring(skillId), "icon_skill_fight01", 0, 0, 0, 0 }
  -- Tooltip `ability` is the skillset's key in tooltip.abilityColor / locale.common.abilityCategory;
  -- the numbers FIGHT=1, DEATH=5, WILD=6, MAGIC=7, VOCATION=8, LOVE=10 index this list.
  local abilityKeys = { "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation",
                        "romance", "love" }
  local ability = "fight"
  for _, a in ipairs(ABILITY_TYPE) do
    for _, id in ipairs(ABILITY_SKILL[a]) do if id == skillId then ability = abilityKeys[a] end end
  end
  return { tipType = "skill", name = s[1], path = "ui/icon/" .. s[2] .. ".dds", ability = ability,
           skillLevel = 0, description = s[1], mana = s[3], cooldown = s[4], casting = s[5],
           channeling = 0, minRange = 0, maxRange = s[6], targetAreaRadius = 0 }
end
X2LoginCharacter.ShowSkillAction = function(self, ability) CC.skill_action(ability) end
-- NewCharacterCBT2(unit, { name, race, gender, ability1, ... }, enterIntro): true when the
-- request went out.
X2LoginCharacter.NewCharacterCBT2 = function(self, unit, info, enterIntro)
  return CC.create(info.name, info.ability1 or 0)
end
X2LoginCharacter.EditCharacterPOBT = function(self, unit, info) return false end
