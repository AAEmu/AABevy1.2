-- The original default task decorator owns layout, modal behavior, Escape and buttons.
local active={}
function HUD.expedition_close_tasks()
  local old=active
  active={}
  for _,entry in pairs(old) do X2DialogManager:OnCancel(entry.wnd:GetId()) end
end
function HUD.expedition_show_task(task,token,info)
  local old=active[task]
  active[task]=nil
  if old then X2DialogManager:OnCancel(old.wnd:GetId()) end
  local wnd=X2DialogManager:RequestTaskDialog(task,info,'expedition')
  if not wnd then HUD.expedition_choose(token,false);return end
  active[task]={wnd=wnd,token=token}
end
local ok,cancel=X2DialogManager.OnOK,X2DialogManager.OnCancel
local function choose(id,accepted)
  for task,entry in pairs(active) do
    if entry.wnd:GetId()==id then
      active[task]=nil
      HUD.expedition_choose(entry.token,accepted)
      return
    end
  end
end
function X2DialogManager:OnOK(id)
  choose(id,true)
  return ok(self,id)
end
function X2DialogManager:OnCancel(id)
  choose(id,false)
  return cancel(self,id)
end
