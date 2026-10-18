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
use bevy::ecs::system::SystemParam;

use crate::session::{Client, Command, Event};
use crate::actability::Actabilities;
use crate::mate::Mates;
use crate::slave::Slaves;
use crate::gimmick::Gimmicks;
use crate::ui_data::UiData;
use crate::state::{
    Auction, ActionBar, Buffs, Characters, Chat, Doodads, Friends, Inventory, Loot, LootRolls, Mailbox, Party, Player, PlayerStats, Quests, Repair, Shop,
    Skills, Status, Target, Trade, Units, WorldInfo, Worlds,
};

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
        app.add_message::<NetCommand>().add_message::<NetEvent>();
        init_state(app);
        app.add_systems(PreUpdate, (send_commands, receive_events).chain());
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

fn receive_events(client: Option<Res<NetClient>>, mut out: MessageWriter<NetEvent>, mut state: NetState) {
    let Some(client) = client else { return };
    let mut client = client.0.lock().expect("net client poisoned");
    while let Some(event) = client.try_recv() {
        state.apply(&event);
        out.write(NetEvent(event));
    }
}

/// `apply` on the bare data without marking the resource changed when nothing happened.
trait ApplyIfChanged {
    fn apply_if_changed(&mut self, e: &Event) -> bool;
}

/// Declares the state resources once: the resources the plugin adds, the system parameter that
/// holds them all (a plain system parameter list stops at 16), and the fold of an event into each.
macro_rules! net_state {
    ($($field:ident: $t:ty),* $(,)?) => {
        $(
            impl ApplyIfChanged for ResMut<'_, $t> {
                fn apply_if_changed(&mut self, e: &Event) -> bool {
                    // `bypass_change_detection` avoids flagging the resource for events it ignores.
                    self.bypass_change_detection().apply(e)
                }
            }
        )*

        #[derive(SystemParam)]
        struct NetState<'w> {
            $($field: ResMut<'w, $t>,)*
        }

        impl NetState<'_> {
            fn apply(&mut self, event: &Event) {
                // `apply` reports whether anything changed, so change detection stays meaningful.
                $(
                    if self.$field.apply_if_changed(event) {
                        self.$field.set_changed();
                    }
                )*
                if self.trade.bypass_change_detection().seed_inventory(event, &self.inventory) {
                    self.trade.set_changed();
                }
                if self.party.bypass_change_detection().seed_visible(event, &self.player, &self.units) {
                    self.party.set_changed();
                }
            }
        }

        fn init_state(app: &mut App) {
            $(app.init_resource::<$t>();)*
        }
    };
}

net_state! {
    worlds: Worlds,
    characters: Characters,
    player: Player,
    units: Units,
    doodads: Doodads,
    status: Status,
    stats: PlayerStats,
    buffs: Buffs,
    actions: ActionBar,
    inventory: Inventory,
    world: WorldInfo,
    chat: Chat,
    target: Target,
    skills: Skills,
    quests: Quests,
    loot: Loot,
    party: Party,
    shop: Shop,
    friends: Friends,
    repair: Repair,
    mailbox: Mailbox,
    trade: Trade,
    auction: Auction,
    loot_rolls: LootRolls,
    actabilities: Actabilities,
    mates: Mates,
    slaves: Slaves,
    gimmicks: Gimmicks,
    ui_data: UiData,
}
