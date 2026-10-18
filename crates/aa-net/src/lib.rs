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
pub mod friend;
pub mod game;
pub mod interaction;
pub mod items;
pub mod login;
pub mod loot_roll;
pub mod mail;
pub mod mate;
pub mod model;
pub mod net;
pub mod opcodes;
pub mod party;
pub mod quest;
pub mod repair;
pub mod session;
pub mod skill;
pub mod shop;
pub mod state;
pub mod stream;
pub mod trade;
pub mod unit_msg;
pub mod wire;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use auction::{Auction, AuctionBid, AuctionCommand, AuctionEvent, AuctionFailure, AuctioneerIds, AuctionLot, AuctionPurchaseMail, AuctionQuery, AuctionQuote, AuctionSearch};
pub use friend::{BlockedUser, FriendCommand, FriendEvent, FriendFailure, FriendInfo, Friends};
pub use game::LeaveTarget;
pub use actability::{Actabilities, Actability, ActabilityCommand, ActabilityEvent, ActabilityFailure, ActabilityValue};
pub use login::Secret;
pub use loot_roll::{LootRollCommand, LootRollEvent, LootRollFailure, LootRollKey, LootRollNotice, LootRollPrompt, LootRollResult, LootRollRound, LootRolls, LootRollSummary};
pub use mail::{CachedMailBody, MailAttachment, MailBody, MailCommand, MailEvent, MailFailure, MailHeader, MailItemLocation, MailSlot, Mailbox, SendMail, UnreadMail};
pub use mate::{DynamicUnitState, MateCommand, MateEvent, MateSpawn, Mates, OwnedMate, SlaveState};
pub use model::{Character, CreateCharacter, Doodad, Unit};
pub use session::{Client, ClientOptions, Command, Event, MoveCommand, MoveState, Phase, PlayerSpawn};
pub use state::WorldState;
pub use party::{LootingRule, Party, PartyCommand, PartyEvent, PartyInvite, PartyMember};
pub use repair::{Repair, RepairCommand, RepairEvent, RepairReport, RepairedItem};
pub use trade::{Trade, TradeCommand, TradeEvent, TradeFailure, TradeOffer};
pub use wire::Pos;

#[cfg(feature = "bevy")]
pub use plugin::{NetCommand, NetEvent, NetPlugin};
