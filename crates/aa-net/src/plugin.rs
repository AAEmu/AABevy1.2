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
use crate::housing::Houses;
use crate::ui_data::UiData;
use crate::initial_config::GameConfig;
use crate::combat_status::CombatStatus;
use crate::conflict::ConflictZones;
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
    state.advance_battlefield_clock();
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
            battlefield: ResMut<'w, crate::battlefield::Battlefield>,
            $($field: ResMut<'w, $t>,)*
        }

        impl NetState<'_> {
            fn advance_battlefield_clock(&mut self) {
                if let Some(now)=self.server_clock.mapped_now() {
                    if self.battlefield.bypass_change_detection().advance_server_time(now) {self.battlefield.set_changed();}
                }
            }
            fn apply(&mut self, event: &Event) {
                self.advance_battlefield_clock();
                // `apply` reports whether anything changed, so change detection stays meaningful.
                $(
                    if self.$field.apply_if_changed(event) {
                        self.$field.set_changed();
                    }
                )*
                let now=self.server_clock.mapped_now();
                if self.battlefield.bypass_change_detection().apply_with_time(event,now) {self.battlefield.set_changed();}
                self.advance_battlefield_clock();
                if self.units.bypass_change_detection().apply_mate_equipment_flags(event, &self.mates) {
                    self.units.set_changed();
                }
                if self.trade.bypass_change_detection().seed_inventory(event, &self.inventory) {
                    self.trade.set_changed();
                }
                if self.party.bypass_change_detection().seed_visible(event, &self.player, &self.units) {
                    self.party.set_changed();
                }
            }
        }

        fn init_state(app: &mut App) {
            app.init_resource::<crate::battlefield::Battlefield>();
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
    server_clock: crate::clock::ServerClock,
    actabilities: Actabilities,
    mates: Mates,
    slaves: Slaves,
    gimmicks: Gimmicks,
    houses: Houses,
    transfer_radar: crate::radar::TransferRadar,
    ship_radar: crate::radar::ShipRadar,
    fish_radar: crate::radar::FishRadar,
    ui_data: UiData,
    game_config: GameConfig,
    combat_status: CombatStatus,
    conflict_zones: ConflictZones,
}


#[cfg(test)]
mod equipment_flags_tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn plugin_fold_publishes_correlated_battlefield_queue_and_resets() {
        use crate::battlefield::{Battlefield, BattlefieldCommand as C, BattlefieldEvent as E, BattlefieldQueue};
        let mut app=App::new();init_state(&mut app);
        for event in [E::Requested(C::Apply { battlefield_id:7,corps:2 }),E::Applied { battlefield_id:7,corps:2,error:0 },E::QueueState(BattlefieldQueue { battlefield_id:7,corps_size:3,counts:[9,1,9] })] {
            app.world_mut().run_system_once(move |mut state:NetState| state.apply(&Event::Battlefield(event.clone()))).unwrap();
        }
        assert_eq!(app.world().resource::<Battlefield>().joined_infos(),(6,3,2));
        app.world_mut().run_system_once(|mut state:NetState| state.apply(&Event::Kicked { reason:1,message:"test".into() })).unwrap();
        assert_eq!(app.world().resource::<Battlefield>(),&Battlefield::default());
    }
    #[test]
    fn plugin_fold_publishes_received_transfer_radar_snapshot() {
        use crate::radar::{RadarEvent, RadarToggle, ShipRadar, TransferRadar};
        let mut app = App::new();
        init_state(&mut app);
        for event in [
            Event::Radar(RadarEvent::TransferToggled(RadarToggle { on: true, range: 50.0 })),
            Event::Radar(RadarEvent::TransferContacts { last: true, contacts: vec![] }),
            Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: true, range: 75.0 })),
            Event::Radar(RadarEvent::ShipContacts { last: true, contacts: vec![] }),
            Event::Radar(RadarEvent::FishToggled(RadarToggle { on: true, range: 75.0 })),
            Event::Radar(RadarEvent::FishContacts { last: true, contacts: vec![] }),
        ] {
            app.world_mut().run_system_once(move |mut state: NetState| state.apply(&event)).unwrap();
        }
        assert_eq!(app.world().resource::<TransferRadar>().contacts, Some(vec![]));
        assert_eq!(app.world().resource::<ShipRadar>().contacts, Some(vec![]));
        assert_eq!(app.world().resource::<crate::radar::FishRadar>().contacts, Some(vec![]));
        assert!(app.world().resource::<Units>().by_id.is_empty());
    }
    #[test]
    fn plugin_fold_updates_existing_owned_equipment_flags() {
        let mut app = App::new();
        init_state(&mut app);
        let mut events = crate::equipment_notice_tests::flags_fixture();
        events.push(Event::Mate(crate::mate::MateEvent::EquipmentFlagsChanged {
            mate_tl_id: 0x102, container: 0xfc, slot: 1, flags: 0xa5,
        }));
        for event in events {
            app.world_mut().run_system_once(move |mut state: NetState| state.apply(&event)).unwrap();
        }
        assert_eq!(app.world().resource::<Units>().by_id[&51].equipment.slots[1].as_ref().unwrap().flags, 0xa5);
    }
    #[test]
    fn plugin_clock_samples_survive_delivery_delay_and_advance_battlefield_without_wire_commands() {
        use crate::{clock::{ClockSample,ServerClock},proxy::ProxyPong,battlefield::{Battlefield,BattlefieldPhase,BattlefieldStart}};
        let mut app=App::new();init_state(&mut app);
        {
            let mut bf=app.world_mut().resource_mut::<Battlefield>();bf.phase=Some(BattlefieldPhase::Opening);
            bf.start=Some(BattlefieldStart{zone_id:8,instance_id:9,now_ms:0,start_ms:6000});
        }
        let at=std::time::Instant::now();
        let sample=ClockSample{pong:ProxyPong{tm:0,when:0,elapsed:0,remote:5000000,local:1000,world:5000},local_receive_ms:1011,received_at:at,local_origin:at-std::time::Duration::from_millis(1011)};
        app.world_mut().run_system_once(move |mut state:NetState|state.apply(&Event::ClockSample(sample))).unwrap();
        assert!(app.world().resource::<ServerClock>().mapped_now().is_some());
        assert_eq!(app.world().resource::<Battlefield>().phase,Some(BattlefieldPhase::Playing));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert_eq!(app.world().resource::<ServerClock>().mapped_now(),None);
    }

}
