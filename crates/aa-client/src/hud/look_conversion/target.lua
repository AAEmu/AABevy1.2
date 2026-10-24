-- Task19 keeps the original dialog's three-field payload and authored layout.
local active=nil
local originalDelete=X2DialogManager.Delete
X2DialogManager.Delete=function(self,task)
  if task~=19 then return originalDelete(self,task) end
  if active then
    local old=active;active=nil
    HUD.look_task_finish(old.token,false)
    self:OnCancel(old.wnd:GetId())
  end
end
HUD.show_look_task=function(token,info)
  X2DialogManager:Delete(19)
  local wnd=X2DialogManager:RequestTaskDialog(19,info,'bag')
  if not wnd then return false end
  active={token=token,wnd=wnd}
  wnd.OkProc=function()
    HUD.look_task_finish(token,true)
    if active and active.token==token then active=nil end
  end
  wnd.CancelProc=function()
    HUD.look_task_finish(token,false)
    if active and active.token==token then active=nil end
  end
  return true
end
