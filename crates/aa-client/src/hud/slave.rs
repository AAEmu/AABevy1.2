//! Boarding and leaving a cart or other slave unit, with the original client's checks.
//!
//! A right click on a slave (`UnitKind::Slave`, unit kind byte 2) sends `CSChangeTarget` 0x02c
//! (when the slave is not the target yet) and `CSStartInteraction` 0x068 with `extra_info` 1 and
//! mouse button 3 (`FUN_390f3780` -> `FUN_393be030`; live capture 2026-10-28). It never sends
//! `CSBindSlave` 0x031 directly: the server answers the 0x068 with `SCNpcInteractionSkillList`
//! 0x07b, whose FIRST skill is acted on at once (`FUN_393be850` -> `FUN_393be280`). The slave
//! bind itself is the "occupy" skill 12076 (`FUN_393bf310` -> `FUN_39362f50` -> `FUN_393464a0`),
//! after the reach check of `FUN_393bf270` / `FUN_39093860`: `DAT_39ac79fc` 9.0, or
//! `DAT_39ac7a00` 12.25 when the player unit's +0x3c26 mounted byte is set, square-rooted to
//! 3.0 / 3.5 m and compared strictly (`gap < limit`) against the 3D SURFACE gap between the
//! collision shapes (`FUN_3989bf90` -> `FUN_398b1ab0` -> `FUN_398b1af0`: the minimum over the
//! target's colliders of `|centre_a - centre_b| - (r_a + r_b)`, not clamped at 0). Then the
//! local checks of `FUN_39346290` / `FUN_393464a0`: the slave is dead (0x144), our own unit is
//! dead (`CANT_DO_WHILE_YOU_DEAD` 0x14c), we are bound to another slave (0x66), we are bound to
//! this one (silent), a bind is already pending (0x13), and the same gap strictly below
//! `DAT_39ac7408` 16.0 -> 4.0 m (0x9a). 1.2 minimises over per-collider shapes that are not
//! recovered; this client uses one scaled actor-model radius per unit, an approximation
//! (unverified). The auto-walk out of reach (`FUN_390a6af0`) is not modelled: it refuses 0x9a.
//!
//! A right click on the slave we are bound to or seated on sends only `CSChangeTarget` 0x02c
//! (live capture 2026-10-28, case e), never 0x068.
//!
//! Skills 13863 (passenger) and 19165 (destroy) go to `CSStartSkill` in 1.2; this client has no
//! generic interaction skill-cast path yet, so they are ignored. Any other first skill (0 or
//! unknown) does nothing.
//!
//! The interactive key (F) sends nothing for a cart; NPC talk keeps the path it had. The player
//! flags at +0x3a70..+0x3a73/+0x3a7b (native 0x13), the parent unit type 0xe (native silent
//! refusal) and the dynamic action bar 12076 path (`FUN_393b1de0`) are not ported: their meaning
//! is unverified.
//!
//! Leaving is `X2SiegeWeapon:DiscardSiegeWeapon` (`FUN_394aa090` -> `FUN_39345aa0`): `CSDiscardSlave`
//! 0x032 with the bound timeline, refused with 0x67 while the slave's speed squared is above 25.0
//! (`_DAT_3999f85c`). A passenger (attach point above 1) walks off on movement input through
//! `FUN_39362170`, throttled to one request per 1500 ms (`_DAT_3a169948`); that native path has no
//! speed check. The speed source is unverified (see the docs).

use std::time::Duration;

use aa_net::interaction::{Interact, InteractEvent};
use aa_net::model::{Unit, UnitKind};
use aa_net::slave_control::SlaveCommand;
use aa_net::state::{Player, Units};
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use web_time::Instant;

use super::alerts;
use super::api::{HudBridge, HudData, Request, Shared};

/// `DAT_39ac79fc` (9.0): `FUN_393bf270`'s slave interaction reach, sqrt(9.0) = 3.0 m, compared
/// strictly (`FUN_39093860`: `gap < limit`) against the 3D surface gap.
pub(super) const INTERACT_REACH: f32 = 3.0;
/// `DAT_39ac7a00` (12.25): the same reach while the player unit's +0x3c26 mounted byte is set,
/// sqrt(12.25) = 3.5 m.
pub(super) const MOUNTED_INTERACT_REACH: f32 = 3.5;
/// `DAT_39ac7408` (16.0): `FUN_39346290`'s bind reach, sqrt(16.0) = 4.0 m.
pub(super) const BIND_REACH: f32 = 4.0;
/// `_DAT_3999f85c` (25.0): the driver's unbind speed limit, 5 m/s. The speed source is unverified.
pub(super) const UNBIND_SPEED_SQUARED: f32 = 25.0;
/// `_DAT_3a169948`: the passenger walk-off throttle.
pub(super) const WALK_OFF_THROTTLE: Duration = Duration::from_millis(1500);

