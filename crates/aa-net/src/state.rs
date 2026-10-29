//! What the app knows about the server, built by folding [`Event`]s: the world list, the character
//! list, the local player and the units and doodads in view.
//!
//! Each piece is a small struct with `apply(&Event)`; the Bevy plugin registers them as separate
//! resources, [`WorldState`] bundles them for everyone else.

use std::collections::HashMap;

use crate::login::{AccountInfo, WorldList, WorldQueue};
use crate::model::{Attachment, Character, Doodad, Unit};
use crate::session::{Event, Phase, PlayerSpawn};

macro_rules! resource {
    ($(#[$m:meta])* pub struct $name:ident $body:tt) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default)]
        #[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
        pub struct $name $body
    };
}

mod gear;
mod loot;
mod quests;
mod skills;
mod stats;
mod native_buffs;
pub use native_buffs::{NativeBuffRule, NativeBuffs};
mod world;
pub use gear::{ActionBar, Inventory};
pub use loot::{Loot, LootFailure, item_index, item_owner, owner_type};
pub use quests::Quests;
pub use skills::{Casting, KnownSkill, NativeCooldown, NativeCooldownOrigin, ReceivedCooldown, SkillFailure, Skills};
pub use stats::{BuffEntry, Buffs, PlayerStats, LABOR_MAX, LABOR_MAX_PREMIUM};
pub use world::{Chat, ChatChannel, Target, TimeOfDay, WorldInfo, CHAT_LOG_LIMIT};
pub use crate::party::Party;
pub use crate::shop::Shop;
pub use crate::friend::Friends;
pub use crate::repair::Repair;
pub use crate::mail::Mailbox;
pub use crate::auction::Auction;
pub use crate::loot_roll::LootRolls;
pub use crate::actability::Actabilities;
pub use crate::mate::Mates;
pub use crate::slave::Slaves;
pub use crate::trade::Trade;
pub use crate::gimmick::Gimmicks;
pub use crate::housing::Houses;
pub use crate::farm::Farms;
pub use crate::radar::{FishRadar, ShipRadar, TransferRadar};
pub use crate::ui_data::UiData;
pub use crate::initial_config::GameConfig;
pub use crate::combat_status::CombatStatus;
pub use crate::conflict::ConflictZones;

resource! {
    /// The login server's worlds and the account's characters on them, plus the queue position.
    pub struct Worlds {
        pub list: WorldList,
        pub queue: Option<WorldQueue>,
        pub account: Option<AccountInfo>,
    }
}

impl Worlds {
    /// Returns true when the data changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::LoginSucceeded(a) => self.account = Some(*a),
            Event::WorldList(l) => {
                self.list = l.clone();
                self.queue = None;
            }
            Event::WorldQueue(q) => self.queue = Some(*q),
            Event::ReadyToConnect { .. } | Event::EnterWorldCancelled | Event::EnterWorldDenied { .. } => {
                self.queue = None
            }
            Event::Disconnected { .. } | Event::LoginFailed { .. } => *self = Self::default(),
            _ => return false,
        }
        true
    }
}

resource! {
    /// The characters on the connected world.
    pub struct Characters {
        pub list: Vec<Character>,
        /// Congestion per race (index = race id), for character creation.
        pub race_congestion: [u8; 9],
        /// The character whose labor power the account manages (`SCLpManaged`), this session.
        pub lp_managed: Option<u32>,
        /// Received extra slots (`SCGetSlotCount`), distinct from the login server's base limit.
        /// `None` means no receipt on this connection; `Some(0)` means known zero extras.
        pub extra_slots: Option<u8>,
        /// Latest payment header; these raw fields do not establish an active subscription.
        pub payment: Option<crate::account::AccountPayment>,
        /// Latest account-level premium receipt. Per-character points remain in `list`.
        pub premium_points: Option<crate::account::PremiumPoints>,
    }
}

