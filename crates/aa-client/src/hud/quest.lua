-- Quests and NPC dialogs: the client side of the original quest windows' X2Quest calls (the quest log,
-- the tracker, the quest dialog of questcontext/quest_context_directing.lua), over the quest rows the
-- app makes from the server's quest state and the game database (dialog.rs: HUD.quest_list,
-- HUD.quest_info, HUD.quest_npc, HUD.quest_bubbles). Loaded after ingame.lua: it replaces the fake
-- quests of fake_quest.lua, function by function, with the same shapes.
--
-- The dialog: the app raises NPC_INTERACTION_START("quest", "start" | "complete" | "talk", npcId);
-- npc_interaction.lua asks CallQuestUi / CallQuestTalk here, which raise START_QUEST_CONTEXT_NPC /
-- COMPLETE_QUEST_CONTEXT_NPC / PROGRESS_TALK_QUEST_CONTEXT(qtype, true, npcId); the directing window
-- asks TryStartQuestContext (...) here, which plays the step's lines as CHAT_MSG_QUEST, one at a time
-- (X2:FastForwardQuestChat / FastBackwardQuestChat), and its last button sends the request
-- (AcceptDirectingQuest, CompleteDirectingQuest, ProgressTalkDirectingQuest). The server's answer comes
-- back as QUEST_CONTEXT_UPDATED, which closes the window.

local CBK_SYSTEM = _G.CBK_SYSTEM or 3

-- The quest rows ----------------------------------------------------------------------------------------------
local QUESTS, BY_TYPE, INFO = {}, {}, {}
local version = -1

local function sync()
  local v = HUD.quest_version()
  if v == version then return end
  version = v
  QUESTS = HUD.quest_list() or {}
  BY_TYPE, INFO = {}, {}
  for i, q in ipairs(QUESTS) do
    q.index = i
    BY_TYPE[q.type] = q
  end
end

local function byIndex(idx)
  sync()
  return QUESTS[tonumber(idx) or 0]
end

-- An active quest, or any quest of the database (the dialog shows a quest before it is taken).
local function byType(qtype)
  sync()
  qtype = tonumber(qtype)
  if qtype == nil then return nil end
  local q = BY_TYPE[qtype]
  if q ~= nil then return q end
  q = INFO[qtype]
  if q == nil then
    q = HUD.quest_info(qtype) or false
    INFO[qtype] = q
  end
  return q or nil
end

local function field(name, default)
  return function(self, qtype)
    local q = byType(qtype)
    if q == nil or q[name] == nil then return default end
    return q[name]
  end
end

-- The list of the active quests ------------------------------------------------------------------------------
X2Quest.GetActiveQuestListCount = function(self) sync(); return #QUESTS end
X2Quest.GetActiveQuestType = function(self, idx)
  local q = byIndex(idx)
  return q and q.type or nil
end
X2Quest.GetActiveQuestTitle = function(self, qtype) local q = byType(qtype); return q and q.name or false end
X2Quest.GetActiveQuestLevel = function(self, idx) local q = byIndex(idx); return q and q.level or 1 end
X2Quest.GetActiveQuestComparedLevel = function(self, idx) return 0 end
-- 1 = in progress, 3 = ready to be reported.
X2Quest.GetActiveQuestListStatus = function(self, idx) local q = byIndex(idx); return q and q.status or 0 end
X2Quest.GetActiveQuestObjectiveText = function(self, qtype, i)
  local q = byType(qtype)
  local o = q and q.objectives[i]
  return o and o.text or false
