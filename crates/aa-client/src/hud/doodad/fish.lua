-- Fish-stand confirmation dialogs (native FUN_393b8870 buy / FUN_393b8a30 convert).
--
-- The checks and the equipped-fish item are in Rust (fish.rs); this file opens the original
-- dialog_handler.alb task 12/13 through X2DialogManager:RequestTaskDialog with the itemInfo table
-- the handler formats, and remembers whether OK or cancel ended it. The Rust side sends the stored
-- doodad func skill only for "ok".
local FISH_DIALOG

local function clear()
  FISH_DIALOG = nil
end

HUD.fish_open = function(buy, template, grade, count, refund)
  if FISH_DIALOG then
    X2DialogManager:OnCancel(FISH_DIALOG.id)
    FISH_DIALOG = nil
  end
  local tip = HUD.item_tip(template, grade, count)
  if tip == nil then
    tip = { name = tostring(template), itemType = template, lookType = template, itemGrade = grade, gradeColor = "FFFFFFFF" }
  end
  tip.itemType = tip.itemType or template
  tip.lookType = tip.lookType or template
  tip.itemGrade = tip.itemGrade or grade
  if buy then
    tip.refund = refund
  else
    tip.refund = nil
  end
  local task = buy and DLG_TASK_BUY_FISH or DLG_TASK_CONVERT_FISH
  local info = { type = task, buttonType = DBT_OK_CANCEL, isModal = true, itemInfo = tip, itemType = template, itemCount = count }
  local wnd = X2DialogManager:RequestTaskDialog(task, info, "fishStand")
  if wnd == nil then
    return false
  end
  local id = wnd:GetId()
  local manager = X2DialogManager
  local originalOK, originalCancel = manager.OnOK, manager.OnCancel
  local wrappedOK, wrappedCancel
  local function finish(state)
    if FISH_DIALOG and FISH_DIALOG.id == id then
      FISH_DIALOG.state = state
    end
    if manager.OnOK == wrappedOK then manager.OnOK = originalOK end
    if manager.OnCancel == wrappedCancel then manager.OnCancel = originalCancel end
  end
  wrappedOK = function(self, widgetId)
    if widgetId == id then finish("ok") end
    return originalOK(self, widgetId)
  end
  wrappedCancel = function(self, widgetId)
    if widgetId == id then finish("cancel") end
    return originalCancel(self, widgetId)
  end
  manager.OnOK = wrappedOK
  manager.OnCancel = wrappedCancel
  FISH_DIALOG = { id = id, state = "open", wnd = wnd, info = info }
  return true
end

HUD.fish_state = function()
  return FISH_DIALOG and FISH_DIALOG.state or ""
end

HUD.fish_id = function()
  return FISH_DIALOG and FISH_DIALOG.id or 0
end

HUD.fish_visible = function()
  return FISH_DIALOG ~= nil and FISH_DIALOG.wnd:IsVisible()
end

HUD.fish_clear = function()
  clear()
end