impl Characters {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterSlotCount(count) => return replace_lobby_value(&mut self.extra_slots, *count),
            Event::AccountPayment(payment) => return replace_lobby_value(&mut self.payment, *payment),
            Event::PremiumPoints(points) => return replace_lobby_value(&mut self.premium_points, *points),
            Event::CharacterList(l) => self.list = l.clone(),
            Event::RaceCongestion(c) | Event::Account(crate::AccountEvent::CharacterListRefreshed { race_congestion: c }) => {
                self.race_congestion = *c
            }
            Event::Account(crate::AccountEvent::LpManaged { character_id }) => self.lp_managed = Some(*character_id),
            Event::CharacterDeleteCancelled { character_id, restored: true } => {
                let Some(c) = self.list.iter_mut().find(|c| c.id == *character_id) else { return false };
                (c.delete_request_time, c.delete_time) = (0, 0);
            }
            Event::Disconnected { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::WorldSelect) => *self = Self::default(),
            Event::PhaseChanged(Phase::ReadyToConnect)
            | Event::Kicked { .. }
            | Event::LoginFailed { .. } => {
                let changed = self.extra_slots.is_some() || self.payment.is_some() || self.premium_points.is_some();
                self.extra_slots = None;
                self.payment = None;
                self.premium_points = None;
                return changed;
            }
            _ => return false,
        }
        true
    }

    pub fn by_id(&self, id: u32) -> Option<&Character> {
        self.list.iter().find(|c| c.id == id)
    }
}

fn replace_lobby_value<T: PartialEq>(slot: &mut Option<T>, value: T) -> bool {
    if slot.as_ref() == Some(&value) {
        return false;
    }
    *slot = Some(value);
    true
}

resource! {
    /// The local player, once it is in the world.
    pub struct Player {
        pub spawn: Option<PlayerSpawn>,
    }
}

