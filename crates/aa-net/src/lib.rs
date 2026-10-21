//! Login, game and stream protocol client for the AAEmu 1.2 server.
//!
//! Layers, from the bottom:
//!
//! * [`wire`], [`opcodes`], [`login`], [`game`], [`stream`], [`model`]: packet reader / writer and
//!   typed messages for what the client uses. Plain Rust, no I/O, no Bevy.
//! * [`net`]: tokio connections (`LoginConn`, `GameConn`, `StreamConn`) for code that wants to drive
//!   the protocol step by step.
//! * [`session`]: [`Client`], one background task that runs the whole login to in-game flow from
//!   [`Command`]s and reports [`Event`]s. [`state`] folds events into world, character, unit and
//!   doodad tables.
//! * [`plugin`] (cargo feature `bevy`): `NetPlugin`, the session as Bevy resources and messages.
//!
//! See `docs/aa-net.md` for the flow and how to run against the local server.

pub mod trial;
pub use trial::{TrialCommand, TrialCrimeRecord, TrialDefendantData, TrialEvent, TrialHistory, TrialNotice, TrialRecords, TrialRulingStatus, Trials};
pub mod crime;
pub mod duel;
pub use duel::{ActiveDuel, DuelEvent, DuelNotice, DuelResult, Duels};
pub mod emote;
pub use emote::{BubbleBody, EmoteEvent};
pub mod appellation;
pub use appellation::{AppellationEvent, Appellations};
pub use crime::{CrimeEvent, CrimeNotice, CrimeReputation, Crimes};
pub mod achievement;
pub use achievement::{AchievementEvent, AchievementNotice, AchievementProgress, AchievementRule, Achievements};
pub mod actability;
pub mod auction;
pub mod buff;
pub mod bank;
mod compression;
pub mod clock;
pub use clock::{ServerClock, NativeTimeKeeper, ClockSample};
pub mod combat_status;
pub mod conflict;
pub mod craft;
pub mod friend;
pub mod game;
pub mod gimmick;
pub mod housing;
pub mod farm;
pub mod radar;
pub mod battlefield;
pub mod interaction;
pub mod initial_config;
pub mod items;
pub mod specialty;
pub mod item_effect;
pub mod item_task_projection;
pub mod native_item_records;
pub use native_item_records::{NativeItemRecords, NativeItemRowOutput};
pub mod login;
pub mod loot_roll;
pub mod mail;
pub mod mate;
pub mod mate_equipment;
pub mod model;
pub mod net;
pub mod opcodes;
pub mod party;
pub mod portal;
pub use portal::{PortalChange, PortalCommand, PortalEntry, PortalEvent, Portals};
pub mod proxy;
pub mod quest;
pub mod repair;
pub mod scheduled;
pub mod schedule;
pub use scheduled::{ScheduledEventStarted,ScheduledEvents};
pub mod session;
pub mod skill;
pub mod shop;
pub mod slave;
pub mod slave_control;
pub mod state;
pub mod stream;
pub mod trade;
pub mod unit_msg;
pub mod ui_data;
pub mod wire;
pub mod world_msg;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use auction::{Auction, AuctionBid, AuctionCommand, AuctionEvent, AuctionFailure, AuctioneerIds, AuctionLot, AuctionPurchaseMail, AuctionQuery, AuctionQuote, AuctionSearch};
pub use friend::{BlockedUser, FriendCommand, FriendEvent, FriendFailure, FriendInfo, Friends};
pub use game::LeaveTarget;
pub use bank::{BankCommand, BankEvent};
pub use radar::{FishRadar, FishRadarContact, ShipRadar, ShipRadarContact, RadarEvent, RadarToggle, TransferRadar, TransferRadarContact};
pub use housing::{ConstructHouseTax, House, HouseCommand, HouseFarm, HouseSale, HouseSnapshot, HouseTaxInfo, Houses, HousingAttachment, HousingEvent, housing_attachments};
pub use farm::{CommonFarmEntry, CommonFarmList, CreateDoodad, FarmEvent, Farms, ShowCommonFarm};
pub use gimmick::{Gimmick, GimmickEvent, GimmickJoint, GimmickMovement, GimmickSpawn, Gimmicks};
pub use actability::{Actabilities, Actability, ActabilityCommand, ActabilityEvent, ActabilityFailure, ActabilityValue};
pub use craft::{CraftFailureNotice, CraftItemUnlock};
pub use combat_status::{CombatStatus, EnvironmentDamage, UnitCombatFlags};
pub use conflict::{ConflictZones, ConflictZoneState};
pub use items::ItemLifespan;
pub use interaction::WorldInteractionSkillList;
pub use initial_config::{GameConfig, InitialConfig};
pub use world_msg::{BlinkUnit, InstanceLoad};
pub use login::Secret;
pub use loot_roll::{LootRollCommand, LootRollEvent, LootRollFailure, LootRollKey, LootRollNotice, LootRollPrompt, LootRollResult, LootRollRound, LootRolls, LootRollSummary};
pub use mail::{CachedMailBody, MailAttachment, MailBody, MailCommand, MailEvent, MailFailure, MailHeader, MailItemLocation, MailSlot, Mailbox, SendMail, UnreadMail};
pub use mate::{DynamicUnitState, MateCommand, MateEvent, MateSpawn, Mates, OwnedMate, SlaveState};
pub use mate_equipment::{MateEquipmentRequest, MateEquipmentChange, MateEquipmentEntry, MateItemLocation};
pub use model::{Character, CreateCharacter, Doodad, Unit, UnitStateAttachments};
pub use session::{Client, ClientOptions, Command, Event, MoveCommand, MoveState, Phase, PlayerSpawn};
pub use state::WorldState;
pub use skill::{SkillCasterData, SkillTargetData, SkillPosition, SkillResult, Cooldown, SkillCooldowns, COOLDOWN_LIMIT};
pub use party::{LootingRule, Party, PartyCommand, PartyEvent, PartyInvite, PartyMember};
pub use repair::{Repair, RepairCommand, RepairEvent, RepairReport, RepairedItem};
pub use trade::{Trade, TradeCommand, TradeEvent, TradeFailure, TradeOffer};
pub use slave::{SlaveCreated, SlaveEquipmentChange, SlaveEscape, SlaveEvent, SlavePositionInfo, Slaves};
pub use slave_control::{SlaveCommand, SlaveItemLocation, SlaveSpawnRequest};
pub use wire::Pos;
pub use ui_data::{UiData, UiDataCommand, UiDataEvent, UiDataResponse};