end
X2Quest.GetActiveQuestContextConditionMessage = function(self) return HUD.quest_condition() end
X2Quest.IsMainQuest = function(self, qtype) local q = byType(qtype); return q ~= nil and q.main == true end
X2Quest.IsTaskQuest = field("task", false)
X2Quest.IsHiddenQuest = field("hidden", false)
X2Quest.IsGroupQuest = field("group", false)
X2Quest.IsSelectiveQuest = field("selective", false)
X2Quest.IsLivelihoodQuest = field("livelihood", false)
X2Quest.IsScoreQuest = function(self, qtype) return false end
X2Quest.IsDailyQuest = function(self, qtype) local q = byType(qtype); return q ~= nil and q.daily == true, nil end
X2Quest.IsLetItDoneQuest = function(self, idx) return false end
X2Quest.IsOverDoneQuest = function(self, idx) return false end
X2Quest.IsLetItDoneQuestByType = function(self, qtype) return false end
X2Quest.IsOverDoneQuestByType = function(self, qtype) return false end
X2Quest.IsReadyForCompleteQuest = function(self, qtype) local q = byType(qtype); return q ~= nil and q.status == 3 end
X2Quest.IsProgressQuestInJournal = function(self, qtype) local q = byType(qtype); return q ~= nil and q.status == 1 end
X2Quest.IsReadyQuestInJournal = function(self, qtype) local q = byType(qtype); return q ~= nil and q.status == 3 end
X2Quest.NowIsAggroComponent = function(self, qtype) return false end
X2Quest.GetObjectiveComponentCount = function(self, qtype)
  local q = byType(qtype)
  return q and #q.objectives or 0
end
X2Quest.GetQuestContextGrade = function(self, qtype) return "normal" end
X2Quest.GetQuestContextLeftTime = function(self, qtype) return 0 end
X2Quest.GetUseTypeQuestItems = function(self, qtype, all) return {} end
X2Quest.GetUseTypeQuestItemsByObjIndex = function(self, qtype, index) return {} end
X2Quest.IsQuestStartItem = function(self, itemType) return false end
X2Quest.IsUseItemInActiveQuest = function(self, itemType) return false end
X2Quest.GetQuestReportNpcTypeByQuestType = field("reportNpc", nil)
X2Quest.GetQuestReportNpcNameByQuestType = field("reportNpcName", nil)
X2Quest.GetQuestLinkText = function(self, qtype) local q = byType(qtype); return q and string.format("|q%d;", tonumber(qtype)) or "" end
-- Give the quest up (the quest log's button; the list's index).
X2Quest.QuestContextDrop = function(self, idx)
  local q = byIndex(idx)
  if q ~= nil then HUD.quest_drop(q.type) end
end