impl Player {
    pub fn clear_expedition(&mut self, expedition_id:u32)->bool {
        let Some(spawn)=self.spawn.as_mut() else{return false};
        if spawn.unit.expedition_id!=expedition_id || expedition_id==0 {return false;}
        spawn.unit.expedition_id=0;spawn.state.character.expedition_id=0;true
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::PlayerSpawn(s) => self.spawn = Some((**s).clone()),
            Event::Expedition(receipt) => return self.spawn.as_mut().is_some_and(|spawn| {
                let changed=spawn.unit.apply_expedition(receipt);
                if changed {spawn.state.character.expedition_id=spawn.unit.expedition_id;spawn.state.character.faction_id=spawn.unit.faction_id;}
                changed
            }),
            Event::Bounty(crate::BountyEvent::UnitMoney{obj_id,money}) => return self.spawn.as_mut().is_some_and(|spawn| {if spawn.obj_id!=*obj_id{return false;} let changed=spawn.unit.bounty_money!=Some(*money);spawn.unit.bounty_money=Some(*money);changed}),
            Event::Crime(receipt) => return self.spawn.as_mut().is_some_and(|spawn| {
                match *receipt {
                    crate::CrimeEvent::Changed{crime_points,infamy_points,crime_state,..} => {
                        let c=&mut spawn.state.character;
                        let changed=(c.crime_point,c.infamy_point,c.crime_state)!=(crime_points,infamy_points,crime_state);
                        (c.crime_point,c.infamy_point,c.crime_state)=(crime_points,infamy_points,crime_state);changed
                    }
                    crate::CrimeEvent::JuryPointChanged{jury_point} => {
                        let changed=spawn.state.jury_point!=jury_point;spawn.state.jury_point=jury_point;changed
                    }
                    crate::CrimeEvent::JuryWaitingNumber{..} => false,
                }
            }),
            Event::Appellation(crate::appellation::AppellationEvent::Changed { obj_id, id }) => {
                return self.spawn.as_mut().is_some_and(|spawn| spawn.obj_id == *obj_id && std::mem::replace(&mut spawn.unit.appellation, *id) != *id);
            }
            Event::UnitPvpPointsChanged(change) => return self.spawn.as_mut().is_some_and(|spawn| spawn.unit.apply_pvp_points(change)),
            Event::UnitGmModeChanged(change) => return self.spawn.as_mut().is_some_and(|spawn| spawn.unit.apply_gm_mode(change)),
            // the player gets on and off a mount (its own mate, or a seat on another's) like any unit
            Event::UnitAttached { obj_id, mount, point, reason } => {
                let attached = Some(Attachment { obj_id: *mount, point: *point, reason: *reason });
                return self.spawn.as_mut().is_some_and(|s| s.obj_id == *obj_id && std::mem::replace(&mut s.unit.attached_to, attached) != attached);
            }
            Event::UnitDetached { obj_id, .. } | Event::Resurrected { obj_id, .. } => {
                return self.spawn.as_mut().is_some_and(|s| s.obj_id == *obj_id && s.unit.attached_to.take().is_some());
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.spawn.take().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// Units in view, by object id (the local player is in [`Player`], not here).
    pub struct Units {
        pub by_id: HashMap<u32, Unit>,
    }
}

impl Units {
    pub fn clear_expedition(&mut self, expedition_id:u32)->bool {
        if expedition_id==0{return false;}
        let mut changed=false;
        for unit in self.by_id.values_mut() {if unit.expedition_id==expedition_id {unit.expedition_id=0;changed=true;}}
        changed
    }

    /// Apply received flags only to the existing, uniquely correlated current owned mate.
    pub fn apply_mate_equipment_flags(&mut self, event: &Event, mates: &Mates) -> bool {
        let Event::Mate(crate::mate::MateEvent::EquipmentFlagsChanged { mate_tl_id, container, slot, flags }) = event else { return false };
        if *container != 0xfc || *slot > 27 { return false; }
        let Some(own) = &mates.own else { return false };
        if own.spawn.tl_id != *mate_tl_id { return false; }
        let Some(unit) = own.obj_id.and_then(|id| self.by_id.get_mut(&id)) else { return false };
        if !mates.is_current_owned_unit(unit, *mate_tl_id) { return false; }
        let Some(item) = unit.equipment.slots[*slot as usize].as_mut() else { return false };
        if item.flags == *flags { return false; }
        item.flags = *flags;
        true
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::UnitSpawned(u) => {
                self.by_id.insert(u.obj_id, (**u).clone());
            }
            Event::Expedition(receipt) => {
                use crate::ExpeditionEvent as E;
                match receipt {
                    E::MembershipChanged{obj_id,..} | E::UnitFactionChanged{obj_id,..} =>
                        return self.by_id.get_mut(obj_id).is_some_and(|unit|unit.apply_expedition(receipt)),
                    E::Created{owner_obj_id,members,..} => {
                        let mut changed=self.by_id.get_mut(owner_obj_id).is_some_and(|unit|unit.apply_expedition(receipt));
                        for row in members.iter().take(4) {changed|=self.by_id.get_mut(&row.obj_id).is_some_and(|unit|unit.apply_expedition(receipt));}
                        return changed;
                    }
                    _=>return false,
                }
            }
            Event::Bounty(crate::BountyEvent::UnitMoney{obj_id,money})=>return self.by_id.get_mut(obj_id).is_some_and(|unit|{let changed=unit.bounty_money!=Some(*money);unit.bounty_money=Some(*money);changed}),
            Event::Appellation(crate::appellation::AppellationEvent::Changed { obj_id, id }) => {
                return self.by_id.get_mut(obj_id).is_some_and(|unit| std::mem::replace(&mut unit.appellation, *id) != *id);
            }
            Event::UnitPvpPointsChanged(change) => return self.by_id.get_mut(&change.obj_id).is_some_and(|unit| unit.apply_pvp_points(change)),
            Event::UnitGmModeChanged(change) => return self.by_id.get_mut(&change.obj_id).is_some_and(|unit| unit.apply_gm_mode(change)),
            Event::Slave(crate::slave::SlaveEvent::Escaped(escape)) => {
                let Some(unit) = self.by_id.get_mut(&escape.obj_id) else { return false };
                if !matches!(unit.kind, crate::model::UnitKind::Slave { .. }) { return false; }
                unit.pos = escape.position();
                unit.rotation[2] = escape.yaw;
            }
            Event::UnitMoved { obj_id, movement } => {
                // Ship requests carry only throttle and steering. Keep the last transform and
                // movement report; replacing either with this request would put a ship at (0, 0, 0).
                if movement.move_type == 4 {
                    return false;
                }
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                // a unit standing on an object (a deck, a vehicle) reports where it is relative to it, and in the world as well
                u.pos = movement.support.as_ref().and_then(|s| s.world).map_or(movement.pos, |(world, _)| world);
                u.rotation[2] = movement.yaw();
                u.movement = Some(movement.clone());
            }
            Event::UnitAttached { obj_id, mount, point, reason } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.attached_to = Some(Attachment { obj_id: *mount, point: *point, reason: *reason });
            }
            Event::UnitDetached { obj_id, .. } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.attached_to = None;
            }
            Event::UnitPosture { obj_id, kind, anim_action, activate } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.anim_action = if *kind == 4 && *activate { *anim_action } else { 0 };
                if *kind != 1 { u.house_posture = None; }
            }
            Event::Housing(crate::housing::HousingEvent::UnitPosture { obj_id, posture }) => {
                let Some(unit) = self.by_id.get_mut(obj_id) else { return false; };
                if unit.house_posture == Some(*posture) { return false; }
                unit.house_posture = Some(*posture);
            }
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.by_id.remove(id);
                }
            }
            Event::UnitPoints { obj_id, hp, mp } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                (u.hp, u.mp) = (*hp, *mp);
                u.max_hp = u.max_hp.max(*hp);
                u.max_mp = u.max_mp.max(*mp);
                if *hp > 0 {
                    u.dead = false;
                }
            }
            Event::UnitDeath { obj_id } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                (u.dead, u.hp) = (true, 0);
                u.movement = None;
            }
            Event::Resurrected { obj_id, pos, yaw } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.dead = false;
                u.pos = *pos;
                u.rotation[2] = *yaw;
                u.movement = None;
                u.attached_to = None;
            }
            Event::LevelChanged { obj_id, level } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                u.level = *level;
            }
            Event::EquipmentChanged { obj_id, items } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                for (slot, item) in items {
                    if let Some(s) = u.equipment.slots.get_mut(usize::from(*slot)) {
                        *s = item.clone();
                    }
                }
            }
            Event::UnitNameChanged { obj_id, name } => {
                let Some(unit) = self.by_id.get_mut(obj_id) else { return false };
                unit.name = name.clone();
            }
            // Compatibility-only event: active SC01b decoding uses the raw Expedition receipt.
            Event::UnitFactionChanged { obj_id, faction_id, faction_id2, .. } => {
                let Some(u) = self.by_id.get_mut(obj_id) else { return false };
                if u.faction_id!=*faction_id || u.faction_id==*faction_id2 {return false;}
                u.faction_id = *faction_id2;
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_id.is_empty() {
                    return false;
                }
                self.by_id.clear();
            }
            _ => return false,
        }
        true
    }

    pub fn get(&self, obj_id: u32) -> Option<&Unit> {
        self.by_id.get(&obj_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values()
    }

    pub fn characters(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values().filter(|u| u.is_character())
    }

    pub fn npcs(&self) -> impl Iterator<Item = &Unit> {
        self.by_id.values().filter(|u| u.is_npc())
    }
}

