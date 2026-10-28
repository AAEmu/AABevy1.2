-- Picking up and dropping items and skills: what the client does natively behind X2Bag:HandleClick, X2Equipment
-- :PickupEquippedItem and the `slot` widgets, on top of the toolkit's pointer hand (X2Cursor:SetHeld, aa-ui lua/drag.rs).
--
-- What is held (X2Cursor:GetHeld()):
--   { kind = "bag",    slot = real bag slot (1 based), count, total, partial, id = item type, grade, icon, frame }
--   { kind = "equip",  slot = ES_* (equipment slot, 1 based), id, grade, icon, frame }
--   { kind = "action", slot = action bar slot (1 based), what = "skill" | "item", id, icon, frame, count }
--   { kind = "skill",  id = skill id, icon }        a skill of the skill window (a copy)
--   { kind = "item",   id = item type, grade, icon, frame }
-- The moves themselves are requests to the server (HUD.drag_*, hud/drag.rs); the windows follow the server's answer.

local CONTAINER_EQUIPMENT, CONTAINER_BAG = 1, 2

local GRADE_NAMES = { [0] = "poor", "common", "uncommon", "rare", "ancient", "heroic", "unique", "artifact",
                      "wonder", "epic", "legendary", "mythic" }
local function gradeFrame(grade)
  local name = GRADE_NAMES[grade or -1]
  if name == nil then return nil end
  return string.format("ui/icon/item_grade_%d%s.dds", grade, name)
end

-- What the pointer holds, if it holds something that can be dropped.
local function held()
  local h = X2Cursor:GetHeld()
  if h == nil or h.kind == "image" then return nil end
  return h
end

local function bagKnown() return HUD.bag_slots ~= 0 end

-- The real bag slot behind a cell of the bag window. An empty cell of the All tab is the slot itself; in a
-- filtered tab it is the first free slot.
local function realSlot(virtual)
  if HUD.inventory_dest and HUD.inventory_owner() ~= 0 then return HUD.inventory_dest(CONTAINER_BAG, virtual) end
  local real = X2Bag:SlotByIdx(virtual)
  if real ~= nil then return real end
  if virtual < 1 or virtual > HUD.bag_slots then return nil end
  if X2Bag:CurrentTabIdx() == 1 then return virtual end
  for i = 1, HUD.bag_slots do
    if HUD.bag[i] == nil then return i end
  end
  return nil
end

local function holdBag(real, amount)
  local s = HUD.bag[real]
  if s == nil then return end
  local count = amount or s.count
  X2Cursor:SetHeld({
    kind = "bag", cursorInfo = s.stack and "Item" or "Item_Id", slot = real, uid = s.uid, id = s.id, grade = s.grade, icon = s.icon, frame = gradeFrame(s.grade),
    count = count, total = s.count, partial = count < s.count,
  })
end

-- Where an item can be worn (ES_*), in the order it is tried.
local function wornSlots(itemType)
  return HUD.equip_slots(itemType) or {}
end

local function canWearAt(itemType, es)
  for _, slot in ipairs(wornSlots(itemType)) do
    if slot == es then return true end
  end
  return false
end

-- The slot a right click wears an item in: the first free one it fits, else the first it fits.
local function autoSlot(itemType)
  local slots = wornSlots(itemType)
  for _, es in ipairs(slots) do
    if HUD.equipment[es] == nil then return es end
  end
  return slots[1]
end

