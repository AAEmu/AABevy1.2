//! Native keyboard capture, physical key names, and modifier state.
//! `EnableKeyboard` is independent of edit focus: the original binding page captures releases.

use std::collections::{HashMap, HashSet};

use bevy::input::keyboard::KeyCode;
use mlua::{Lua, MultiValue, Table, Value};

use super::{call_handler, edit, method, none, ui};
use crate::tree::{Id, Tree};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
}

#[derive(Default)]
pub struct Keyboard {
    capture: Option<Id>,
    focusable: HashSet<Id>,
    pressed: HashSet<KeyCode>,
    /// UI-owned presses remain consumed until release, even if Lua releases capture earlier.
    owned: HashMap<KeyCode, Id>,
    consumed: bool,
}

impl Keyboard {
    pub(crate) fn removed(&mut self, id: Id) {
        if self.capture == Some(id) {
            self.capture = None;
        }
        self.focusable.remove(&id);
        // Keep UI-owned key edges until release, even after their original owner is gone.
    }

    pub(crate) fn consume(&mut self) {
        self.consumed = true;
    }

    pub fn capture(&self, tree: &Tree) -> Option<Id> {
        self.capture.filter(|&id| tree.visible(id))
    }

    pub fn takes_focus(&self, id: Id) -> bool {
        self.focusable.contains(&id)
    }

    pub fn owner(&self, tree: &Tree, focus: Option<Id>) -> Option<Id> {
        self.capture(tree).or(focus.filter(|&id| tree.visible(id)))
    }

    pub fn captured(&self, tree: &Tree, focus: Option<Id>) -> bool {
        self.blocks(tree, focus) || self.consumed || !self.owned.is_empty()
    }

    fn blocks(&self, tree: &Tree, focus: Option<Id>) -> bool {
        self.capture(tree).is_some()
            || focus.is_some_and(|id| {
                tree.visible(id) && tree.widget(id).is_some_and(|w| edit::is_edit(&w.ty))
            })
    }

    pub fn modifiers(&self) -> Modifiers {
        let down = |a, b| self.pressed.contains(&a) || self.pressed.contains(&b);
        Modifiers {
            shift: down(KeyCode::ShiftLeft, KeyCode::ShiftRight),
            control: down(KeyCode::ControlLeft, KeyCode::ControlRight),
            alt: down(KeyCode::AltLeft, KeyCode::AltRight),
        }
    }
}

/// Clear the per-frame consumption latch before forwarding OS messages. Held UI keys stay owned.
pub fn begin_frame(lua: &Lua) {
    {
        let mut u = ui(lua);
        u.keyboard.consumed = false;
        if u.keyboard.capture.is_some_and(|id| !u.tree.visible(id)) {
            u.keyboard.capture = None;
        }
    }
    edit::ime::begin_frame(lua);
}

/// Losing OS focus clears held keys without inventing binding release events.
pub fn focus_lost(lua: &Lua) {
    let owner = ui(lua).focus;
    if let Some(owner) = owner {
        edit::ime::release(lua, owner);
    }
    let mut u = ui(lua);
    u.keyboard.consumed |= !u.keyboard.owned.is_empty();
    u.keyboard.owned.clear();
    u.keyboard.pressed.clear();
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "EnableKeyboard",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            if a.b(1, true) {
                // Native 392789e0 -> 392a6960 installs one visible owner; false only releases self.
                if u.tree.visible(id) {
                    u.keyboard.capture = Some(id);
                }
            } else if u.keyboard.capture == Some(id) {
                u.keyboard.capture = None;
            }
            none()
        })?,
    )?;
    m.raw_set(
        "EnableFocus",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            if a.b(1, true) {
                u.keyboard.focusable.insert(id);
            } else {
                u.keyboard.focusable.remove(&id);
            }
            none()
        })?,
    )?;
    Ok(())
}

/// Reinstall after application fallback scripts replace `X2Input` methods.
pub fn install_input(lua: &Lua) -> mlua::Result<()> {
    let input = super::globals::install_game_stub(lua, "X2Input")?;
    for (name, channel) in [
        ("IsShiftKeyDown", 0),
        ("IsControlKeyDown", 1),
        ("IsAltKeyDown", 2),
    ] {
        input.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                let m = ui(lua).keyboard.modifiers();
                Ok([m.shift, m.control, m.alt][channel])
            })?,
        )?;
    }
    Ok(())
}

fn owner(lua: &Lua) -> Option<Id> {
    let u = ui(lua);
    u.keyboard.owner(&u.tree, u.focus)
}

fn dispatch(lua: &Lua, id: Id, name: &str, down: bool, key: Option<&edit::UiKey>) {
    let name = Value::String(lua.create_string(name).unwrap());
    call_handler(
        lua,
        id,
        if down { "OnKeyDown" } else { "OnKeyUp" },
        vec![name],
    );
    if down && owner(lua) == Some(id) {
        let is_edit = ui(lua)
            .tree
            .widget(id)
            .is_some_and(|w| edit::is_edit(&w.ty));
        if is_edit
            && !edit::ime::blocks_key(lua, id)
            && let Some(key) = key
        {
            edit::press(lua, id, key);
        }
    }
}

