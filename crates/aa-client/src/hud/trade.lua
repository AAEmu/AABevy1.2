-- Route the original bag's physical slot into the original trade cart before using/equipping it.
local useBag=X2Bag.HandleUse
X2Bag.HandleUse=function(self,virtual)
  local real=X2Bag:SlotByIdx(virtual)
  if real and IsVisibleTradeWindow and IsVisibleTradeWindow() then
    return PutupTradeItem(real,0)
  end
  return useBag(self,virtual)
end
