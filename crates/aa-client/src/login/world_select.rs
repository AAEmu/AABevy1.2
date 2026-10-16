//! World (server) select: the original `world_select` scripts, `X2World` backed by aa-net.
//!
//! The scripts list `X2World:GetWorldInfo()`; Connect calls `X2World:EnterWorld`, which is
//! `Command::EnterWorld`. The server answers with queue positions (`OPEN_WORLD_QUEUE` /
//! `REFRESH_WORLD_QUEUE`) or `READY_TO_CONNECT_WORLD`, to which the scripts react with
//! `X2LoginCharacter:ConnectToWorld()`. The character list that follows moves the app on to
//! character select (see `login::route_events`). Exit Game is `Console:ExecuteString("quit")`.

use aa_net::state::{Characters, Status, Worlds};
use aa_net::{Event, NetEvent, Phase};
use aa_ui::{Stage, UiRuntime};
use bevy::prelude::*;

use super::{Goto, LoginArgs, LoginBridge, gameapi, host, ui_is};
use crate::Screen;

pub fn plugin(app: &mut App) {
    app.add_systems(Update, (net_events, auto_drive).run_if(ui_is(Screen::WorldSelect)));
}

/// Build the UI of this screen once the login is through and the world list is there.
pub fn build(world: &mut World) -> bool {
    let ready = world.resource::<Worlds>().account.is_some() && world.resource::<Status>().phase == Phase::WorldSelect;
    if !ready {
        return false;
    }
    let shared = world.non_send::<LoginBridge>().0.clone();
    let (worlds, characters) = (world.resource::<Worlds>().clone(), world.resource::<Characters>().clone());
    shared.borrow_mut().sync(&worlds, &characters);
    host::switch_stage(world, |rt| {
        if let Err(e) = gameapi::install(rt, &shared) {
            error!("world select: game API: {e}");
        }
        rt.run_stage(Stage::WorldSelect);
        rt.fire_event("ENTERED_WORLD_SELECT", ());
        rt.fire_event("SHOW_SERVER_SELECT_WINDOW", true);
    })
}

/// What the server tells while this screen is up, as the UI events the client raises.
fn net_events(rt: NonSend<UiRuntime>, mut events: MessageReader<NetEvent>, mut queue_open: Local<bool>) {
    for NetEvent(event) in events.read() {
        match event {
            Event::WorldList(_) => rt.fire_event("SHOW_SERVER_SELECT_WINDOW", true),
            Event::WorldQueue(_) => {
                rt.fire_event(if *queue_open { "REFRESH_WORLD_QUEUE" } else { "OPEN_WORLD_QUEUE" }, ());
                *queue_open = true;
            }
            Event::ReadyToConnect { .. } => {
                *queue_open = false;
                rt.fire_event("READY_TO_CONNECT_WORLD", ());
            }
            Event::EnterWorldCancelled | Event::EnterWorldDenied { .. } | Event::ConnectFailed { .. } => {
                *queue_open = false;
                rt.fire_event("ENTER_WORLD_CANCELLED", ());
            }
            Event::Disconnected { .. } | Event::LoginFailed { .. } => rt.fire_event("DISCONNECT_FROM_AUTH", ()),
            _ => {}
        }
    }
}

/// `--goto character-select|loading`: select the first online server and press Connect.
fn auto_drive(
    rt: NonSend<UiRuntime>,
    args: Res<LoginArgs>,
    worlds: Res<Worlds>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut since: Local<f32>,
) {
    if *done || !args.goto.is_some_and(|g| g != Goto::WorldSelect) {
        return;
    }
    // let the screen settle first
    *since += time.delta_secs();
    if *since < 1.0 {
        return;
    }
    let Some(world) = worlds.list.worlds.iter().find(|w| w.online) else { return };
    *done = true;
    info!("login: --goto: connecting to {}", world.name);
    let code = format!("backgroundWindow.mainWindow:SelectByDataKey({id}) backgroundWindow.enterWorldBtn:OnClick()", id = world.id);
    if let Err(e) = rt.exec("goto", &code) {
        warn!("login: --goto: {e}");
    }
}