/// `ErrorMessageType` ids from the native message table (`FUN_3956b860`).
const MSG_ALREADY_REQUESTED: i16 = 0x13;
const MSG_SLAVE_UNBIND_FIRST: i16 = 0x66;
const MSG_SLAVE_SHUT_OFF_TO_UNBIND: i16 = 0x67;
const MSG_TOO_FAR_AWAY: i16 = 0x9a;
const MSG_SLAVE_CANNOT_BIND_WHILE_IS_DEAD: i16 = 0x144;
const MSG_CANT_DO_WHILE_YOU_DEAD: i16 = 0x14c;

/// What a boarding or leaving request did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// Queued as `Request::Slave`.
    Sent,
    /// Refused without a message (already bound to this slave).
    Silent,
    /// Refused with this `ErrorMessageType`.
    Refused(i16),
}

/// The slave a right click picked. The interact key (F) sends nothing for a slave: the 1.2
/// capture with a cart targeted produced no packet, while plain NPC talk keeps its own path.
pub(super) fn action_target(units: &Units, picked: Option<u32>) -> Option<u32> {
    picked.filter(|id| {
        units
            .get(*id)
            .is_some_and(|unit| matches!(unit.kind, UnitKind::Slave { .. }))
    })
}

/// The reach predicate of `FUN_3989bf90` -> `FUN_398b1ab0` -> `FUN_398b1af0`: the 3D surface
/// gap of two sphere colliders, `|centre_a - centre_b| - (r_a + r_b)`, not clamped at 0. 1.2
/// takes the minimum over the target's several colliders; one scaled actor-model sphere per unit
/// approximates that (unverified).
fn surface_gap(from: Vec3, from_radius: f32, to: Vec3, to_radius: f32) -> f32 {
    from.distance(to) - (from_radius + to_radius)
}

/// The target's scaled actor-model radius, the same source as `api::distance_to`.
fn radius(data: &HudData, unit: &Unit) -> f32 {
    data.game
        .get()
        .map_or(0.0, |game| game.model_size(unit.model_id, unit.scale))
}

/// Where the checks measure from. While attached to a slave the server moves only the slave,
/// and the HUD position (the local entity's transform) can stay at the boarding point: a
/// point-1 seat marks the cart `Driven`, so the client drives the cart from the stale mover
/// instead of following it, and a cart has no `Animator` for the seat pose. Use the attached
/// unit's replicated position then.
fn player_centre(data: &HudData, units: &Units) -> Vec3 {
    data.mounted
        .player_attachment()
        .and_then(|attachment| units.get(attachment.obj_id))
        .map_or(data.pos, |unit| {
            Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z)
        })
}

/// `FUN_393bf270`: the interaction reach in metres (3.5 on a mount). Only riding a mate counts as
/// mounted here; whether a slave seat also sets +0x3c26 is unverified.
fn interact_reach(data: &HudData) -> f32 {
    if data.mounted.riding_mate() {
        MOUNTED_INTERACT_REACH
    } else {
        INTERACT_REACH
    }
}

/// `FUN_390f3780` -> `FUN_393be030`: the right-click interaction request. The captured body for
/// cart bc 0x5f3c is `3c 5f 00 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 00`
/// (`npc_obj` cart, `obj` 0, `extra_info` 1, `pick_id` 0, mouse 3, modifiers 0).
///
/// While bound to or seated on this slave a right click only changes the target (live capture
/// 2026-10-28, case e: 0x02c, no 0x068, no message).
pub(super) fn interact(
    data: &mut HudData,
    units: &Units,
    target: Option<u32>,
    obj_id: u32,
) -> Outcome {
    let retarget = target != Some(obj_id);
    if retarget {
        data.requests.push(Request::Target { obj_id });
    }
    if current_slave(data, units) == Some(obj_id) {
        return if retarget {
            Outcome::Sent
        } else {
            Outcome::Silent
        };
    }
    data.requests.push(Request::Interact(Interact::Start {
        npc_obj_id: obj_id,
        obj_id: 0,
        extra_info: 1,
        pick_id: 0,
        mouse_button: 3,
        modifier_keys: 0,
    }));
    Outcome::Sent
}

