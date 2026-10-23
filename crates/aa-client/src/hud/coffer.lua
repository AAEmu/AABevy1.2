-- The original coffer window (the safe of a house), backed by the slots the server sent (HUD.coffer, 1 based) and item moves.
-- Container 4 is the coffer in a move; the window is opened by the engine event SHOW_COFFER_INVEN once the contents arrived.
local BAG, COFFER = 2, 4
local function fire(event) FAKE.fire_later(event) end
local function items() return HUD.coffer or {} end
local function capacity() return HUD.coffer_slots or 0 end
local function count() local n=0; for _ in pairs(items()) do n=n+1 end; return n end
local function info(slot)
  local s=items()[slot]
  return s and HUD.item_tip(s.id,s.grade,s.count,s) or nil
end
local function free(list,size)
  for i=1,size do if list[i]==nil then return i end end
end
local function hold(slot,amount)
  local s=items()[slot]; if not s then return end
  amount=amount or s.count
  local icons=X2Item:GetItemIconSet(s.id,s.grade)
  X2Cursor:SetHeld({kind='coffer',slot=slot,id=s.id,grade=s.grade,icon=s.icon,
    frame=icons and icons.frameIcon,count=amount,total=s.count,partial=amount<s.count})
end
X2Coffer.Capacity=function() return capacity() end
X2Coffer.Slots=X2Coffer.Capacity
X2Coffer.CountItems=function() return count() end
X2Coffer.GetBagItemInfo=function(self,slot) return info(slot) end
X2Coffer.ItemIdentifier=function(self,slot) local s=items()[slot]; return s and s.id or 0,s and s.grade or 0 end
X2Coffer.ItemStack=function(self,slot) local s=items()[slot]; return s and s.count or 0 end
X2Coffer.ItemDurability=function(self,slot) local s=items()[slot]; return s and s.dur end
X2Coffer.ItemRequireLevel=function(self,slot) local s=items()[slot]; return s and s.level or 0 end
X2Coffer.IsLocked=function() return false end
X2Coffer.IsConfirmedSlot=function() return true end
X2Coffer.IsInvalidSlot=function(self,v) return v<1 or v>capacity() end
X2Coffer.SlotByIdx=function(self,v) return items()[v] and v or nil end
-- one tab with everything in it
X2Coffer.CurrentTabIdx=function() return 1 end
X2Coffer.TabCount=function() return 1 end
X2Coffer.TabInfo=function() end
X2Coffer.TabInfos=function() return {},{} end
X2Coffer.CreateTab=function() end
X2Coffer.EditTab=function() end
X2Coffer.RemoveTab=function() end
X2Coffer.SwitchTab=function() end
X2Coffer.Sort=function() end
X2Coffer.GetLinkText=function() return nil end
X2Coffer.IsMyHouseCoffer=function() return false end
X2Coffer.GetHouseCofferName=function() return '' end
X2Coffer.GetHouseCofferPermission=function() return 1 end
X2Coffer.SetHouseCofferPermission=function() end
X2Coffer.CofferWindowClosed=function() HUD.coffer_close() end
X2Coffer.PickupCofferItemPartial=function(self,slot,amount)
  local s=items()[slot]; amount=tonumber(amount)
  if s and amount and amount>=1 then hold(slot,math.min(amount,s.count)) end
end
X2Coffer.HandleClick=function(self,v)
  local dest=v
  if dest<1 or dest>capacity() then return end
  local h=X2Cursor:GetHeld()
  if not h or h.kind=='image' then
    local s=items()[dest]; if not s then return end
    if HUD.shift_down() and s.stack and s.count>1 then
      FAKE.fire_now('INVEN_SLOT_SPLIT','coffer',dest)
    else hold(dest) end
  elseif h.kind=='coffer' or h.kind=='bag' then
    local source=h.kind=='coffer' and COFFER or BAG
    local target=items()[dest]
    if not (h.kind=='coffer' and h.slot==dest) and (not h.partial or not target or target.id==h.id) then
      HUD.drag_move(source,h.slot,COFFER,dest,h.partial and h.count or 0)
    end
    X2Cursor:ClearCursor()
  end
end
X2Coffer.HandleUse=function(self,v)
  if items()[v] then X2Coffer:MoveToEmptyBagSlot(v) end
end
X2Bag.MoveToEmptyCofferSlot=function(self,slot)
  if not IsActivatedCoffer() or not HUD.bag[slot] then return end
  local dest=free(items(),capacity())
  if dest then HUD.drag_move(BAG,slot,COFFER,dest,0) end
end
X2Coffer.MoveToEmptyBagSlot=function(self,slot)
  if not IsActivatedCoffer() then return false end
  local dest=free(HUD.bag,HUD.bag_slots)
  if dest and items()[slot] then HUD.drag_move(COFFER,slot,BAG,dest,0) end
  return true
end
local clickBag=X2Bag.HandleClick
X2Bag.HandleClick=function(self,v)
  local h=X2Cursor:GetHeld()
  if h and h.kind=='coffer' and IsActivatedCoffer() then
    local dest=X2Bag:SlotByIdx(v)
    if HUD.inventory_dest and HUD.inventory_owner()~=0 then dest=HUD.inventory_dest(BAG,v) end
    if not dest then dest=X2Bag:CurrentTabIdx()==1 and v or free(HUD.bag,HUD.bag_slots) end
    if dest and dest>=1 and dest<=HUD.bag_slots and not X2Bag:IsLocked(dest) then
      local target=HUD.bag[dest]
      if not h.partial or not target or target.id==h.id then HUD.drag_move(COFFER,h.slot,BAG,dest,h.partial and h.count or 0) end
      X2Cursor:ClearCursor()
    end
    return
  end
  return clickBag(self,v)
end
local useBag=X2Bag.HandleUse
X2Bag.HandleUse=function(self,v)
  local slot=X2Bag:SlotByIdx(v)
  if slot and IsActivatedCoffer and IsActivatedCoffer() and HUD.bag[slot] then X2Bag:MoveToEmptyCofferSlot(slot); return end
  return useBag(self,v)
end
