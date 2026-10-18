-- Original decorators own buttons, layout, modal flags and Escape. Rust retains native task captures.
local active={}
function HUD.trial_show_task(task,token,info)
  local wnd=X2DialogManager:RequestTaskDialog(task,info,'trial')
  if not wnd then HUD.trial_visual_failed(token);return end
  active[task]={wnd=wnd,token=token}
end
function HUD.trial_close_task(task)
  local entry=active[task]
  if entry then X2DialogManager:OnCancel(entry.wnd:GetId()) end
end
function HUD.trial_close_tasks()
  for _,task in ipairs({6,7,8}) do HUD.trial_close_task(task) end
end
local ok,cancel=X2DialogManager.OnOK,X2DialogManager.OnCancel
local function choose(id,accepted)
  for task,entry in pairs(active) do
    if entry.wnd:GetId()==id then
      active[task]=nil
      HUD.trial_choose(entry.token,accepted)
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
