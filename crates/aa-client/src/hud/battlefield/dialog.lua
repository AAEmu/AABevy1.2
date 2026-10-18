-- The original task decorator owns the three-button layout and texts. Rust retains wire identity.
local active = nil
function HUD.battlefield_close_invitation()
  if active then
    local old = active
    active = nil
    X2DialogManager:OnCancel(old.wnd:GetId())
  end
end
function HUD.battlefield_show_invitation(token, info)
  HUD.battlefield_close_invitation()
  local wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_JOIN_BATTLE_FIELD, info, "battlefield")
  if wnd then active = { wnd = wnd, token = token } end
end
local custom = X2DialogManager.OnCustom
function X2DialogManager:OnCustom(id, index)
  if active and active.wnd:GetId() == id then
    local old = active
    active = nil
    HUD.battlefield_choose(old.token, index)
  end
  return custom(self, id, index)
end
