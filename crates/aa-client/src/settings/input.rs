//! Native key-down matching and key-up release, shared by movement and HUD actions.
use std::collections::{HashMap, HashSet};

use aa_ui::UiRuntime;
use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use super::bindings::{Binding, Keymap};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Action {
    name: String,
    index: u32,
    button: bool,
}

impl From<&Binding> for Action {
    fn from(b: &Binding) -> Self {
        Self {
            name: b.action.clone(),
            index: b.index,
            button: b.kind == 2,
        }
    }
}

#[derive(Resource, Default)]
pub struct Actions {
    physical: HashSet<KeyCode>,
    latched: HashMap<KeyCode, Action>,
    pressed: HashSet<Action>,
    tapped: Vec<Action>,
    mouse_latched: HashMap<MouseButton, Action>,
    primed: bool,
    zoom_lines: f32,
}

impl Actions {
    pub fn pressed(&self, action: &str, index: u32) -> bool {
        self.pressed.iter().any(|a| !a.button && a.name == action && a.index == index)
    }

    pub fn just_pressed(&self, action: &str, index: u32) -> bool {
        self.tapped.iter().any(|a| !a.button && a.name == action && a.index == index)
    }

    pub fn hotkeys(&self) -> impl Iterator<Item = (&str, u32, bool)> {
        self.tapped.iter().map(|a| (a.name.as_str(), a.index, a.button))
    }

    /// A tap of `action` as if its key had just gone down (flow tests of the systems that read it).
    #[cfg(test)]
    pub(crate) fn tap_for_test(&mut self, action: &str) {
        self.tap_for_test_at(action, 0);
    }

    /// A tap of the `index`th bound `action` (the multi-key action bars).
    #[cfg(test)]
    pub(crate) fn tap_for_test_at(&mut self, action: &str, index: u32) {
        let action = Action { name: action.into(), index, button: false };
        self.pressed.insert(action.clone());
        self.tapped.push(action);
    }

    /// Holds `action` down for the flow tests (`pressed`, no tap).
    #[cfg(test)]
    pub(crate) fn hold_for_test(&mut self, action: &str) {
        let action = Action {
            name: action.into(),
            index: 0,
            button: false,
        };
        self.pressed.insert(action);
    }

    /// Releases a held `action`.
    #[cfg(test)]
    pub(crate) fn release_for_test(&mut self, action: &str) {
        let action = Action {
            name: action.into(),
            index: 0,
            button: false,
        };
        self.pressed.remove(&action);
    }

    /// Clears the one-frame taps after a test frame (the input system does this in the app).
    #[cfg(test)]
    pub(crate) fn clear_taps_for_test(&mut self) {
        self.tapped.clear();
    }

    pub fn zoom_lines(&self) -> f32 {
        self.zoom_lines
    }

