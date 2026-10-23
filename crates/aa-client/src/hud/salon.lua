-- The client side of the beauty salon (hud/salon.rs). The original window (x2ui/customizing/beautyshop) edits the
-- character through a "modelview" widget and X2Customizer; both are made here over the `CC` natives of the character
-- create screens (installed over the edited look when the salon is entered) and the `SALON` natives.

local raceNames = {
  { "RACE_NUIAN", "nuian" }, { "RACE_FAIRY", "fairy" }, { "RACE_DWARF", "dwarf" }, { "RACE_ELF", "elf" },
  { "RACE_HARIHARAN", "hariharan" }, { "RACE_FERRE", "ferre" }, { "RACE_RETURNED", "returned" },
  { "RACE_WARBORN", "warborn" },
}
local raceStr, genderStr, raceNum, genderNum = {}, {}, {}, {}
for _, r in ipairs(raceNames) do
  local v = _G[r[1]]
  if v ~= nil then raceStr[v] = r[2]; raceNum[r[2]] = v end
end
for _, g in ipairs({ { "GENDER_MALE", "male" }, { "GENDER_FEMALE", "female" } }) do
  local v = _G[g[1]]
  if v ~= nil then genderStr[v] = g[2]; genderNum[g[2]] = v end
end
-- the window compares the strings of the model view with these
if X2Unit:GetRaceStr(RACE_NUIAN) == nil then
  X2Unit.GetRaceStr = function(self, race) return raceStr[race] end
  X2Unit.GetGenderStr = function(self, gender) return genderStr[gender] end
end

-- The confirm dialog of the helper NPC (and of the cash shop, `bypass`): Ok casts the skill / enters directly.
function SALON_CONFIRM(bypass)
  local wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_CONFIRM_ENTER_BEAUTYSHOP, { bypass = bypass }, "")
  if wnd == nil then return end
  function wnd:OkProc()
    if bypass then SALON.bypass() else SALON.cast() end
  end
end

-- X2Customizer ------------------------------------------------------------------------------------------------------
local function editRG() return raceNum[CC.edit_race()], genderNum[CC.edit_gender()] end

-- how many entries a list has; the icon sheets (AddCustomizingStyle) have one cell per entry
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
X2Customizer.GetCustomPresetCount = function(self, kind) return 9 end
X2Customizer.IsCosmeticFix = function(self) return CC.fix() end
X2Customizer.FixCosmetic = function(self, fixed) CC.set_fix(fixed and true or false) end
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
X2Customizer.HasTemporaryCustomData = function(self) return false end
X2Customizer.LoadTemporaryCustomData = function(self) end
X2Customizer.DeleteTemporaryCustomData = function(self) end
-- (item type, how many the bag has, how many the new look needs)
X2Customizer.GetBeautyShopConfigInfo = function(self) return SALON.config() end
X2Customizer.TerminateBeautyShop = function(self) SALON.terminate() end

-- The model view -----------------------------------------------------------------------------------------------------
-- The scene is drawn by salon.rs (the stage of the character screens); the widget only carries the calls.
local STYLES = {
  { "Hair", "hair" }, { "HairColor", "hair_color" }, { "Eyebrow", "eyebrow" },
  { "EyebrowColor", "eyebrow_color", "rgb" }, { "PupilColor", "pupil_color", "rgb" },
  { "MakeUp", "makeup", "weight" }, { "LipColor", "lip_color", "rgb" }, { "Deco", "deco", "weight" },
  { "DecoColor", "deco_color", "rgb" }, { "FaceDiffuse", "diffuse" }, { "SkinColor", "skin" },
  { "FaceNormal", "wrinkle", "weight" }, { "Tattoo", "tattoo" }, { "Scar", "scar" },
}

local function installModelView()
  local probe = UIParent:CreateWidget("modelview", "salonModelViewProbe", "UIParent")
  probe:Show(false)
  local meta = getmetatable(probe)
  local m = meta and meta.__index
  if type(m) ~= "table" then
    UIParent:LogAlways("salon: no model view class table")
    return
  end
  for _, s in ipairs(STYLES) do
    local name, key, kind = s[1], s[2], s[3]
    -- GetCustomHair(view) -> index [, weight]; GetCustomEyebrowColor(view) -> r, g, b
    -- (GetCustomizingDecoColor is the one getter spelled with "Customizing")
    local getter = (name == "DecoColor") and "GetCustomizingDecoColor" or ("GetCustom" .. name)
    m[getter] = function(self)
      if kind == "rgb" then return CC.get_rgb(key) end
      return CC.get(key)
    end
    m["SetCustomizing" .. name] = function(self, a, b, c, d, e)
      if kind == "rgb" then
        CC.set_rgb(key, a, b, c)
      elseif key == "scar" then
        CC.set_scar(a, b, c, d, e)
      else
        CC.set(key, a, kind == "weight" and b or nil)
      end
    end
  end
  m.GetScarStatus = function(self) return CC.scar() end
  m.ModifyFaceParamValue = function(self, index, value) CC.set_slider(index, value) end
  m.GetFaceTargetCurValue = function(self, index) return CC.slider(index) end
  m.SetSmile = function(self, smile) CC.set_smile(smile) end
  m.SetCustomizingParam = function(self, part, index, fixed) CC.apply_preset(part, index, fixed) end
  m.InitCustomizerControl = function(self) end
  m.InitBeautyShop = function(self) end
  m.PlayAnimation = function(self, name, loop) end
  m.GetRace = function(self) return CC.edit_race() end
  m.GetGender = function(self) return CC.edit_gender() end
  m.ZoomInOutBeautyShop = function(self, dir) SALON.zoom(dir) end
  m.ResetBeautyShop = function(self) SALON.reset() end
  m.PayBeautyShop = function(self) SALON.pay() end
  -- the plastic parts are the types 2..5 of the windows: eyes, nose, mouth, head shape
  m.IsModifierChanged = function(self, kind)
    if kind < 2 then return false end
    return SALON.modifier_changed(kind - 1)
  end
  m.SetModifierToModel = function(self, kind, original)
    if kind >= 2 then SALON.modifier_original(kind - 1, original and true or false) end
  end
  m.RemoveEquipSlot = function(self, slot)
    if slot == ES_COSPLAY then SALON.hide_costume(true) end
  end
  m.SetEquipSlotFromCharacter = function(self, slot)
    if slot == ES_COSPLAY then SALON.hide_costume(false) end
  end
end

local ok, err = pcall(installModelView)
if not ok then UIParent:LogAlways("salon: " .. tostring(err)) end
