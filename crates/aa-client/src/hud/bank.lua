-- The original inventory/bank windows, backed by server slots and item moves.
local BAG, BANK = 2, 3
local tabs, selected = {}, 1
-- The default inventory categories are shared data; later account settings restore each independently.
for i=2,X2Bag:TabCount() do
  local name,icon,groups=X2Bag:TabInfo(i)
  tabs[#tabs+1]={name=name,icon=icon,groups=groups or {}}
end
local function fire(event) FAKE.fire_later(event) end
local function count(items) local n=0; for _ in pairs(items) do n=n+1 end; return n end
local function info(slot)
  local s=HUD.bank[slot]
  return s and HUD.item_tip(s.id,s.grade,s.count,s) or nil
end
local function free(items,capacity)
  for i=1,capacity do if items[i]==nil then return i end end
end
local function real(virtual)
  if selected==1 then return virtual end
  local tab=tabs[selected-1]
  if not tab then return nil end
  local groups={}; for _,g in ipairs(tab.groups) do groups[g]=true end
  local n=0
  for i=1,HUD.bank_slots do
    if HUD.bank[i] and groups[HUD.bank[i].group] then
      n=n+1; if n==virtual then return i end
    end
  end
end
local function hold(slot,amount)
  local s=HUD.bank[slot]; if not s then return end
  amount=amount or s.count
  local icons=X2Item:GetItemIconSet(s.id,s.grade)
  X2Cursor:SetHeld({kind='bank',slot=slot,id=s.id,grade=s.grade,icon=s.icon,
    frame=icons and icons.frameIcon,count=amount,total=s.count,partial=amount<s.count})
end
X2Bank.Capacity=function() return HUD.bank_slots end
X2Bank.Slots=X2Bank.Capacity
X2Bank.CountItems=function() return count(HUD.bank) end
X2Bank.CountEmptyBankSlots=function() return HUD.bank_slots-count(HUD.bank) end
X2Bank.GetBagItemInfo=function(self,slot) return info(slot) end
X2Bank.ItemIdentifier=function(self,slot) local s=HUD.bank[slot]; return s and s.id or 0,s and s.grade or 0 end
X2Bank.ItemStack=function(self,slot) local s=HUD.bank[slot]; return s and s.count or 0 end
X2Bank.ItemDurability=function(self,slot) local s=HUD.bank[slot]; return s and s.dur end
X2Bank.ItemRequireLevel=function(self,slot) local s=HUD.bank[slot]; return s and s.level or 0 end
X2Bank.IsLocked=function() return false end
X2Bank.IsConfirmedSlot=function() return true end
X2Bank.IsInvalidSlot=function(self,v) return v<1 or v>HUD.bank_slots end
X2Bank.SlotByIdx=function(self,v) local r=real(v); return r and HUD.bank[r] and r or nil end
X2Bank.CurrentTabIdx=function() return selected end
X2Bank.TabCount=function() return #tabs+1 end
X2Bank.TabInfo=function(self,i) local t=tabs[i-1]; if t then return t.name,t.icon,t.groups end end
X2Bank.TabInfos=function()
  local names,icons={},{}; for i,t in ipairs(tabs) do names[i],icons[i]=t.name,t.icon end; return names,icons
end
X2Bank.CreateTab=function(self,icon,name,groups)
  tabs[#tabs+1]={icon=icon,name=name,groups=groups or {}}; selected=#tabs+1; fire('BANK_TAB_CREATED')
end
X2Bank.SwitchTab=function(self,i) if i>=1 and i<=#tabs+1 then selected=i; fire('BANK_TAB_SWITCHED') end end
X2Bank.EditTab=function(self,i,icon,name,groups)
  if tabs[i-1] then tabs[i-1]={icon=icon,name=name,groups=groups or {}}; fire('BANK_TAB_SWITCHED') end
end
X2Bank.RemoveTab=function(self,i)
  if tabs[i-1] then table.remove(tabs,i-1); selected=1; fire('BANK_TAB_REMOVED'); fire('BANK_TAB_SWITCHED') end
end
X2Bank.GetCurrency=function() return CURRENCY_GOLD end
X2Util.GetMyBankMoneyString=function() return HUD.state.bank_money end
X2Bank.Deposit=function(self,money,aa) HUD.bank_deposit(tostring(money)) end
X2Bank.Withdraw=function(self,money,aa) HUD.bank_withdraw(tostring(money)) end
X2Bank.PickupBankItemPartial=function(self,slot,amount)
  local s=HUD.bank[slot]; amount=tonumber(amount)
  if s and amount and amount>=1 then hold(slot,math.min(amount,s.count)) end
end
X2Bank.HandleClick=function(self,v)
  if not IsActivatedBank() then return end
  local dest=real(v)
  if not dest and selected~=1 then dest=free(HUD.bank,HUD.bank_slots) end
  if not dest or dest<1 or dest>HUD.bank_slots then return end
  local h=X2Cursor:GetHeld()
  if not h or h.kind=='image' then
    local s=HUD.bank[dest]; if not s then return end
    if HUD.shift_down() and s.stack and s.count>1 then
      FAKE.fire_now('INVEN_SLOT_SPLIT','bank',dest)
    else hold(dest) end
  elseif h.kind=='bank' or h.kind=='bag' then
    local source=h.kind=='bank' and BANK or BAG
    local target=HUD.bank[dest]
    if not (h.kind=='bank' and h.slot==dest) and (not h.partial or not target or target.id==h.id) then
      HUD.drag_move(source,h.slot,BANK,dest,h.partial and h.count or 0)
    end
    X2Cursor:ClearCursor()
  end
end
X2Bank.HandleUse=function(self,v)
  local slot=X2Bank:SlotByIdx(v)
  if slot then X2Bank:MoveToEmptyBagSlot(slot) end
end
X2Bag.MoveToEmptyBankSlot=function(self,slot)
  if not IsActivatedBank() or not HUD.bag[slot] then return end
  local dest=free(HUD.bank,HUD.bank_slots)
  if dest then HUD.drag_move(BAG,slot,BANK,dest,0) end
end
X2Bank.MoveToEmptyBagSlot=function(self,slot)
  if not IsActivatedBank() then return false end
  local dest=free(HUD.bag,HUD.bag_slots)
  if dest and HUD.bank[slot] then HUD.drag_move(BANK,slot,BAG,dest,0) end
  return true
end
local clickBag=X2Bag.HandleClick
X2Bag.HandleClick=function(self,v)
  local h=X2Cursor:GetHeld()
  if h and h.kind=='bank' and IsActivatedBank() then
    local dest=X2Bag:SlotByIdx(v)
    if not dest then dest=X2Bag:CurrentTabIdx()==1 and v or free(HUD.bag,HUD.bag_slots) end
    if dest and dest>=1 and dest<=HUD.bag_slots and not X2Bag:IsLocked(dest) then
      local target=HUD.bag[dest]
      if not h.partial or not target or target.id==h.id then HUD.drag_move(BANK,h.slot,BAG,dest,h.partial and h.count or 0) end
      X2Cursor:ClearCursor()
    end
    return
  end
  return clickBag(self,v)
end
local useBag=X2Bag.HandleUse
X2Bag.HandleUse=function(self,v)
  local slot=X2Bag:SlotByIdx(v)
  if slot and BagItemToEmptyBankSlot(slot) then return end
  return useBag(self,v)
end