    fn modifiers(&self) -> (bool, bool, bool) {
        let any = |a, b| self.physical.contains(&a) || self.physical.contains(&b);
        (
            any(KeyCode::ControlLeft, KeyCode::ControlRight),
            any(KeyCode::AltLeft, KeyCode::AltRight),
            any(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        )
    }

    fn begin(&mut self, blocked: bool) {
        self.tapped.clear();
        self.zoom_lines = 0.0;
        if blocked {
            self.latched.clear();
            self.pressed.clear();
            self.mouse_latched.clear();
        }
    }

    fn edge(&mut self, code: KeyCode, down: bool, repeat: bool, blocked: bool, map: &Keymap, modes: &[&str]) {
        if !down {
            self.physical.remove(&code);
            self.latched.remove(&code);
            self.refresh_pressed();
            return;
        }
        let modifiers = self.modifiers();
        if !self.physical.insert(code) || repeat || blocked {
            return;
        }
        let Some(key) = aa_ui::native_key_name(code) else { return };
        if let Some(action) = resolve(key, modifiers, map, modes) {
            self.latched.insert(code, action.clone());
            self.pressed.insert(action.clone());
            self.tap(action, 1.0);
        }
    }

    fn refresh_pressed(&mut self) {
        self.pressed = self.latched.values().chain(self.mouse_latched.values()).cloned().collect();
    }

    fn tap(&mut self, action: Action, amount: f32) {
        if action.name == "zoom_in" {
            self.zoom_lines += amount;
        }
        if action.name == "zoom_out" {
            self.zoom_lines -= amount;
        }
        self.tapped.push(action);
    }

    fn mouse(&mut self, button: MouseButton, down: bool, blocked: bool, map: &Keymap, modes: &[&str]) {
        if !down {
            self.mouse_latched.remove(&button);
            self.refresh_pressed();
            return;
        }
        if blocked || self.mouse_latched.contains_key(&button) {
            return;
        }
        let key = match button {
            MouseButton::Middle => "mouse3",
            MouseButton::Back => "mouse4",
            MouseButton::Forward => "mouse5",
            _ => return,
        };
        if let Some(action) = resolve(key, self.modifiers(), map, modes) {
            self.mouse_latched.insert(button, action.clone());
            self.pressed.insert(action.clone());
            self.tap(action, 1.0);
        }
    }
}

fn resolve(key: &str, modifiers: (bool, bool, bool), map: &Keymap, modes: &[&str]) -> Option<Action> {
    let mut full = String::new();
    if modifiers.0 {
        full.push_str("CTRL-");
    }
    if modifiers.1 {
        full.push_str("ALT-");
    }
    if modifiers.2 {
        full.push_str("SHIFT-");
    }
    full.push_str(key);
    // Native393fc270 tries exact and then ALT-only fallback within each active mode,
    // then normal mode. CTRL/SHIFT never fall back; releases use the physical latch.
    let without_alt = modifiers.1.then(|| full.replacen("ALT-", "", 1));
    for mode in modes.iter().copied().chain(std::iter::once("normal_mode")) {
        let found = map
            .live
            .iter()
            .find(|b| b.mode == mode && b.key == full)
            .or_else(|| without_alt.as_ref().and_then(|key| map.live.iter().find(|b| b.mode == mode && b.key == *key)));
        if let Some(binding) = found {
            let action = Action::from(binding);
            return Some(action);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    mut input: MessageReader<KeyboardInput>,
    mut lost: MessageReader<KeyboardFocusLost>,
    mut mouse: MessageReader<MouseButtonInput>,
    mut wheel: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<crate::hud::api::HudBridge>,
    over: Res<crate::hud::HudPointerOverUi>,
    mut actions: ResMut<Actions>,
) {
    let shared = rt
        .as_ref()
        .and_then(|rt| rt.lua.app_data_ref::<super::bindings::Context>().map(|c| c.0.clone()));
    let Ok(shared) = shared.map(Ok).unwrap_or_else(|| super::bindings::map(&super::shared())) else {
        input.clear();
        lost.clear();
        mouse.clear();
        wheel.clear();
        actions.begin(true);
        return;
    };
    let map = shared.lock().unwrap_or_else(|e| e.into_inner());
    let blocked = !map.enabled || rt.is_some_and(|rt| rt.keyboard_captured());
    actions.begin(blocked);
    if lost.read().next().is_some() {
        lost.clear();
        input.clear();
        mouse.clear();
        wheel.clear();
        actions.physical.clear();
        actions.primed = false;
        actions.begin(true);
        return;
    }
    let modes: Vec<_> = bridge.0.borrow().input_modes.priority() .collect();
    let events: Vec<_> = input.read().collect();
    if !actions.primed {
        actions.physical = keys.get_pressed().copied().collect();
        // Reconstruct the state before this frame's ordered edges. Do not turn
        // keys already held on world entry into new action presses.
        for event in events.iter().rev() {
            if event.state.is_pressed() {
                actions.physical.remove(&event.key_code);
            } else {
                actions.physical.insert(event.key_code);
            }
        }
        actions.primed = true;
    }
    for event in events {
        actions.edge(event.key_code, event.state.is_pressed(), event.repeat, blocked, &map, &modes);
    }
    for event in mouse.read() {
        actions.mouse(event.button, event.state.is_pressed(), blocked || over.0, &map, &modes);
    }
    for event in wheel.read() {
        if blocked || over.0 || event.y == 0.0 {
            continue;
        }
        let key = if event.y > 0.0 { "wheelup" } else { "wheeldown" };
        let amount = match event.unit {
            MouseScrollUnit::Line => event.y.abs(),
            MouseScrollUnit::Pixel => event.y.abs() / 40.0,
        };
        if let Some(action) = resolve(key, actions.modifiers(), &map, &modes) {
            actions.tap(action, amount);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_modes_resolve_in_reverse_chronology_with_per_mode_modifier_fallback()
    -> anyhow::Result<()> {
        for order in [
            [1, 2, 4],
            [1, 4, 2],
            [2, 1, 4],
            [2, 4, 1],
            [4, 1, 2],
            [4, 2, 1],
        ] {
            let mut modes = crate::settings::InputModes::default();
            let mut map = Keymap::new(None)?;
            map.live.clear();
            for index in [1, 2, 4] {
                map.live.push(Binding {
                    manager: 1,
                    mode: super::super::input_modes::NAMES[index as usize].into(),
                    action: format!("mode{index}"),
                    index: 0,
                    key: "r".into(),
                    kind: 0,
                });
            }
            for index in order {
                modes.set(index, 1);
            }
            let priority: Vec<_> = modes.priority().collect();
            assert_eq!(
                resolve("r", (false, false, false), &map, &priority)
                    .unwrap()
                    .name,
                format!("mode{}", order[2])
            );
            // A newer mode's ALT fallback wins before the older mode's exact chord.
            map.live.push(Binding {
                manager: 1,
                mode: super::super::input_modes::NAMES[order[0] as usize].into(),
                action: "older_exact".into(),
                index: 0,
                key: "ALT-r".into(),
                kind: 0,
            });
            assert_eq!(
                resolve("r", (false, true, false), &map, &priority)
                    .unwrap()
                    .name,
                format!("mode{}", order[2])
            );
            modes.set(order[0], 1);
            assert_eq!(
                resolve(
                    "r",
                    (false, false, false),
                    &map,
                    &modes.priority().collect::<Vec<_>>()
                )
                .unwrap()
                .name,
                format!("mode{}", order[0])
            );
            modes.set(order[0], 0);
            assert_eq!(
                resolve(
                    "r",
                    (false, false, false),
                    &map,
                    &modes.priority().collect::<Vec<_>>()
                )
                .unwrap()
                .name,
                format!("mode{}", order[2])
            );
        }
        Ok(())
    }

    #[test]
    fn physical_release_clears_movement_even_after_modifiers_change() -> anyhow::Result<()> {
        let map = Keymap::new(None)?;
        let mut a = Actions::default();
        a.edge(KeyCode::KeyW, true, false, false, &map, &[]);
        assert!(a.pressed("moveforward", 0) && a.just_pressed("moveforward", 0));
        a.begin(false);
        a.edge(KeyCode::ShiftLeft, true, false, false, &map, &[]);
        assert!(a.pressed("moveforward", 0));
        a.edge(KeyCode::KeyW, false, false, false, &map, &[]);
        assert!(!a.pressed("moveforward", 0));
        a.edge(KeyCode::KeyW, true, false, false, &map, &[]);
        assert!(!a.pressed("moveforward", 0), "SHIFT does not fall back to plain W");
        a.edge(KeyCode::KeyW, false, false, false, &map, &[]);
        a.edge(KeyCode::ShiftLeft, false, false, false, &map, &[]);
        a.edge(KeyCode::AltLeft, true, false, false, &map, &[]);
        a.edge(KeyCode::KeyW, true, false, false, &map, &[]);
        assert!(a.pressed("moveforward", 0), "ALT falls back to plain W");
        a.begin(true);
        assert!(!a.pressed("moveforward", 0));
        a.begin(false);
        a.edge(KeyCode::KeyW, true, true, false, &map, &[]);
        assert!(!a.pressed("moveforward", 0), "captured held key cannot reactivate from repeat");
        Ok(())
    }

    #[test]
    fn active_quest_mode_and_primary_secondary_keys_keep_original_actions() -> anyhow::Result<()> {
        let map = Keymap::new(None)?;
        let mut a = Actions::default();
        a.edge(KeyCode::KeyD, true, false, false, &map, &["quest_directing_mode"]);
        assert!(a.just_pressed("quest_directing_interaction", 1));
        assert!(!a.pressed("turnright", 0));
        a.edge(KeyCode::KeyW, true, false, false, &map, &[]);
        a.edge(KeyCode::ArrowUp, true, false, false, &map, &[]);
        a.edge(KeyCode::KeyW, false, false, false, &map, &[]);
        assert!(a.pressed("moveforward", 0));
        a.edge(KeyCode::ArrowUp, false, false, false, &map, &[]);
        assert!(!a.pressed("moveforward", 0));
        a.begin(false);
        a.mouse(MouseButton::Back, true, false, &map, &[]);
        assert!(a.just_pressed("autorun", 0) && a.pressed("autorun", 0));
        a.mouse(MouseButton::Back, false, false, &map, &[]);
        assert!(!a.pressed("autorun", 0));
        let wheel = resolve("wheelup", (false, false, false), &map, &[]).unwrap();
        a.tap(wheel, 2.0);
        assert_eq!(a.zoom_lines(), 2.0);
        assert!(!a.pressed("zoom_in", 0), "a wheel pulse is not a held key");
        assert!(resolve("wheelup", (false, false, true), &map, &[]).is_none());
        assert_eq!(
            resolve("wheelup", (false, false, true), &map, &["builder_rotate_mode"]).unwrap().name,
            "builder_rotate_left_large"
        );
        Ok(())
    }
}
