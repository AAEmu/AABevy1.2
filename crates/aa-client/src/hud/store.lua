-- Native side of the original store/buy/sell/rebuy windows. Inventory and money follow server tasks.
X2Store.GetSellItemsMaxCount=function() return 16 end
X2Store.GetStoreNpcItemList=function(self,countOnly)
  local goods=HUD.store_goods()
  if countOnly then return #goods end
  return goods
end
X2Store.GetStoreCurrency=function()
  local goods=HUD.store_goods()
  return goods[1] and goods[1].currency or -1
end
X2Store.BuyStoreItemWithStack=function(self,indices,counts,currencies) HUD.store_buy(indices,counts,currencies) end
local locked={}
X2Bag.LockSlot=function(self,slot) local s=HUD.bag[slot]; if s then locked[slot]=s.uid end; if HUD.item_lock then HUD.item_lock(slot,true) end end
X2Bag.UnlockSlot=function(self,slot) locked[slot]=nil; if HUD.item_lock then HUD.item_lock(slot,false) end end
X2Bag.IsLocked=function(self,slot) if HUD.item_locked then local v=HUD.item_locked(slot); if v~=nil then return v end end; return locked[slot]~=nil end
X2Store.SellStoreItem=function(self,slots,count,isEquip)
  if isEquip then return end
  local ids={}
  for i,slot in ipairs(slots) do ids[i]=locked[slot] or (HUD.bag[slot] and HUD.bag[slot].uid) or '0' end
  HUD.store_sell(slots,ids)
end
X2Store.SoldItemList=function() HUD.store_list_sold() end
X2Store.BuyBackItem=function(self,index) HUD.store_buyback(index) end
X2Store.StoreAddSellItemToBagSlot=function(self,slot) FAKE.fire_now("STORE_ADD_SELL_ITEM",slot) end
X2Store.ClearCursorOnStoreClose=function() X2Cursor:ClearCursor() end
X2Interaction.SelectNpcInteraction=function() end -- the original native only ends the script call
-- Original bag right-click prioritizes a visible shop's sell list over equipping/using the item.
local useBag=X2Bag.HandleUse
X2Bag.HandleUse=function(self,virtual)
  local real=X2Bag:SlotByIdx(virtual)
  if real and IsActivatedStore and IsActivatedStore() and AddSellItem(real) then return end
  return useBag(self,virtual)
end
