-- The HUD fallback supplies tab filtering; keep those closures and persist only its user data.
X2Option.Reset = AA_CLIENT_OPTION_RESET
X2Option.RemoveModifiedOption = AA_CLIENT_OPTION_REMOVE_MODIFIED
local saved = NET.bag_tabs()
if saved ~= nil then
  for i = X2Bag:TabCount(), 2, -1 do X2Bag:RemoveTab(i) end
  for _, t in ipairs(saved.tabs) do X2Bag:CreateTab(t.icon, t.name, t.groups) end
  X2Bag:SwitchTab(math.min(saved.selected, X2Bag:TabCount()))
end
local function saveTabs()
  local tabs = {}
  for i = 2, X2Bag:TabCount() do
    local name, icon, groups = X2Bag:TabInfo(i)
    tabs[#tabs + 1] = { name = name, icon = icon, groups = groups or {} }
  end
  NET.save_bag_tabs(tabs, X2Bag:CurrentTabIdx())
end
for _, method in ipairs({ "CreateTab", "EditTab", "RemoveTab", "SwitchTab" }) do
  local original = X2Bag[method]
  X2Bag[method] = function(self, ...)
    local result = original(self, ...)
    saveTabs()
    return result
  end
end
