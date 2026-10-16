-- Development stand-ins for the client's game API (X2World, X2Unit, ...), so the original
-- login-stage scripts run without a server. The real app registers real objects instead.
-- `FAKE` (set from Rust before this runs) carries the scenario data.

-- RACE_* / GENDER_* are the client's constants (installed by aa-ui).
RACE_WARBORN = RACE_WARBORN or 8

local raceStr = {
  [RACE_NUIAN] = "nuian", [RACE_FAIRY] = "fairy", [RACE_DWARF] = "dwarf", [RACE_ELF] = "elf",
  [RACE_HARIHARAN] = "hariharan", [RACE_FERRE] = "ferre", [RACE_RETURNED] = "returned",
  [RACE_WARBORN] = "warborn",
}
local genderStr = { [GENDER_MALE] = "male", [GENDER_FEMALE] = "female" }

X2Util.GetGameProvider = function(self) return TRION end
X2Locale.GetLocale = function(self) return FAKE.locale end
X2Locale.LocalizeUiText = function(self, category, key, ...)
  local s = FAKE.ui_text(category, key)
  if s == nil then return key end
  local args = { ... }
  if #args > 0 then
    -- $1, $2 ... placeholders.
    s = string.gsub(s, "%$(%d)", function(n) return tostring(args[tonumber(n)] or "") end)
  end
  return s
end
X2Player.GetFeatureSet = function(self) return { dwarfWarborn = false } end
X2Unit.GetRaceStr = function(self, race) return raceStr[race] end
X2Unit.GetGenderStr = function(self, gender) return genderStr[gender] end
X2Debug.GetDevMode = function(self) return false end
X2.GetRaceCongestions = function(self) return { 0, 0, 0, 0 } end
X2.GetFeatureset = function(self) return true end
X2.GetAddonInfos = function(self) return {} end

-- World select -----------------------------------------------------------------------------
X2World.GetWorldInfo = function(self)
  local out = {}
  for i, w in ipairs(FAKE.worlds) do
    local rc = {}
    for _, r in pairs(raceStr) do rc[r] = w.raceCongestion or 0 end
    out[i] = { id = w.id, name = w.name, available = w.available, congestion = w.congestion,
               raceCongestions = rc }
  end
  return out
end
X2World.GetWorldCharactersCount = function(self, id)
  for _, w in ipairs(FAKE.worlds) do
    if w.id == id then return w.characters end
  end
  return 0
end
X2World.IsPreSelectCharacterPeriod = function(self) return false end
X2World.GetTotalCharactersCount = function(self) return 1 end
X2World.GetCurrentTotalCharactersCount = function(self) return 1 end
X2World.GetCurrentTotalCharactersLimit = function(self) return 6 end
X2World.GetTotalCharactersDefaultLimit = function(self) return 6 end
X2World.GetTotalCharactersExpandableLimit = function(self) return 0 end
X2World.GetZoneCount = function(self) return 0 end
X2World.EnterWorld = function(self, worldId, zoneId)
  UIParent:LogAlways(string.format("fake: EnterWorld(%s, %s)", tostring(worldId), tostring(zoneId)))
end
X2World.RequestWorldListRefresh = function(self) UIParent:LogAlways("fake: RequestWorldListRefresh") end
-- The last played world: the first one with characters (its name).
X2LoginCharacter.GetLastPlayedWorldInfo = function(self)
  for _, w in ipairs(FAKE.worlds) do
    if w.characters > 0 then return w.name end
  end
  return nil
end
X2LoginCharacter.GetWorldQueueExpectedTime = function(self) return 120 end
X2LoginCharacter.GetWorldQueuePosition = function(self) return 0 end
X2LoginCharacter.UsePremiumEntrance = function(self) return false end

-- Options: the screen resolution is the window's; script options are unset (nil), so the
-- scripts use their defaults like on a fresh install.
X2Option.GetOptionItemValue = function(self, id)
  if id == OPTION_ITEM_SCREEN_RESOLUTION_WIDTH then return UIParent:GetScreenWidth() * UIParent:GetUIScale() end
  if id == OPTION_ITEM_SCREEN_RESOLUTION_HEIGHT then return UIParent:GetScreenHeight() * UIParent:GetUIScale() end
  return nil
