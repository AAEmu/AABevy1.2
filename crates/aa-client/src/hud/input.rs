//! Who gets the keyboard and the mouse: the HUD's edit boxes and windows, or the game.
//!
//! * While an edit box has the keyboard focus (the chat input) the game's key bindings must not
//!   fire: [`swallow_keys`] clears `ButtonInput<KeyCode>` for the game's systems that frame (the UI
//!   itself reads the `KeyboardInput` messages, which stay), and [`HudKeyboardFocus`] says so.
//! * [`HudPointerOverUi`] is true while the mouse is over a clickable widget of the HUD, or a button
//!   that went down over one is still held (a window being dragged): the camera orbit and the click
//!   to move should ignore that frame.
//! * Configured window actions go to the toolkit's `hotkey`; Escape window/target handling stays
//!   in `targeting::escape_key`.

use aa_ui::{UiRuntime, UiVirtualPointer};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::HudKeyboardFocus;

/// True while the mouse is over the HUD: the game should not take the mouse this frame.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudPointerOverUi(pub bool);

/// Clears the keys for the game while an edit box has the keyboard (runs right after Bevy turned
/// the OS events into `ButtonInput`, before any game system looks at them). The focus flag is the
/// last frame's; the keys that opened the input were taken by the HUD anyway.
pub fn swallow_keys(stats: Res<aa_ui::UiStats>, focus: Res<HudKeyboardFocus>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if (stats.keyboard_focus || focus.0) && (keys.get_pressed().next().is_some() || keys.get_just_released().next().is_some()) {
        keys.reset_all();
    }
}

/// Whether the cursor is over a clickable widget; recomputed only when the cursor, the buttons or the
/// widget tree changed.
#[allow(clippy::type_complexity)]
pub fn pointer_over_ui(
    rt: Option<NonSend<UiRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    virt: Res<UiVirtualPointer>,
    mut over: ResMut<HudPointerOverUi>,
    mut cache: Local<(Option<(Vec2, u64, bool)>, bool, bool)>,
) {
    let Some(rt) = rt else {
        if over.0 {
            over.0 = false;
        }
        return;
    };
    let Ok(window) = windows.single() else { return };
    let scale = rt.ui_scale().max(0.01);
    let sf = window.resolution.scale_factor();
    let cursor = match virt.pos {
        Some(p) => Some(p / scale),
        None => window.cursor_position().map(|p| p * sf / scale),
    };
    let generation = aa_ui::lua::ui(&rt.lua).tree.generation;
    let held = mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right);
    let key = cursor.map(|c| (c, generation, held));
    let (last, picked, latched) = &mut *cache;
    if *last != key || mouse.get_just_pressed().next().is_some() {
        *last = key;
        *picked = cursor.is_some_and(|c| aa_ui::input::pick(&rt.lua, c.x, c.y).is_some());
    }
    // a button that went down over the HUD keeps the mouse until it is released
    if mouse.just_pressed(MouseButton::Left) || mouse.just_pressed(MouseButton::Right) {
        *latched = *picked;
    } else if !held {
        *latched = false;
    }
    let now = *picked || *latched;
    if over.0 != now {
        over.0 = now;
    }
}

/// The client's window key bindings.
pub fn hotkeys(actions: Res<crate::settings::Actions>, focus: Res<HudKeyboardFocus>, rt: Option<NonSend<UiRuntime>>) {
    let Some(rt) = rt else { return };
    if focus.0 || rt.keyboard_captured() {
        return;
    }
    for (action, index, button) in actions.hotkeys() {
        if button {
            if let Ok(Some(value)) = rt.lua.globals().get::<Option<mlua::Value>>(action)
                && let Some(id) = aa_ui::lua::value_id(&rt.lua, &value)
            {
                let enabled = {
                    let u = aa_ui::lua::ui(&rt.lua);
                    u.tree.visible(id) && u.tree.widget(id).is_some_and(|w| w.enabled)
                };
                if enabled {
                    aa_ui::lua::call_handler(&rt.lua, id, "OnClick", vec![mlua::Value::String(rt.lua.create_string("LeftButton").unwrap())]);
                }
            }
        } else if index == 0 && action == "toggle_post" {
            super::mail::toggle(&rt);
        } else if index == 0
            && action != "open_config"
            && action != "open_chat"
            && let Err(e) = rt.hotkey(action)
        {
            warn!("hud: {action}: {e}");
        }
    }
}