/// Forward a physical press/release. Text is used for editing, physical names for bindings.
pub fn event(lua: &Lua, code: KeyCode, down: bool, text: Option<&str>) -> bool {
    let target = owner(lua);
    let (target, consumed) = {
        let mut u = ui(lua);
        let blocking = u.keyboard.blocks(&u.tree, u.focus);
        let k = &mut u.keyboard;
        if down {
            k.pressed.insert(code);
            if blocking && let Some(id) = target {
                k.owned.entry(code).or_insert(id);
            }
            let owned = k.owned.get(&code).copied();
            let consumed = blocking || owned.is_some();
            k.consumed |= consumed;
            (
                target.filter(|&id| owned.is_none_or(|old| old == id)),
                consumed,
            )
        } else {
            k.pressed.remove(&code);
            let old = k.owned.remove(&code);
            let consumed = blocking || old.is_some();
            k.consumed |= consumed;
            (
                target.filter(|&id| old.is_none_or(|old| old == id)),
                consumed,
            )
        }
    };
    if let Some(id) = target {
        let modifiers = ui(lua).keyboard.modifiers();
        let control = modifiers.control;
        let key = if control && modifiers.shift && code == KeyCode::Insert {
            Some(edit::UiKey::PasteAndCopy)
        } else if (control && code == KeyCode::KeyV) || (modifiers.shift && code == KeyCode::Insert)
        {
            Some(edit::UiKey::Paste)
        } else if control && matches!(code, KeyCode::KeyC | KeyCode::Insert) {
            Some(edit::UiKey::Copy)
        } else if control && code == KeyCode::KeyX {
            Some(edit::UiKey::Cut)
        } else if control && code == KeyCode::KeyA {
            Some(edit::UiKey::SelectAll)
        } else {
            edit_key(code, text).filter(|k| !control || !matches!(k, edit::UiKey::Text(_)))
        };
        if let Some(name) = native_key_name(code) {
            dispatch(lua, id, name, down, key.as_ref());
        } else if down && let Some(key) = key {
            let is_edit = ui(lua)
                .tree
                .widget(id)
                .is_some_and(|w| edit::is_edit(&w.ty));
            if is_edit && !edit::ime::blocks_key(lua, id) {
                edit::press(lua, id, &key);
            }
        }
    }
    consumed
}

/// The key name of a mouse button the game binds like a key (the left and right buttons are clicks, not keys).
pub fn button_name(button: bevy::input::mouse::MouseButton) -> Option<&'static str> {
    use bevy::input::mouse::MouseButton::*;
    match button {
        Middle => Some("mouse3"),
        Back => Some("mouse4"),
        Forward => Some("mouse5"),
        _ => None,
    }
}

/// A press or release of a mouse button the game binds like a key: the middle and the two side buttons (`mouse3`, `mouse4`,
/// `mouse5`, the names of `default_binding.g`). Only a visible key capture takes it, as `OnKeyDown` / `OnKeyUp` with that name (the
/// binding editor of the Options window); no other widget sees them as keys. True when a capture took it.
pub fn button_event(lua: &Lua, name: &str, down: bool) -> bool {
    let capture = {
        let u = ui(lua);
        u.keyboard.capture(&u.tree)
    };
    let Some(id) = capture else { return false };
    ui(lua).keyboard.consume();
    dispatch(lua, id, name, down, None);
    true
}

/// Convenience press+release for tests and programmatic typing. OS input uses `event`.
pub fn key(lua: &Lua, key: &edit::UiKey) -> bool {
    let Some(id) = owner(lua) else { return false };
    ui(lua).keyboard.consumed = true;
    let name = key.name();
    dispatch(lua, id, &name, true, Some(key));
    dispatch(lua, id, &name, false, None);
    true
}

fn edit_key(code: KeyCode, text: Option<&str>) -> Option<edit::UiKey> {
    use edit::UiKey as K;
    Some(match code {
        KeyCode::Backspace => K::Backspace,
        KeyCode::Delete => K::Delete,
        KeyCode::ArrowLeft => K::Left,
        KeyCode::ArrowRight => K::Right,
        KeyCode::ArrowUp => K::Up,
        KeyCode::ArrowDown => K::Down,
        KeyCode::Home => K::Home,
        KeyCode::End => K::End,
        KeyCode::Enter | KeyCode::NumpadEnter => K::Enter,
        KeyCode::Escape => K::Escape,
        KeyCode::Tab => K::Tab,
        _ => K::Text(
            text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))?
                .to_string(),
        ),
    })
}

