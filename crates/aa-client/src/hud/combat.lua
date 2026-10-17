-- The native behaviour of the combat text widgets ("damagedisplay"), run once the stage's scripts are
-- loaded. combattext/combat_text.lua fills 30 widgets for every hit, heal, miss, "Battle Start!" and
-- "XP +N" through SetUnitId, SetText, SetInitPos, the styles' SetColor, SetAnimFrameInfo and
-- Animation(true), and picks a free widget with IsVisible; the client's native widget then floats the
-- text over the unit and plays the frame table. aa-ui has no such widget, so the widget's methods are
-- replaced here: they collect what the script says and Animation(true) hands the text to the app
-- (hud/combat.rs draws it).

local frame = combatTextFrame
if frame == nil or frame.combatTexts == nil then return end

for i = 1, COMBAT_TEXT_MAX_COUNT do
  local w = frame.combatTexts[i]
  if w ~= nil then
    local pending = { x = 0, y = 0, text = "", color = { 1, 1, 1, 1 } }

    function w:SetUnitId(source, target)
      pending.target = target
    end

    function w:SetText(text)
      pending.text = tostring(text)
      -- the exp text grows while it is up (several kills in a row)
      HUD.combat_text_text(i, pending.text)
    end

    function w:SetInitPos(x, y)
      pending.x, pending.y = x or 0, y or 0
    end

    function w:SetAnimFrameInfo(frames)
      pending.frames = frames
    end

    function w:Animation(on)
      if on and pending.frames ~= nil then
        HUD.combat_text_start(i, pending.target, pending.text, pending.x, pending.y, pending.color, pending.frames)
      elseif not on then
        HUD.combat_text_stop(i)
      end
    end

    function w:IsVisible()
      return HUD.combat_text_active(i)
    end

    local function setColor(self, r, g, b, a)
      pending.color = { r, g, b, a or 1 }
    end
    w.style.SetColor = setColor
    w.extraStyle.SetColor = setColor
  end
end