-- Titles, categories ---------------------------------------------------------------------------------------
X2Quest.GetQuestContextMainTitle = function(self, qtype) local q = byType(qtype); return q and q.name or "" end
X2Quest.GetQuestContextSummary = field("summary", "")
X2Quest.GetQuestContextReadySummary = field("summary", "")
X2Quest.GetQuestContextProgressSummary = field("summary", "")
X2Quest.GetQuestContextBody = function(self, qtype) return "" end
-- The folder of a main quest is named after its chapter (the locale's chapter text) and the zone.
local function chapterText(q)
  local get = locale and locale.questContext and locale.questContext.Get_category
  return (get and get(q.chapter)) or ""
end
X2Quest.GetQuestJournalSubTitleByType = function(self, qtype)
  local q = byType(qtype)
  if q == nil or not q.main then return nil end
  local chapter = chapterText(q)
  return (chapter:sub(-1) == " " and "" or " ") .. (q.zone or "")
end
X2Quest.GetQuestCategoryTypeByQuestType = field("category", 0)
X2Quest.GetQuestCategoryNameByQuestType = field("categoryName", "")
X2Quest.GetQuestCategoryName = function(self, ctype)
  sync()
  for _, q in ipairs(QUESTS) do
    if q.category == ctype then
      if q.main then return chapterText(q) .. (X2Quest.GetQuestJournalSubTitleByType(self, q.type) or "") end
      return q.categoryName or ""
    end
  end
  return HUD.quest_category(tonumber(ctype) or 0)
end
X2Quest.GetQuestContextQuestChapterIdxByType = field("chapter", 0)

-- The journal on the right of the quest log ----------------------------------------------------------------
X2Quest.GetQuestJournalTitle = function(self, idx) local q = byIndex(idx); return q and q.name or "" end
X2Quest.GetQuestJournalObjectiveCount = function(self, idx) local q = byIndex(idx); return q and #q.objectives or 0 end
-- The summary of the objective is the paragraph under the title, `status` the progress line.
local function journalObjective(q, k)
  local o = q and q.objectives[k]
  if o == nil then return nil end
  return { summary = q.summary, status = o.status, done = o.done }
end
X2Quest.GetQuestJournalObjectiveText = function(self, idx, k) return journalObjective(byIndex(idx), k) end
X2Quest.GetQuestJournalObjectiveTextByType = function(self, qtype, k) return journalObjective(byType(qtype), k) end
X2Quest.GetQuestJournalMainBody = function(self, idx) return "" end
-- { who, text, kind }: what was said when the quest was accepted.
X2Quest.AcceptBubbleText = function(self, qtype)
  local q = byType(qtype)
  if q == nil then return nil end
  local out = {}
  for i, b in ipairs(q.bubbles) do out[i] = { who = b.who, text = b.text, kind = b.kind } end
  return out
end

-- Rewards -------------------------------------------------------------------------------------------------------
X2Quest.GetQuestContextRewardExp = field("exp", "0")
X2Quest.GetQuestContextRewardCopper = field("copper", 0)
X2Quest.GetQuestContextRewardAAPoint = function(self, qtype) return 0 end
X2Quest.RewardLaborPower = field("labor", nil)
X2Quest.RewardHonorPoint = field("honor", nil)
X2Quest.RewardCrimePoint = field("crime", nil)
X2Quest.RewardLivingPoint = field("living", nil)
X2Quest.GetQuestContextRewardAppellation = function(self, qtype) return nil end
local function items(qtype, key)
  local q = byType(qtype)
  return q and q[key] or {}
end
X2Quest.GetQuestContextRewardItemAllCount = function(self, qtype) return #items(qtype, "items") end
X2Quest.GetQuestContextRewardSelectiveItemAllCount = function(self, qtype) return #items(qtype, "selectiveItems") end
local function tooltip(it)
  if it == nil then return nil end
  return { name = it.name, itemType = it.id, itemGrade = it.grade }
end
X2Quest.GetQuestContextItemTooltip = function(self, qtype, i) return tooltip(items(qtype, "items")[i]) end
X2Quest.GetQuestContextSelectiveItemTooltip = function(self, qtype, i) return tooltip(items(qtype, "selectiveItems")[i]) end
X2Quest.GetQuestContextRewardItemCount = function(self, qtype, i)
  local it = items(qtype, "items")[i]
  return it and it.count or 1
end
X2Quest.GetQuestContextRewardSelectiveItemCount = function(self, qtype, i)
  local it = items(qtype, "selectiveItems")[i]
  return it and it.count or 1
end

-- No quest guide decals (the path on the ground) yet.
X2Decal.CanMakeGuidedDecal = function(self, qtype) return false end

-- The tracker lists every active quest when it starts (no saved list), then each one accepted.
local notifierValues = { listOpen = true }
X2.GetQuestNotifierListUiData = function(self) return notifierValues end
X2.SetQuestNotifierListUiData = function(self, values) notifierValues = values end

-- Text macros (@NPC_NAME(id), @PC_NAME(0) ...) and the quest texts' |nn; highlights.
X2Util.ApplyUIMacroString = function(self, text) return HUD.quest_macro(text) end

-- The NPC talked to -------------------------------------------------------------------------------------------
local function npc()
  return HUD.quest_npc() or { npcId = "", start = {}, complete = {}, talk = {} }
end
X2Quest.GetNpcQuestContextCountStart = function(self) return #npc().start end
X2Quest.GetNpcQuestContextQuestTypeStart = function(self, i) return npc().start[i] end
X2Quest.GetNpcQuestContextCountComplete = function(self) return #npc().complete end
X2Quest.GetNpcQuestContextQuestTypeComplete = function(self, i) return npc().complete[i] end
X2Quest.GetNpcQuestContextCountTalk = function(self) return #npc().talk end
-- { qtype, ctype, atype }: the quest, component and act of the i-th talk objective.
X2Quest.GetNpcQuestContextQuestTypeTalk = function(self, i) return npc().talk[i] end
X2Quest.GetNumActiveQuest = function(self) return #npc().complete end
X2Quest.GetNumAvailableQuest = function(self) return #npc().start end
X2Quest.GetNumTalkObjective = function(self) return #npc().talk end
X2Interaction.CancelNPCInteraction = function(self) HUD.quest_end() end

-- The dialog (directing mode) ---------------------------------------------------------------------------------
local directing = false
-- The lines being played: the step's chain, the one shown, and what the last button sends.
local play = { chain = {}, pos = 0, qtype = 0, ctype = 0, atype = 0 }
local talkCalled = false

-- 0 = accept (START_QUEST_CONTEXT_NPC), 2 = hand in (COMPLETE_QUEST_CONTEXT_NPC).
X2Quest.CallQuestUi = function(self, kind, qtype, npcId)
  if kind == 2 then
    FAKE.fire_later("COMPLETE_QUEST_CONTEXT_NPC", qtype, true, npcId)
  else
    FAKE.fire_later("START_QUEST_CONTEXT_NPC", qtype, true, npcId)
  end
end
-- npc_interaction.lua calls this for every talk objective the NPC has: one dialog plays them.
X2Quest.CallQuestTalk = function(self, qtype, ctype, npcId)
  if talkCalled then return end
  talkCalled = true
  FAKE.fire_later("PROGRESS_TALK_QUEST_CONTEXT", qtype, true, npcId)
end

-- The close-up camera (quest::try_context, x2game+0x389c70): the Options' close-up camera box is the console variable
-- quest_camera_use, 1023 (one bit for each quest detail) or 0; the talk of a quest gets the close-up when the quest's
-- use_quest_camera is set and its detail's bit is (ingame/quest_framing.rs, close_up_on). While it is on each line frames its
-- speaker (HUD.quest_frame, which ingame/quest_camera.rs turns into the camera) and the player is hidden.
local QUEST_CAMERA_ON = 1023
local useOption = QUEST_CAMERA_ON
local closeUp = false
local function startCloseUp(qtype)
  closeUp = HUD.quest_close_up(qtype, useOption) == true
  HUD.quest_camera(closeUp)
end

local function showLine()
  local b = play.chain[play.pos]
  if b == nil then return end
  -- the window shows Next while there is a line after this one (its id), the confirm button on the last (0)
  local after = play.chain[play.pos + 1]
  local nextId = after and after.id or 0
  -- each quest line frames its speaker (quest::bubble_run)
  if closeUp and b.id ~= 0 then HUD.quest_frame(b.id) end
  FAKE.fire_later("CHAT_MSG_QUEST", b.text, b.author, b.authorId, b.kind, 5000, 1000, nextId, play.qtype, false)
end

-- The window's text: no tab stops (|t170; after the speaker), the macros of "my_self" expanded.
local patched = false
local function patchWindow()
  if patched or ADDON == nil or ADDON.GetContent == nil then return end
  local w = ADDON:GetContent(UIC_QUEST_CINEMA_WND)
  if w == nil or w.messageWidget == nil then return end
  patched = true
  local mw = w.messageWidget
  -- the toolkit has no page size for a message widget: no scroll bar (one page holds the line)
  if mw:GetPagePerMaxLines() == nil then
    mw.GetPagePerMaxLines = function(self) return 1000 end
  end
  local add = mw.AddMessage
  mw.AddMessage = function(self, text, ...)
    text = string.gsub(tostring(text or ""), "|t%d+;", "")
    return add(self, HUD.quest_macro(text), ...)
  end
end

-- The fade window hides itself on its first OnUpdate and waits for its OnVisibleChanged to enable the
-- dialog's buttons; that event does not reach the handler it sets there (aa-ui), so the fade ends here.
local fadePatched = false
local function patchFade()
  if fadePatched or ADDON == nil or ADDON.GetContent == nil then return end
  local fw = ADDON:GetContent(UIC_QUEST_CINEMA_FADE_WND)
  if fw == nil or fw.SetFadeEndFunc == nil then return end
  fadePatched = true
  local setFadeEndFunc = fw.SetFadeEndFunc
  fw.SetFadeEndFunc = function(self, onFadeIn, onFadeOut)
    setFadeEndFunc(self, onFadeIn, onFadeOut)
    local update = self.OnUpdateFadeIn
    self:SetHandler("OnUpdate", function(this, dt)
      update(this, dt)
      if this.fadingNow and not this:IsVisible() then
        this:ReleaseHandler("OnVisibleChanged")
        this.fadingNow = false
        onFadeOut()
      end
    end)
  end
end

local function playLines(which, qtype, ctype, atype)
  play = { chain = HUD.quest_bubbles(which, qtype, ctype or 0) or {}, pos = 1, qtype = qtype, ctype = ctype or 0, atype = atype or 0 }
  HUD.quest_cinema_permission(qtype, which)
  startCloseUp(qtype)
  if #play.chain == 0 then
    -- a step without lines: what it asks
    local q = byType(qtype)
    play.chain = { { id = 0, text = q and q.summary or "", author = "", authorId = "", kind = CBK_SYSTEM } }
  end
  showLine()
end

X2.FastForwardQuestChat = function(self, nextBubbleId)
  if play.pos < #play.chain then
    play.pos = play.pos + 1
    showLine()
  end
end
X2.FastBackwardQuestChat = function(self, nextBubbleId)
  if play.pos > 1 then
    play.pos = play.pos - 1
    showLine()
  end
end
X2.IsFirstQuestChat = function(self, nextBubbleId) return play.pos <= 1 end
X2.StopChatBubble = function(self) end

X2Quest.EnterQuestDirectingMode = function(self)
  patchWindow()
  patchFade()
  directing = true
  HUD.quest_directing(true)
end
X2Quest.LeaveQuestDirectingMode = function(self)
  directing = false
  talkCalled = false
  play = { chain = {}, pos = 0, qtype = 0, ctype = 0, atype = 0 }
  -- a cut back to the follow camera and the player shown (quest_camera::set_active(0))
  closeUp = false
  HUD.quest_camera(false)
  HUD.quest_directing(false)
end
X2Quest.IsQuestDirectingMode = function(self) return directing end
-- The console variable is 1023 or 0 (the Options' box calls SetUseQuestCamera with its state, true or 1 for checked).
X2Quest.UseQuestCamera = function(self) return useOption == QUEST_CAMERA_ON end
X2Quest.SetUseQuestCamera = function(self, use)
  useOption = (use == true or (type(use) == "number" and use ~= 0)) and QUEST_CAMERA_ON or 0
end
X2Quest.DeclineDirectingQuest = function(self) end
X2Quest.AcceptDirectingQuest = function(self) HUD.quest_accept(play.qtype) end
X2Quest.ProgressTalkDirectingQuest = function(self) HUD.quest_talk(play.qtype, play.ctype, play.atype) end
X2Quest.CompleteDirectingQuest = function(self, selected) HUD.quest_complete(play.qtype, selected or 0) end

-- In the dialog these play the step's lines; without it (a quest from an item, a sphere) they send at once.
X2Quest.TryStartQuestContext = function(self, who, qtype, stype, doodadId, npcId)
  if directing then playLines("start", qtype) else HUD.quest_accept(qtype) end
end
X2Quest.TryCompleteQuestContext = function(self, who, qtype, doodadId, npcId, selected)
  if directing then playLines("complete", qtype) else HUD.quest_complete(qtype, selected or 0) end
end
X2Quest.TryProgressTalkQuestComponent = function(self, qtype, ctype, atype, npcId)
  if directing then
    -- the window asks for every talk objective of the NPC: the first one is played
    if #play.chain == 0 then playLines("talk", qtype, ctype, atype) end
  else
    HUD.quest_talk(qtype, ctype, atype)
  end
end
-- After a hand-in the dialog offers the NPC's next quest at once.
X2Quest.IsNextQuestAcceptableForDirecting = function(self, qtype)
  local n = npc()
  if n.start[1] ~= nil then return true, n.start[1] end
  return false, 0
end

-- Quest-directing bindings come from the original default_binding.g and saved X2Hotkey state.
