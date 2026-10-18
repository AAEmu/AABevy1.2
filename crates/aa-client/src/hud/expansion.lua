-- The original task dialog supplies its authored cost/item layout and buttons.
local active = nil
X2Bag.GetAutoUseAAPoint = function() return HUD.auto_aa_point() end
X2Bag.SetAutoUseAAPoint = function(self, use)
  HUD.set_auto_aa_point(use == true)
end
X2Bag.ExpansionMaxSlotCount = function() return HUD.expansion_max(2) end
X2Bank.ExpansionMaxSlotCount = function() return HUD.expansion_max(3) end
local originalDelete = X2DialogManager.Delete
X2DialogManager.Delete = function(self, task)
  if task ~= DLG_TASK_EXPAND_INVENTORY then return originalDelete(self, task) end
  if active then
    HUD.expansion_cancel(active.token)
    self:OnCancel(active.wnd:GetId())
    active = nil
  end
end
local function expand(container)
  X2DialogManager:Delete(DLG_TASK_EXPAND_INVENTORY)
  local info = HUD.expansion_quote(container)
  if not info then return end
  local token = info.token
  info.token = nil
  local wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_EXPAND_INVENTORY, info,
    container == 3 and "bank" or "bag")
  if not wnd then HUD.expansion_cancel(token); return end
  active = { wnd = wnd, token = token }
  wnd.OkProc = function()
    HUD.expansion_confirm(token, X2Bag:GetAutoUseAAPoint())
    if active and active.token == token then active = nil end
  end
  wnd.CancelProc = function()
    HUD.expansion_cancel(token)
    if active and active.token == token then active = nil end
  end
end
X2Bag.Expand = function() expand(2) end
X2Bank.Expand = function() expand(3) end
