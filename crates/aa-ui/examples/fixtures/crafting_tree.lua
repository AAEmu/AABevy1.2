-- Minimal game data for the unmodified original crafting/category scripts.
X2Craft.EnumActabilityGroupTypes=function() return {1} end
X2Craft.GetActabilityCategoryTypesGroupType=function() return {101,102,103} end
X2Craft.GetActabilityCategoryInfoByTypes=function(self,id)
  return {name=({[101]='Basic Materials',[102]='Advanced Materials',[103]='Special Materials'})[id]}
end
X2Craft.GetActabilityCategoryCraftTypesByTypes=function(self,group,category)
  local out={}; for i=1,(CRAFT_TREE_ROWS or 24) do out[i]=category*100+i end; return out
end
X2Craft.GetCraftName=function(self,id) return string.format('Alchemy recipe %d',id) end
X2Craft.GetExecutableCraftCount=function() return 7 end
CRAFT_TREE_SELECTED={}
X2Craft.GetCraftBaseInfo=function(self,id) table.insert(CRAFT_TREE_SELECTED,id); return nil end
X2Craft.GetCraftMaterialInfo=function() return nil end
X2Craft.GetUpperCraftsInfo=function() return nil end
