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

pub mod buff;
pub mod game;
pub mod items;
pub mod login;
pub mod model;
pub mod net;
pub mod opcodes;
pub mod quest;
pub mod session;
pub mod skill;
pub mod state;
pub mod stream;
pub mod wire;

#[cfg(feature = "bevy")]
pub mod plugin;

pub use game::LeaveTarget;
pub use login::Secret;
pub use model::{Character, CreateCharacter, Doodad, Unit};
pub use session::{Client, ClientOptions, Command, Event, MoveCommand, MoveState, Phase, PlayerSpawn};
pub use state::WorldState;
pub use wire::Pos;

#[cfg(feature = "bevy")]
pub use plugin::{NetCommand, NetEvent, NetPlugin};