/// `FUN_393be850` / `FUN_393be280`: `SCNpcInteractionSkillList` 0x07b for a slave acts on the
/// first skill at once. 12076 is the bind route; 13863 (passenger), 19165 (destroy) and any
/// other skill need a generic interaction skill cast this client does not have yet.
pub(super) fn receive(data: &mut HudData, units: &Units, event: &Event) -> Outcome {
    const SKILL_OCCUPY: u32 = 12076;
    let Event::Interaction(InteractEvent::SkillList {
        npc_obj_id, skills, ..
    }) = event
    else {
        return Outcome::Silent;
    };
    let Some(unit) = units.get(*npc_obj_id) else {
        return Outcome::Silent;
    };
    if !matches!(unit.kind, UnitKind::Slave { .. }) || skills.first() != Some(&SKILL_OCCUPY) {
        return Outcome::Silent;
    }
    let at = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let gap = surface_gap(
        player_centre(data, units),
        data.player_radius,
        at,
        radius(data, unit),
    );
    if gap >= interact_reach(data) {
        return Outcome::Refused(MSG_TOO_FAR_AWAY);
    }
    bind(data, units, *npc_obj_id)
}

/// Feeds `SCNpcInteractionSkillList` 0x07b for slaves to [`receive`].
pub(super) fn track(
    mut events: MessageReader<NetEvent>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    rt: Option<NonSend<UiRuntime>>,
) {
    for NetEvent(event) in events.read() {
        let outcome = receive(&mut bridge.0.borrow_mut(), &units, event);
        if let Outcome::Refused(code) = outcome
            && let Some(rt) = rt.as_deref()
        {
            announce(rt, code);
        }
    }
}

/// The slave we are on or bound to: a received attachment wins over the SCSlaveBound fallback.
fn current_slave(data: &HudData, units: &Units) -> Option<u32> {
    if let Some(attachment) = data.mounted.player_attachment()
        && units
            .get(attachment.obj_id)
            .is_some_and(|unit| matches!(unit.kind, UnitKind::Slave { .. }))
    {
        return Some(attachment.obj_id);
    }
    data.mounted.bound_slave().map(|(obj_id, _)| obj_id)
}

/// `FUN_39346290` / `FUN_393464a0`: the client's checks before `CSBindSlave`.
pub(super) fn bind(data: &mut HudData, units: &Units, target: u32) -> Outcome {
    let Some(unit) = units.get(target) else {
        return Outcome::Silent;
    };
    let UnitKind::Slave { tl_id, .. } = unit.kind else {
        return Outcome::Silent;
    };
    if unit.dead {
        return Outcome::Refused(MSG_SLAVE_CANNOT_BIND_WHILE_IS_DEAD);
    }
    // FUN_39346290 shows a dead player through FUN_39390900(2), a different message table;
    // 0x14c is the proven `CANT_DO_WHILE_YOU_DEAD` text of the interact path.
    if data.mounted.player_dead() {
        return Outcome::Refused(MSG_CANT_DO_WHILE_YOU_DEAD);
    }
    if let Some(bound) = current_slave(data, units) {
        if bound != target {
            return Outcome::Refused(MSG_SLAVE_UNBIND_FIRST);
        }
        return Outcome::Silent;
    }
    if data.mounted.bind_pending(Instant::now()) {
        return Outcome::Refused(MSG_ALREADY_REQUESTED);
    }
    let at = Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let gap = surface_gap(
        player_centre(data, units),
        data.player_radius,
        at,
        radius(data, unit),
    );
    if gap >= BIND_REACH {
        return Outcome::Refused(MSG_TOO_FAR_AWAY);
    }
    // Not a 1.2 value: a zero timeline is not sendable, so stay silent like the same-slave case.
    if tl_id == 0 {
        return Outcome::Silent;
    }
    data.mounted.note_bind(target, tl_id);
    data.requests
        .push(Request::Slave(SlaveCommand::Bind { tl_id }));
    Outcome::Sent
}

/// `FUN_39345aa0` through `X2SiegeWeapon:DiscardSiegeWeapon`: `CSDiscardSlave` for the bound slave.
pub(super) fn discard(data: &mut HudData) -> Outcome {
    discard_at(data, Instant::now())
}

fn discard_at(data: &mut HudData, now: Instant) -> Outcome {
    let Some((obj_id, tl_id)) = data.mounted.discard_slave() else {
        return Outcome::Silent;
    };
    // No replicated movement, or one older than the staleness bound, means a standing slave.
    if data.mounted.speed_squared(obj_id, now).unwrap_or(0.0) > UNBIND_SPEED_SQUARED {
        return Outcome::Refused(MSG_SLAVE_SHUT_OFF_TO_UNBIND);
    }
    data.requests
        .push(Request::Slave(SlaveCommand::Unbind { tl_id }));
    Outcome::Sent
}

