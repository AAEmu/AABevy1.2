-- The shipped Lua allocates130 cells; the supplied expansion table permits150 bag slots.
-- Reuse the original private factory and its existing adapter/injector closures.
return function(required)
  if not newbag or not newbag.slots then return end
  local slots=newbag.slots
  local wanted=math.max(130,required,X2Bag:ExpansionMaxSlotCount())
  if #slots.btns>=wanted then return end
  assert(wanted<=256,'inventory widget pool exceeds application input budget')
  local function upvalue(fn,want)
    for i=1,20 do
      local name,value=debug.getupvalue(fn,i)
      if name==nil then break end
      if name==want then return value end
    end
    error('original inventory closure missing '..want)
  end
  local factory=upvalue(SetViewOfItemSlots,'CreateInvenCoolDownActionButton')
  local adapter=upvalue(slots.btns[1].Update,'adapter')
  local injector=upvalue(slots.btns[1].Update,'injector')
  for idx=#slots.btns+1,wanted do
    local y,x=math.floor((idx-1)/10)+1,(idx-1)%10+1
    local button=factory('item['..x..']['..y..']',slots,adapter,injector)
    button:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
    button:SetItemIcon(nil)
    button:Show(false)
    button:SetVirtualSlotIdx(idx)
    ApplySlotSkin(button,button.back,SLOT_STYLE.BAG_DEFAULT)
    local yInset=0
    for _,row in ipairs({3,6,8,11}) do if y>=row then yInset=yInset+3 end end
    button:AddAnchor('TOPLEFT',slots,(x-1)*(ICON_SIZE.DEFAULT+5)+(x>=6 and 5 or 0),(y-1)*(ICON_SIZE.DEFAULT+5)+yInset)
    slots.btns[idx]=button
  end
end
