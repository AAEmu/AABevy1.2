-- Real Achievement category generation/handlers and the original icon-list wrapper.
achievement=CreateAchievementWindow('nativeAchievementArt','UIParent')
achievement:ShowProc();achievement:Show(true)
category=achievement.categoryWnd.categoryList
tree=category.content
counts=category.countList

iconWindow=CreateWindow('nativeIconList','UIParent')
iconWindow:SetTitle('Controls');iconWindow:SetExtent(280,235)
iconWindow:AddAnchor('TOPLEFT',UIParent,180,320);iconWindow:Show(true)
icons=CreateSliderList('nativeIconRows',iconWindow)
icons:AddAnchor('TOPLEFT',iconWindow,18,55);icons:SetExtent(245,145);icons:Show(true)
icons:AppendItem('Screenshot controls',11,'ui/button/screenshot_mode.dds')
icons.list:AppendItem('Ranking period',22,0.4,0.15,0.05,1,'ui/ranking/period.dds')
icons:AppendItem('Plain row',33)