/// The message of a refusal as the scripts show one (`ERROR_MSG` -> `SYSMSG`).
fn text(lua: &Lua, code: i16) -> String {
    let name = alerts::error_name(code).unwrap_or("UNKNOWN");
    alerts::localized_lua(lua, "ERROR_MSG", name).unwrap_or_else(|| name.to_owned())
}

/// Shows a refusal through the original system-message window.
pub(super) fn announce(rt: &UiRuntime, code: i16) {
    let text = text(&rt.lua, code);
    info!("hud: slave refusal {code:#x}: {text}");
    rt.fire_event("SYSMSG", text);
}

fn announce_lua(lua: &Lua, code: i16) -> mlua::Result<()> {
    let text = text(lua, code);
    info!("hud: slave refusal {code:#x}: {text}");
    aa_ui::lua::fire_ui_event(
        lua,
        "SYSMSG",
        vec![Value::String(lua.create_string(&text)?)],
    );
    Ok(())
}

/// `X2SiegeWeapon:DiscardSiegeWeapon()` (native `394aa090`).
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let namespace: Table = lua.globals().get("X2SiegeWeapon")?;
    let shared = state.clone();
    namespace.set(
        "DiscardSiegeWeapon",
        lua.create_function(move |lua, _: MultiValue| {
            let outcome = discard(&mut shared.borrow_mut());
            if let Outcome::Refused(code) = outcome {
                announce_lua(lua, code)?;
            }
            Ok(())
        })?,
    )
}

/// Movement input on a passenger seat walks off (`FUN_39404d50` -> `FUN_39362170`).
/// The input guards `talk_input` uses: typing in a field or a loading screen is not movement.
fn input_blocked(
    suspended: bool,
    teleport_loading: bool,
    focus: bool,
    keyboard_captured: bool,
) -> bool {
    suspended || teleport_loading || focus || keyboard_captured
}

#[allow(clippy::too_many_arguments)]
pub(super) fn walk_off(
    actions: Res<crate::settings::Actions>,
    mouse: Res<ButtonInput<MouseButton>>,
    focus: Res<super::HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    teleport: Option<Res<crate::ingame::teleport::Teleport>>,
    suspended: Res<super::HudSuspended>,
    over_ui: Option<Res<super::HudPointerOverUi>>,
    player: Res<Player>,
    units: Res<Units>,
    bridge: NonSend<HudBridge>,
) {
    if input_blocked(
        suspended.0,
        teleport.is_some_and(|t| t.loading()),
        focus.0,
        rt.as_ref().is_some_and(|rt| rt.keyboard_captured()),
    ) {
        return;
    }
    let Some(spawn) = player.spawn.as_ref() else {
        return;
    };
    let Some(attachment) = spawn.unit.attached_to else {
        return;
    };
    if attachment.point <= 1 {
        return;
    }
    let Some(unit) = units.get(attachment.obj_id) else {
        return;
    };
    let UnitKind::Slave { tl_id, .. } = unit.kind else {
        return;
    };
    // The player controller's mapping (x2game+0x412590): both world mouse buttons run
    // forward and either button makes A / D strafe, so any input that moves the player
    // on foot also leaves the passenger seat (`FUN_39362170`). NumLock autorun is a
    // toggle rather than a held direction and keeps its tap.
    let over_ui = over_ui.is_some_and(|o| o.0);
    let held = crate::ingame::player::MoveKeys {
        forward: actions.pressed("moveforward", 0),
        back: actions.pressed("moveback", 0),
        left: actions.pressed("moveleft", 0),
        right: actions.pressed("moveright", 0),
        turn_left: actions.pressed("turnleft", 0),
        turn_right: actions.pressed("turnright", 0),
    };
    let (forward, strafe, turn) = crate::ingame::player::resolve_keys(
        held,
        mouse.pressed(MouseButton::Left) && !over_ui,
        mouse.pressed(MouseButton::Right) && !over_ui,
        0,
    );
    let moving =
        forward != 0.0 || strafe != 0.0 || turn != 0.0 || actions.just_pressed("autorun", 0);
    if !moving {
        return;
    }
    let mut data = bridge.0.borrow_mut();
    if !data.mounted.walk_off_throttle(web_time::Instant::now()) {
        return;
    }
    data.requests
        .push(Request::Slave(SlaveCommand::Unbind { tl_id }));
}

#[cfg(test)]
mod tests;
