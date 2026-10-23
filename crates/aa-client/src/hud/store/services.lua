-- Native side of the NPC service windows: the priest (blessing, prayer), the ability changer, and the interaction list
-- an NPC with several services opens (the doodads' list of interaction.lua, filled with the NPC's services).

-- X2Priest: the blessings are the priest_buffs rows; the price the original works out is the level times the row's cost.
X2Priest.GetPriestBuffCount = function(self) return #HUD.service_info().buffs end
X2Priest.GetPriestBuff = function(self, index) return HUD.service_info().buffs[index] end
X2Priest.BuyPriestBuff = function(self, index) HUD.service_request("bless", index) end
-- Nothing to recover: no values at all. The money price is formula 6, the labor price formula 7.
X2Priest.GetCostForRecoverExp = function(self)
  local info = HUD.service_info()
  if (info.recoverable or 0) <= 0 then return nil end
  return info.recover_money, info.recover_labor
end
X2Priest.RecoverExp = function(self) HUD.service_request("pray") end

-- X2Ability: swapping an ability costs the level times a constant; the original answers the cost only when it can be paid.
X2Ability.GetAbilityChangeCost = function(self) return HUD.service_info().ability_cost end
X2Ability.CanBuyAbilityChange = function(self)
  local info = HUD.service_info()
  if (tonumber(info.money) or 0) >= info.ability_cost then return info.ability_cost end
  return nil
end
X2Ability.GetCurrencyForAbilityChange = function(self) return CURRENCY_GOLD or 0 end
X2Ability.SwapAbility = function(self, old, new) HUD.request("swap_ability", old, new) end

-- The interaction list: the rows of the NPC's services while one is up, the doodad's (or crafting's) otherwise.
local count, id, name = X2Interaction.GetInteractionSkillCount, X2Interaction.GetInteractionSkillId, X2Interaction.GetInteractionSkillName
local select, cancel = X2Interaction.SelectInteraction, X2Interaction.CancelInteraction
X2Interaction.GetInteractionSkillCount = function(self, ...)
  local menu = HUD.npc_menu()
  if menu then return #menu end
  return count(self, ...)
end
X2Interaction.GetInteractionSkillId = function(self, k, ...)
  local menu = HUD.npc_menu()
  if menu then return menu[k] and menu[k].id end
  return id(self, k, ...)
end
X2Interaction.GetInteractionSkillName = function(self, k, ...)
  local menu = HUD.npc_menu()
  if menu then return menu[k] and menu[k].name end
  return name(self, k, ...)
end
X2Interaction.SelectInteraction = function(self, pick, ...)
  if HUD.npc_menu() then return HUD.npc_menu_pick(pick) end
  return select(self, pick, ...)
end
X2Interaction.CancelInteraction = function(self, ...)
  if HUD.npc_menu() then return HUD.npc_menu_cancel() end
  if cancel then return cancel(self, ...) end
end