resource! {
    /// Doodads in view, by object id.
    pub struct Doodads {
        pub by_id: HashMap<u32, Doodad>,
    }
}

impl Doodads {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::DoodadsSpawned(v) => {
                for d in v {
                    self.by_id.insert(d.obj_id, d.clone());
                }
            }
            Event::DoodadsRemoved(ids) => {
                for id in ids {
                    self.by_id.remove(id);
                }
            }
            Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, puzzle_group, item_template_id } => {
                let Some(d) = self.by_id.get_mut(obj_id) else { return false };
                d.func_group_id = *func_group_id;
                d.time_left = *time_left;
                d.puzzle_group = *puzzle_group;
                d.item_template_id = *item_template_id;
            }
            Event::DoodadChanged { obj_id, data } => {
                let Some(d) = self.by_id.get_mut(obj_id) else { return false };
                d.data = *data;
            }
            Event::DoodadOriginator { obj_id, new_owner_id, .. } => {
                let Some(d) = self.by_id.get_mut(obj_id) else { return false };
                d.owner_id = *new_owner_id;
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_id.is_empty() {
                    return false;
                }
                self.by_id.clear();
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// The session phase (see [`Phase`]).
    pub struct Status {
        pub phase: Phase,
        /// Last connection or login failure, for display.
        pub last_error: Option<String>,
    }
}

impl Status {
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::PhaseChanged(p) => self.phase = *p,
            Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                self.last_error = Some(message.clone())
            }
            Event::LoginSucceeded(_) | Event::LobbyReady => self.last_error = None,
            _ => return false,
        }
        true
    }
}

