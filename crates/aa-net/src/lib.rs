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

pub mod actability;
pub mod auction;
pub mod buff;
pub mod bank;
mod compression;
pub mod combat_status;
pub mod craft;
pub mod friend;
pub mod game;
pub mod gimmick;
pub mod interaction;
pub mod items;
pub mod login;
pub mod loot_roll;
pub mod mail;
pub mod mate;
pub mod mate_equipment;
pub mod model;
pub mod net;
pub mod opcodes;
pub mod party;
pub mod quest;
pub mod repair;
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
pub use gimmick::{Gimmick, GimmickEvent, GimmickJoint, GimmickMovement, GimmickSpawn, Gimmicks};
pub use actability::{Actabilities, Actability, ActabilityCommand, ActabilityEvent, ActabilityFailure, ActabilityValue};
pub use craft::CraftFailureNotice;
pub use combat_status::{CombatStatus, EnvironmentDamage, UnitCombatFlags};
pub use items::ItemLifespan;
pub use world_msg::{BlinkUnit, InstanceLoad};
pub use login::Secret;
pub use loot_roll::{LootRollCommand, LootRollEvent, LootRollFailure, LootRollKey, LootRollNotice, LootRollPrompt, LootRollResult, LootRollRound, LootRolls, LootRollSummary};
pub use mail::{CachedMailBody, MailAttachment, MailBody, MailCommand, MailEvent, MailFailure, MailHeader, MailItemLocation, MailSlot, Mailbox, SendMail, UnreadMail};
pub use mate::{DynamicUnitState, MateCommand, MateEvent, MateSpawn, Mates, OwnedMate, SlaveState};
pub use mate_equipment::{MateEquipmentChange, MateEquipmentEntry, MateItemLocation};
pub use model::{Character, CreateCharacter, Doodad, Unit};
pub use session::{Client, ClientOptions, Command, Event, MoveCommand, MoveState, Phase, PlayerSpawn};
pub use state::WorldState;
pub use party::{LootingRule, Party, PartyCommand, PartyEvent, PartyInvite, PartyMember};
pub use repair::{Repair, RepairCommand, RepairEvent, RepairReport, RepairedItem};
pub use trade::{Trade, TradeCommand, TradeEvent, TradeFailure, TradeOffer};
pub use slave::{SlaveCreated, SlaveEscape, SlaveEvent, SlavePositionInfo, Slaves};
pub use slave_control::{SlaveCommand, SlaveItemLocation, SlaveSpawnRequest};
pub use wire::Pos;
pub use ui_data::{UiData, UiDataCommand, UiDataEvent, UiDataResponse};

#[cfg(feature = "bevy")]
pub use plugin::{NetCommand, NetEvent, NetPlugin};
