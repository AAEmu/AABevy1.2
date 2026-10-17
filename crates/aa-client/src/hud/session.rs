//! The connection while in game: what the server says about it, as the UI events the client raises,
//! and what the player asks of it (leave the game, back to the character list or the server list).
//!
//! * A lost connection (`Event::Disconnected` not asked for) or a kick shows the client's disconnect
//!   dialog (`DISCONNECTED_BY_WORLD(title, body)`, `baselib/disconnect.lua`); its OK button calls
//!   `X2World:LeaveWorld(EXIT_CLIENT)`, which with no connection left returns to the server list: a
//!   fresh login, `Screen::WorldSelect`.
//! * `X2World:LeaveWorld(how)` sends `CSLeaveWorld`; the server answers with the countdown
//!   (`LEAVING_WORLD_STARTED(waitMs, target, idleKick)` opens the "leaving in N seconds" frame) and
//!   then lets go (`LeftWorld`): the game exits, or the character / server list comes back.

use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::HudBuilt;
use super::api::HudBridge;
use crate::Screen;
use crate::ingame::enter::Credentials;

/// Where a leave request goes (`aa_net::LeaveTarget`, as the scripts' `EXIT_*` constants number it).
pub fn leave_target(how: i64) -> Option<aa_net::LeaveTarget> {
    match how {
        0 => Some(aa_net::LeaveTarget::Quit),
        1 => Some(aa_net::LeaveTarget::CharacterSelect),
        2 => Some(aa_net::LeaveTarget::ServerSelect),
        _ => None,
    }
}

/// The text of the dialog for a dropped connection.
fn disconnect_text(event: &Event) -> (String, String) {
    match event {
        Event::Kicked { message, .. } if !message.is_empty() => ("Disconnected".into(), message.clone()),
        Event::Kicked { .. } => ("Disconnected".into(), "You have been disconnected from the server.".into()),
        Event::Disconnected { message } if !message.is_empty() && message != "disconnected" => ("Disconnected".into(), message.clone()),
        _ => ("Disconnected".into(), "The connection to the server was lost.".into()),
    }
}

/// Server events about the connection.
pub fn route_session(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<HudBuilt>,
    bridge: NonSend<HudBridge>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    for NetEvent(event) in events.read() {
        match event {
            Event::LeavePending { time_ms, target } if built.up => {
                if let Some(rt) = &rt {
                    rt.fire_event("LEAVING_WORLD_STARTED", (i64::from(*time_ms), i64::from(*target), false));
                }
                bridge.0.borrow_mut().leaving = Some(*target);
            }
            Event::LeaveCancelled if built.up => {
                if let Some(rt) = &rt {
                    rt.fire_event("LEAVING_WORLD_CANCELED", ());
                }
                bridge.0.borrow_mut().leaving = None;
            }
            Event::LeftWorld { target } => {
                info!("hud: left the world (target {target})");
                bridge.0.borrow_mut().leaving = Some(*target);
                match *target {
                    // the session closes: `Disconnected` follows and ends the game
                    0 => {}
                    1 => next.set(Screen::CharacterSelect),
                    _ => next.set(Screen::WorldSelect),
                }
            }
            Event::Disconnected { .. } | Event::Kicked { .. } => {
                let leaving = bridge.0.borrow().leaving;
                match leaving {
                    // logged out on purpose
                    Some(0) => {
                        exit.write(AppExit::Success);
                    }
                    Some(_) => {}
                    None => {
                        let (title, body) = disconnect_text(event);
                        warn!("hud: disconnected: {body}");
                        bridge.0.borrow_mut().disconnected = true;
                        match &rt {
                            Some(rt) if built.up => rt.fire_event("DISCONNECTED_BY_WORLD", (title, body)),
                            // no HUD (--no-hud): nothing to tell the player with
                            _ => {
                                exit.write(AppExit::error());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The player pressed OK on the disconnect dialog: log in again and show the server list.
pub fn return_to_world_select(credentials: Option<&Credentials>, net: &mut MessageWriter<NetCommand>, next: &mut NextState<Screen>) {
    // `--enter-world` keeps the credentials to itself: the same flags give the same ones
    let command = match credentials {
        Some(c) => c.command(),
        None => Credentials::from_args().command(),
    };
    net.write(NetCommand(command));
    next.set(Screen::WorldSelect);
}

/// `Command::LeaveWorld` for a script's `X2World:LeaveWorld(how)`.
pub fn leave(how: i64, net: &mut MessageWriter<NetCommand>) {
    if let Some(target) = leave_target(how) {
        net.write(NetCommand(Command::LeaveWorld(target)));
    }
}

/// `--hud-disconnect T` (show the disconnect dialog T seconds after the HUD is up, as if the server
/// dropped the connection) and `--hud-click X Y T` (a left click at physical pixel X, Y): for
/// unattended runs.
#[derive(Resource, Default)]
pub struct ScriptedSession {
    disconnect: Option<f32>,
    clicks: Vec<(f32, Vec2)>,
    /// `--hud-open ACTION T`: a key binding action (`toggle_character`, `toggle_bag`, ...) at T seconds.
    opens: Vec<(f32, String)>,
    started: Option<f32>,
    release: bool,
}

impl ScriptedSession {
    pub fn from_args() -> ScriptedSession {
        let args: Vec<String> = std::env::args().collect();
        let num = |i: usize| args.get(i).and_then(|v| v.parse::<f32>().ok());
        let mut s = ScriptedSession::default();
        for (i, a) in args.iter().enumerate() {
            match a.as_str() {
                "--hud-disconnect" => s.disconnect = num(i + 1),
                "--hud-open" => {
                    if let (Some(action), Some(t)) = (args.get(i + 1), num(i + 2)) {
                        s.opens.push((t, action.clone()));
                    }
                }
                "--hud-click" => {
                    if let (Some(x), Some(y), Some(t)) = (num(i + 1), num(i + 2), num(i + 3)) {
                        s.clicks.push((t, Vec2::new(x, y)));
                    }
                }
                _ => {}
            }
        }
        s.clicks.sort_by(|a, b| b.0.total_cmp(&a.0)); // pop from the end
        s.opens.sort_by(|a, b| b.0.total_cmp(&a.0));
        s
    }
}

pub fn scripted_session(
    time: Res<Time>,
    mut script: ResMut<ScriptedSession>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
) {
    let Some(rt) = rt else { return };
    let now = time.elapsed_secs();
    let started = *script.started.get_or_insert(now);
    let t = now - started;
    if script.release {
        script.release = false;
        pointer.release = true;
    }
    if script.disconnect.is_some_and(|d| t >= d) {
        script.disconnect = None;
        bridge.0.borrow_mut().disconnected = true;
        rt.fire_event("DISCONNECTED_BY_WORLD", ("Disconnected", "The connection to the server was lost."));
    }
    while script.opens.last().is_some_and(|o| t >= o.0) {
        let (_, action) = script.opens.pop().unwrap();
        if let Err(e) = rt.hotkey(&action) {
            warn!("hud: --hud-open {action}: {e}");
        }
    }
    if script.clicks.last().is_some_and(|c| t >= c.0) {
        let (_, at) = script.clicks.pop().unwrap();
        pointer.pos = Some(at);
        pointer.press = true;
        script.release = true;
    }
}
