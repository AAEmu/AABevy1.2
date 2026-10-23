-- The slash commands of the chat input that the toolkit does not switch channels for (see commands.rs).
-- HUD.slash(text) is called by AAUI.ChatSend with the line typed; it returns true when the line was a command
-- and is done with, false when it is an ordinary line.
--
-- Teams, friends, family and the guild go through the X2* methods the original windows call, so their checks
-- and answers are the windows' own.

local function system(text)
  X2Chat:DispatchChatMessage(CMF_SYSTEM, text)
end

-- "Name rest of the line" -> "Name", "rest of the line"
local function split(args)
  local first, rest = string.match(args or "", "^(%S+)%s*(.*)$")
  return first, rest or ""
end

-- The name typed, else the name of the targeted player.
local function nameOrTarget(args)
  local name = split(args)
  if name == nil then name = X2Unit:UnitName("target") end
  if name == nil or name == "" then return nil end
  return name
end

local function invite(args, party)
  local name = nameOrTarget(args)
  if name ~= nil then X2Team:InviteToTeam(name, party) end
end

local function teamInvite(forced)
  return function(args)
    local party = forced
    if party == nil then party = not X2Team:IsRaidTeam() end
    invite(args, party)
  end
end

-- A team command that only applies to a party (`kind` true) or a raid (false); nil: either.
local function inTeam(kind)
  if kind == nil then return true end
  if kind then return X2Team:IsPartyTeam() end
  return X2Team:IsRaidTeam()
end

local function teamLeave(kind)
  return function()
    if inTeam(kind) then X2Team:LeaveTeam() end
  end
end

local function teamKick(kind)
  return function(args)
    local name = split(args)
    if name ~= nil and inTeam(kind) then X2Team:KickTeamMemberByName(name) end
  end
end

-- name of a player channel -> the identity the server knows it by
HUD.user_channels = HUD.user_channels or {}

local function nameAndPassword(args)
  local name, password = split(args)
  return name, password
end

local functions = {
  -- 1 invite, 2 leave, 3 kick: party or raid, whichever the player is in
  [1] = teamInvite(nil),
  [2] = teamLeave(nil),
  [3] = teamKick(nil),
  -- 4 create a channel, 5 join, 6 leave
  [4] = function(args)
    local name, password = nameAndPassword(args)
    if name ~= nil then HUD.chat_command("join", name, password or "", true) end
  end,
  [5] = function(args)
    local name, password = nameAndPassword(args)
    if name ~= nil then HUD.chat_command("join", name, password or "", false) end
  end,
  [6] = function(args)
    local name = split(args)
    local identity = name ~= nil and HUD.user_channels[string.lower(name)] or nil
    if identity ~= nil then HUD.chat_command("leave", identity) end
  end,
  -- 7 roll the dice: /roll [sides], 100 by default
  [7] = function(args)
    local first, rest = split(args)
    local sides = tonumber(first)
    if sides ~= nil then
      -- "/roll 1 20" rolls 1-20
      local second = tonumber((split(rest)))
      if second ~= nil then sides = second end
    end
    HUD.chat_command("roll", sides or 100)
  end,
  -- 8 web cast, 9 web journal: the web client's, nothing to do here
  [8] = function() end,
  [9] = function() end,
  -- 12 invite to the guild, 13 leave it, 41 kick from it
  [12] = function(args)
    local name = nameOrTarget(args)
    if name ~= nil then X2Faction:InviteToExpedition(name) end
  end,
  [13] = function() X2Faction:LeaveExpedition() end,
  [41] = function(args)
    local name = split(args)
    if name ~= nil then X2Faction:KickFromExpedition(name) end
  end,
  -- 15 block, 16 unblock, 42 add a friend, 43 delete one
  [15] = function(args) local name = nameOrTarget(args); if name ~= nil then X2Friend:BlockUser(name) end end,
  [16] = function(args) local name = split(args); if name ~= nil then X2Friend:UnblockUser(name) end end,
  [42] = function(args) local name = nameOrTarget(args); if name ~= nil then X2Friend:AddFriend(name) end end,
  [43] = function(args) local name = split(args); if name ~= nil then X2Friend:DeleteFriend(name) end end,
  -- 17 family list: the roster is the window's
  [17] = function() end,
  -- 18 invite to the family: /addfamily Name Title, 19 leave, 21 kick, 22 change a title
  [18] = function(args)
    local name, title = split(args)
    if name == nil or title == "" then system(locale.family.invite_syntax) return end
    X2Family:Invite(name, title)
  end,
  [19] = function() X2Family:Leave() end,
  [21] = function(args) local name = split(args); if name ~= nil then X2Family:Kick(name) end end,
  [22] = function(args)
    local name, title = split(args)
    if name == nil or title == "" then system(locale.family.title_syntax) return end
    X2Family:ChangeTitle(name, title)
  end,
  -- 23-25 the raid's invite, leave and kick; 26-28 the party's
  [23] = teamInvite(false),
  [24] = teamLeave(false),
  [25] = teamKick(false),
  [26] = teamInvite(true),
  [27] = teamLeave(true),
  [28] = teamKick(true),
  -- 40: the skill of the row (/report Name, the Korean unstuck command), on the player named
  [40] = function(args, skill)
    if skill ~= nil and skill ~= 0 then HUD.chat_command("skill", skill, (split(args))) end
  end,
  -- 46: select a unit by name
  [46] = function(args)
    local name = split(args)
    if name ~= nil then HUD.chat_command("target", name) end
  end,
}

-- The sender of the last whisper received, for /r.
HUD.last_whisper = nil

HUD.slash = function(text)
  local kind, id, rest, skill = HUD.slash_parse(text)
  if kind == nil then return false end
  if kind == "channel" then
    -- a bare channel command only switches the channel; the toolkit does that for the ones it knows
    if rest == "" then return true end
    if id == -5 then
      -- /r: answer the last whisper
      if HUD.last_whisper ~= nil then HUD.request("chat", -3, rest, HUD.last_whisper) end
    else
      HUD.request("chat", id, rest, "")
    end
    return true
  end
  local f = functions[id]
  if f ~= nil then f(rest, skill) end
  -- (the commands the client has no use for here are done with as well: not a chat line)
  return true
end

-- The line the dice of anyone in range make: "<name> rolled a <value> on the dice. (1-<max>)"
HUD.dice_text = function(name, max, value)
  return X2Locale:LocalizeUiText(CHAT_SYSTEM_TEXT, "msg_dice_value", name, tostring(value), tostring(max))
end
