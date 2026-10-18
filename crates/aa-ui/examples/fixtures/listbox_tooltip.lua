-- Unmodified original list wrapper and OnTooltip -> SetTooltipOnPos handler.
rows=CreateSliderList('nativeTooltipRows','UIParent')
rows:AddAnchor('TOPLEFT',UIParent,400,340);rows:SetExtent(245,165);rows:Show(true)
list=rows.list;list.itemStyle:SetEllipsis(true);list:ShowTooltip(true)
list:SetListItemStateTextureInset(-15,-2,-10,-1)
rows:AppendItem('Restores Health over time while exploring Erenor',11)
rows:AppendItem('Mana potion',22)
for i=3,12 do rows:AppendItem('Alchemy recipe '..i,100+i) end
rows:SetMinMaxValues(0,list:GetMaxTop())
