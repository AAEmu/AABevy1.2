//! Bevy integration: [`NetPlugin`] runs the [`Client`] session on a background thread and bridges
//! it to the ECS.
//!
//! * Send [`NetCommand`] messages (a [`Command`] each) from any system.
//! * Read [`NetEvent`] messages (an [`Event`] each) for things that happened.
//! * Read the resources for the current state: [`Worlds`], [`Characters`], [`Player`], [`Units`],
//!   [`Doodads`] and [`Status`]. They are updated before `Update` runs, and Bevy change detection
//!   tells you which of them changed this frame.
//!
//! Nothing here blocks the app: the network I/O lives on its own thread.

use std::sync::Mutex;

use bevy::app::{App, Plugin, PreUpdate};
use bevy::ecs::message::{Message, MessageReader, MessageWriter};
use bevy::ecs::prelude::*;

use crate::session::{Client, Command, Event};
use crate::state::{Characters, Doodads, Player, Status, Units, Worlds};

/// Ask the session to do something.
#[derive(Message, Clone, Debug)]
pub struct NetCommand(pub Command);

impl From<Command> for NetCommand {
    fn from(c: Command) -> Self {
        Self(c)
    }
}

/// Something the session reports.
#[derive(Message, Clone, Debug)]
pub struct NetEvent(pub Event);

/// The running session.
#[derive(Resource)]
struct NetClient(Mutex<Client>);

/// Adds the network session, its messages and the state resources to an app.
#[derive(Default)]
pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NetCommand>()
            .add_message::<NetEvent>()
            .init_resource::<Worlds>()
            .init_resource::<Characters>()
            .init_resource::<Player>()
            .init_resource::<Units>()
            .init_resource::<Doodads>()
            .init_resource::<Status>()
            .add_systems(PreUpdate, (send_commands, receive_events).chain());
        match Client::spawn_thread() {
            Ok(client) => {
                app.insert_resource(NetClient(Mutex::new(client)));
            }
            Err(e) => tracing::error!("aa-net: cannot start the network thread: {e}"),
        }
    }
}

fn send_commands(client: Option<Res<NetClient>>, mut commands: MessageReader<NetCommand>) {
    let Some(client) = client else {
        commands.clear();
        return;
    };
    let client = client.0.lock().expect("net client poisoned");
    for NetCommand(cmd) in commands.read() {
        if !client.send(cmd.clone()) {
            tracing::warn!("aa-net: the session has ended, dropping {cmd:?}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_events(
    client: Option<Res<NetClient>>,
    mut out: MessageWriter<NetEvent>,
    mut worlds: ResMut<Worlds>,
    mut characters: ResMut<Characters>,
    mut player: ResMut<Player>,
    mut units: ResMut<Units>,
    mut doodads: ResMut<Doodads>,
    mut status: ResMut<Status>,
) {
    let Some(client) = client else { return };
    let mut client = client.0.lock().expect("net client poisoned");
    while let Some(event) = client.try_recv() {
        // `apply` reports whether anything changed, so change detection stays meaningful.
        if worlds.apply_if_changed(&event) {
            worlds.set_changed();
        }
        if characters.apply_if_changed(&event) {
            characters.set_changed();
        }
        if player.apply_if_changed(&event) {
            player.set_changed();
        }
        if units.apply_if_changed(&event) {
            units.set_changed();
        }
        if doodads.apply_if_changed(&event) {
            doodads.set_changed();
        }
        if status.apply_if_changed(&event) {
            status.set_changed();
        }
        out.write(NetEvent(event));
    }
}

/// `apply` on the bare data without marking the resource changed when nothing happened.
trait ApplyIfChanged {
    fn apply_if_changed(&mut self, e: &Event) -> bool;
}

macro_rules! apply_if_changed {
    ($($t:ty),*) => {$(
        impl ApplyIfChanged for ResMut<'_, $t> {
            fn apply_if_changed(&mut self, e: &Event) -> bool {
                // `bypass_change_detection` avoids flagging the resource for events it ignores.
                self.bypass_change_detection().apply(e)
            }
        }
    )*};
}
apply_if_changed!(Worlds, Characters, Player, Units, Doodads, Status);