/// All of the above in one value.
#[derive(Clone, Debug, Default)]
pub struct WorldState {
    pub worlds: Worlds,
    pub characters: Characters,
    pub player: Player,
    pub units: Units,
    pub doodads: Doodads,
    pub status: Status,
    pub stats: PlayerStats,
    pub buffs: Buffs,
    pub actions: ActionBar,
    pub inventory: Inventory,
    pub world: WorldInfo,
    pub chat: Chat,
    pub target: Target,
    pub skills: Skills,
    pub quests: Quests,
    pub loot: Loot,
    pub party: Party,
    pub shop: Shop,
    pub friends: Friends,
    pub repair: Repair,
    pub mailbox: Mailbox,
    pub trade: Trade,
    pub auction: Auction,
    pub loot_rolls: LootRolls,
    pub scheduled_events: crate::scheduled::ScheduledEvents,
    pub dominion: crate::Dominion,
    pub trials: crate::trial::Trials,
    pub crimes: crate::crime::Crimes,
    pub duels: crate::duel::Duels,
    pub appellations: crate::appellation::Appellations,
    pub bounties:crate::Bounties,
    pub rankings:crate::Rankings,
    pub character_names:crate::CharacterNames,
    pub achievements: crate::achievement::Achievements,
    pub expeditions: crate::expedition::Expeditions,
    pub families: crate::family::Families,
    pub server_clock: crate::clock::ServerClock,
    pub battlefield: crate::battlefield::Battlefield,
    pub actabilities: Actabilities,
    pub mates: Mates,
    pub slaves: Slaves,
    pub doodad_bonds: crate::DoodadBonds,
    pub gimmicks: Gimmicks,
    pub houses: Houses,
    pub farms: Farms,
    pub transfer_radar: TransferRadar,
    pub ship_radar: ShipRadar,
    pub fish_radar: FishRadar,
    pub ui_data: UiData,
    pub game_config: GameConfig,
    pub combat_status: CombatStatus,
    pub conflict_zones: ConflictZones,
}

impl WorldState {
    pub fn apply(&mut self, e: &Event) {
        if let Some(id)=self.expeditions.admitted_dismissal(e) {
            self.player.clear_expedition(id);self.units.clear_expedition(id);
        }
        self.worlds.apply(e);
        self.characters.apply(e);
        self.player.apply(e);
        self.units.apply(e);
        self.doodads.apply(e);
        self.status.apply(e);
        self.stats.apply(e);
        self.buffs.apply(e);
        self.actions.apply(e);
        self.inventory.apply(e);
        self.world.apply(e);
        self.chat.apply(e);
        self.target.apply(e);
        self.skills.apply(e);
        self.quests.apply(e);
        self.loot.apply(e);
        self.party.apply(e);
        self.party.seed_visible(e, &self.player, &self.units);
        self.shop.apply(e);
        self.friends.apply(e);
        self.repair.apply(e);
        self.mailbox.apply(e);
        self.auction.apply(e);
        self.loot_rolls.apply(e);
        self.scheduled_events.apply(e);
        self.dominion.apply(e);
        self.trials.apply(e);
        self.crimes.apply(e);
        self.duels.apply(e);
        self.appellations.apply(e);
        self.bounties.apply(e);
        self.rankings.apply(e);
        self.character_names.apply(e);
        self.achievements.apply(e);
        self.expeditions.apply(e);
        self.families.apply(e);
        self.server_clock.apply(e);
        if let Some(now)=self.server_clock.mapped_now() {self.battlefield.advance_server_time(now);}
        self.battlefield.apply_with_time(e,self.server_clock.mapped_now());
        self.actabilities.apply(e);
        self.mates.apply(e);
        self.units.apply_mate_equipment_flags(e, &self.mates);
        self.slaves.apply(e);
        self.doodad_bonds.apply(e);
        self.gimmicks.apply(e);
        self.houses.apply(e);
        self.farms.apply(e);
        self.transfer_radar.apply(e);
        self.ship_radar.apply(e);
        self.fish_radar.apply(e);
        self.ui_data.apply(e);
        self.game_config.apply(e);
        self.combat_status.apply(e);
        self.conflict_zones.apply(e);
        self.trade.apply(e);
        self.trade.seed_inventory(e, &self.inventory);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Equipment, ModelParams, Movement, UnitKind};
    use crate::wire::Pos;

