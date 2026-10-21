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
use crate::farm::Farms;
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
        let options=app.world().get_resource::<crate::ClientOptions>().cloned().unwrap_or_default();
        match Client::spawn_thread_with(options) {
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
                if let Some(id)=self.expeditions.admitted_dismissal(event) {
                    if self.player.bypass_change_detection().clear_expedition(id) {self.player.set_changed();}
                    if self.units.bypass_change_detection().clear_expedition(id) {self.units.set_changed();}
                }
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
    scheduled_events: crate::scheduled::ScheduledEvents,
    dominion: crate::Dominion,
    trials: crate::trial::Trials,
    crimes: crate::crime::Crimes,
    duels: crate::duel::Duels,
    appellations: crate::appellation::Appellations,
    bounties:crate::Bounties,
    rankings:crate::Rankings,
    portals: crate::portal::Portals,
    character_names:crate::CharacterNames,
    achievements: crate::achievement::Achievements,
    expeditions: crate::expedition::Expeditions,
    families: crate::family::Families,
    server_clock: crate::clock::ServerClock,
    actabilities: Actabilities,
    mates: Mates,
    slaves: Slaves,
    gimmicks: Gimmicks,
    houses: Houses,
    farms: Farms,
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

    #[test]
    fn plugin_security_receipt_updates_flags_and_signed_times_without_creating_items() {
        use crate::{items::{ItemTask,ItemTasks},model::Item};
        let mut app=App::new();init_state(&mut app);
        {let mut inv=app.world_mut().resource_mut::<Inventory>();inv.bag_slots=1;inv.bag=vec![Some(Item{id:55,template_id:99,..Default::default()})];}
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::ItemsChanged(ItemTasks{reason:114,tasks:vec![ItemTask::Security{container:2,slot:0,item_id:55,bits:0x11,is_unsecure_excess:0,is_unsecure_set:0,is_unpack:1,unsecure_time:-3,unpack_time:99}],..Default::default()}))).unwrap();
        let inv=app.world().resource::<Inventory>();let item=inv.bag[0].as_ref().unwrap();assert_eq!((item.flags,item.unsecure_time,item.unpack_time),(0x11,-3,99));
    }

    #[test]
    fn plugin_gm_modes_update_existing_unit_and_clear_with_lifecycle() {
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))))).unwrap();
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged{obj_id:8,mode:1,value:255}))).unwrap();
        assert_eq!(app.world().resource::<Units>().by_id[&8].gm_modes,[0,255,0,0]);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged{obj_id:99,mode:0,value:1}))).unwrap();
        assert_eq!(app.world().resource::<Units>().by_id.len(),1);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert!(app.world().resource::<Units>().by_id.is_empty());
    }

    #[test]
    fn plugin_family_receipts_hydrate_and_reset_the_public_resource() {
        use crate::family::{Families,FamilyEvent,FamilyRoster};
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Family(FamilyEvent::Described(FamilyRoster{family_id:7,members:vec![]})))).unwrap();
        assert!(app.world().resource::<Families>().rosters.contains_key(&7));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert!(app.world().resource::<Families>().rosters.is_empty());
    }

    #[test]
    fn plugin_retains_complete_scheduled_receipt_and_received_ratio() {
        let mut app=App::new();init_state(&mut app);
        let start=crate::scheduled::ScheduledEventStarted{id:3,values:[-7;21],changes:u32::MAX,priority:0,start_date:1,period_date:2,hour:3,minute:4,period_minutes:5,week_day:6,open_message:b"notice".to_vec()};
        app.world_mut().run_system_once(move |mut state:NetState|state.apply(&Event::ScheduledEventStarted(start.clone()))).unwrap();
        assert_eq!(app.world().resource::<crate::battlefield::Battlefield>().my_corps_kill_death_ratio,-7);
        assert_eq!(app.world().resource::<crate::scheduled::ScheduledEvents>().starts[&3].open_message,b"notice");
    }

    #[test]
    fn plugin_trial_wait_cache_and_defendant_gate_follow_received_state() {
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState| {
            state.apply(&Event::Trial(crate::TrialEvent::SummonDefendant{trial_id:7}));
            state.apply(&Event::Trial(crate::TrialEvent::JuryWaiting{count:2,total:5,sentence_time_ms:-1}));
        }).unwrap();
        let state=app.world().resource::<crate::Trials>();assert_eq!(state.defendant_context,Some(7));assert_eq!(state.jury_counts,Some([2,5]));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert_eq!(*app.world().resource::<crate::Trials>(),crate::Trials::default());
    }

    #[test]
    fn plugin_character_names_and_plain_world_fold_same_raw_receipts(){
        let mut app=App::new();init_state(&mut app);let mut plain=crate::state::WorldState::default();
        for event in [Event::CharacterNameRequested{character_id:42,requested_id:7},Event::CharacterName(crate::CharacterName{character_id:7,name:vec![255,0,88]}),Event::CharacterNameRequested{character_id:42,requested_id:8},Event::CharacterNameStreamClosed]{plain.apply(&event);app.world_mut().run_system_once(move |mut state:NetState|state.apply(&event)).unwrap();assert_eq!(app.world().resource::<crate::CharacterNames>(),&plain.character_names);}
        assert_eq!(plain.character_names.by_id[&7],vec![255]);assert!(plain.character_names.pending.is_empty());
    }

    #[test]
    fn plugin_ranking_resource_forwards_raw_mail_without_fabricating_board(){
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Ranking(crate::RankingEvent::RewardMail{rank_id:7}))).unwrap();
        let state=app.world().resource::<crate::Rankings>();assert!(state.by_id.is_empty());assert_eq!(state.last_receipt,Some(crate::RankingEvent::RewardMail{rank_id:7}));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();assert_eq!(*app.world().resource::<crate::Rankings>(),crate::Rankings::default());
    }

    #[test]
    fn plugin_bounty_existing_actor_and_cache_retention() {
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState| {
            state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))));
            state.apply(&Event::Bounty(crate::BountyEvent::UnitMoney{obj_id:8,money:-1}));
        }).unwrap();
        assert_eq!(app.world().resource::<Units>().by_id[&8].bounty_money,Some(-1));
        assert!(matches!(app.world().resource::<crate::Bounties>().last_receipt,Some(crate::BountyEvent::UnitMoney{money:-1,..})));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::LeftWorld{target:0})).unwrap();
        assert!(app.world().resource::<crate::Bounties>().last_receipt.is_some());
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert_eq!(*app.world().resource::<crate::Bounties>(),crate::Bounties::default());
    }

    #[test]
    fn plugin_crime_receipts_fold_selected_header_without_queue_cache_fabrication() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState| {
            state.apply(&Event::CharacterSelected(Box::new(crate::model::CharacterState::default())));
            state.apply(&Event::Crime(crate::CrimeEvent::JuryPointChanged{jury_point:-7}));
        }).unwrap();
        assert_eq!(app.world().resource::<crate::Crimes>().jury_point,Some(-7));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert_eq!(*app.world().resource::<crate::Crimes>(),crate::Crimes::default());
    }

    #[test]
    fn plugin_achievement_cache_retains_batch_and_lifecycle() {
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Achievement(crate::AchievementEvent::List{raw_count:1,rows:vec![crate::AchievementProgress{id:7,amount:-1,completed_at:i64::MIN}]}))).unwrap();
        assert_eq!(app.world().resource::<crate::Achievements>().entries[&7].completed_at,i64::MIN);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert!(app.world().resource::<crate::Achievements>().entries.is_empty());
    }

    #[test]
    fn plugin_expedition_cache_hydrates_and_resets() {
        let mut app=App::new();init_state(&mut app);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Expedition(crate::ExpeditionEvent::Factions(vec![crate::ExpeditionFaction{id:9,..Default::default()}])))).unwrap();
        assert!(app.world().resource::<crate::Expeditions>().factions.contains_key(&9));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Disconnected{message:String::new()})).unwrap();
        assert!(app.world().resource::<crate::Expeditions>().factions.is_empty());
    }

    #[test]
    fn plugin_expedition_dismissal_admits_before_cache_delete_and_clears_player_and_units() {
        let mut app=App::new();init_state(&mut app);
        let mut unit=crate::state::tests::unit(77);unit.expedition_id=9;
        let mut character=crate::model::CharacterState::default();character.character.expedition_id=9;
        app.world_mut().resource_mut::<Player>().spawn=Some(crate::session::PlayerSpawn{obj_id:77,unit:unit.clone(),state:character});
        app.world_mut().resource_mut::<Units>().by_id.insert(77,unit);
        app.world_mut().resource_mut::<crate::Expeditions>().factions.insert(9,crate::ExpeditionFaction{id:9,mother_id:1,..Default::default()});
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Expedition(crate::ExpeditionEvent::Dismissed{expedition_id:9,success:0}))).unwrap();
        assert_eq!(app.world().resource::<Units>().by_id[&77].expedition_id,9);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Expedition(crate::ExpeditionEvent::Dismissed{expedition_id:9,success:2}))).unwrap();
        assert_eq!(app.world().resource::<Units>().by_id[&77].expedition_id,0);
        assert_eq!(app.world().resource::<Player>().spawn.as_ref().unwrap().unit.expedition_id,0);
        assert!(!app.world().resource::<crate::Expeditions>().factions.contains_key(&9));
    }

    #[test]
    fn plugin_created_and_faction_change_fold_existing_local_actor_and_cache() {
        use crate::{ExpeditionEvent as E,ExpeditionFaction};
        let mut app=App::new();init_state(&mut app);
        let mut unit=crate::state::tests::unit(77);unit.kind=crate::model::UnitKind::Character{char_id:7};unit.faction_id=101;
        let mut state=crate::model::CharacterState::default();state.character.id=7;state.character.faction_id=101;
        let spawn=crate::session::PlayerSpawn{obj_id:77,unit:unit.clone(),state};
        app.world_mut().run_system_once(move |mut state:NetState|state.apply(&Event::PlayerSpawn(Box::new(spawn.clone())))).unwrap();
        app.world_mut().resource_mut::<Units>().by_id.insert(77,unit);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Expedition(E::Created{faction:ExpeditionFaction{id:9,mother_id:101,owner_id:7,..Default::default()},owner_obj_id:77,members:vec![]}))).unwrap();
        assert_eq!(app.world().resource::<crate::Expeditions>().own_id,9);assert_eq!(app.world().resource::<Units>().by_id[&77].expedition_id,9);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::Expedition(E::UnitFactionChanged{obj_id:77,name:vec![],old_faction_id:101,new_faction_id:102,temporary:2}))).unwrap();
        assert_eq!(app.world().resource::<crate::Expeditions>().current_faction_id,102);assert_eq!(app.world().resource::<Units>().by_id[&77].faction_id,102);
        assert_eq!(app.world().resource::<Player>().spawn.as_ref().unwrap().state.character.faction_id,102);
    }

    #[test]
    fn plugin_dominion_fold_keeps_real_gate_and_coverage_loss_then_resets() {
        let mut app=App::new();init_state(&mut app);
        assert_eq!(app.world().resource::<crate::Dominion>().owner_change_blocked(77),Some(false));
        let mut data=crate::DominionData{zone_group_id:25,..Default::default()};data.siege_timers.siege_period=4;data.siege_timers.offense.expedition_id=77;
        app.world_mut().run_system_once(move |mut state:NetState|state.apply(&Event::Dominion(crate::DominionEvent::Data{data:data.clone(),newly_declared:0,final_data_by_request:1}))).unwrap();
        assert_eq!(app.world().resource::<crate::Dominion>().owner_change_blocked(77),Some(true));
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::UnhandledGamePacket{level:4,opcode:0x3ff,len:0})).unwrap();
        assert_eq!(app.world().resource::<crate::Dominion>().owner_change_blocked(77),None);
        app.world_mut().run_system_once(|mut state:NetState|state.apply(&Event::LeftWorld{target:0})).unwrap();
        assert_eq!(app.world().resource::<crate::Dominion>().owner_change_blocked(77),Some(false));
    }

}
