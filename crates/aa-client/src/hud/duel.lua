-- Duel dialogs. The original raises DLG_TASK_CHALLENGE_DUEL and the "duel_result" message box from
-- native code with the texts of locale_helper (`locale.duel`, `locale.messageBox.duel_result`); no
-- script registers the dialog, so this builds the same default dialog from those texts.
local challenged
function HUD.duel_show_challenge(challenger, name)
  if challenged then X2DialogManager:DeleteByOwnerWindow("duelChallenge") end
  challenged = challenger
  X2DialogManager:RequestDefaultDialog(function(wnd)
    wnd:SetTitle(locale.duel.challenge)
    wnd:SetContent(locale.duel.ask_duel_msg(name))
    wnd.OkProc = function()
      if challenged == challenger then challenged = nil; HUD.duel_answer(challenger, true) end
    end
    wnd.CancelProc = function()
      if challenged == challenger then challenged = nil; HUD.duel_answer(challenger, false) end
    end
  end, "duelChallenge")
end
-- The challenge went away (the duel started, ended or the world was left).
function HUD.duel_close_challenge()
  if challenged then challenged = nil; X2DialogManager:DeleteByOwnerWindow("duelChallenge") end
end
-- Presses the open prompt's OK / Cancel (the script runner's `lua HUD.duel_press(true)`).
function HUD.duel_press(ok)
  local who = challenged
  if who then challenged = nil; X2DialogManager:DeleteByOwnerWindow("duelChallenge"); HUD.duel_answer(who, ok) end
end
function HUD.duel_show_result(key)
  local box = locale.messageBox.duel_result
  X2DialogManager:RequestNoticeDialog(function(wnd)
    wnd:SetTitle(box.title)
    wnd:SetContent(box.body(key))
  end, "duelResult")
end
-- "Duel will begin in <n> sec." (`DUEL_TEXT` "countdown").
function HUD.duel_countdown_text(seconds)
  return X2Locale:LocalizeUiText(DUEL_TEXT, "countdown", tostring(seconds))
end
-- "Duel with <name> started." (`DUEL_TEXT` "start").
function HUD.duel_start_text(name)
  return X2Locale:LocalizeUiText(DUEL_TEXT, "start", name)
end
