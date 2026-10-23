-- Original action-slot constructor and hover/tooltip handlers; the item provider is a fixture.
SCORE_TOOLTIP_CALLS=0;SCORE_RECEIVED=false
FAKE.add_item(28051,{name='Music Paper',icon='icon_item_1260.dds',price=0,refund=0,
  category='Instrument',sellable=false,maxStack=1})
local function scoreTooltip(...)
  assert(select('#',...)==0)
  SCORE_TOOLTIP_CALLS=SCORE_TOOLTIP_CALLS+1
  return {name='Music Paper',itemType=28051,lookType=28051,itemGrade=1,grade='Basic',gradeColor='FFBA976D',
    item_impl='etc',category='Instrument',stack=1,isStackable=false,sellable=false,refund=0,
    level_requirement=0,level_limit=0,
    userMusicTitle=SCORE_RECEIVED and 'Harbor' or nil,userMusicRequireGrade=SCORE_RECEIVED and 0 or nil}
end
local tip=SCORE_SNAPSHOT and scoreTooltip() or scoreTooltip
local originalContent=AAUI.SlotContent
AAUI.SlotContent=function(kind,index)
  if kind==ISLOT_ACTION and index==9001 then
    return {type='item',id=28051,grade=1,icon='ui/icon/icon_item_1260.dds',tooltip=tip}
  end
  return originalContent(kind,index)
end
SCORE_PANEL=CreateEmptyWindow('scoreProviderPanel','UIParent')
SCORE_PANEL:SetExtent(42,42);SCORE_PANEL:AddAnchor('TOPLEFT',UIParent,600,300);SCORE_PANEL:Show(true)
SCORE_SLOT=CreateActionSlot(SCORE_PANEL,'scoreProviderSlot',ISLOT_ACTION,9001)
SCORE_SLOT:AddAnchor('TOPLEFT',SCORE_PANEL,0,0);SCORE_SLOT:Show(true);SCORE_SLOT.index=9001
ActionBar_SettingTooltip({buttonStartIndex=1,buttonEndIndex=1,slot={SCORE_SLOT}})
