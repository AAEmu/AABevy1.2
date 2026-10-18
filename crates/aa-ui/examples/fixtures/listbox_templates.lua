-- Observe native image return values while running the unmodified Achievement constructor.
local original=CreateScrollListBox
function CreateScrollListBox(...)
  local window=original(...)
  for method,field in pairs({CreateOpenedImageDrawable='openedTemplate',
      CreateClosedImageDrawable='closedTemplate',CreateSeparatorImageDrawable='separatorTemplate'}) do
    local create=window.content[method]
    window.content[method]=function(self,...)
      local image=create(self,...);self[field]=image;return image
    end
  end
  return window
end
achievement=CreateAchievementWindow('nativeTemplateAchievement','UIParent')
CreateScrollListBox=original
achievement:ShowProc();achievement:Show(true)
category=achievement.categoryWnd.categoryList;tree=category.content;counts=category.countList
opened=tree.openedTemplate;closed=tree.closedTemplate;separator=tree.separatorTemplate
assert(opened:IsVisible() and closed:IsVisible() and separator:IsVisible())
opened:SetAnimFrameInfo({{x=0,y=0,w=21,h=14,time=700,scale=1},
    {x=44,y=0,w=13,h=21,time=700,scale=1}})
opened:Animation(true,true)

window=CreateWindow('nativeTemplateStates','UIParent')
window:SetTitle('Alchemy');window:SetExtent(280,235)
window:AddAnchor('TOPLEFT',UIParent,180,320);window:Show(true)
rows=CreateSliderList('nativeFallbackRows',window)
rows:AddAnchor('TOPLEFT',window,18,55);rows:SetExtent(245,145);rows:Show(true)
rows:AppendItem('Health potion',11);rows:AppendItem('Mana potion',22)
rows:AppendItem('Rejuvenation potion',33);rows.list:Select(0)