    fn doodad_fixture(obj_id: u32) -> Doodad {
        Doodad {
            obj_id,
            template_id: 2336,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: Pos::new(50.0, 60.0, 7.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 1,
            owner_id: 12,
            ucc_id: 0,
            item_template_id: 1,
            type2: 0,
            time_left: 1000,
            plant_time: 1700000000,
            quest_glow: 0,
            family: 0,
            puzzle_group: 5,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        }
    }

    #[test]
    fn doodad_phase_updates_only_the_existing_authoritative_object_and_can_clear_fields() {
        let obj_id = 0x010203;
        let doodad = doodad_fixture(obj_id);
        let mut state = Doodads::default();
        let event = Event::DoodadPhaseChanged {
            obj_id,
            func_group_id: 4652,
            time_left: 30000,
            puzzle_group: -1,
            item_template_id: 27606,
        };
        assert!(!state.apply(&event));
        assert!(state.by_id.is_empty());
        state.apply(&Event::DoodadsSpawned(vec![doodad.clone()]));
        assert!(state.apply(&event));
        let changed = state.by_id.get(&obj_id).unwrap();
        assert_eq!(
            (
                changed.func_group_id,
                changed.time_left,
                changed.puzzle_group,
                changed.item_template_id
            ),
            (4652, 30000, -1, 27606)
        );
        assert_eq!(
            (
                changed.pos,
                changed.template_id,
                changed.owner_id,
                changed.plant_time
            ),
            (
                doodad.pos,
                doodad.template_id,
                doodad.owner_id,
                doodad.plant_time
            )
        );
        state.apply(&Event::DoodadPhaseChanged {
            obj_id,
            func_group_id: 4652,
            time_left: 0,
            puzzle_group: 0,
            item_template_id: 0,
        });
        let changed = state.by_id.get(&obj_id).unwrap();
        assert_eq!(
            (
                changed.time_left,
                changed.puzzle_group,
                changed.item_template_id
            ),
            (0, 0, 0)
        );
        state.apply(&Event::DoodadsRemoved(vec![obj_id]));
        assert!(!state.apply(&event));
        assert!(state.by_id.is_empty());
    }

    #[test]
    fn doodad_data_and_originator_update_known_objects_without_inventing_owner_object_or_faction() {
        let obj_id = 8;
        let mut state = Doodads::default();
        let data = Event::DoodadChanged { obj_id, data: -3 };
        let owner = Event::DoodadOriginator {
            obj_id,
            new_owner_id: 0xf1234567,
            new_faction: 0x80000000,
        };
        let sound = Event::DoodadSound {
            obj_id,
            sound_id: u32::MAX,
        };
        for event in [&data, &owner, &sound] {
            assert!(!state.apply(event));
        }
        assert!(state.by_id.is_empty());
        let mut original = doodad_fixture(obj_id);
        original.owner_obj_id = 0x112233;
        original.parent_obj_id = 0x445566;
        original.owner_type = 3;
        original.owner_db_id = 12345;
        state.apply(&Event::DoodadsSpawned(vec![original.clone()]));
        assert!(state.apply(&data));
        assert!(state.apply(&owner));
        let changed = state.by_id.get(&obj_id).unwrap();
        assert_eq!((changed.data, changed.owner_id), (-3, 0xf1234567));
        let mut expected = original.clone();
        expected.data = -3;
        expected.owner_id = 0xf1234567;
        assert_eq!(
            changed, &expected,
            "owner object/attachment/permission/phase metadata is not on this wire"
        );
        assert!(!state.apply(&sound));
        assert_eq!(state.by_id.get(&obj_id), Some(&expected));
        state.apply(&Event::DoodadChanged { obj_id, data: 0 });
        state.apply(&Event::DoodadOriginator {
            obj_id,
            new_owner_id: 0,
            new_faction: 0,
        });
        assert_eq!(
            (state.by_id[&obj_id].data, state.by_id[&obj_id].owner_id),
            (0, 0)
        );
        state.apply(&Event::DoodadsRemoved(vec![obj_id]));
        for event in [&data, &owner, &sound] {
            assert!(!state.apply(event));
        }
        assert!(state.by_id.is_empty());
    }

    pub(crate) fn unit(obj_id: u32) -> Unit {
        Unit {
            obj_id,
            name: format!("u{obj_id}"),
            kind: UnitKind::Npc { template_id: 1, owner_id: 0 },
            owner_name: String::new(),
            pos: Pos::new(1.0, 2.0, 3.0),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 1,
            model_id: 1,
            equipment: Equipment::default(),
            model_params: ModelParams::default(),
            hp: 10,
            mp: 0,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0;4],
            bounty_money:None,
            pvp_honor:0,
            pvp_kills:0,
            skills: Vec::new(),
            passive_buffs: Vec::new(),
            ability_exp: Vec::new(),
            active_abilities: Vec::new(),
            buffs: Vec::new(),
            anim_action: 0,
            house_posture: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
            max_hp: 10,
            max_mp: 0,
            dead: false,
        }
    }