#[cfg(feature = "bevy")]
pub use plugin::{NetCommand, NetEvent, NetPlugin};

#[cfg(test)]
mod unit_state_tests;

#[cfg(test)]
mod equipment_notice_tests;

pub use battlefield::{Battlefield, BattlefieldCommand, BattlefieldEvent, BattlefieldInvitation, BattlefieldQueue};

pub mod dominion;
pub use dominion::{Dominion, DominionEvent, DominionData, DominionTerritory, DominionSiegeTimers, DominionParticipant, DominionPosition};
pub mod expedition;
pub use expedition::{ExpeditionNotice, Expeditions, ExpeditionEvent, ExpeditionCreatedMember, ExpeditionFaction, ExpeditionMember, ExpeditionRolePolicy};
pub mod family;
pub mod expedition_command;
pub use expedition_command::ExpeditionCommand;
pub mod family_command;
pub use family_command::FamilyCommand;
pub use family::{Families, FamilyEvent, FamilyInvitation, FamilyMember, FamilyRoster, FamilyTitle};

pub mod unpack;
pub use unpack::UnpackItemRequest;

mod bounty;
pub use bounty::{Bounties,BountyCommand,BountyEvent,BountyNotice,BountyRow};

mod ranking;
pub use ranking::{Rankings,RankingEvent,RankingNotice,RankingCommand,RankBoard,RankCharacterRow,RankSnapshot,RankSnapshotEntry,RankSnapshotScope};

mod character_names;
pub use character_names::{CharacterName,CharacterNames};