/// ArcheAge 1.2 names from the native key table at 39435b00–39436d90.
/// Physical names stay unchanged by Shift/CapsLock; the Lua editor adds modifier prefixes.
pub fn native_key_name(code: KeyCode) -> Option<&'static str> {
    use KeyCode::*;
    Some(match code {
        KeyA => "a",
        KeyB => "b",
        KeyC => "c",
        KeyD => "d",
        KeyE => "e",
        KeyF => "f",
        KeyG => "g",
        KeyH => "h",
        KeyI => "i",
        KeyJ => "j",
        KeyK => "k",
        KeyL => "l",
        KeyM => "m",
        KeyN => "n",
        KeyO => "o",
        KeyP => "p",
        KeyQ => "q",
        KeyR => "r",
        KeyS => "s",
        KeyT => "t",
        KeyU => "u",
        KeyV => "v",
        KeyW => "w",
        KeyX => "x",
        KeyY => "y",
        KeyZ => "z",
        Digit0 => "0",
        Digit1 => "1",
        Digit2 => "2",
        Digit3 => "3",
        Digit4 => "4",
        Digit5 => "5",
        Digit6 => "6",
        Digit7 => "7",
        Digit8 => "8",
        Digit9 => "9",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        F13 => "f13",
        F14 => "f14",
        F15 => "f15",
        Numpad0 => "number0",
        Numpad1 => "number1",
        Numpad2 => "number2",
        Numpad3 => "number3",
        Numpad4 => "number4",
        Numpad5 => "number5",
        Numpad6 => "number6",
        Numpad7 => "number7",
        Numpad8 => "number8",
        Numpad9 => "number9",
        NumpadAdd => "number+",
        NumpadSubtract => "number-",
        NumpadMultiply => "*",
        NumpadDivide | Slash => "/",
        NumpadDecimal | Period => ".",
        Enter | NumpadEnter => "enter",
        Space => "space",
        Escape => "escape",
        Tab => "tab",
        Backspace => "backspace",
        Delete => "delete",
        Insert => "insert",
        ArrowLeft => "left",
        ArrowRight => "right",
        ArrowUp => "up",
        ArrowDown => "down",
        Home => "home",
        End => "end",
        PageUp => "pageup",
        PageDown => "pagedown",
        ShiftLeft | ShiftRight => "shift",
        ControlLeft | ControlRight => "ctrl",
        AltLeft | AltRight => "alt",
        SuperLeft | SuperRight => "windows",
        Minus => "minus",
        Equal => "equals",
        BracketLeft => "{",
        BracketRight => "}",
        Backslash | IntlBackslash => "backslash",
        Semicolon => ";",
        Quote => "apostrophe",
        Backquote => "`",
        Comma => ",",
        CapsLock => "capslock",
        NumLock => "numlock",
        ScrollLock => "scrolllock",
        PrintScreen => "print",
        Pause => "pause",
        ContextMenu => "apps",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::{UiInput, UiRuntime, UiStats};

    #[derive(Resource, Default)]
    struct GameReleases(u32);
    use bevy::input::keyboard::{Key, KeyboardInput};
    use bevy::input::{ButtonState, InputPlugin};
    use bevy::prelude::*;

    #[test]
    fn bevy_input_set_publishes_capture_through_the_release_frame() {
        let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
        ui.exec(
            "@capture",
            r#"
            frame = UIParent:CreateWidget('emptywidget', 'capture', 'UIParent')
            frame:Show(true); frame:EnableKeyboard(true)
            up = 0
            frame:SetHandler('OnKeyUp', function(self, key)
                up = up + 1; self:EnableKeyboard(false)
            end)
        "#,
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<bevy::window::Ime>()
            .init_resource::<UiStats>()
            .init_resource::<GameReleases>()
            .insert_non_send(ui)
            .add_systems(Update, crate::ui_keyboard.in_set(UiInput))
            .add_systems(
                Update,
                (|stats: Res<UiStats>,
                  keys: Res<ButtonInput<KeyCode>>,
                  mut game: ResMut<GameReleases>| {
                    if !stats.keyboard_focus && keys.just_released(KeyCode::F2) {
                        game.0 += 1;
                    }
                })
                .after(UiInput),
            );
        let send = |app: &mut App, state| {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::F2,
                logical_key: Key::F2,
                state,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
            app.update();
        };
        send(&mut app, ButtonState::Pressed);
        assert!(app.world().resource::<UiStats>().keyboard_focus);
        assert_eq!(
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("up")
                .eval::<u32>()
                .unwrap(),
            0
        );
        send(&mut app, ButtonState::Released);
        assert!(app.world().resource::<UiStats>().keyboard_focus);
        assert_eq!(
            app.world()
                .non_send::<UiRuntime>()
                .lua
                .load("up")
                .eval::<u32>()
                .unwrap(),
            1
        );
        app.update();
        assert!(!app.world().resource::<UiStats>().keyboard_focus);
        assert_eq!(app.world().resource::<GameReleases>().0, 0);
        send(&mut app, ButtonState::Pressed);
        send(&mut app, ButtonState::Released);
        assert_eq!(app.world().resource::<GameReleases>().0, 1);
    }
}