    #[test]
    fn units_follow_spawn_move_points_and_removal() {
        let mut s = WorldState::default();
        s.apply(&Event::UnitSpawned(Box::new(unit(5))));
        s.apply(&Event::UnitSpawned(Box::new(unit(6))));
        assert_eq!(s.units.by_id.len(), 2);
        s.apply(&Event::UnitMoved {
            obj_id: 5,
            movement: Movement::unit(Pos::new(9.0, 8.0, 7.0), 0.0, 1),
        });
        assert!((s.units.get(5).unwrap().pos.x - 9.0).abs() < 1e-6);
        s.apply(&Event::UnitPoints { obj_id: 5, hp: 3, mp: 4 });
        assert_eq!(s.units.get(5).unwrap().hp, 3);
        // a death marks the unit dead until it is resurrected or has health again
        assert!(!s.units.get(5).unwrap().dead);
        s.apply(&Event::UnitPoints { obj_id: 5, hp: 0, mp: 4 });
        assert!(!s.units.get(5).unwrap().dead, "health 0 alone is not the death packet");
        assert!(s.units.apply(&Event::UnitDeath { obj_id: 5 }));
        assert!(s.units.get(5).unwrap().dead && s.units.get(5).unwrap().hp == 0);
        assert!(s.units.apply(&Event::Resurrected { obj_id: 5, pos: Pos::default(), yaw: 0.0 }));
        assert!(!s.units.get(5).unwrap().dead);
        assert!(!s.units.apply(&Event::UnitDeath { obj_id: 99 }));
        s.apply(&Event::UnitPoints { obj_id: 5, hp: 3, mp: 4 });
        s.apply(&Event::UnitsRemoved(vec![5]));
        assert!(s.units.get(5).is_none() && s.units.get(6).is_some());
        // Leaving the world clears the view.
        s.apply(&Event::LeftWorld { target: 1 });
        assert!(s.units.by_id.is_empty());
        // A move for an unknown unit changes nothing.
        assert!(!s.units.apply(&Event::UnitMoved {
            obj_id: 99,
            movement: Movement::unit(Pos::default(), 0.0, 1),
        }));
    }

