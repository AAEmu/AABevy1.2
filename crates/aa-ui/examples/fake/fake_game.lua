-- Development stand-ins for the client's game API (X2World, X2Unit, ...), so the original
-- login-stage scripts run without a server. The real app registers real objects instead.
-- `FAKE` (set from Rust before this runs) carries the scenario data.

RACE_NUIAN, RACE_FAIRY, RACE_DWARF, RACE_ELF, RACE_HARIHARAN, RACE_FERRE, RACE_RETURNED, RACE_WARBORN =
  1, 2, 3, 4, 5, 6, 7, 8
GENDER_MALE, GENDER_FEMALE = 1, 2

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
X2LoginCharacter.GetLastPlayedWorldInfo = function(self) return FAKE.last_world end
X2LoginCharacter.GetWorldQueueExpectedTime = function(self) return 120 end
X2LoginCharacter.GetWorldQueuePosition = function(self) return 3 end
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
