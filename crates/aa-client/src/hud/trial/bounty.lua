-- Compatibility shim for the original 1.2 bounty bulletin (usertrial/bounty_bullet.alb).
--
-- The original FillData reads the globals listCtrl / bountyRecords / index, which no shipped
-- script assigns, so any non-empty list aborts before Show(true). Its reward button reads the
-- globals curSelected / bountyRecords, which are equally unset. The 1.2 client itself stays
-- unchanged; this shim repairs the missing bindings in the Bevy client only.
-- Evidence: docs/aa-client-bounty.md.

local userTrial = rawget(_G, "userTrial")
if type(userTrial) ~= "table" then return end

-- Re-running this file (stage reload, a late first event) must not wrap the window twice.
local mt = getmetatable(userTrial)
if mt and mt.__aa_bounty_shim then
  local wnd = rawget(userTrial, "bountyBulletWnd")
  if wnd ~= nil then mt.__aa_bounty_shim(wnd) end
  return
end

-- bounty_bullet.alb FillData columns 1..5 read record fields 6, 2, 3, 7, 5.
local COLUMNS = { 6, 2, 3, 7, 5 }
local patched = setmetatable({}, { __mode = "k" })

local function setStringCell(cell, value, visible)
  cell:Show(visible ~= false)
  cell:SetText(tostring(value))
end

local function patchWindow(wnd)
  if patched[wnd] then return end
  local list = wnd.pageListCtrl
  if list == nil or list.listCtrl == nil then return end
  patched[wnd] = true

  -- Column 1 is LCCIT_STRING; the original's FillData would have written it through the
  -- unset global listCtrl. Give it the same per-cell setter the LCCIT_WINDOW columns have,
  -- so PageListCtrl:UpdateView fills it for the current page.
  local key = list.listCtrl.column[1]
  if key ~= nil and key.dataSetFunc == nil then
    key.dataSetFunc = setStringCell
  end

  local records
  local fillLeft = wnd.FillLeftFrame
  wnd.FillLeftFrame = function(self, sel, recs)
    -- The reward handler (bounty_bullet.alb main.0.1) reads exactly these two globals;
    -- ShowBountyBulletWindow and the selection callback pass the values here.
    curSelected = sel
    bountyRecords = recs
    records = recs
    return fillLeft(self, sel, recs)
  end

  wnd.FillData = function(self, page)
    -- Original behaviour: clear the page list and its selection, then append every record.
    -- FillData ignored the page argument; the PageListCtrl's UpdatePage does the paging.
    list:DeleteAllDatas()
    list.listCtrl:ClearSelection()
    if records == nil then return end
    for i = 1, #records do
      local record = records[i]
      for column = 1, #COLUMNS do
        list:InsertData(i, column, tostring(record[COLUMNS[column]]))
      end
    end
  end
end

local oldNewIndex = mt and mt.__newindex
local shim = {}
if mt then
  for field, value in pairs(mt) do shim[field] = value end
end
shim.__newindex = function(t, field, value)
  if field == "bountyBulletWnd" and value ~= nil then patchWindow(value) end
  if type(oldNewIndex) == "function" then
    oldNewIndex(t, field, value)
  elseif type(oldNewIndex) == "table" then
    oldNewIndex[field] = value
  else
    rawset(t, field, value)
  end
end
shim.__aa_bounty_shim = patchWindow
setmetatable(userTrial, shim)

local window = rawget(userTrial, "bountyBulletWnd")
if window ~= nil then patchWindow(window) end

-- The shipped setup_bounty.alb only registers TOGGLE_BOUNTY_BULLETIN; SCSetBountyPermitted's
-- SET_BOUNTY event has no script listener (the popup-menu proc is dead code).
UIParent:SetEventHandler("SET_BOUNTY", function(id, name, money)
  ShowSetupBountyWindow(id, name, money)
end)