    #[test]
    fn the_player_gets_on_and_off_a_mount() {
        use crate::model::Attachment;
        let mut player = Player::default();
        player.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn { obj_id: 5, state: Default::default(), unit: unit(5) })));
        assert!(!player.apply(&Event::UnitAttached { obj_id: 6, mount: 9, point: 1, reason: 2 }), "another unit");
        // the seat keeps the reply's get-on reason (live capture 2026-10-28: it repeats CSMountMate's)
        assert!(player.apply(&Event::UnitAttached { obj_id: 5, mount: 9, point: 1, reason: 2 }));
        assert_eq!(player.spawn.as_ref().unwrap().unit.attached_to, Some(Attachment { obj_id: 9, point: 1, reason: 2 }));
        assert!(!player.apply(&Event::UnitAttached { obj_id: 5, mount: 9, point: 1, reason: 2 }), "no change");
        assert!(player.apply(&Event::UnitDetached { obj_id: 5, reason: 9 }));
        assert_eq!(player.spawn.as_ref().unwrap().unit.attached_to, None);
        player.apply(&Event::UnitAttached { obj_id: 5, mount: 9, point: 2, reason: 1 });
        assert!(player.apply(&Event::Resurrected { obj_id: 5, pos: Pos::default(), yaw: 0.0 }));
        assert_eq!(player.spawn.as_ref().unwrap().unit.attached_to, None);
    }

    #[test]
    fn units_follow_mounting_poses_and_what_they_stand_on() {
        use crate::model::{actor_flags, Attachment, MoveSupport};
        let mut units = Units::default();
        units.apply(&Event::UnitSpawned(Box::new(unit(5))));
        assert!(units.apply(&Event::UnitAttached { obj_id: 5, mount: 9, point: 1, reason: 2 }));
        assert_eq!(units.get(5).unwrap().attached_to, Some(Attachment { obj_id: 9, point: 1, reason: 2 }));
        assert!(units.apply(&Event::UnitDetached { obj_id: 5, reason: 4 }));
        assert_eq!(units.get(5).unwrap().attached_to, None);
        assert!(!units.apply(&Event::UnitDetached { obj_id: 99, reason: 4 }));
        // an NPC takes a pose, and leaves it (the packet repeats the animation action)
        assert!(units.apply(&Event::UnitPosture { obj_id: 5, kind: 4, anim_action: 104, activate: true }));
        assert_eq!(units.get(5).unwrap().anim_action, 104);
        assert!(units.apply(&Event::UnitPosture { obj_id: 5, kind: 4, anim_action: 104, activate: false }));
        assert_eq!(units.get(5).unwrap().anim_action, 0);
        // standing on a deck: the unit is where its world position says, not at the deck-relative one
        let mut movement = Movement::unit(Pos::new(1.0, 2.0, 0.5), 0.0, 1);
        movement.actor_flags = actor_flags::STANDING_ON_OBJECT;
        movement.support = Some(MoveSupport { obj_id: 9, world: Some((Pos::new(500.0, 600.0, 7.0), [0; 3])), ..MoveSupport::default() });
        units.apply(&Event::UnitMoved { obj_id: 5, movement });
        let pos = units.get(5).unwrap().pos;
        assert_eq!((pos.x, pos.y), (500.0, 600.0));
        // a vehicle reports its facing as a rotation, not a direction byte
        let mut cart = Movement::unit(Pos::new(1.0, 2.0, 3.0), 0.0, 2);
        (cart.move_type, cart.vehicle_yaw) = (2, Some(1.5));
        units.apply(&Event::UnitMoved { obj_id: 5, movement: cart });
        assert!((units.get(5).unwrap().yaw() - 1.5).abs() < 1e-6);
    }

    #[test]
    fn ship_requests_keep_the_last_position_and_movement() {
        let mut units = Units::default();
        units.apply(&Event::UnitSpawned(Box::new(unit(5))));
        let mut ship = Movement::unit(Pos::new(500.0, 600.0, 7.0), 0.0, 1);
        (ship.move_type, ship.vehicle_yaw) = (3, Some(1.5));
        units.apply(&Event::UnitMoved { obj_id: 5, movement: ship.clone() });
        // This is what Movement::read_typed(4) returns: no transform on the wire.
        let mut request = Movement::unit(Pos::default(), 0.0, 2);
        request.move_type = 4;
        assert!(!units.apply(&Event::UnitMoved { obj_id: 5, movement: request }));
        let ship_unit = units.get(5).unwrap();
        assert_eq!(ship_unit.pos, ship.pos);
        assert_eq!(ship_unit.yaw(), 1.5);
        assert_eq!(ship_unit.movement.as_ref(), Some(&ship));
    }

    #[test]
    fn resurrection_replaces_the_corpses_transform_and_old_motion() {
        let mut units = Units::default();
        let mut rider = unit(5);
        rider.attached_to = Some(crate::model::Attachment { obj_id: 9, point: 1, reason: 1 });
        units.apply(&Event::UnitSpawned(Box::new(rider)));
        units.apply(&Event::UnitMoved { obj_id: 5, movement: Movement::unit(Pos::new(9.0, 8.0, 7.0), 0.5, 1) });
        units.apply(&Event::UnitDeath { obj_id: 5 });
        assert!(units.get(5).unwrap().movement.is_none());
        let pos = Pos::new(500.0, 600.0, 7.0);
        assert!(units.apply(&Event::Resurrected { obj_id: 5, pos, yaw: -1.25 }));
        let revived = units.get(5).unwrap();
        assert!(!revived.dead);
        assert_eq!(revived.pos, pos);
        assert_eq!(revived.yaw(), -1.25);
        assert!(revived.movement.is_none());
        assert!(revived.attached_to.is_none());
    }

    #[test]
    fn a_cancelled_delete_clears_the_timer() {
        let mut s = Characters::default();
        let pending = Character { id: 3, name: "A".into(), delete_request_time: 100, delete_time: 160, ..Default::default() };
        s.list.push(pending);
        assert!(s.by_id(3).unwrap().delete_requested());
        // status 4 (unknown character) restores nothing
        assert!(!s.apply(&Event::CharacterDeleteCancelled { character_id: 3, restored: false }));
        assert!(s.by_id(3).unwrap().delete_requested());
        assert!(s.apply(&Event::CharacterDeleteCancelled { character_id: 3, restored: true }));
        let c = s.by_id(3).unwrap();
        assert!(!c.delete_requested());
        assert_eq!(c.delete_time, 0);
        // a cancel for a character that is not listed changes nothing
        assert!(!s.apply(&Event::CharacterDeleteCancelled { character_id: 9, restored: true }));
    }

    #[test]
    fn worlds_characters_and_status() {
        let mut s = WorldState::default();
        s.apply(&Event::PhaseChanged(Phase::WorldSelect));
        assert_eq!(s.status.phase, Phase::WorldSelect);
        s.apply(&Event::CharacterList(vec![Character { id: 3, name: "A".into(), ..Default::default() }]));
        assert_eq!(s.characters.by_id(3).unwrap().name, "A");
        s.apply(&Event::LoginFailed { message: "bad".into() });
        assert_eq!(s.status.last_error.as_deref(), Some("bad"));
        s.apply(&Event::Disconnected { message: "gone".into() });
        assert!(s.characters.list.is_empty());
    }
}
