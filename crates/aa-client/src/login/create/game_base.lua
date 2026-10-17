-- The game API every login-stage script reads (X2Util, X2Unit, X2Option, ...), in Lua over the
-- objects `base.rs` registered. State lives in Rust (the `CC` table, see api.rs).

RACE_WARBORN = RACE_WARBORN or 8

local raceStr = {
  [RACE_NUIAN] = "nuian", [RACE_FAIRY] = "fairy", [RACE_DWARF] = "dwarf", [RACE_ELF] = "elf",
  [RACE_HARIHARAN] = "hariharan", [RACE_FERRE] = "ferre", [RACE_RETURNED] = "returned",
  [RACE_WARBORN] = "warborn",
}
local genderStr = { [GENDER_MALE] = "male", [GENDER_FEMALE] = "female" }

X2Util.GetGameProvider = function(self) return TRION end
X2Unit.GetRaceStr = function(self, race) return raceStr[race] end
X2Unit.GetGenderStr = function(self, gender) return genderStr[gender] end
X2Debug.GetDevMode = function(self) return false end
X2.GetFeatureset = function(self) return true end
X2.GetAddonInfos = function(self) return {} end
X2.GetRaceCongestions = function(self) return CC.race_congestions() end

Console.GetAttribute = function(self, name) return nil end
Console.ExecuteString = function(self, cmd) UIParent:LogAlways("console " .. tostring(cmd)) end

-- Account limits: the create screen only asks them in the "pre select" period, which 1.2 does
-- not have.
X2World.IsPreSelectCharacterPeriod = function(self) return false end
X2World.GetTotalCharactersCount = function(self) return 0 end
X2World.GetCurrentTotalCharactersCount = function(self) return 0 end
X2World.GetCurrentTotalCharactersLimit = function(self) return 6 end
X2World.GetTotalCharactersDefaultLimit = function(self) return 6 end
X2World.GetTotalCharactersExpandableLimit = function(self) return 0 end
X2LoginCharacter.GetWorldQueuePosition = function(self) return 0 end
X2LoginCharacter.StopLoginMovie = function(self) end
X2LoginCharacter.IsRecustomizing = function(self) return false end
