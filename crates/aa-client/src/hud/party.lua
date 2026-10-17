-- The original unitframe/party and raidframe scripts use one-based team1..team50 aliases.
local function T() return HUD.team end
local function member(index) return T().members[index] end
local function unit(value)
  if value == "player" then return member(T().own_index) end
  if value == "target" then value = HUD.target and HUD.target.unit_id end
  return value and HUD.team_units[value]
end
X2Team.GetMaxMembers = function() return 50 end
X2Team.GetMaxParties = function() return 10 end
X2Team.GetMaxPartyMembers = function() return 5 end
X2Team.IsPartyTeam = function() return T().id ~= 0 and T().is_party end
X2Team.IsRaidTeam = function() return T().id ~= 0 and not T().is_party end
X2Team.GetTeamPlayerIndex = function() return T().own_index end
X2Team.GetTeamPlayerParty = function() return math.floor(math.max(0, T().own_index-1)/5)+1 end
X2Team.GetTeamPlayerSlot = function() return (math.max(1,T().own_index)-1)%5+1 end
X2Team.GetTeamPlayerPartyHeadIndex = function() return math.floor(math.max(0,T().own_index-1)/5)*5+1 end
X2Team.CountTeamMembers = function() return T().count end
X2Team.CountTeamMembersInParty = function(self, party)
  local count = 0
  for i = (party-1)*5+1, party*5 do if member(i) then count = count+1 end end
  return count
end
X2Team.IsTeamOwner = function(self, index) local m=member(index); return m ~= nil and m.leader end
X2Team.IsTeamOfficer = function() return false end
X2Team.GetRole = function(self, index) local m=member(index); return m and m.role or 0 end
X2Team.IsTargetMyTeam = function() return unit("target") ~= nil end
X2Team.GetTeamDistributorName = function()
  for _, m in pairs(T().members) do if m.id == T().loot_master then return m.name end end
end
X2Unit.UnitIsTeamMember = function(self, value) return unit(value) ~= nil end
X2Unit.UnitIsOffline = function(self, value) local m=unit(value); return m ~= nil and not m.online end
X2Unit.UnitTeamAuthority = function(self, value) local m=unit(value); if m and m.leader then return "leader" end end
X2Team.InviteToTeam = function(self, name, isParty) HUD.team_request("invite", nil, name, isParty) end
X2Team.LeaveTeam = function() HUD.team_request("leave") end
X2Team.DismissTeam = function() HUD.team_request("dismiss") end
X2Team.KickTeamMember = function(self, index) HUD.team_request("kick", index) end
X2Team.KickTeamMemberByName = function(self, name)
  for i, m in pairs(T().members) do if m.name == name then HUD.team_request("kick", i); return end end
end
X2Team.MakeTeamOwner = function(self, index) HUD.team_request("owner", index) end
local oldTarget = X2Unit.TargetUnit
X2Unit.TargetUnit = function(self, value)
  local m=unit(value)
  if m then HUD.team_request("target", m.index)
  elseif value == "player" then HUD.team_request("target", 0)
  elseif oldTarget then return oldTarget(self, value) end
end
-- Local display preferences expected by the original raid manager.
local prefs={PartyFrameVisible=true,RaidFrameVisible=true,SimpleView=false,RefuseAreaInvitation=false}
for _, name in ipairs({"PartyFrameVisible","RaidFrameVisible","SimpleView","RefuseAreaInvitation"}) do
  local name = name
  X2Team["Get"..name]=function() return prefs[name] end
  X2Team["Set"..name]=function(self,value)
    prefs[name]=value
    if name=="PartyFrameVisible" then FAKE.fire_now("TOGGLE_PARTY_FRAME",value)
    elseif name=="RaidFrameVisible" then FAKE.fire_now("TOGGLE_RAID_FRAME",value)
    elseif name=="SimpleView" then FAKE.fire_now("RAID_FRAME_SIMPLE_VIEW",value) end
  end
end
local visible={}
X2Team.GetPartyVisible=function(self,index) return visible[index] ~= false end
X2Team.SetPartyVisible=function(self,index,value)
  visible[index]=value
  FAKE.fire_now("TOGGLE_RAID_FRAME_PARTY",index,value)
end
local inviteWindow, inviteSender
HUD.show_team_invite=function()
  local invite=T().invite
  if inviteWindow then X2DialogManager:DeleteByOwnerWindow("teamInvitation"); inviteWindow=nil end
  inviteSender=invite and invite.sender_id
  if not invite then return end
  local sender=invite.sender_id
  inviteWindow=X2DialogManager:RequestDefaultDialog(function(wnd)
    local key=invite.is_party and "ask_party_invite" or "ask_raidteam_invite"
    wnd:SetTitle(X2Locale:LocalizeUiText(MSG_BOX_TITLE_TEXT, key))
    wnd:SetContent(X2Locale:LocalizeUiText(MSG_BOX_BODY_TEXT,key,invite.name))
    wnd.btnOk:SetText(X2Locale:LocalizeUiText(MSG_BOX_BTN_TEXT,"invite"))
    wnd.btnCancel:SetText(X2Locale:LocalizeUiText(MSG_BOX_BTN_TEXT,"reject"))
    wnd.OkProc=function() if inviteSender==sender then HUD.team_request("reply",sender,nil,true) end end
    wnd.CancelProc=function() if inviteSender==sender then HUD.team_request("reply",sender,nil,false) end end
  end, "teamInvitation")
end
