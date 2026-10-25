local window = territoryFrame
if window == nil or window.aa_guard_events then return end
window.aa_guard_events = true
window:SetHandler("OnEvent", function(self, event)
  if event == "NPC_INTERACTION_END" then
    self:Show(false)
  elseif (event == "HOUSE_UNIT_CREATED" or event == "HOUSE_UNIT_REMOVED") and self:IsVisible() then
    local rows, gates, walls = X2House:GetGuardTowerStepInfo()
    if rows == nil then
      self:Show(false)
      return
    end
    local page = self.tab.window[2]
    page:FillGuardTowerStepInfoItem(rows)
    page:FillGuardTowerMaxInfo(gates, walls)
  end
end)
