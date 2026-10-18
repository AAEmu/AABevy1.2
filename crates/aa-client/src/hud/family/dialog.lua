-- The original type4 decorator owns layout, text, Escape and buttons.
local active
function HUD.family_close_invitation()
  if not active then return end
  local old=active
  active=nil
  X2DialogManager:OnCancel(old.wnd:GetId())
end
function HUD.family_show_invitation(token,info)
  local wnd=X2DialogManager:RequestTaskDialog(DLG_TASK_JOIN_FAMILY,info,'family')
  if not wnd then HUD.family_choose(token,false);return end
  active={wnd=wnd,token=token}
end
local ok,cancel=X2DialogManager.OnOK,X2DialogManager.OnCancel
function X2DialogManager:OnOK(id)
  if active and active.wnd:GetId()==id then
    local old=active;active=nil
    HUD.family_choose(old.token,true)
  end
  return ok(self,id)
end
function X2DialogManager:OnCancel(id)
  if active and active.wnd:GetId()==id then
    local old=active;active=nil
    HUD.family_choose(old.token,false)
  end
  return cancel(self,id)
end