-- X2Bag ------------------------------------------------------------------------------------------------------
-- A click or the start / end of a drag on a bag cell: pick the item up, or drop what is held there.
X2Bag.HandleClick = function(self, virtual)
  if not bagKnown() then return end
  local real = realSlot(virtual)
  if real == nil then return end
  if X2Bag:IsLocked(real) then return end
  if HUD.look_target and HUD.look_target(real, "LeftButton") then return end
  local h = held()
  if h == nil then
    local s = HUD.bag[real]
    if s == nil then return end
    -- Shift + click on a stack asks how many (INVEN_SLOT_SPLIT opens the bag's amount window)
    if HUD.shift_down() and s.count > 1 and s.stack then
      FAKE.fire_now("INVEN_SLOT_SPLIT", "bag", real)
      return
    end
    holdBag(real)
  elseif h.kind == "bag" then
    if h.slot ~= real and HUD.bag[h.slot] ~= nil then
      local target = HUD.bag[real]
      if h.partial and target ~= nil and target.id ~= h.id then
        return -- part of a stack goes to an empty cell or onto the same kind
      end
      HUD.drag_move(CONTAINER_BAG, h.slot, CONTAINER_BAG, real, h.partial and h.count or 0)
    end
    X2Cursor:ClearCursor()
  elseif h.kind == "equip" then
    HUD.drag_move(CONTAINER_EQUIPMENT, h.slot, CONTAINER_BAG, real, 0)
    X2Cursor:ClearCursor()
  else
    X2Cursor:ClearCursor()
  end
end

-- Pick part of a stack up (the amount window of Shift + click).
X2Bag.PickupBagItemPartial = function(self, bagId, slot, amount)
  local s = HUD.bag[slot]
  amount = tonumber(amount)
  if s == nil or amount == nil or amount < 1 then return end
  holdBag(slot, math.min(amount, s.count))
end

X2Bag.HasPickedItem = function(self)
  local h = held()
  return h ~= nil and h.kind == "bag"
end

-- A right click on a bag item: gear is worn, anything else is used.
X2Bag.HandleUse = function(self, virtual)
  local real = realSlot(virtual)
  local s = real and HUD.bag[real]
  if s == nil then return end
  if X2Bag:IsLocked(real) then return end
  if HUD.look_target and HUD.look_target(real, "RightButton") then return end
  local es = autoSlot(s.id)
  if es ~= nil then
    HUD.drag_equip(real, es)
  elseif AAUI.UseSlot ~= nil then
    AAUI.UseSlot(ISLOT_BAG, real, "RightButton")
  end
end

-- Wear the bag item (the candidates listed under an equipment slot).
X2Bag.EquipBagItem = function(self, bagSlot, isAux)
  local s = HUD.bag[bagSlot]
  local es = s and autoSlot(s.id)
  if es ~= nil then HUD.drag_equip(bagSlot, es) end
end

-- X2Equipment ------------------------------------------------------------------------------------------------
-- A click or a drag on an equipment slot: pick the worn item up, or drop what is held (wear it).
X2Equipment.PickupEquippedItem = function(self, es)
  if not bagKnown() then return end
  local h = held()
  if h == nil then
    local s = HUD.equipment[es]
    if s == nil then return end
    X2Cursor:SetHeld({ kind = "equip", cursorInfo = s.stack and "Item" or "Item_Id", slot = es, uid = s.uid, id = s.id, grade = s.grade, icon = s.icon, frame = gradeFrame(s.grade) })
  elseif h.kind == "bag" then
    if not h.partial and canWearAt(h.id, es) then HUD.drag_equip(h.slot, es) end
    X2Cursor:ClearCursor()
  elseif h.kind == "equip" then
    -- two worn items change places (rings, earrings, the hands)
    local other = HUD.equipment[h.slot]
    if h.slot ~= es and other ~= nil and canWearAt(other.id, es) then
      HUD.drag_move(CONTAINER_EQUIPMENT, h.slot, CONTAINER_EQUIPMENT, es, 0)
    end
    X2Cursor:ClearCursor()
  else
    X2Cursor:ClearCursor()
  end
end

X2Equipment.IsInventoriable = function(self, es) return true end

-- Take a worn item off into the first free bag slot.
X2Equipment.UnequipItem = function(self, es)
  if HUD.equipment[es] ~= nil then HUD.drag_unequip(es) end
end

-- The slot widgets (action bars, skill window) --------------------------------------------------------------
-- A bound slot was dragged: the content of an action bar slot (unless the bar is locked: Shift unlocks it for a
-- move), a skill of the skill window or an item of a slot bound to one is picked up.
AAUI.PickupSlot = function(info)
  local content = info.content
  if content == nil then return end
  if info.kind == "slot" and info.slotType == ISLOT_ACTION then
    local locked = IsActionBarLocked ~= nil and IsActionBarLocked() == ACTIONBAR_STATE.LOCK
    if locked and not HUD.shift_down() then return end
    local s = HUD.actions[HUD.action_slot(info.slotIdx) or info.slotIdx]
    if s == nil then return end
    X2Cursor:SetHeld({
      kind = "action", cursorInfo = s.kind == "skill" and "Spell" or "Item", slot = info.slotIdx, what = s.kind, id = s.id, icon = s.icon,
      frame = s.kind == "item" and gradeFrame(s.grade) or nil, count = s.stack and s.count or 0,
    })
  elseif content.type == "skill" and content.learned ~= false then
    X2Cursor:SetHeld({ kind = "skill", cursorInfo = "Spell", id = content.id, icon = content.icon })
  elseif content.type == "item" then
    X2Cursor:SetHeld({ kind = "item", cursorInfo = "Item", id = content.id, grade = content.grade, icon = content.icon, frame = gradeFrame(content.grade) })
  end
end

-- Something held was dropped on a slot (a left click on a slot does the same): the action bar takes skills and items.
AAUI.DropSlot = function(info)
  local h = held()
  if h == nil then return end
  if info.slotType == ISLOT_ACTION then
    if h.kind == "action" then
      if h.slot ~= info.slotIdx then HUD.drag_action_swap(h.slot, info.slotIdx) end
    elseif h.kind == "skill" then
      HUD.drag_action_set(info.slotIdx, "skill", h.id)
    elseif h.kind == "bag" or h.kind == "equip" or h.kind == "item" then
      HUD.drag_action_set(info.slotIdx, "item", h.id)
    end
  end
  X2Cursor:ClearCursor()
end

-- Dropped on the world (overUi false) or on a window that takes nothing: a thing from the bag or the character window
-- asks whether to destroy the item, an icon of the action bar leaves the bar, the rest goes back.
AAUI.DropOnNothing = function(overUi)
  if HUD.look_world_drop and HUD.look_world_drop(overUi) then return end
  local h = held()
  if h == nil then return end
  X2Cursor:ClearCursor()
  if overUi then return end
  if h.kind == "bag" or h.kind == "equip" then
    HUD.ask_destroy(h)
  elseif h.kind == "action" then
    HUD.drag_action_set(h.slot, nil)
  end
end

-- The confirmation of the original (DLG_TASK_DESTROY_ITEM, dialoghandler/dialog_handler.lua: "Do you want to destroy
-- the item?" with the item, a Destroy button).
HUD.ask_destroy = function(h)
  local container = h.kind == "bag" and CONTAINER_BAG or CONTAINER_EQUIPMENT
  local s = h.kind == "bag" and HUD.bag[h.slot] or HUD.equipment[h.slot]
  if s == nil then return end
  local count = h.kind == "bag" and h.count or 1
  local info = HUD.item_tip(s.id, s.grade, count)
  if info == nil then
    info = { name = s.name, gradeColor = "FFFFFFFF", itemType = s.id, lookType = s.id, itemGrade = s.grade }
  end
  info.itemType = info.itemType or s.id
  info.lookType = info.lookType or s.id
  info.itemGrade = info.itemGrade or s.grade
  local wnd = X2DialogManager:RequestTaskDialog(DLG_TASK_DESTROY_ITEM, {
    itemInfo = info, itemType = s.id, itemCount = count, affectQuest = 0,
  }, "destroyItem")
  if wnd == nil then return end
  wnd.OkProc = function() HUD.drag_destroy(container, h.slot, count) end
end