end
X2Option.GetOptionItemValueByName = function(self, name) return nil end

Console.GetAttribute = function(self, name) return nil end
Console.ExecuteString = function(self, cmd) UIParent:LogAlways("fake: console " .. tostring(cmd)) end

-- Character select ---------------------------------------------------------------------------
local function ch(i) return FAKE.characters[i] end
X2World.GetCurrentWorldName = function(self) return FAKE.worlds[1] and FAKE.worlds[1].name or "" end
X2World.GetExpandedCharacterCount = function(self) return 0 end
X2World.LeaveWorld = function(self, how) UIParent:LogAlways("fake: LeaveWorld") return true end
X2LoginCharacter.GetNumLoginCharacters = function(self) return #FAKE.characters end
X2LoginCharacter.GetLastPlayedCharacterIndex = function(self) return #FAKE.characters > 0 and 1 or nil end
X2LoginCharacter.GetLpManagedCharacterIdx = function(self) return 0 end
X2LoginCharacter.IsUniverseLp = function(self) return true end
X2LoginCharacter.GetLoginCharacterName = function(self, i) return ch(i) and ch(i).name end
X2LoginCharacter.GetLoginCharacterLevel = function(self, i) return ch(i) and ch(i).level end
X2LoginCharacter.GetLoginCharacterAbilities = function(self, i)
  local c = ch(i)
  return c and { c.ability, ABILITY_MAX, ABILITY_MAX } or {}
end
X2LoginCharacter.GetLoginCharacterRace = function(self, i) return ch(i) and ch(i).race end
X2LoginCharacter.GetLoginCharacterGender = function(self, i) return ch(i) and ch(i).gender end
X2LoginCharacter.GetLoginCharacterZone = function(self, i) return ch(i) and ch(i).zone end
X2LoginCharacter.GetLoginCharacterFaction = function(self, i) return 1 end
X2LoginCharacter.GetLoginCharacterFactionName = function(self, i) return ch(i) and ch(i).faction end
X2LoginCharacter.GetLoginCharacterMoney = function(self, i) return "0" end
X2LoginCharacter.GetLoginCharacterLaborPower = function(self, i) return 85 end
X2LoginCharacter.GetLoginCharacterMaxLaborPower = function(self, i) return 2000 end
X2LoginCharacter.GetLoginCharacterBmPoint = function(self, i) return 0 end
X2LoginCharacter.GetLoginCharacterPremiumGrade = function(self, i) return 0 end
X2LoginCharacter.GetLoginCharacterInfoHouse = function(self, i, h) return {} end
X2LoginCharacter.GetLoginCharacterInfo = function(self, i)
  local c = ch(i)
  if c == nil then return nil end
  return { validCharPos = true, inDirtyZone = false, posX = c.x or 15578, posY = c.y or 15382, houseCount = 0 }
end
X2LoginCharacter.IsDeleteRequestedCharacter = function(self, i) return false end
X2LoginCharacter.IsTransferRequestedCharacter = function(self, i) return false end
X2LoginCharacter.IsInEnableStartingLocation = function(self, i) return true end
X2LoginCharacter.CanShowClientDrivenSkipDialog = function(self, i) return false end
X2LoginCharacter.GetCharacterDeleteWaitingTime = function(self, i) return 0 end
X2LoginCharacter.IsInPremiumQueue = function(self) return false end
X2LoginCharacter.GetWorldNormalQueueLength = function(self) return 0 end
X2LoginCharacter.GetWorldPremiumQueueLength = function(self) return 0 end
X2LoginCharacter.IsRecustomizing = function(self) return false end
X2LoginCharacter.RequestCharacterListRefresh = function(self, force)
  FAKE.fire_later("LOGIN_CHARACTER_UPDATED", "refreshed")
end
X2LoginCharacter.SelectCharacter = function(self, i, skip)
  UIParent:LogAlways("fake: SelectCharacter(" .. tostring(i) .. ")")
  return true
end
X2LoginCharacter.CreateCharacter = function(self) UIParent:LogAlways("fake: CreateCharacter") end
