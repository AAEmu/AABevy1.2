//! Sounds the scripts ask for, handed to the app through `AAUI` hooks (the app owns the audio):
//!
//! ```text
//! AAUI.PlaySound(name, duplicable) -> id | nil   X2Sound:PlayUISound(name, duplicable): a sound pack item
//!                                                (`login_stage_try_login`, `event_mail_alarm` ...); the id goes to StopSound
//! AAUI.StopSound(id, mode)                       X2Sound:StopSound(id, mode); `mode` 1 = SOUND_STOP_MODE_AT_ONCE
//! AAUI.IsSoundPlaying(id) -> bool                X2Sound:IsPlaying(id)
//! AAUI.PlayMusic(name)                           X2Sound:PlayMusic(name): a pack item (`login_stage_music_creator` ...)
//! AAUI.StopMusic()                               X2Sound:StopMusic()
//! AAUI.WidgetSound(set, event)                   a widget with `SetSounds(set)` reached `event`
//! ```
//!
//! `WidgetSound` events: `show` / `hide` (a `Show` that changed the widget's state), `left_up` / `right_up`
//! (a click on an enabled button), `over` (the pointer comes onto a button), `text_added` / `text_deleted`
//! (typing in an edit box). The game's sound packs name the sound `<set>_<event>`: `window` pack
//! (`bag_show`), `button` pack (`default_left_up`, `default_over`), `global` pack (`edit_box_text_added`).
//!
//! Every hook is optional: without it the call does nothing, as before. Hooks are called outside the
//! toolkit's borrows, so they may use any native.

use mlua::{Function, Lua, Table};

use super::ui;
use crate::tree::Id;

fn hook(lua: &Lua, name: &str) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get(name).ok()
}

/// `widget:SetSounds(set)`: the set name the widget's events are played under (`""` clears it).
pub(crate) fn set_sounds(lua: &Lua, id: Id, set: String) {
    let mut u = ui(lua);
    if set.is_empty() {
        u.widget_sounds.remove(&id);
    } else {
        u.widget_sounds.insert(id, set);
    }
}

/// Tells the app that `event` happened to the widget `id`, when it has a sound set.
pub(crate) fn widget_sound(lua: &Lua, id: Id, event: &str) {
    let set = {
        let u = ui(lua);
        if u.widget_sounds.is_empty() {
            return;
        }
        match u.widget_sounds.get(&id) {
            Some(s) => s.clone(),
            None => return,
        }
    };
    let Some(f) = hook(lua, "WidgetSound") else { return };
    if let Err(e) = f.call::<()>((set, event)) {
        bevy::log::warn!("aa-ui: AAUI.WidgetSound({event}): {e}");
    }
}

/// The pointer came onto `id`: an enabled widget with a sound set plays `over`.
pub(crate) fn over_sound(lua: &Lua, id: Id) {
    let enabled = {
        let u = ui(lua);
        !u.widget_sounds.is_empty() && u.widget_sounds.contains_key(&id) && u.tree.widget(id).is_some_and(|w| w.enabled)
    };
    if enabled {
        widget_sound(lua, id, "over");
    }
}

/// The nearest button at or above `target` that has a sound set and plays `event` for a click.
pub(crate) fn click_sound(lua: &Lua, target: Id, button: &str) {
    let found = {
        let u = ui(lua);
        if u.widget_sounds.is_empty() {
            return;
        }
        let mut t = Some(target);
        let mut found = None;
        while let Some(id) = t {
            if let Some(w) = u.tree.widget(id) {
                if u.widget_sounds.contains_key(&id) {
                    found = w.enabled.then_some(id);
                    break;
                }
                // a button without a set of its own is silent: the click stops at the first button
                if w.button.is_some() {
                    break;
                }
            }
            t = u.tree.get(id).parent;
        }
        found
    };
    if let Some(id) = found {
        widget_sound(lua, id, if button == "RightButton" { "right_up" } else { "left_up" });
    }
}

/// The `X2Sound` natives, forwarding to the hooks above.
pub(crate) const X2SOUND_LUA: &str = r#"
function X2Sound:PlayUISound(name, duplicable)
  local f = AAUI.PlaySound
  if f and name ~= nil then return f(tostring(name), duplicable) end
end
function X2Sound:StopSound(id, mode)
  local f = AAUI.StopSound
  if f and id ~= nil then f(id, mode) end
end
function X2Sound:IsPlaying(id)
  local f = AAUI.IsSoundPlaying
  if f and id ~= nil then return f(id) end
  return false
end
function X2Sound:PlayMusic(name)
  local f = AAUI.PlayMusic
  if f and name ~= nil then f(tostring(name)) end
end
function X2Sound:StopMusic()
  local f = AAUI.StopMusic
  if f then f() end
end
"#;
