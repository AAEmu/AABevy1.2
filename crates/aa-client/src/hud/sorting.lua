-- Native default tabs keep a physical-slot permutation; item tasks only order its empty entries.
local views = {}
local function items(container) return container == 2 and HUD.bag or HUD.bank end
local function capacity(container) return container == 2 and HUD.bag_slots or HUD.bank_slots end
local function api(container) return container == 2 and X2Bag or X2Bank end
local function identity(size)
  local order={}; for i=1,size do order[i]=i end; return order
end
local function emptyOrder(view, current)
  local positions,empty={},{}
  for i,physical in ipairs(view.order) do
    if not current[physical] then positions[#positions+1]=i; empty[#empty+1]=physical end
  end
  table.sort(empty)
  for i,position in ipairs(positions) do view.order[position]=empty[i] end
end
local function restore(saved,current,size)
  if type(saved)~='table' or #saved<size then return nil end
  local order,seen={},{}
  for i,entry in ipairs(saved) do
    local physical=entry.slot
    if type(physical)~='number' or physical%1~=0 or physical<1 or physical>#saved or seen[physical] then return nil end
    if current[physical] and current[physical].uid~=entry.uid then return nil end
    seen[physical]=true; order[i]=physical
  end
  for physical,item in pairs(current) do if not seen[physical] then return nil end end
  return order
end
local function viewFor(container)
  if container~=2 and container~=3 then return nil end
  local owner,cap=HUD.inventory_owner(),capacity(container)
  if owner==0 or cap==0 then views[container]=nil; return nil end
  local current=items(container)
  local view=views[container]
  if not view or view.owner~=owner then
    local size=math.max(130,cap)
    view={owner=owner,order=restore(NET.inventory_order(owner,container),current,size) or identity(size)}
    views[container]=view
  end
  if view.current~=current or view.capacity~=cap then
    for i=#view.order+1,math.max(130,cap) do view.order[i]=i end
    emptyOrder(view,current)
    view.current,view.capacity,view.filtered=current,cap,{}
  end
  return view
end
-- Native390105e0 compares unsigned bytes to NUL, without case folding or locale collation.
local function byteCompare(x,y)
  local i=1
  while true do
    local a,b=string.byte(x,i) or 0,string.byte(y,i) or 0
    if a~=b then return a<b and -1 or 1 end
    if a==0 then return 0 end
    i=i+1
  end
end
local function less(current,a,b)
  local x,y=current[a],current[b]
  if not x or not y then return x~=nil and y==nil or x==nil and y==nil and a<b end
  if x.category_order~=y.category_order then return (x.category_order or 0)<(y.category_order or 0) end
  if x.grade~=y.grade then return (x.grade or 0)>(y.grade or 0) end
  local names=byteCompare(x.name or '',y.name or '')
  if names~=0 then return names<0 end
  return a<b
end
local function uidLess(current,a,b)
  local x,y=current[a].uid,current[b].uid
  if #x~=#y then return #x<#y end
  if x~=y then return byteCompare(x,y)<0 end
  return a<b
end
local function filtered(container,view,tab)
  local _,_,groups=api(container):TabInfo(tab)
  local cached=view.filtered[tab]
  if cached and cached.groups==groups then return cached.order end
  local want,order={},{}
  for _,group in ipairs(groups or {}) do want[group]=true end
  for slot=1,view.capacity do
    local item=view.current[slot]
    if item and want[item.group] then order[#order+1]=slot end
  end
  table.sort(order,function(a,b) return uidLess(view.current,a,b) end)
  view.filtered[tab]={groups=groups,order=order}
  return order
end
local function physical(container,virtual)
  local view=viewFor(container)
  if not view or type(virtual)~='number' or virtual%1~=0 or virtual<1 or virtual>view.capacity then return nil end
  local tab=api(container):CurrentTabIdx()
  return tab==1 and view.order[virtual] or filtered(container,view,tab)[virtual]
end
HUD.inventory_dest=function(container,virtual)
  local slot=physical(container,virtual)
  if slot then return slot end
  local view=viewFor(container)
  if not view or type(virtual)~='number' or virtual%1~=0 or virtual<1 or virtual>view.capacity then return nil end
  if api(container):CurrentTabIdx()~=1 then
    for i=1,view.capacity do if not view.current[i] then return i end end
  end
end
for _,container in ipairs({2,3}) do
  local container=container
  local object=api(container)
  local original=object.SlotByIdx
  object.SlotByIdx=function(self,virtual)
    if HUD.inventory_owner()==0 or capacity(container)==0 then return original(self,virtual) end
    local slot=physical(container,virtual)
    if slot and items(container)[slot] then return slot end
  end
  local function notify(self)
    FAKE.fire_now(container==2 and 'BAG_TAB_SORTED' or 'BANK_TAB_SORTED',self:CurrentTabIdx())
  end
  object.Sort=function(self)
    local view=viewFor(container)
    if view and self:CurrentTabIdx()~=1 then
      view.filtered[self:CurrentTabIdx()]=nil
      filtered(container,view,self:CurrentTabIdx())
    end
    notify(self)
  end
  object.ManualSort=function(self)
    local view=viewFor(container)
    if view then
      if self:CurrentTabIdx()==1 then table.sort(view.order,function(a,b) return less(view.current,a,b) end) end
      local saved={}
      for i,slot in ipairs(view.order) do saved[i]={slot=slot,uid=view.current[slot] and view.current[slot].uid or '0'} end
      NET.save_inventory_order(view.owner,container,saved)
    end
    notify(self)
  end
end
