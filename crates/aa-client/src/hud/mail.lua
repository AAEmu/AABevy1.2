-- Original bag right-click routing, preserving physical slots and the store/bank wrappers.
local useBag=X2Bag.HandleUse
X2Bag.HandleUse=function(self,virtual)
  local real=X2Bag:SlotByIdx(virtual)
  if real and mailFrame and mailFrame:IsVisible() and GetWriteMailWindow():IsVisible() then
    if AddMailItem(1,real) then return end
  end
  return useBag(self,virtual)
end
