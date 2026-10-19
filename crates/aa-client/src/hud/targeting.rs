//! Choosing a target: a left click on a unit in the 3D view, `Tab` for the next nearest unit,
//! `Escape` to drop it (after the windows that close on it). The server answers with
//! `TargetChanged`, which the target frame follows (see `panels.rs`).

use aa_net::state::{Player, Target, Units};
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::api::HudBridge;
use super::{HudKeyboardFocus, HudPointerOverUi};

/// How far `Tab` looks, metres.
const TAB_RANGE: f32 = 40.0;
/// How near the view ray has to pass a unit's middle for a click to select it, metres.
const PICK_RADIUS: f32 = 1.1;
/// A press that moves more than this many pixels before it is released was a camera drag.
const CLICK_SLOP: f32 = 6.0;

fn unit_centre(pos: aa_net::Pos, scale: f32) -> Vec3 {
    aa_world::cry_to_bevy(Vec3::new(pos.x, pos.y, pos.z + 0.9 * scale.max(0.5)))
}

/// `Tab`: the nearest enemy that is alive and not the one targeted, the next one after it when pressed again (the
/// client's "target nearest enemy"; with no enemy in range, any unit).
#[allow(clippy::too_many_arguments)]
pub fn tab_target(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    target: Res<Target>,
    player: Res<Player>,
    mut script: ResMut<super::chat::ScriptedKeys>,
    mut net: MessageWriter<NetCommand>,
) {
    let scripted = script.take_tab();
    let mode = [
        ("cycle_hostile_forward", false, false),
        ("cycle_hostile_backward", true, false),
        ("cycle_friendly_forward", false, true),
        ("cycle_friendly_backward", true, true),
    ]
    .into_iter()
    .find(|(name, _, _)| actions.just_pressed(name, 0));
    if focus.0 || !(scripted || mode.is_some()) {
        return;
    }
    let (_, backwards, friendly) = mode.unwrap_or(("cycle_hostile_forward", false, false));
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let (me, shared) = {
        let d = bridge.0.borrow();
        (d.pos, d.game.clone())
    };
    let own_faction = player.spawn.as_ref().map_or(0, |s| s.character().faction_id);
    let mut near: Vec<(f32, u32, bool)> = units
        .iter()
        .filter(|u| u.obj_id != own)
        .filter(|u| !friendly || (!u.dead && super::plates::relation_of(shared.get(), own_faction, u) == "friendly"))
        .map(|u| {
            (me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)), u.obj_id, !u.dead && super::plates::relation_of(shared.get(), own_faction, u) == "hostile")
        })
        .filter(|(d, _, _)| *d <= TAB_RANGE)
        .collect();
    if near.iter().any(|n| n.2) {
        near.retain(|n| n.2);
    }
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    if near.is_empty() {
        return;
    }
    let next = match target.obj_id.and_then(|t| near.iter().position(|(_, id, _)| *id == t)) {
        Some(i) => near[(i + if backwards { near.len() - 1 } else { 1 }) % near.len()].1,
        None => near[if backwards { near.len() - 1 } else { 0 }].1,
    };
    net.write(NetCommand(Command::Target { obj_id: next }));
}

/// A left click (not a drag, not on the HUD) on a unit selects it.
#[allow(clippy::too_many_arguments)]
pub fn click_target(
    mouse: Res<ButtonInput<MouseButton>>,
    over_ui: Res<HudPointerOverUi>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    units: Res<Units>,
    player: Res<Player>,
    mut script_pick: ResMut<crate::script::ScriptPick>,
    mut net: MessageWriter<NetCommand>,
    mut pressed_at: Local<Option<Vec2>>,
) {
    // a pick the script runner made (`target pick ...`) is the click's result
    if let Some(obj_id) = script_pick.take_left() {
        net.write(NetCommand(Command::Target { obj_id }));
        return;
    }
    let Ok(window) = windows.single() else { return };
    let cursor = window.cursor_position();
    if mouse.just_pressed(MouseButton::Left) {
        *pressed_at = if over_ui.0 { None } else { cursor };
    }
    if !mouse.just_released(MouseButton::Left) {
        return;
    }
    let (Some(start), Some(end)) = (pressed_at.take(), cursor) else { return };
    if start.distance(end) > CLICK_SLOP || over_ui.0 {
        return;
    }
    let Some((camera, transform)) = cameras.iter().find(|(c, _)| c.is_active) else { return;
    };
    let Ok(ray) = camera.viewport_to_world(transform, end) else { return };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let mut best: Option<(f32, u32)> = None;
    for u in units.iter().filter(|u| u.obj_id != own) {
        let centre = unit_centre(u.pos, u.scale);
        let along = (centre - ray.origin).dot(*ray.direction);
        if along <= 0.0 {
            continue;
        }
        let miss = (ray.origin + *ray.direction * along).distance(centre);
        if miss <= PICK_RADIUS * u.scale.max(0.7) && best.is_none_or(|(a, _)| along < a) {
            best = Some((along, u.obj_id));
        }
    }
    if let Some((_, obj_id)) = best {
        net.write(NetCommand(Command::Target { obj_id }));
    }
}

/// `Escape`: close the topmost window that closes on it; with none open, drop the target; else
/// the toolkit opens the system menu.
pub fn escape_key(
    keys: Res<ButtonInput<KeyCode>>,
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    target: Res<Target>,
    mut net: MessageWriter<NetCommand>,
    mut script: ResMut<super::chat::ScriptedKeys>,
) {
    let Some(rt) = rt else { return };
    let scripted = script.take_escape();
    let mapped = actions.just_pressed("open_config", 0);
    let physical = keys.just_pressed(KeyCode::Escape);
    let escape = scripted || physical || mapped;
    if focus.0 || rt.keyboard_captured() || !escape {
        return;
    }
    if mapped && !physical && !scripted {
        if let Err(e) = rt.hotkey("open_config") {
            warn!("hud: open_config: {e}");
        }
        return;
    }
    if super::repair::escape(&rt) { return; }
    if rt.holding() {
        rt.clear_cursor();
        return;
    }
    let window_open = {
        let u = aa_ui::lua::ui(&rt.lua);
        u.tree.paint_order().into_iter().any(|id| u.tree.get(id).alive && u.tree.widget(id).is_some_and(|w| w.close_on_escape) && u.tree.visible(id))
    };
    if !window_open && target.obj_id.is_some() {
        net.write(NetCommand(Command::Target { obj_id: 0 }));
    } else if window_open || mapped || scripted {
        if let Err(e) = rt.escape() {
        warn!("hud: escape: {e}");
    }
}
}
