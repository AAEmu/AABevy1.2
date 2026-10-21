-- Options are shared across stages and saved per account when the original UI calls Save.
local optionValues = {}
local optionDefaults = {}
local fallbackValue = X2Option.GetOptionItemValue
-- the native options (no item created by the scripts) whose 1.2 default is not 0, id -> value (`settings::native_default`): the window
-- shows them before anything is saved, and Reset sets them again
local nativeDefaults = NET.option_defaults and NET.option_defaults() or {}
local transaction
local function soundValue(id, value)
  if AAUI.OptionChanged and type(id) == "number" and type(value) == "number" then AAUI.OptionChanged(id, value) end
end
local function defaultOption(self, id, value)
  optionDefaults[id] = value
  if optionValues[id] == nil then
    local saved = NET.option_get(id)
    if saved ~= nil then value = saved end
    optionValues[id] = value
    soundValue(id, value)
  end
end
local function setOption(self, id, value)
  optionValues[id] = value
  NET.option_set(id, value)
  soundValue(id, value)
end
X2Option.CreateOptionItemFloat, X2Option.CreateOptionItemString = defaultOption, defaultOption
X2Option.SetItemDefaultFloatValue, X2Option.SetItemDefaultStringValue = defaultOption, defaultOption
X2Option.SetItemDefaultFloatValueByName, X2Option.SetItemDefaultStringValueByName = defaultOption, defaultOption
X2Option.SetItemFloatValue, X2Option.SetItemStringValue = setOption, setOption
X2Option.SetItemFloatValueByName, X2Option.SetItemStringValueByName = setOption, setOption
X2Option.SetItemFloatValueWithoutModify = setOption
X2Option.GetOptionItemValue = function(self, id)
  if id == OPTION_ITEM_SCREEN_RESOLUTION_WIDTH then return UIParent:GetScreenWidth() * UIParent:GetUIScale() end
  if id == OPTION_ITEM_SCREEN_RESOLUTION_HEIGHT then return UIParent:GetScreenHeight() * UIParent:GetUIScale() end
  if optionValues[id] ~= nil then return optionValues[id] end
  local saved = NET.option_get(id)
  if saved == nil and AAUI.OptionValue and type(id) == "number" then saved = AAUI.OptionValue(id) end
  if saved == nil and fallbackValue then saved = fallbackValue(self, id) end
  if saved == nil then saved = nativeDefaults[id] end
  if saved ~= nil then optionValues[id] = saved end
  return saved
end
X2Option.GetOptionItemValueByName = function(self, name)
  if optionValues[name] ~= nil then return optionValues[name] end
  local saved=NET.option_get(name)
  if saved ~= nil then optionValues[name]=saved end
  return saved
end
X2Option.Save = function(self)
  NET.save_settings()
  if transaction then transaction.done=true end
end
-- The original Reset reinitializes controls, including each control's originalValue.
-- Keep the opening values separately so Cancel/Escape/close can still undo a reset.
AA_CLIENT_OPTION_REMOVE_MODIFIED = function()
  local baseline={}
  for id,value in pairs(optionValues) do baseline[id]=value end
  transaction={done=false}
  local current=transaction
  local function restore()
    if current.done then return end
    for id,value in pairs(baseline) do setOption(nil,id,value) end
    current.done=true
  end
  local frame=optionFrame
  if frame then
    frame._aaOriginalCancel=frame._aaOriginalCancel or frame.Cancel
    frame.Cancel=function(self)
      if self._aaOriginalCancel then self:_aaOriginalCancel() end
      restore()
    end
    frame:SetHandler("OnCloseByEsc",frame.Cancel)
    frame._aaOriginalHide=frame._aaOriginalHide or frame.OnHide
    frame.OnHide=function(self)
      restore()
      if self._aaOriginalHide then self:_aaOriginalHide() end
    end
    frame:SetHandler("OnHide",frame.OnHide)
  end
end
X2Option.RemoveModifiedOption=AA_CLIENT_OPTION_REMOVE_MODIFIED
AA_CLIENT_OPTION_RESET = function()
  for id, value in pairs(optionDefaults) do setOption(nil, id, value) end
  for id, value in pairs(nativeDefaults) do if optionDefaults[id] == nil then setOption(nil, id, value) end end
end
X2Option.Reset = AA_CLIENT_OPTION_RESET
-- Apply saved sound volumes before a stage lazily opens its Options window.
for id = 35, 39 do
  local saved = NET.option_get(id)
  if saved ~= nil then soundValue(id, saved) end
end
X2Option.IsRestartOption = function(self) return false end
-- The options window is also initialized lazily when Start saves the login-stage settings.
-- Keep the same graphics defaults as the HUD until renderer settings are exposed here.
X2Option.GetNextSysSpecFullValue = function(self) return 4 end
X2Option.GetResolutionCount = function(self) return 1 end
X2Option.GetResolution = function(self, index)
  return UIParent:GetScreenWidth()*UIParent:GetUIScale(), UIParent:GetScreenHeight()*UIParent:GetUIScale(), 32
end
X2Option.EnumAAFormats = function(self)
  return { { samples = 2, quality = 0, txaa = false }, { samples = 4, quality = 0, txaa = false } }
end
X2Option.GetModifiedRestartOption = function(self) return false end
X2Option.GetMinxMaxOfMouseSensitivity = function(self) return 0.1, 3 end
