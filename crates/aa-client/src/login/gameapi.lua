-- The game API the login-stage scripts (world select, character select, loading) read, over
-- the `NET` table of Rust functions that gameapi.rs registers:
--   NET.worlds()   world list          NET.chars()   character list     NET.account()  account limits
--   NET.queue()    world queue or nil  NET.last_played()  index or nil   NET.world_name()  connected world
--   NET.now()      unix seconds        NET.request(kind, a, b)  ask the app to do something
-- State lives in Rust (the Bevy resources of aa-net); everything here is glue.

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
X2.GetRaceCongestions = function(self) return NET.race_congestions() end
X2.GetFeatureset = function(self) return true end
X2.GetAddonInfos = function(self) return {} end
X2.IsEnteredWorld = function(self) return false end

Console.GetAttribute = function(self, name) return nil end
Console.ExecuteString = function(self, cmd)
  if cmd == "quit" then NET.request("quit") end
end

-- World select ---------------------------------------------------------------------------------
X2World.GetWorldInfo = function(self) return NET.worlds() end
X2World.GetWorldCharactersCount = function(self, id)
  for _, w in ipairs(NET.worlds()) do
    if w.id == id then return w.characters end
  end
  return 0
end
X2World.IsPreSelectCharacterPeriod = function(self) return NET.account().pre_select end
X2World.GetTotalCharactersCount = function(self) return NET.account().total_characters end
X2World.GetCurrentTotalCharactersCount = function(self) return #NET.chars() end
X2World.GetCurrentTotalCharactersLimit = function(self) return NET.account().max_characters end
X2World.GetTotalCharactersDefaultLimit = function(self) return NET.account().max_characters end
X2World.GetTotalCharactersExpandableLimit = function(self) return NET.account().extra_characters end
X2World.GetExpandedCharacterCount = function(self) return 0 end
X2World.GetZoneCount = function(self) return 0 end
X2World.GetCurrentWorldName = function(self) return NET.world_name() end
X2World.EnterWorld = function(self, worldId, zoneId) NET.request("enter_world", worldId, zoneId) end
X2World.RequestWorldListRefresh = function(self) NET.request("refresh_worlds") end
-- Back to the server list (EXIT_TO_WORLD_LIST); true = the request went out.
X2World.LeaveWorld = function(self, how)
  NET.request("to_world_select")
  return true
end
X2World.CancelLeaveWorld = function(self) end

-- The last played world: the first one with characters (its name).
X2LoginCharacter.GetLastPlayedWorldInfo = function(self)
  for _, w in ipairs(NET.worlds()) do
    if w.characters > 0 then return w.name end
  end
  return nil
end
X2LoginCharacter.CancelWorldQueue = function(self) NET.request("cancel_queue") end
X2LoginCharacter.ConnectToWorld = function(self) NET.request("connect") end
X2LoginCharacter.GetWorldQueuePosition = function(self)
  local q = NET.queue()
  return q and q.position or 0
end
-- The server sends no expected wait: the client estimates it from the place in line (~5 s each).
X2LoginCharacter.GetWorldQueueExpectedTime = function(self)
  local q = NET.queue()
  if q == nil then return -1 end
  return q.position * 5
end
X2LoginCharacter.UsePremiumEntrance = function(self) return false end
X2LoginCharacter.IsInPremiumQueue = function(self)
  local q = NET.queue()
  return q ~= nil and q.premium
end
X2LoginCharacter.GetWorldNormalQueueLength = function(self)
  local q = NET.queue()
  return q and q.normal or 0
end
X2LoginCharacter.GetWorldPremiumQueueLength = function(self)
  local q = NET.queue()
  return q and q.premium_length or 0
end

-- Character select ---------------------------------------------------------------------------
local function ch(i) return NET.chars()[i] end
X2LoginCharacter.GetNumLoginCharacters = function(self) return #NET.chars() end
X2LoginCharacter.GetLastPlayedCharacterIndex = function(self) return NET.last_played() end
X2LoginCharacter.GetLpManagedCharacterIdx = function(self) return NET.lp_managed() end
X2LoginCharacter.RequestLpManageCharacter = function(self, i) NET.request("lp_manage", i) end
X2LoginCharacter.IsUniverseLp = function(self) return true end
X2LoginCharacter.GetLoginCharacterName = function(self, i) return ch(i) and ch(i).name end
X2LoginCharacter.GetLoginCharacterLevel = function(self, i) return ch(i) and ch(i).level end
X2LoginCharacter.GetLoginCharacterAbilities = function(self, i)
  return ch(i) and ch(i).abilities or {}
end
X2LoginCharacter.GetLoginCharacterRace = function(self, i) return ch(i) and ch(i).race end
X2LoginCharacter.GetLoginCharacterGender = function(self, i) return ch(i) and ch(i).gender end
X2LoginCharacter.GetLoginCharacterZone = function(self, i) return ch(i) and ch(i).zone end
X2LoginCharacter.GetLoginCharacterFaction = function(self, i) return ch(i) and ch(i).faction_id end
X2LoginCharacter.GetLoginCharacterFactionName = function(self, i) return ch(i) and ch(i).faction end
X2LoginCharacter.GetLoginCharacterMoney = function(self, i) return ch(i) and ch(i).money end
X2LoginCharacter.GetLoginCharacterLaborPower = function(self, i) return ch(i) and ch(i).labor end
X2LoginCharacter.GetLoginCharacterMaxLaborPower = function(self, i) return 2000 end
X2LoginCharacter.GetLoginCharacterBmPoint = function(self, i) return ch(i) and ch(i).bm_point end
X2LoginCharacter.GetLoginCharacterPremiumGrade = function(self, i) return ch(i) and ch(i).premium_grade or 1 end
X2LoginCharacter.GetLoginCharacterInfoHouse = function(self, i, h) return ch(i) and ch(i).houses[h] end
X2LoginCharacter.GetLoginCharacterInfo = function(self, i)
  local c = ch(i)
  if c == nil then return nil end
  return { validCharPos = true, inDirtyZone = false, posX = c.x, posY = c.y, houseCount = #c.houses }
end
X2LoginCharacter.IsDeleteRequestedCharacter = function(self, i)
  return ch(i) ~= nil and ch(i).delete_remain ~= nil
end
X2LoginCharacter.IsTransferRequestedCharacter = function(self, i) return ch(i) ~= nil and ch(i).transfer end
X2LoginCharacter.IsInEnableStartingLocation = function(self, i) return true end
X2LoginCharacter.CanShowClientDrivenSkipDialog = function(self, i) return false end
X2LoginCharacter.GetCharacterDeleteWaitingTime = function(self, i) return ch(i) and ch(i).delete_remain end
X2LoginCharacter.IsRecustomizing = function(self) return false end
X2LoginCharacter.RequestCharacterListRefresh = function(self, force) NET.request("refresh_characters") end
-- The 3D character of the select screen follows the selected slot.
X2LoginCharacter.ShowSelectedCharacter = function(self, i) NET.request("show_character", i) end
-- Start: true = the request went out and the screen goes to loading.
X2LoginCharacter.SelectCharacter = function(self, i, skip)
  NET.request("start_game", i)
  return true
end
-- An empty slot was clicked: the create screens.
X2LoginCharacter.CreateCharacter = function(self) NET.request("create") end
X2LoginCharacter.DeleteCharacterCheckName = function(self, i, name) NET.request("delete", i, name) end
X2LoginCharacter.CancelCharacterDelete = function(self, i) NET.request("cancel_delete", i) end
