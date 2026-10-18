-- The original repair window supplies its controls and receives confirmed item updates.
X2Item.IsInRepairMode = function() return HUD.repair_mode() end
X2Item.EnterRepairMode = function() HUD.repair_mode(true) end
X2Item.LeaveRepairMode = function() HUD.repair_mode(false) end
X2Item.RepairAllCost = function() return HUD.repair_all_cost() end
X2Item.RepairAll = function() HUD.repair_all(X2Bag:GetAutoUseAAPoint()) end
X2Bag.ItemRepair = function(self, slot) HUD.repair_item(2, slot, X2Bag:GetAutoUseAAPoint()) end
X2Equipment.ItemRepair = function(self, slot) HUD.repair_item(1, slot, X2Bag:GetAutoUseAAPoint()) end
X2Bag.ItemDurability = function(self, slot, check) return HUD.item_durability(2, slot, check ~= false) end
X2Equipment.ItemDurability = function(self, slot, check) return HUD.item_durability(1, slot, check ~= false) end
X2Equipment.DameagedItems = function(self, count, check)
  if type(count) ~= "number" then return nil end
  return HUD.damaged_items(count, check ~= false)
end

local click = X2Bag.HandleClick
local function repair(virtual)
  if not X2Item:IsInRepairMode() then return false end
  local physical = X2Bag:SlotByIdx(virtual)
  if physical then X2Bag:ItemRepair(physical) end
  -- Native repair mode consumes the click even when an empty/healthy item fails preflight.
  return true
end
X2Bag.HandleClick = function(self, virtual, ...)
  if not repair(virtual) then return click(self, virtual, ...) end
end

local cancel = X2Interaction.CancelNPCInteraction
X2Interaction.CancelNPCInteraction = function(self, ...)
  cancel(self, ...)
  -- Native clears mode after NPC_INTERACTION_END listeners, rather than on window hide.
  HUD.repair_cancel()
end
