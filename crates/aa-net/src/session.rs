//! The client session: one task that owns the login, game and stream connections and turns
//! [`Command`]s from the app into protocol traffic and server packets into [`Event`]s.
//!
//! ```text
//!  Login ──► WorldList ──► EnterWorld ──► (WorldQueue) ──► ReadyToConnect ──► ConnectToWorld
//!    ──► LobbyReady ──► CharacterList ──► (CreateCharacter) ──► EnterGame ──► PlayerSpawn
//!    ──► EnteredWorld ──► units, doodads, chat, movement ... ──► LeaveWorld
//! ```
//!
//! The session never blocks the caller: [`Client::send`] queues a command and
//! [`Client::try_recv`] / [`Client::recv`] return events.

use std::time::Duration;
use web_time::Instant;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::buff::{Buff, BuffCaster};
use crate::game::{self, LeaveTarget, VisualOptions};
use crate::items::{ActionSlot, InvenChunk, ItemMove, ItemTasks};
use crate::quest::{CompletedBlock, Quest};
use crate::skill::{PlotEvent, SkillFired, SkillStarted, SkillTarget, StartCaster, UnitDamage, UnitHeal};
use crate::unit_msg::AggroEntry;
use crate::login::{self, AccountInfo, Secret, WorldCookie, WorldList, WorldQueue};

#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod music_tests;
#[cfg(test)]
mod music_compose_tests;
#[cfg(test)]
mod doodad_bond_tests;
#[cfg(test)]
mod damage_tests;
#[cfg(test)]
mod look_conversion_tests;
#[cfg(test)]
mod ingame_shop_menu_tests;
#[cfg(test)]
mod dominion_tests;
#[cfg(test)]
mod lobby_tests;
use crate::model::{
    ChatMessage, Character, CharacterState, CreateCharacter, Doodad, Item, Movement, Unit, UnitKind,
    actor_flags, move_flags, stance,
};
use crate::net::{self, GameConn, LoginConn, NetError, StreamConn};
use crate::stream;
use crate::wire::Pos;

/// The channels one `SCJoinedChatChannel` / `SCLeavedChatChannel` stands for. The server has one channel
/// per zone group (type shout, sub type the zone group id) and routes shout, trade and party search
/// lines through it (`chat_type::TRADE` / `GROUP_FIND` messages carry the same sub type). The client
/// announces all three when it joins ("Entering Chat: 1.Shout.Two Crowns", "2.Trade.Two Crowns",
/// "3.Party Search.Two Crowns", checked in `reference/ingame/spawn` and `royal_palace`) and so do we.
fn channel_types(chat_type: i16) -> Vec<i16> {
    use crate::model::chat_type as t;
    if chat_type == t::SHOUT { vec![t::SHOUT, t::TRADE, t::GROUP_FIND] } else { vec![chat_type] }
}

/// How long a request may stay unanswered before the session reports a failure.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Default interval of the keep-alive ping.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// Tunables of a session. The defaults are what an app wants; the fields exist for tests.
#[derive(Clone, Debug)]
#[cfg_attr(feature="bevy", derive(bevy::prelude::Resource))]
pub struct ClientOptions {
    /// Interval of the keep-alive `PingPacket`, `None` for no pings at all.
    ///
    /// The server drops a game connection whose last ping is older than 30 s. It checks every 30 s
    /// and treats a connection that never pinged as dead, so a client must ping soon after it
    /// connects and then at least every few seconds.
    pub ping_interval: Option<Duration>,
    /// How long after the game connection opens the first ping goes out (default: at once).
    pub first_ping_delay: Duration,
    /// Complete sparse template->recharge buff catalog for native item task guards.
    /// None preserves unknown equipment-unpack behavior; absent entries in Some mean0.
    pub item_recharge_catalog: Option<std::sync::Arc<std::collections::HashMap<u32,u32>>>,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { ping_interval: Some(PING_INTERVAL), first_ping_delay: Duration::ZERO, item_recharge_catalog:None }
    }
}

/// Where the character's feet are, as the movement packet reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveState {
    /// Standing, walking or running on the ground (actor flag "standing on solid").
    #[default]
    Ground,
    /// Rising after a jump (actor flag "jumping", move flags moving + stopping).
    Jump,
    /// In the air without a jump, or on the way down: no ground under the feet.
    Fall,
}

/// A movement the app wants to send for the local player. All fields are public; build one with
/// [`MoveCommand::new`] (or [`MoveCommand::running`]) and struct update syntax:
/// `MoveCommand { stance: stance::COMBAT, ..MoveCommand::new(pos, yaw) }`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveCommand {
    pub pos: Pos,
    /// Facing in radians (0 faces +y, positive turns left).
    pub yaw: f32,
    /// False for the packet that ends a movement (the "stopping" flag is sent instead of "moving").
    pub moving: bool,
    /// [`stance`] byte.
    pub stance: u8,
    /// Velocity in 1/1024 m/s per axis, in world axes.
    pub velocity: [i16; 3],
    /// What the player is pressing, relative to the facing: `[right, forward, up]`, each in
    /// -127..=127. Walking straight ahead is `[0, 127, 0]`, as the server's own movers send it.
    pub delta_movement: [i8; 3],
    /// Ground, jump or fall.
    pub state: MoveState,
    /// [`alertness`](crate::model::alertness) byte.
    pub alertness: u8,
    /// Sets the "in combat" movement flag.
    pub in_combat: bool,
    /// Fall velocity in wire units ([`FALL_VELOCITY_PER_MPS`](crate::model::FALL_VELOCITY_PER_MPS)
    /// per m/s). Non-zero only in the packet of the landing; the server applies fall damage from
    /// it (nothing below 20 m/s).
    pub fall_vel: u16,
}

impl Default for MoveCommand {
    fn default() -> Self {
        Self {
            pos: Pos::default(),
            yaw: 0.0,
            moving: true,
            stance: stance::RELAXED,
            velocity: [0; 3],
            delta_movement: [0; 3],
            state: MoveState::Ground,
            alertness: 0,
            in_combat: false,
            fall_vel: 0,
        }
    }
}

impl MoveCommand {
    pub fn new(pos: Pos, yaw: f32) -> Self {
        Self { pos, yaw, ..Self::default() }
    }

    /// Moving straight ahead at `speed` m/s: the velocity along the facing and full forward input.
    pub fn running(pos: Pos, yaw: f32, speed: f32) -> Self {
        let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
        Self {
            velocity: [to_wire(-yaw.sin() * speed), to_wire(yaw.cos() * speed), 0],
            delta_movement: [0, 127, 0],
            ..Self::new(pos, yaw)
        }
    }

    /// The packet that ends a movement: standing still, "stopping" flag.
    pub fn stopped(pos: Pos, yaw: f32) -> Self {
        Self { moving: false, ..Self::new(pos, yaw) }
    }

    /// The same movement in the given state.
    pub fn with_state(self, state: MoveState) -> Self {
        Self { state, ..self }
    }

    /// The same movement, landing at `fall_speed` m/s (ground state, fall velocity set).
    pub fn landing(self, fall_speed: f32) -> Self {
        let vel = (fall_speed.max(0.0) * crate::model::FALL_VELOCITY_PER_MPS).min(f32::from(u16::MAX));
        Self { state: MoveState::Ground, fall_vel: vel as u16, ..self }
    }

    /// The wire movement for this command at `time_ms` of the session clock.
    pub fn to_movement(&self, time_ms: u32) -> Movement {
        let mut m = Movement::unit(self.pos, self.yaw, time_ms);
        m.velocity = self.velocity;
        m.delta_movement = self.delta_movement;
        m.stance = self.stance;
        m.alertness = self.alertness;
        m.flags = if self.moving { move_flags::MOVING } else { move_flags::STOPPING };
        m.actor_flags = match self.state {
            MoveState::Ground => actor_flags::STANDING_ON_SOLID,
            MoveState::Jump => actor_flags::JUMPING,
            MoveState::Fall => 0,
        };
        if self.state == MoveState::Jump {
            m.flags = move_flags::JUMPING;
        }
        if self.in_combat {
            m.flags |= move_flags::IN_COMBAT;
        }
        if self.fall_vel != 0 {
            m.actor_flags |= actor_flags::FALL_VELOCITY;
            m.fall_vel = self.fall_vel;
        }
        m
    }
}

/// What the app can ask the session to do.
#[derive(Clone, Debug)]
pub enum Command {
    /// Connect to the login server at `addr` (`host:port`) and authenticate. On success the world
    /// list follows.
    Login { addr: String, user: String, password: Secret },
    /// `CAListWorld`.
    RefreshWorldList,
    /// Ask to enter `world_id`; answered by `WorldQueue`s and then `ReadyToConnect`.
    EnterWorld { world_id: u8 },
    /// Leave the queue of a world.
    CancelEnterWorld,
    /// Connect to the game server (after `ReadyToConnect`) and open the lobby.
    ConnectToWorld,
    /// Request the character list again.
    RefreshCharacters,
    CreateCharacter(CreateCharacter),
    DeleteCharacter { id: u32 },
    CancelDeleteCharacter { id: u32 },
    /// Select the character and enter the world (select, spawn, notify in game).
    EnterGame { character_id: u32 },
    Move(MoveCommand),
    /// The movement of another unit the player controls: the own mate it drives (`CSMoveUnit` with the mate's object id;
    /// the server ignores the rider's own movement while it rides).
    MoveUnit { obj_id: u32, movement: MoveCommand },
    Chat { chat_type: i16, target: String, message: String },
    /// Put `action` into hotbar slot `slot` (0 based). The server does not echo it: the session
    /// reports `ActionSlotChanged` itself.
    SetActionSlot { slot: u8, action: ActionSlot },
    /// Move the item in one slot to another, swap it with the item there or merge two stacks (also wearing and taking
    /// off gear, see [`ItemMove`]). The server answers with `ItemsChanged` (reason 5), or `ServerError` when it refuses.
    SwapItems(ItemMove),
    /// Move `count` of a stack to another slot (reason 6).
    SplitItem { mv: ItemMove, count: i32 },
    /// Throw `count` of the item in `container` / `slot` away (reason 1).
    DestroyItem { item_id: u64, container: u8, slot: u8, count: i32 },
    /// Select a unit (`obj_id`) as the target, 0 clears the target. The server answers with
    /// `TargetChanged` for the player (also to the sender).
    Target { obj_id: u32 },
    /// Sell the equipped trade pack at the specialty NPC `obj_id` (`CSSellBackpackGoods`).
    SellBackpackGoods { obj_id: u32 },
    /// Request explicit Bloodlust state; the native toggle sends the inverse of current state.
    /// A local send is not an acknowledgement and never changes CombatStatus.
    SetForceAttack { on: bool },
    /// Cast `skill_id` (a skill the character knows, `Unit::skills`) at the unit `target_obj_id`
    /// (0 = the player itself, for self buffs and heals). The answer is `SkillStarted`: with a
    /// non zero `result` when the server refuses.
    UseSkill { skill_id: u32, target_obj_id: u32 },
    /// Cast an interaction/gathering skill with target kind4. No local learned-skill restriction;
    /// the server validates the doodad, range, permissions and requirements.
    UseDoodadSkill { skill_id: u32, doodad_obj_id: u32 },
    /// Submit a recipe batch once (`CSExecuteCraft`), without sending an additional skill.
    /// `craft_id` is from the static recipe catalog; count must be positive and the doodad ID
    /// nonzero and 24-bit. The server validates the bench, materials, labor and proficiency.
    /// `CraftRequested` only reports a local send, not batch acceptance/completion. Existing
    /// `StopCasting` cancels an active cast; the lane does not invalidate queued craft tasks.
    ExecuteCraft { craft_id: u32, doodad_obj_id: u32, count: i32 },
    /// Cast a resolved combat skill as a mate. Both the raw mount skill ID and combat skill ID
    /// are explicit; target must be a real nonzero 24-bit unit ID (self buffs name the mate).
    /// No local learned-skill gate or automatic target/slot/auto-use choice is imposed.
    UseMateSkill { skill_id: u32, mate_obj_id: u32, mount_skill_id: u32, target_obj_id: u32 },
    /// Cast `skill_id` at a spot of the ground (area skills).
    UseSkillAt { skill_id: u32, pos: Pos },
    /// Cancel the cast or channel of the player: `tl` is the timeline of a plain skill (`SkillStarted::tl`), `plot_tl` the one
    /// of a plot skill (`PlotEvent::tl`), the other 0. The server answers `CastingStopped` (or `PlotStopped`) if the skill was
    /// still running.
    StopCasting { tl: u16, plot_tl: u16 },
    /// Use a bag item: `item_id` is `Item::id`, `template_id` its template and `skill_id` the
    /// template's `use_skill_id` (`items.use_skill_id` in the game database). `target_obj_id` as
    /// in [`Command::UseSkill`]. The effect arrives as `ItemsChanged` (the stack shrinks), buffs
    /// and `UnitPoints`.
    UseItem { item_id: u64, template_id: u32, skill_id: u32, target_obj_id: u32 },
    /// [`Command::UseItem`] for an item whose skill aims at a spot of the ground (a shovel, a thrown bomb): the position target.
    UseItemAt { item_id: u64, template_id: u32, skill_id: u32, pos: Pos },
    /// Take everything out of the loot of the corpse `obj_id` (`loot_all`) or open it (the answer is `LootBag` with what
    /// is left). Only corpses that came as `Lootable` have loot for the player.
    LootOpen { obj_id: u32, loot_all: bool },
    /// Take one item out of an opened bag: `item_index` and the owner are in the item id ([`state::item_index`],
    /// [`state::item_owner`]). Answered by `LootItemTook` (and `ItemsChanged`) or `LootItemFailed`.
    LootItem { owner_type: u16, owner_obj_id: u32, item_index: u16 },
    /// The loot window of the corpse was closed.
    LootClose { owner_type: u16, owner_obj_id: u32 },
    /// Spend skill points on a skill (`CSLearnSkill`): answered by `SkillLearned`.
    LearnSkill { skill_id: u32 },
    /// Learn a passive buff by its `passive_buffs` id (`CSLearnBuff`): answered by `PassiveBuffLearned`.
    LearnBuff { passive_id: u32 },
    /// Give the skill points of `ability` back (`CSResetSkills`): answered by `SkillsReset`.
    ResetSkills { ability: u8 },
    /// Replace the active ability `old` (11: an empty slot) by `ability` (`CSSwapAbility`): answered by `SkillsReset` for the old one
    /// and `AbilitySwapped`.
    SwapAbility { old: u8, ability: u8 },
    /// Talk to an NPC, accept / hand in / drop a quest, report a talk objective
    /// ([`crate::interaction::Interact`]); the answers are quest events and `Interaction`.
    Interact(crate::interaction::Interact),
    Shop(crate::shop::ShopCommand),
    /// Captured persistent identity is local-only; CS0x11b has an empty body.
    /// Feature admission, single-flight waiting and late-reply ownership are app-owned.
    RequestInGameShopMenu { character_id: u32 },
    Bank(crate::bank::BankCommand),
    /// Request only; getter changes on the received action 18 item task.
    SetAutoUseAaPoint(bool),
    ItemEffect { character_id:u32, request:crate::item_effect::ItemEffectRequest },
    /// Captured native BAG records; catalog/planner and mode gates remain caller-owned.
    ItemLook { character_id: u32, request: crate::look_conversion::ItemLookRequest },
    UnpackItem(crate::unpack::UnpackItemRequest),
    /// Captured persistent character identity is local-only, never part of the wire body.
    Expedition { character_id: u32, command: crate::ExpeditionCommand },
    /// Captured local persistent identity; permissions and dialog context are caller-owned.
    Dominion { character_id: u32, command: crate::DominionCommand },
    /// Captured persistent character identity is local-only, never part of the Family wire body.
    Family { character_id: u32, command: crate::FamilyCommand },
    /// Captured character and received trial context; never an optimistic state change.
    Trial { character_id:u32,command:crate::TrialCommand },
    Bounty { character_id:u32,command:crate::BountyCommand },
    QueryCharacterName { character_id:u32,requested_id:u32 },
    Ranking { character_id:u32,command:crate::RankingCommand },
    /// The portal book and portals: record, rename, open, delete, enter, the visited sub zone ([`crate::portal`]).
    Portal(crate::portal::PortalCommand),
    /// Housing requests (`CS*House*`), see [`crate::housing::HouseCommand`].
    House(crate::housing::HouseCommand),
    /// Plant `item_id` as the doodad template `id` (`CSCreateDoodad`).
    CreateDoodad(crate::farm::CreateDoodad),
    /// Open or close a coffer, move items in and out ([`crate::coffer`]).
    Coffer(crate::coffer::CofferCommand),
    /// Ask for the player's planted public-farm crops (`CSRequestCommonFarmList`).
    RequestCommonFarmList,
    /// App-captured persistent identity; the native CS079 body is empty.
    RequestJuryWaitingNumber { character_id:u32 },
    /// Challenge the player `challenged_id` (a character id) to a duel; the answer is `Event::Duel`.
    ChallengeDuel { challenged_id: u32 },
    /// Accept or refuse the duel challenge of `challenger_id` (the character id `SCDuelChallenged` named).
    AnswerDuel { challenger_id: u32, accept: bool },
    /// Express emotion `emotion_id` (an `express_texts` id), at `target_obj_id` (0: nobody); the server answers every unit in range with
    /// `Event::Emote(EmoteEvent::Expressed)`, the player included.
    ExpressEmotion { emotion_id: u32, target_obj_id: u32 },
    /// A beauty salon request ([`crate::salon`]); the answers are `Event::Salon`.
    Salon(crate::SalonCommand),
    /// A lobby request ([`crate::account`]); the answers are `Event::Account`.
    Account(crate::AccountCommand),
    /// `/roll` and the player channel requests; the answers are `Event::Dice` and the usual channel events.
    ChatCommand(crate::ChatCommand),
    /// Show the appellation (title) `id`, 0 for none; the answer is `Event::Appellation(Changed)` for the own unit.
    ChangeAppellation { id: u32 },
    /// Self-only explicit cancellation; app resolves the native good-buff ordinal/system gate.
    RemoveBuff { character_id:u32, obj_id:u32, buff_index:u32 },
    /// Opaque per-character UI data; save does not have a server acknowledgement.
    UiData(crate::ui_data::UiDataCommand),
    /// Player-score wire requests for this captured active character. Play/pause use skills.
    Music { character_id: u32, command: crate::MusicCommand },
    /// SaveNotes cast only. The app uploads its staged notes on the actual own SkillFired.
    ComposeMusic { character_id: u32, request: crate::MusicComposeRequest },
    /// Party invitations, replies, leaving, kicking and leadership (`crate::party`).
    Party(crate::party::PartyCommand),
    LootRoll(crate::loot_roll::LootRollCommand),
    Battlefield(crate::battlefield::BattlefieldCommand),
    Actability(crate::actability::ActabilityCommand),
    Mate(crate::mate::MateCommand),
    Slave(crate::slave_control::SlaveCommand),
    UnbondDoodad(crate::UnbondDoodadRequest),
    /// Legacy unscoped transport API; new app actions should capture persistent identity below.
    Auction(crate::auction::AuctionCommand),
    AuctionForCharacter { character_id:u32,command:crate::auction::AuctionCommand },
    Trade(crate::trade::TradeCommand),
    Mail(crate::mail::MailCommand),
    Friend(crate::friend::FriendCommand),
    Repair(crate::repair::RepairCommand),
    /// Stand up after dying: `in_place` where the character fell, else at the return point (a
    /// shrine). Answered by `Resurrected` and `UnitPoints`.
    Resurrect { in_place: bool },
    /// Leave the game: `Quit` logs out, `CharacterSelect` returns to the character list.
    LeaveWorld(LeaveTarget),
    CancelLeave,
    /// From the character list, back to the world list.
    ReturnToWorldSelect,
    /// Close every connection.
    Disconnect,
}

/// Coarse state of the session, for UIs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Logging in.
    Authenticating,
    /// Logged in, looking at the world list.
    WorldSelect,
    /// Waiting in a world's queue.
    Queued,
    /// The login server gave us a game server address; call `ConnectToWorld`.
    ReadyToConnect,
    /// Connecting to the game server and running the CryNetwork state handshake.
    Connecting,
    /// Connected to a world's lobby (character list).
    CharacterSelect,
    /// A character was selected and is loading.
    EnteringWorld,
    InWorld,
    /// The logout countdown is running.
    Leaving,
}

/// The local player once it is in the world.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerSpawn {
    /// Object id of the player (used in movement packets).
    pub obj_id: u32,
    /// Everything the server sent when the character was selected.
    pub state: CharacterState,
    /// The player's own unit as announced when spawning (position, model, equipment).
    pub unit: Unit,
}

impl PlayerSpawn {
    pub fn character(&self) -> &Character {
        &self.state.character
    }

    pub fn name(&self) -> &str {
        &self.unit.name
    }

    pub fn position(&self) -> Pos {
        self.unit.pos
    }

    pub fn zone_id(&self) -> u32 {
        self.state.character.zone_id
    }

    /// Facing at spawn, radians.
    pub fn yaw(&self) -> f32 {
        self.state.rotation[2]
    }
}

/// What the session tells the app.
#[derive(Clone, Debug)]
pub enum Event {
    PhaseChanged(Phase),
    LoginSucceeded(AccountInfo),
    /// Login was denied or the server is unreachable.
    LoginFailed { message: String },
    WorldList(WorldList),
    WorldQueue(WorldQueue),
    EnterWorldDenied { reason: u8 },
    EnterWorldCancelled,
    /// The login server accepted us into `world_id`; send `ConnectToWorld`.
    ReadyToConnect { world_id: u8, cookie: WorldCookie },
    /// Connecting to the game server failed.
    ConnectFailed { message: String },
    /// Genuine proxy 0x13 receipt; no clock mapping or phase advancement is inferred.
    Pong(crate::proxy::ProxyPong),
    ClockSample(crate::clock::ClockSample),
    /// The game connection is established and in the lobby.
    LobbyReady,
    /// `SCGetSlotCount`: extra slots for this account/world, not the total character limit.
    CharacterSlotCount(u8),
    /// Exact `SCAccountInfo` receipt; no payment policy is inferred.
    AccountPayment(crate::account::AccountPayment),
    /// Exact `SCUpdatePremiumPoint` receipt, including the initial handshake answer.
    /// No character identity is present; this never rewrites a character-list entry.
    PremiumPoints(crate::account::PremiumPoints),
    /// Congestion per race (index = race id) for character creation.
    RaceCongestion([u8; 9]),
    CharacterList(Vec<Character>),
    CharacterCreated(Box<Character>),
    CharacterCreateFailed { reason: u8 },
    CharacterDeleteResponse { id: u32, status: u8, delete_request_time: i64, delete_time: i64 },
    /// Answer to `CancelDeleteCharacter`: `restored` is true when the pending delete was cancelled
    /// (status 3); status 4 means the server does not know the character.
    CharacterDeleteCancelled { character_id: u32, restored: bool },
    CharacterDeleted { id: u32, name: String },
    /// The server accepted `EnterGame`; the character is loading.
    CharacterSelected(Box<CharacterState>),
    /// The local player exists in the world.
    PlayerSpawn(Box<PlayerSpawn>),
    /// The enter-world sequence is complete; units and doodads around start to arrive.
    EnteredWorld,
    UnitSpawned(Box<Unit>),
    /// A unit moved: one `SCOneUnitMovement`, or an entry of a batched `SCUnitMovements`.
    UnitMoved { obj_id: u32, movement: Movement },
    UnitsRemoved(Vec<u32>),
    /// `obj_id` sits on `mount` (a mate, a vehicle, a ship) at attach point `point`; the server moves it with the mount.
    UnitAttached { obj_id: u32, mount: u32, point: u8, reason: u8 },
    /// `obj_id` got off its mount and stands where the mount is.
    UnitDetached { obj_id: u32, reason: u8 },
    /// An NPC took (`activate`) or left an idle pose: `anim_action` is an `anim_actions.id`, `kind` the `ModelPostureType`.
    UnitPosture { obj_id: u32, kind: u8, anim_action: u32, activate: bool },
    UnitPoints { obj_id: u32, hp: i32, mp: i32 },
    EnvDamage(crate::combat_status::EnvironmentDamage),
    SetBreath { breath: u32 },
    /// Received local dive/surface report. No Lua event or breathing timer is synthesized.
    UnderWater { start: bool },
    /// World-level report; raw state and timestamp bits, independent of own spawn.
    ConflictZoneState(crate::conflict::ConflictZoneState),
    ForceAttackSet { obj_id: u32, on: bool },
    /// Successful local socket write only; wait for ForceAttackSet to update received state.
    ForceAttackRequested { on: bool },
    ForceAttackRejected { on: bool, message: String },
    UnitInvisible { obj_id: u32, invisible: bool },
    UnitDeath { obj_id: u32 },
    /// The details of a death (follows [`Event::UnitDeath`] for the same packet).
    DeathDetails {
        obj_id: u32,
        /// `KillReason`.
        kill_reason: u8,
        /// How long until the unit can resurrect, ms.
        resurrect_wait_ms: i32,
        lost_exp: i32,
        /// Object id and name of the killer, when a unit did it.
        killer: Option<(u32, String)>,
    },
    /// `SCNotifyResurrection`: an in-place resurrection is offered. A free low-level offer
    /// has unit caster 0 and can arrive before the death packet; preserve it until dead.
    ResurrectionOffered { caster: BuffCaster },
    /// A character stands up again at `pos`, facing `yaw`.
    Resurrected { obj_id: u32, pos: Pos, yaw: f32 },
    /// The unit or doodad has loot for the player (`has_loot`) or no longer has ([`state::Loot`]).
    Lootable { owner_type: u16, obj_id: u32, has_loot: bool },
    /// A unit started (`looting` 1) or stopped looting.
    UnitLooting { obj_id: u32, looting: u8 },
    /// The items left in the loot the player opened; `loot_all`: it was opened with "loot all", the rest is what could not be taken.
    LootBag { items: Vec<crate::model::Item>, loot_all: bool },
    /// The player took an item out of a corpse.
    LootItemTook { template_id: u32, item_index: u16, owner_type: u16, owner_obj_id: u32, count: i32 },
    /// The server refused an item (`error`: an `ErrorMessageType`, bag full ...).
    LootItemFailed { error: i32, item_index: u16, owner_type: u16, owner_obj_id: u32, item_template_id: u32 },
    /// `delta` experience gained (negative: lost) by the unit; the total is in
    /// `state::PlayerStats::exp`.
    ExpChanged { obj_id: u32, delta: i32, add_ability_exp: bool },
    /// An ability's experience is now `exp` (a total).
    AbilityExpChanged { obj_id: u32, ability: u8, exp: i32 },
    RecoverableExp { obj_id: u32, recoverable: i32, penaltied: i32, reason: i32 },
    LevelChanged { obj_id: u32, level: u8 },
    AbilitySwapped { obj_id: u32, old_ability: u8, ability: u8 },
    /// Labor points added (`amount` > 0: "Gained N Labor Point(s)") or spent (`amount` < 0) for
    /// `action` (an actability id, 0 for none), which gained `point` actability points.
    LaborChanged { amount: i32, action: i32, point: i32, step: u8 },
    /// Honor and vocation (living) points, sent when the character is selected.
    GamePoints { honor: i32, vocation: i32, other: [i32; 8] },
    /// `kind` 0 honor, 1 vocation: `amount` was added.
    GamePointChanged { kind: u8, amount: i32 },
    /// The loyalty token balance.
    LoyaltyChanged { bm_point: i64 },
    /// The cash shop balance (`SCICSCashPoint`, sent after the login and after a purchase).
    CashPoint { point: i32 },
    /// A unit entered or left combat.
    CombatChanged { obj_id: u32, engaged: bool },
    /// `hitter` struck `victim` first: the fight starts (`hit_type` as in [`UnitDamage`]).
    CombatFirstHit { victim: u32, hitter: u32, hit_type: u32 },
    /// The hostile units on an NPC's aggro list; empty when the NPC left combat.
    AiAggro { npc: u32, hostiles: Vec<AggroEntry> },
    /// An NPC switched its target (`target` 0: it went back to idle).
    AggroTargetChanged { npc: u32, target: u32 },
    UnitNameChanged { obj_id: u32, name: String },
    UnitFactionChanged { obj_id: u32, name: String, faction_id: u32, faction_id2: u32, temporary: bool },
    /// The game clock: `hour` 0..24, `speed` in game hours per real second when the server sent it
    /// (the first packet after spawning; later ones only move `hour`).
    TimeOfDay { hour: f32, speed: Option<f32> },
    /// Snow switched on or off.
    Snow(bool),
    /// A chat channel was joined: `chat_type` (see `model::chat_type`), `sub_type` (the zone group
    /// id for zone channels), `faction_id` and the channel's name (empty for zone channels). The
    /// server's zone channel is reported as three: shout, trade and party search (as the client does).
    ChatChannelJoined { chat_type: i16, sub_type: i16, faction_id: u32, name: String },
    ChatChannelLeft { chat_type: i16, sub_type: i16, faction_id: u32 },
    /// A unit selected (or dropped, `target_obj_id` 0) a target. For the local player this is the
    /// server's answer to [`Command::Target`].
    TargetChanged { obj_id: u32, target_obj_id: u32 },
    /// The whole hotbar, 85 slots, sent when the character is selected.
    ActionSlots(Vec<ActionSlot>),
    /// One hotbar slot changed by [`Command::SetActionSlot`].
    ActionSlotChanged { slot: u8, action: ActionSlot },
    /// Bag and bank sizes; their contents follow as [`Event::InventoryChunk`]s.
    InventoryInit { bag_slots: u32, bank_slots: u32 },
    /// Ten consecutive slots of the bag or bank (see `items::container`).
    InventoryChunk(InvenChunk),
    /// Items were added, removed, moved or changed (loot, shop, use ...), or money changed.
    ItemsChanged(ItemTasks),
    /// Equipment slots of a unit changed; `None` = emptied.
    EquipmentChanged { obj_id: u32, items: Vec<(u8, Option<Item>)> },
    /// Native SC095 changes persistent IDs only, preserving other record fields.
    EquipmentIdsChanged { obj_id: u32, ids: [u64; 28] },
    /// Raw native specialty sale ratio receipt; zero is an error/reset path.
    SpecialtyRatio(i32),
    /// A container grew to `slots` slots.
    InventoryExpanded { container: u8, slots: u8 },
    /// A unit began casting `skill_id` (or, with a non zero `result` and `tl` 0, the server
    /// refused a skill the player asked for).
    SkillStarted(SkillStarted),
    /// The cast completed: the effects follow.
    SkillFired(SkillFired),
    /// The cast `tl` is over.
    SkillEnded { tl: u16 },
    /// The unit's skill was interrupted.
    SkillStopped { obj_id: u32, skill_id: u32 },
    /// The cast `tl` was cancelled.
    CastingStopped { tl: u16, duration_ms: u32 },
    /// Received skill/tag cooldown lists for the current player; empty is not a reset.
    Cooldowns(crate::skill::SkillCooldowns),
    /// The cooldown of `skill_id` / `tag_id` and optionally the global cooldown was reset.
    SkillCooldownReset { obj_id: u32, skill_id: u32, tag_id: u32, gcd: bool },
    /// The player's active quests, listed when the character spawns.
    QuestsLoaded(Vec<Quest>),
    /// The completed quests as blocks of 64 flags (see [`CompletedBlock`]).
    CompletedQuests(Vec<CompletedBlock>),
    QuestStarted(Quest),
    /// A quest moved to another step or status (a dropped quest comes as status `DROPPED`).
    QuestUpdated(Quest),
    /// Quest `quest_id` was handed in; `block` is its updated block of completed flags.
    QuestCompleted { quest_id: u32, block: CompletedBlock },
    /// Quest `quest_id` can be taken again.
    QuestReset { quest_id: u32, block: CompletedBlock },
    /// An NPC's function list, a doodad's quest offer, NPC chat ([`crate::interaction::InteractEvent`]).
    Interaction(crate::interaction::InteractEvent),
    Shop(crate::shop::ShopEvent),
    Party(crate::party::PartyEvent),
    LootRoll(crate::loot_roll::LootRollEvent),
    Battlefield(crate::battlefield::BattlefieldEvent),
    ScheduledEventStarted(crate::scheduled::ScheduledEventStarted),
    Dominion(crate::DominionEvent),
    /// Actual decoder metadata, including compressed batches whose remaining tail was abandoned.
    UnhandledGamePacket { level:u8, opcode:u16, len:usize },
    /// Actual wire decode failure; no complete event can be reconstructed from this receipt.
    GameDecodeFailed { message:String },
    Trial(crate::trial::TrialEvent),
    Crime(crate::crime::CrimeEvent),
    Appellation(crate::appellation::AppellationEvent),
    /// `CSChangeAppellation` was sent (`Command::ChangeAppellation`).
    AppellationRequested { id: u32 },
    Portal(crate::portal::PortalEvent),
    /// A portal request went out (the book drops a deleted entry on [`Event::PortalDeleteSent`]).
    PortalRequested(crate::portal::PortalCommand),
    /// `CSDeletePortal` was sent: the lane answers nothing, the entry is gone from the book.
    PortalDeleteSent { portal_type: u8, portal_id: u32 },
    PortalRequestRejected { command: crate::portal::PortalCommand, message: String },
    Duel(crate::duel::DuelEvent),
    /// A duel request was sent (`Command::ChallengeDuel` / `Command::AnswerDuel`).
    DuelRequested { challenged_id: u32 },
    DuelAnswered { challenger_id: u32, accept: bool },
    DuelRequestRejected { message: String },
    /// An emote or a speech bubble ([`crate::emote`]).
    Emote(crate::emote::EmoteEvent),
    /// The beauty salon toggled, or a character got a new look ([`crate::salon`]).
    Salon(crate::SalonEvent),
    Notice(crate::NoticeEvent),
    /// Local successful socket write only; no native save/play success receipt exists.
    MusicRequested { character_id: u32, command: crate::MusicCommand },
    MusicRequestRejected { character_id: u32, command: crate::MusicCommand, message: String },
    /// CS052 was written locally. This is neither a skill receipt nor a saved-score ACK.
    MusicComposeRequested { character_id: u32, request: crate::MusicComposeRequest },
    MusicComposeRejected { character_id: u32, request: crate::MusicComposeRequest, message: String },
    UnitFx(crate::UnitFxEvent),
    Account(crate::AccountEvent),
    CashShop(crate::CashShopEvent),
    /// Local successful socket write only; not feature admission or a received menu.
    InGameShopMenuRequested { character_id: u32 },
    InGameShopMenuRequestRejected { character_id: u32, message: String },
    ChatSpamDelay(crate::ChatSpamDelay),
    /// Received proxy level descriptor; it does not acknowledge a loaded world.
    GameType(crate::proxy::GameType),
    /// Someone in range rolled the dice.
    Dice(crate::DiceRolled),
    Bounty(crate::BountyEvent),
    Ranking(crate::RankingEvent),
    RankingRequested { character_id:u32,command:crate::RankingCommand },
    RankingRequestRejected { character_id:u32,command:crate::RankingCommand,message:String },
    CharacterName(crate::CharacterName),
    CharacterNameRequested { character_id:u32,requested_id:u32 },
    CharacterNameRequestRejected { character_id:u32,requested_id:u32,message:String },
    CharacterNameStreamClosed,
    /// Local successful CS072 transport write, never an authoritative jury/seat response.
    JurySummonReplyRequested { character_id:u32,trial_id:u32,court:u32,jury_number:i32 },
    JurySummonReplyRejected { character_id:u32,trial_id:u32,court:u32,jury_number:i32,message:String },
    TrialRequested { character_id:u32,command:crate::TrialCommand },
    TrialRequestRejected { character_id:u32,command:crate::TrialCommand,message:String },
    BountyRequested { character_id:u32,command:crate::BountyCommand },
    BountyRequestRejected { character_id:u32,command:crate::BountyCommand,message:String },
    BuffRemovalRequested { character_id:u32, obj_id:u32, buff_index:u32 },
    BuffRemovalRequestRejected { character_id:u32, obj_id:u32, buff_index:u32, message:String },
    JuryWaitingNumberRequested { character_id:u32 },
    JuryWaitingNumberRequestRejected { character_id:u32, message:String },
    Achievement(crate::achievement::AchievementEvent),
    Expedition(crate::expedition::ExpeditionEvent),
    Family(crate::family::FamilyEvent),
    UnitGmModeChanged(crate::unit_msg::UnitGmModeChanged),
    UnitPvpPointsChanged(crate::unit_msg::UnitPvpPointsChanged),
    ExpeditionRequested { character_id: u32, command: crate::ExpeditionCommand },
    /// Local socket write only, never a server acknowledgement or received state mutation.
    DominionRequested { character_id: u32, command: crate::DominionCommand },
    DominionRequestRejected { character_id: u32, command: crate::DominionCommand, message: String },
    ExpeditionRequestRejected { character_id: u32, command: crate::ExpeditionCommand, message: String },
    FamilyRequested { character_id: u32, command: crate::FamilyCommand },
    FamilyRequestRejected { character_id: u32, command: crate::FamilyCommand, message: String },
    UnpackItemRequested(crate::unpack::UnpackItemRequest),
    UnpackItemRequestRejected { request:crate::unpack::UnpackItemRequest,message:String },
    Actability(crate::actability::ActabilityEvent),
    CraftFailureNotice(crate::craft::CraftFailureNotice),
    InitialConfig(crate::initial_config::InitialConfig),
    CraftItemUnlocked(crate::craft::CraftItemUnlock),
    /// Raw loading directive; the app must plan the load/response separately.
    LoadInstance(crate::world_msg::InstanceLoad),
    /// Raw blink directive; does not by itself change a unit transform.
    BlinkUnit(crate::world_msg::BlinkUnit),
    /// Raw item timer metadata; does not change inventory contents or guess an expiry.
    ItemLifespan(crate::items::ItemLifespan),
    ItemDetailUpdated(crate::items::ItemDetailUpdate),
    ItemOperationError(crate::items::ItemOperationError),
    Gimmick(crate::gimmick::GimmickEvent),
    Housing(crate::housing::HousingEvent),
    Farm(crate::farm::FarmEvent),
    Coffer(crate::coffer::CofferEvent),
    Radar(crate::radar::RadarEvent),
    Mate(crate::mate::MateEvent),
    Slave(crate::slave::SlaveEvent),
    /// Genuine fixed transport receipt only; no prediction or state fold is implied.
    Transport(crate::TransportReceipt),
    DoodadBond(crate::DoodadBondEvent),
    /// Full repeated own SCUnitState receipt, without player respawn or UI/transform reset.
    OwnUnitSnapshot(Box<Unit>),
    Auction(crate::auction::AuctionEvent),
    Trade(crate::trade::TradeEvent),
    Mail(crate::mail::MailEvent),
    Friend(crate::friend::FriendEvent),
    Repair(crate::repair::RepairEvent),
    Bank(crate::bank::BankEvent),
    AutoUseAaPointRequested(bool),
    AutoUseAaPointRequestRejected { value: bool, message: String },
    UiData(crate::ui_data::UiDataEvent),
    SkillLearned { skill_id: u32, level: u8 },
    SkillUpgraded { skill_id: u32, level: u8 },
    /// The skills of `ability` were forgotten.
    SkillsReset { obj_id: u32, ability: u8 },
    /// A hit: damage numbers for the combat text.
    UnitDamaged(UnitDamage),
    UnitHealed(UnitHeal),
    CombatText { source: u32, target: u32, text_type: u8 },
    /// A step of a plot skill (`casting_ms` > 0: the next step is cast).
    PlotEvent(PlotEvent),
    PlotEnded { tl: u16 },
    PlotStopped { tl: u16, duration_ms: u32, last_event: u8, channeling: bool },
    /// A buff or debuff started on a unit (or was refreshed: same `index`).
    BuffCreated { obj_id: u32, buff: Buff },
    BuffRemoved { obj_id: u32, index: u32 },
    BuffUpdated { obj_id:u32, index:u32, stacks:i32, charged:i32 },
    /// Received result only; does not grant, consume or modify any item.
    ScheduleItems { declared_count:u8, items:Vec<crate::schedule::ScheduleItem> },
    ScheduleItemSent { schedule_type:u32, raw_by_mail:u8 },
    ItemUccDataChanged { ucc_id:u64, player_id:u32, target_item_id:u64 },
    ItemEffectRequested { character_id:u32, request:crate::item_effect::ItemEffectRequest },
    ItemEffectRejected { character_id:u32, request:crate::item_effect::ItemEffectRequest, message:String },
    /// Local send only. Received item/skill events determine subsequent state.
    ItemLookRequested { character_id: u32, request: crate::look_conversion::ItemLookRequest },
    ItemLookRejected { character_id: u32, request: crate::look_conversion::ItemLookRequest, message: String },
    GradeEnchantResult(crate::item_effect::GradeEnchantResult),
    GradeEnchantBroadcast(crate::item_effect::GradeEnchantBroadcast),
    LunagemSocketingResult { raw_result:u8, item_id:u64, template_id:u32, raw_install:u8 },
    LunastoneSocketingResult { raw_result:u8, item_id:u64, template_id:u32 },
    /// A passive buff was learned.
    PassiveBuffLearned { obj_id: u32, buff_id: u32 },
    DoodadsSpawned(Vec<Doodad>),
    DoodadsRemoved(Vec<u32>),
    DoodadChanged { obj_id: u32, data: i32 },
    /// Owner is a stored character ID, not an object ID. Faction is preserved as raw wire data.
    DoodadOriginator { obj_id: u32, new_owner_id: u32, new_faction: u32 },
    DoodadSound { obj_id: u32, sound_id: u32 },
    VegetationCutdown { unit_obj_id: u32, doodad_obj_id: u32 },
    DoodadPhaseChanged { obj_id: u32, func_group_id: u32, time_left: u32, puzzle_group: i32, item_template_id: u32 },
    /// Local command rejection, distinct from an authoritative SkillStarted failure.
    DoodadSkillRejected { skill_id: u32, doodad_obj_id: u32, message: String },
    /// Local successful send only. The lane has no authoritative batch-completion ACK.
    CraftRequested { craft_id: u32, doodad_obj_id: u32, count: i32 },
    /// Local validation failure, not a server crafting result.
    CraftRejected { craft_id: u32, doodad_obj_id: u32, count: i32, message: String },
    /// The server moved the local player (answered automatically).
    Teleported { pos: Pos, yaw: f32, reason: u8 },
    Chat(ChatMessage),
    ServerError { code: i16 },
    Kicked { reason: u8, message: String },
    /// The logout countdown started.
    LeavePending { time_ms: i32, target: u8 },
    LeaveCancelled,
    /// The server let us leave the world (`target` as in [`LeaveTarget`]).
    LeftWorld { target: u8 },
    /// A request got no answer in time (the session keeps running).
    RequestTimedOut { what: String },
    /// A connection was lost or closed; the session is back to idle.
    Disconnected { message: String },
}

/// Handle to a running session. Dropping it ends the session and closes the connections.
pub struct Client {
    commands: UnboundedSender<Command>,
    events: UnboundedReceiver<Event>,
}

impl Client {
    /// Starts the session task on an existing tokio runtime.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn_on(handle: &tokio::runtime::Handle) -> Self {
        Self::spawn_on_with(handle, ClientOptions::default())
    }

    /// [`Client::spawn_on`] with explicit [`ClientOptions`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn_on_with(handle: &tokio::runtime::Handle, options: ClientOptions) -> Self {
        let (commands, cmd_rx) = unbounded_channel();
        let (evt_tx, events) = unbounded_channel();
        handle.spawn(Session::new(evt_tx, options).run(cmd_rx));
        Self { commands, events }
    }

    /// Starts the session task on its own thread with a private single-threaded runtime. This is
    /// what apps without tokio (Bevy) use.
    pub fn spawn_thread() -> std::io::Result<Self> {
        Self::spawn_thread_with(ClientOptions::default())
    }

    /// [`Client::spawn_thread`] with explicit [`ClientOptions`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn spawn_thread_with(options: ClientOptions) -> std::io::Result<Self> {
        let (commands, cmd_rx) = unbounded_channel();
        let (evt_tx, events) = unbounded_channel();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("aa-net".into())
            .spawn(move || rt.block_on(Session::new(evt_tx, options).run(cmd_rx)))?;
        Ok(Self { commands, events })
    }

    /// Browser: there are no threads, the session runs as a task on the page's event loop.
    #[cfg(target_arch = "wasm32")]
    pub fn spawn_thread_with(options: ClientOptions) -> std::io::Result<Self> {
        let (commands, cmd_rx) = unbounded_channel();
        let (evt_tx, events) = unbounded_channel();
        wasm_bindgen_futures::spawn_local(Session::new(evt_tx, options).run(cmd_rx));
        Ok(Self { commands, events })
    }

    /// Queues a command. Returns false when the session has ended.
    pub fn send(&self, cmd: Command) -> bool {
        self.commands.send(cmd).is_ok()
    }

    /// The next event if one is ready.
    pub fn try_recv(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    /// Waits for the next event; `None` when the session has ended.
    pub async fn recv(&mut self) -> Option<Event> {
        self.events.recv().await
    }

    /// A clonable sender for commands, for code that does not own the client.
    pub fn sender(&self) -> UnboundedSender<Command> {
        self.commands.clone()
    }
}

struct Session {
    evt: UnboundedSender<Event>,
    options: ClientOptions,
    /// When the next keep-alive ping is due (set when the game connection opens).
    next_ping: Option<Instant>,
    started: Instant,
    phase: Phase,
    login: Option<LoginConn>,
    game: Option<GameConn>,
    stream: Option<StreamConn>,
    stream_joined:bool,
    pending_names:std::collections::BTreeSet<u32>,
    login_addr: String,
    afs: Option<u64>,
    account: Option<AccountInfo>,
    /// World being entered and the game server cookie once granted.
    pending_world: u8,
    cookie: Option<(u8, WorldCookie)>,
    stream_token: u32,
    stream_port: u16,
    chars_buf: Vec<Character>,
    selected: u32,
    selected_state: Option<CharacterState>,
    self_obj: u32,
    battlefield: std::cell::RefCell<crate::battlefield::Battlefield>,
    server_clock: std::cell::RefCell<crate::clock::ServerClock>,
    received_inventory: std::cell::RefCell<crate::state::Inventory>,
    received_native_records: std::cell::RefCell<crate::NativeItemRecords>,
    received_trials: std::cell::RefCell<crate::Trials>,
    received_rankings:std::cell::RefCell<crate::Rankings>,
    received_doodad_bonds:std::cell::RefCell<crate::DoodadBonds>,
    /// What we wait for and until when.
    waiting: Option<(&'static str, Instant)>,
}

async fn recv_login(c: &mut Option<LoginConn>) -> net::Result<login::ServerMsg> {
    match c {
        Some(c) => c.recv().await,
        None => std::future::pending().await,
    }
}

async fn recv_game(c: &mut Option<GameConn>) -> net::Result<game::ServerMsg> {
    match c {
        Some(c) => c.recv().await,
        None => std::future::pending().await,
    }
}

async fn recv_stream(c: &mut Option<StreamConn>) -> net::Result<stream::ServerMsg> {
    match c {
        Some(c) => c.recv().await,
        None => std::future::pending().await,
    }
}

impl Session {
    fn new(evt: UnboundedSender<Event>, options: ClientOptions) -> Self {
        let mut native_records=crate::NativeItemRecords::default();
        if let Some(catalog)=&options.item_recharge_catalog {native_records.set_recharge_catalog(catalog.clone());}
        Self {
            evt,
            options,
            next_ping: None,
            started: Instant::now(),
            phase: Phase::Idle,
            login: None,
            game: None,
            stream: None,
            stream_joined:false,
            pending_names:Default::default(),
            login_addr: String::new(),
            afs: None,
            account: None,
            pending_world: 0,
            cookie: None,
            stream_token: 0,
            stream_port: 0,
            chars_buf: Vec::new(),
            selected: 0,
            selected_state: None,
            self_obj: 0,
            battlefield: Default::default(),
            server_clock: Default::default(),
            received_inventory: Default::default(),
            received_native_records: std::cell::RefCell::new(native_records),
            received_trials: Default::default(),
            received_rankings:Default::default(),
            received_doodad_bonds:Default::default(),
            waiting: None,
        }
    }

    async fn run(mut self, mut cmd_rx: UnboundedReceiver<Command>) {
        let mut tick = crate::timer::Ticker::new(Duration::from_millis(500));
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    self.command(cmd).await;
                }
                r = recv_login(&mut self.login) => self.login_msg(r).await,
                r = recv_game(&mut self.game) => self.game_msg(r).await,
                r = recv_stream(&mut self.stream) => self.stream_msg(r),
                _ = tick.tick() => {
                    self.check_timeout();
                    self.keep_alive().await;
                }
            }
        }
        self.close_all().await;
    }

    // ---- helpers ------------------------------------------------------------------------------

    /// Sends the keep-alive ping when it is due. The server closes a game connection without a
    /// ping in the last 30 s, and a connection that never pinged at its next sweep (every 30 s), so
    /// the first ping goes out as soon as the game connection opens.
    async fn keep_alive(&mut self) {
        let (Some(due), Some(every)) = (self.next_ping, self.options.ping_interval) else { return };
        if self.game.is_none() || Instant::now() < due {
            return;
        }
        self.next_ping = Some(Instant::now() + every);
        let time_ms = self.started.elapsed().as_millis() as i64;
        self.send_game(game::ClientMsg::Ping { time_ms }).await;
    }

    fn advance_battlefield_clock(&self) {
        if let Some(now)=self.server_clock.borrow().mapped_now() {self.battlefield.borrow_mut().advance_server_time(now);}
    }
    fn emit(&self, e: Event) {
        self.received_native_records.borrow_mut().apply(&e);

        if matches!(e,Event::Kicked{..}) {*self.received_inventory.borrow_mut()=Default::default();}
        self.received_inventory.borrow_mut().apply(&e);
        self.received_trials.borrow_mut().apply(&e);
        self.received_rankings.borrow_mut().apply(&e);
        self.received_doodad_bonds.borrow_mut().apply(&e);
        self.server_clock.borrow_mut().apply(&e);
        self.advance_battlefield_clock();
        self.battlefield.borrow_mut().apply_with_time(&e,self.server_clock.borrow().mapped_now());
        let _ = self.evt.send(e);
    }

    fn set_phase(&mut self, phase: Phase) {
        if self.phase != phase {
            self.phase = phase;
            self.emit(Event::PhaseChanged(phase));
        }
    }

    fn wait_for(&mut self, what: &'static str) {
        self.waiting = Some((what, Instant::now() + REQUEST_TIMEOUT));
    }

    fn check_timeout(&mut self) {
        let Some((what, deadline)) = self.waiting else { return };
        if Instant::now() < deadline {
            return;
        }
        self.waiting = None;
        let message = format!("timed out waiting for {what}");
        match self.phase {
            Phase::Authenticating => {
                self.login = None;
                self.set_phase(Phase::Idle);
                self.emit(Event::LoginFailed { message });
            }
            Phase::Connecting => {
                self.game = None;
                self.clear_character_name_stream();self.stream = None;
                self.set_phase(Phase::ReadyToConnect);
                self.emit(Event::ConnectFailed { message });
            }
            Phase::EnteringWorld => {
                self.set_phase(Phase::CharacterSelect);
                self.emit(Event::RequestTimedOut { what: what.to_string() });
            }
            _ => self.emit(Event::RequestTimedOut { what: what.to_string() }),
        }
    }

    async fn send_login(&mut self, msg: login::ClientMsg) -> bool {
        let Some(c) = &mut self.login else { return false };
        match c.send(&msg).await {
            Ok(()) => true,
            Err(e) => {
                self.login_lost(e);
                false
            }
        }
    }

    async fn send_game(&mut self, msg: game::ClientMsg) -> bool {
        let Some(c) = &mut self.game else { return false };
        match c.send(&msg).await {
            Ok(()) => true,
            Err(e) => {
                self.game_lost(e);
                false
            }
        }
    }

    fn clear_character_name_stream(&mut self) {
        let existed=self.stream.is_some() || self.stream_joined || !self.pending_names.is_empty();
        self.stream_joined=false;self.pending_names.clear();
        if existed {self.emit(Event::CharacterNameStreamClosed);}
    }

    async fn close_all(&mut self) {
        if let Some(mut c) = self.login.take() {
            c.shutdown().await;
        }
        if let Some(mut c) = self.game.take() {
            c.shutdown().await;
        }
        self.clear_character_name_stream();
        if let Some(mut c) = self.stream.take() {
            c.shutdown().await;
        }
        self.self_obj = 0;
        self.selected = 0;
        self.selected_state = None;
        self.chars_buf.clear();
        self.next_ping = None;
    }

    fn login_lost(&mut self, e: NetError) {
        self.login = None;
        match self.phase {
            Phase::Authenticating => {
                self.set_phase(Phase::Idle);
                self.emit(Event::LoginFailed { message: e.to_string() });
            }
            Phase::WorldSelect | Phase::Queued | Phase::ReadyToConnect => {
                self.set_phase(Phase::Idle);
                self.emit(Event::Disconnected { message: format!("login server: {e}") });
            }
            // The login connection is idle while we play; losing it changes nothing.
            _ => tracing::debug!("login connection closed: {e}"),
        }
    }

    fn game_lost(&mut self, e: NetError) {
        self.game = None;
        self.clear_character_name_stream();self.stream = None;
        self.waiting = None;
        self.self_obj = 0;
        self.selected = 0;
        self.selected_state = None;
        self.chars_buf.clear();
        self.next_ping = None;
        let message = format!("game server: {e}");
        if self.phase == Phase::Connecting {
            self.set_phase(Phase::ReadyToConnect);
            self.emit(Event::ConnectFailed { message });
        } else {
            self.set_phase(Phase::Idle);
            self.emit(Event::Disconnected { message });
        }
    }

    // ---- commands -----------------------------------------------------------------------------

    async fn command(&mut self, cmd: Command) {
        self.advance_battlefield_clock();
        tracing::debug!("command {cmd:?}");
        match cmd {
            Command::Login { addr, user, password } => self.start_login(addr, user, password).await,
            Command::RefreshWorldList => {
                self.send_login(login::ClientMsg::ListWorlds).await;
            }
            Command::EnterWorld { world_id } => {
                self.pending_world = world_id;
                if self.send_login(login::ClientMsg::EnterWorld { world_id }).await {
                    self.set_phase(Phase::Queued);
                }
            }
            Command::CancelEnterWorld => {
                let world_id = self.pending_world;
                if self.send_login(login::ClientMsg::CancelEnterWorld { world_id }).await {
                    self.cookie = None;
                    self.set_phase(Phase::WorldSelect);
                    self.emit(Event::EnterWorldCancelled);
                }
            }
            Command::ConnectToWorld => self.connect_world().await,
            Command::RefreshCharacters => {
                self.chars_buf.clear();
                self.wait_for("the character list");
                self.send_game(game::ClientMsg::ListCharacters).await;
            }
            Command::CreateCharacter(req) => {
                self.send_game(game::ClientMsg::CreateCharacter(req)).await;
            }
            Command::DeleteCharacter { id } => {
                self.send_game(game::ClientMsg::DeleteCharacter { id }).await;
            }
            Command::CancelDeleteCharacter { id } => {
                self.send_game(game::ClientMsg::CancelDeleteCharacter { id }).await;
            }
            Command::EnterGame { character_id } => {
                self.selected = character_id;
                self.selected_state = None;
                self.self_obj = 0;
                self.set_phase(Phase::EnteringWorld);
                self.wait_for("the character state");
                self.send_game(game::ClientMsg::SelectCharacter { id: character_id }).await;
            }
            Command::Move(m) => self.send_move(self.self_obj, m).await,
            Command::MoveUnit { obj_id, movement } => self.send_move(obj_id, movement).await,
            Command::Chat { chat_type, target, message } => {
                self.send_game(game::ClientMsg::Chat { chat_type, target, message }).await;
            }
            Command::SetActionSlot { slot, action } => {
                if self.send_game(game::ClientMsg::UpdateActionSlot { slot, action }).await {
                    self.emit(Event::ActionSlotChanged { slot, action });
                }
            }
            Command::SwapItems(mv) => {
                self.send_game(game::ClientMsg::SwapItems(mv)).await;
            }
            Command::SplitItem { mv, count } => {
                self.send_game(game::ClientMsg::SplitItem { mv, count }).await;
            }
            Command::DestroyItem { item_id, container, slot, count } => {
                self.send_game(game::ClientMsg::DestroyItem { item_id, container, slot, count }).await;
            }
            Command::SetForceAttack { on } => {
                if self.phase != Phase::InWorld || self.self_obj == 0 {
                    self.emit(Event::ForceAttackRejected { on, message: "force attack requires the active player in world".into() });
                } else if self.send_game(game::ClientMsg::SetForceAttack { on }).await {
                    self.emit(Event::ForceAttackRequested { on });
                }
            }
            Command::Target { obj_id } => {
                self.send_game(game::ClientMsg::ChangeTarget { obj_id }).await;
            }
            Command::SellBackpackGoods { obj_id } => {
                self.send_game(game::ClientMsg::SellBackpackGoods { obj_id }).await;
            }
            Command::Resurrect { in_place } => {
                self.send_game(game::ClientMsg::Resurrect { in_place }).await;
            }
            Command::LootOpen { obj_id, loot_all } => {
                self.send_game(game::ClientMsg::LootOpenBag { obj_id, loot_all }).await;
            }
            Command::LootItem { owner_type, owner_obj_id, item_index } => {
                self.send_game(game::ClientMsg::LootItem { item_index, owner_type, owner_obj_id }).await;
            }
            Command::LootClose { owner_type, owner_obj_id } => {
                self.send_game(game::ClientMsg::LootCloseBag { owner_type, owner_obj_id }).await;
            }
            Command::LearnSkill { skill_id } => {
                self.send_game(game::ClientMsg::LearnSkill { skill_id }).await;
            }
            Command::LearnBuff { passive_id } => {
                self.send_game(game::ClientMsg::LearnBuff { passive_id }).await;
            }
            Command::ResetSkills { ability } => {
                self.send_game(game::ClientMsg::ResetSkills { ability }).await;
            }
            Command::SwapAbility { old, ability } => {
                let obj_id = self.self_obj;
                self.send_game(game::ClientMsg::SwapAbility { obj_id, old, ability }).await;
            }
            Command::Interact(m) => {
                self.send_game(game::ClientMsg::Interact(m)).await;
            }
            Command::Shop(m) => {
                if let Err(message) = m.validate() {
                    self.emit(Event::Shop(crate::shop::ShopEvent::RequestRejected(message.into())));
                } else {
                    self.send_game(game::ClientMsg::Shop(m)).await;
                }
            }
            Command::RequestInGameShopMenu { character_id } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0
                    || self.selected != character_id || self.self_obj == 0 || self.self_obj > 0xff_ffff
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != character_id)
                {
                    Some("in-game shop menu request requires the same active captured character")
                } else if self.game.is_none() {
                    Some("in-game shop menu request requires a game connection")
                } else {
                    None
                };
                if let Some(message) = invalid {
                    self.emit(Event::InGameShopMenuRequestRejected { character_id, message: message.into() });
                } else if self.send_game(game::ClientMsg::RequestInGameShopMenu).await {
                    self.emit(Event::InGameShopMenuRequested { character_id });
                } else {
                    self.emit(Event::InGameShopMenuRequestRejected { character_id, message: "in-game shop menu request was not sent".into() });
                }
            }
            Command::UiData(command) => {
                use crate::ui_data::{UiDataCommand, UiDataEvent};
                let invalid = command.validate().err().or_else(|| match &command {
                    UiDataCommand::Request { .. } if !matches!(self.phase,
                        Phase::CharacterSelect | Phase::EnteringWorld | Phase::InWorld) =>
                        Some("UI data request requires a connected character lobby or world"),
                    UiDataCommand::Save { character_id, .. } if self.phase != Phase::InWorld
                        || *character_id != self.selected || self.selected == 0 =>
                        Some("UI data save must match the active selected character"),
                    _ => None,
                });
                if let Some(message) = invalid {
                    self.emit(Event::UiData(UiDataEvent::RequestRejected { command, message: message.into() }));
                } else if self.send_game(game::ClientMsg::UiData(command.clone())).await {
                    self.emit(Event::UiData(UiDataEvent::Requested(command)));
                }
            }
            Command::ComposeMusic { character_id, request } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0
                    || self.selected != character_id || self.self_obj == 0 || self.self_obj > 0xff_ffff
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != character_id)
                {
                    Some("music composition requires the same active received character")
                } else if self.received_native_records.borrow().owner() != Some(character_id) {
                    Some("music composition native record owner is unknown or changed")
                } else {
                    request.recheck_native(&self.received_native_records.borrow()).err()
                };
                if let Some(message) = invalid {
                    self.emit(Event::MusicComposeRejected { character_id, request, message: message.into() });
                } else if self.send_game(game::ClientMsg::ComposeMusic { self_obj_id: self.self_obj, request: request.clone() }).await {
                    self.emit(Event::MusicComposeRequested { character_id, request });
                } else {
                    self.emit(Event::MusicComposeRejected { character_id, request, message: "music composition cast was not sent".into() });
                }
            }
            Command::Music { character_id, command } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0
                    || self.selected != character_id || self.self_obj == 0
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != character_id)
                {
                    Some("music request requires the same active captured character")
                } else {
                    command.validate().err()
                };
                if let Some(message) = invalid {
                    self.emit(Event::MusicRequestRejected { character_id, command, message: message.into() });
                } else if self.send_game(game::ClientMsg::Music(command.clone())).await {
                    self.emit(Event::MusicRequested { character_id, command });
                } else {
                    self.emit(Event::MusicRequestRejected { character_id, command, message: "music request was not sent".into() });
                }
            }
            Command::Dominion { character_id, command } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0 || self.selected != character_id
                    || self.selected_state.as_ref().is_none_or(|state| state.character.id != character_id)
                {
                    Some("Dominion request requires the actual active captured character header")
                } else {
                    command.validate().err()
                };
                if let Some(message) = invalid {
                    self.emit(Event::DominionRequestRejected { character_id, command, message: message.into() });
                } else if self.send_game(game::ClientMsg::Dominion(command)).await {
                    self.emit(Event::DominionRequested { character_id, command });
                } else {
                    self.emit(Event::DominionRequestRejected { character_id, command, message: "Dominion request was not sent".into() });
                }
            }
            Command::Expedition { character_id, command } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0 || self.selected != character_id {
                    Some("expedition request requires the same active selected character")
                } else { command.validate().err() };
                if let Some(message) = invalid {
                    self.emit(Event::ExpeditionRequestRejected { character_id, command, message: message.into() });
                } else if self.send_game(game::ClientMsg::Expedition(command.clone())).await {
                    self.emit(Event::ExpeditionRequested { character_id, command });
                } else {
                    self.emit(Event::ExpeditionRequestRejected { character_id, command, message: "expedition request was not sent".into() });
                }
            }
            Command::RemoveBuff { character_id, obj_id, buff_index } => {
                let message = if self.phase != Phase::InWorld || character_id == 0 || self.selected != character_id
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != character_id)
                    || obj_id == 0 || obj_id > 0x00ff_ffff || self.self_obj != obj_id {
                    Some("buff cancellation belongs to an inactive character or actor")
                } else if self.send_game(game::ClientMsg::RemoveBuff { obj_id, buff_index, reason:0 }).await {
                    self.emit(Event::BuffRemovalRequested { character_id, obj_id, buff_index });
                    None
                } else { Some("buff cancellation was not written to the game connection") };
                if let Some(message) = message {
                    self.emit(Event::BuffRemovalRequestRejected { character_id, obj_id, buff_index, message:message.into() });
                }
            }
            Command::Portal(command) => {
                let invalid = if self.phase != Phase::InWorld { Some("not in the world") } else { command.validate().err() };
                if let Some(message) = invalid {
                    self.emit(Event::PortalRequestRejected { command, message: message.into() });
                } else if self.send_game(game::ClientMsg::Portal { self_obj_id: self.self_obj, command: command.clone() }).await {
                    if let crate::portal::PortalCommand::Delete { portal_type, portal_id } = command {
                        self.emit(Event::PortalDeleteSent { portal_type, portal_id });
                    }
                    self.emit(Event::PortalRequested(command));
                } else {
                    self.emit(Event::PortalRequestRejected { command, message: "portal request was not sent".into() });
                }
            }
            Command::Ranking{character_id,command}=>{
                let invalid=if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || self.selected_state.as_ref().is_none_or(|s|s.character.id!=character_id){Some("ranking request belongs to an inactive character")}
                else if let crate::RankingCommand::RequestSnapshots{rank_id}=command {let cache=self.received_rankings.borrow();if cache.character_id!=Some(character_id)||!cache.by_id.contains_key(&rank_id){Some("ranking snapshot requires an actual received board for this character")}else{None}}
                else {None};
                if let Some(message)=invalid {self.emit(Event::RankingRequestRejected{character_id,command,message:message.into()});}
                else if self.send_game(game::ClientMsg::Ranking(command)).await {self.emit(Event::RankingRequested{character_id,command});}
                else {self.emit(Event::RankingRequestRejected{character_id,command,message:"ranking request was not sent".into()});}
            }
            Command::QueryCharacterName{character_id,requested_id}=>{
                let invalid=if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || self.selected_state.as_ref().is_none_or(|s|s.character.id!=character_id) {Some("character-name request belongs to an inactive character")}
                else if requested_id==0 {Some("invalid persistent character ID")}
                else if !self.stream_joined || self.stream.is_none(){Some("stream connection has not accepted the join")}
                else if self.pending_names.contains(&requested_id){Some("character-name query is already pending")}else{None};
                if let Some(message)=invalid {self.emit(Event::CharacterNameRequestRejected{character_id,requested_id,message:message.into()});}
                else {
                    self.pending_names.insert(requested_id);
                    let result=self.stream.as_mut().unwrap().send(&stream::ClientMsg::QueryCharacterName{character_id:requested_id}).await;
                    match result {Ok(())=>self.emit(Event::CharacterNameRequested{character_id,requested_id}),Err(e)=>{self.pending_names.remove(&requested_id);self.emit(Event::CharacterNameRequestRejected{character_id,requested_id,message:e.to_string()});}}
                }
            }
            Command::Bounty{character_id,command} => {
                let invalid=if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || !self.selected_state.as_ref().is_some_and(|s|s.character.id==character_id) {
                    Some("bounty request requires the actual active captured character header")
                }else{command.validate().err()};
                if let Some(message)=invalid {self.emit(Event::BountyRequestRejected{character_id,command,message:message.into()});}
                else if self.send_game(game::ClientMsg::Bounty(command.clone())).await {self.emit(Event::BountyRequested{character_id,command});}
                else {self.emit(Event::BountyRequestRejected{character_id,command,message:"bounty request was not sent".into()});}
            }
            Command::Trial{character_id,command} => {
                let invalid=if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || !self.selected_state.as_ref().is_some_and(|s|s.character.id==character_id) {
                    Some("trial request requires the actual active captured character header")
                }else{command.validate_received(&self.received_trials.borrow()).err()};
                if let Some(message)=invalid {self.emit(Event::TrialRequestRejected{character_id,command,message:message.into()});}
                else if self.send_game(game::ClientMsg::Trial(command)).await {self.emit(Event::TrialRequested{character_id,command});}
                else {self.emit(Event::TrialRequestRejected{character_id,command,message:"trial request was not sent".into()});}
            }
            Command::ChallengeDuel { challenged_id } => {
                if self.phase != Phase::InWorld || self.self_obj == 0 || challenged_id == 0 {
                    // Challenging oneself is the server's to refuse (BAD_DUEL_SELF 513); the HUD's popup never offers it.
                    self.emit(Event::DuelRequestRejected { message: "a duel needs the player in world and a character".into() });
                } else if self.send_game(game::ClientMsg::ChallengeDuel { challenged_id }).await {
                    self.emit(Event::DuelRequested { challenged_id });
                }
            }
            Command::AnswerDuel { challenger_id, accept } => {
                if self.phase != Phase::InWorld || self.self_obj == 0 || challenger_id == 0 {
                    self.emit(Event::DuelRequestRejected { message: "a duel answer needs the player in world".into() });
                } else {
                    let error = if accept { 0 } else { crate::duel::REJECTED };
                    if self.send_game(game::ClientMsg::StartDuel { challenger_id, error }).await {
                        self.emit(Event::DuelAnswered { challenger_id, accept });
                    }
                }
            }
            Command::ExpressEmotion { emotion_id, target_obj_id } => {
                if self.phase == Phase::InWorld && self.self_obj != 0 && emotion_id != 0 {
                    let obj_id = self.self_obj;
                    self.send_game(game::ClientMsg::ExpressEmotion { obj_id, target_obj_id, emotion_id }).await;
                }
            }
            Command::Account(command) => {
                if self.phase == Phase::CharacterSelect {
                    self.send_game(game::ClientMsg::Account(command)).await;
                }
            }
            Command::Salon(command) => {
                if self.phase == Phase::InWorld && self.self_obj != 0 {
                    self.send_game(game::ClientMsg::Salon(command)).await;
                }
            }
            Command::ChatCommand(command) => {
                if self.phase == Phase::InWorld && self.self_obj != 0 && command.validate().is_ok() {
                    self.send_game(game::ClientMsg::ChatCommand(command)).await;
                }
            }
            Command::ChangeAppellation { id } => {
                if self.phase == Phase::InWorld && self.self_obj != 0 && self.send_game(game::ClientMsg::ChangeAppellation { id }).await {
                    self.emit(Event::AppellationRequested { id });
                }
            }
            Command::RequestJuryWaitingNumber { character_id } => {
                if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id {
                    self.emit(Event::JuryWaitingNumberRequestRejected{character_id,message:"jury number request requires the same active selected character".into()});
                } else if self.send_game(game::ClientMsg::RequestJuryWaitingNumber).await {
                    self.emit(Event::JuryWaitingNumberRequested{character_id});
                } else {
                    self.emit(Event::JuryWaitingNumberRequestRejected{character_id,message:"jury number request was not sent".into()});
                }
            }
            Command::Family { character_id, command } => {
                let invalid = if self.phase != Phase::InWorld || character_id == 0 || self.selected != character_id {
                    Some("family request requires the same active selected character")
                } else { command.validate().err() };
                if let Some(message) = invalid {
                    self.emit(Event::FamilyRequestRejected { character_id, command, message: message.into() });
                } else if self.send_game(game::ClientMsg::Family(command.clone())).await {
                    self.emit(Event::FamilyRequested { character_id, command });
                } else {
                    self.emit(Event::FamilyRequestRejected { character_id, command, message: "family request was not sent".into() });
                }
            }
            Command::ItemEffect {character_id,request} => {
                let invalid=if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || self.self_obj==0 || self.self_obj>0xff_ffff || self.selected_state.as_ref().is_none_or(|s|s.character.id!=character_id) {Some("item effect requires the same active received character")}
                    else if self.received_native_records.borrow().owner()!=Some(character_id) {Some("item effect native record owner is unknown or changed")}
                    else {request.recheck_native(&self.received_native_records.borrow(),self.received_inventory.borrow().auto_use_aa_point_raw).err()};
                if let Some(message)=invalid {self.emit(Event::ItemEffectRejected{character_id,request,message:message.into()});}
                else if self.send_game(game::ClientMsg::ItemEffect{self_obj_id:self.self_obj,request:request.clone()}).await {self.emit(Event::ItemEffectRequested{character_id,request});}
                else {self.emit(Event::ItemEffectRejected{character_id,request,message:"item effect request was not sent".into()});}
            }
            Command::ItemLook { character_id, request } => {
                let invalid = if self.phase != Phase::InWorld
                    || character_id == 0 || self.selected != character_id
                    || self.self_obj == 0 || self.self_obj > 0xff_ffff
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != character_id)
                {
                    Some("item look requires the same active received character")
                } else if self.received_native_records.borrow().owner() != Some(character_id) {
                    Some("item look native record owner is unknown or changed")
                } else {
                    request.recheck_native(&self.received_native_records.borrow()).err()
                };
                if let Some(message) = invalid {
                    self.emit(Event::ItemLookRejected { character_id, request, message: message.into() });
                } else if self.send_game(game::ClientMsg::ItemLook { self_obj_id: self.self_obj, request: request.clone() }).await {
                    self.emit(Event::ItemLookRequested { character_id, request });
                } else {
                    self.emit(Event::ItemLookRejected { character_id, request, message: "item look request was not sent".into() });
                }
            }
            Command::UnpackItem(request) => {
                let invalid=if self.phase!=Phase::InWorld || self.selected==0 {Some("unpack requires an active selected character")}
                    else {request.recheck(&self.received_inventory.borrow()).err()};
                if let Some(message)=invalid {self.emit(Event::UnpackItemRequestRejected{request,message:message.into()});}
                else if self.send_game(game::ClientMsg::UnpackItem(request)).await {self.emit(Event::UnpackItemRequested(request));}
                else {self.emit(Event::UnpackItemRequestRejected{request,message:"unpack request was not sent".into()});}
            }
            Command::SetAutoUseAaPoint(value) => {
                if self.phase != Phase::InWorld {
                    self.emit(Event::AutoUseAaPointRequestRejected { value, message: "AA point setting requires an active character".into() });
                } else if self.send_game(game::ClientMsg::SetAutoUseAaPoint(value)).await {
                    self.emit(Event::AutoUseAaPointRequested(value));
                } else {
                    self.emit(Event::AutoUseAaPointRequestRejected { value, message: "AA point setting was not sent".into() });
                }
            }
            Command::Bank(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Bank(crate::bank::BankEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::Bank(crate::bank::BankEvent::RequestRejected { command, message: "bank service requires an active character".into() }));
                } else if self.send_game(game::ClientMsg::Bank(command)).await {
                    self.emit(Event::Bank(crate::bank::BankEvent::Requested(command)));
                } else {
                    self.emit(Event::Bank(crate::bank::BankEvent::RequestRejected { command, message: "bank request was not sent".into() }));
                }
            }
            Command::Party(m) => {
                if let Err(message) = m.validate() {
                    self.emit(Event::Party(crate::party::PartyEvent::RequestRejected(message.into())));
                    return;
                }
                let replied = match &m {
                    crate::party::PartyCommand::Reply { invite, .. } => Some(invite.sender_id),
                    _ => None,
                };
                let character_name = self.selected_state.as_ref().map(|s| s.character.name.clone()).unwrap_or_default();
                if self.send_game(game::ClientMsg::Party { command: m, character_name }).await {
                    if let Some(sender_id) = replied {
                        self.emit(Event::Party(crate::party::PartyEvent::Replied { sender_id }));
                    }
                }
            }
            Command::Friend(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Friend(crate::friend::FriendEvent::RequestRejected {
                        command,
                        message: message.into(),
                    }));
                } else if self.send_game(game::ClientMsg::Friend(command.clone())).await {
                    self.emit(Event::Friend(crate::friend::FriendEvent::Requested(command)));
                }
            }
            Command::Battlefield(command) => {
                let validation = self.battlefield.borrow().validate_command(command);
                if let Err(message) = validation {
                    self.emit(Event::Battlefield(crate::battlefield::BattlefieldEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::Battlefield(crate::battlefield::BattlefieldEvent::RequestRejected { command, message: "battlefield request requires an active character".into() }));
                } else if self.send_game(game::ClientMsg::Battlefield(command)).await {
                    self.emit(Event::Battlefield(crate::battlefield::BattlefieldEvent::Requested(command)));
                } else {
                    self.emit(Event::Battlefield(crate::battlefield::BattlefieldEvent::RequestRejected { command, message: "battlefield request was not sent".into() }));
                }
            }
            Command::Actability(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Actability(crate::actability::ActabilityEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::Actability(crate::actability::ActabilityEvent::RequestRejected { command, message: "proficiency request requires an active character".into() }));
                } else if self.send_game(game::ClientMsg::Actability(command)).await {
                    self.emit(Event::Actability(crate::actability::ActabilityEvent::Requested(command)));
                } else {
                    self.emit(Event::Actability(crate::actability::ActabilityEvent::RequestRejected { command, message: "proficiency request was not sent".into() }));
                }
            }
            Command::UnbondDoodad(request) => {
                let invalid = if self.phase != Phase::InWorld || request.character_id == 0
                    || self.selected != request.character_id || self.self_obj != request.actor_obj_id
                    || self.selected_state.as_ref().is_none_or(|s| s.character.id != request.character_id) {
                    Some("unbond requires the same active received character and actor")
                } else if request.actor_obj_id == 0 || request.actor_obj_id > 0xff_ffff
                    || request.bond.owner_obj_id == 0 || request.bond.owner_obj_id > 0xff_ffff {
                    Some("unbond object IDs must fit nonzero BC3")
                } else if !self.received_doodad_bonds.borrow().matches_request(&request) {
                    Some("unbond requires the exact current received own bond")
                } else if self.game.is_none() { Some("not connected to the game server") } else { None };
                if let Some(message) = invalid {
                    self.emit(Event::DoodadBond(crate::DoodadBondEvent::UnbondRejected { request, message: message.into() }));
                } else if self.send_game(game::ClientMsg::UnbondDoodad(request)).await {
                    self.emit(Event::DoodadBond(crate::DoodadBondEvent::UnbondRequested(request)));
                }
            }
            Command::Slave(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Slave(crate::slave::SlaveEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase == Phase::InWorld && self.send_game(game::ClientMsg::Slave(command.clone())).await {
                    self.emit(Event::Slave(crate::slave::SlaveEvent::Requested(command)));
                }
            }
            Command::Mate(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Mate(crate::mate::MateEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::Mate(crate::mate::MateEvent::RequestRejected { command, message: "mate service requires an active character".into() }));
                } else if self.send_game(game::ClientMsg::Mate(command.clone())).await {
                    self.emit(Event::Mate(crate::mate::MateEvent::Requested(command)));
                } else {
                    self.emit(Event::Mate(crate::mate::MateEvent::RequestRejected { command, message: "mate request was not sent".into() }));
                }
            }
            Command::LootRoll(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::LootRoll(crate::loot_roll::LootRollEvent::RequestRejected { command, message: message.into() }));
                } else if self.send_game(game::ClientMsg::LootRoll(command)).await {
                    self.emit(Event::LootRoll(crate::loot_roll::LootRollEvent::Requested(command)));
                }
            }
            Command::Auction(command) => {self.send_auction(command).await;}
            Command::AuctionForCharacter{character_id,command}=>{
                if self.phase!=Phase::InWorld || character_id==0 || self.selected!=character_id || self.selected_state.as_ref().is_none_or(|s|s.character.id!=character_id){
                    self.emit(Event::Auction(crate::auction::AuctionEvent::RequestRejected{command,message:"auction request belongs to an inactive character".into()}));
                }else{self.send_auction(command).await;}
            }
            Command::Trade(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Trade(crate::trade::TradeEvent::RequestRejected { command, message: message.into() }));
                } else if self.send_game(game::ClientMsg::Trade(command.clone())).await {
                    self.emit(Event::Trade(crate::trade::TradeEvent::Requested(command)));
                }
            }
            Command::Mail(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Mail(crate::mail::MailEvent::RequestRejected { command, message: message.into() }));
                } else if self.send_game(game::ClientMsg::Mail(command.clone())).await {
                    self.emit(Event::Mail(crate::mail::MailEvent::Requested(command)));
                }
            }
            Command::Repair(command) => {
                if let Err(message) = command.validate() {
                    self.emit(Event::Repair(crate::repair::RepairEvent::RequestRejected { command, message: message.into() }));
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::Repair(crate::repair::RepairEvent::RequestRejected { command, message: "repair requires an active character".into() }));
                } else if self.send_game(game::ClientMsg::Repair(command)).await {
                    self.emit(Event::Repair(crate::repair::RepairEvent::Requested(command)));
                } else {
                    self.emit(Event::Repair(crate::repair::RepairEvent::RequestRejected { command, message: "repair request was not sent".into() }));
                }
            }
            Command::House(command) => {
                if self.phase == Phase::InWorld {
                    self.send_game(game::ClientMsg::House(command)).await;
                }
            }
            Command::CreateDoodad(request) => {
                if self.phase == Phase::InWorld {
                    self.send_game(game::ClientMsg::CreateDoodad(request)).await;
                }
            }
            Command::Coffer(command) => {
                if self.phase == Phase::InWorld && self.self_obj != 0 && command.validate().is_ok() {
                    self.send_game(game::ClientMsg::Coffer(command)).await;
                }
            }
            Command::RequestCommonFarmList => {
                if self.phase == Phase::InWorld {
                    self.send_game(game::ClientMsg::RequestCommonFarmList).await;
                }
            }
            Command::UseSkill { skill_id, target_obj_id } => {
                let target = SkillTarget::unit(if target_obj_id == 0 { self.self_obj } else { target_obj_id });
                self.start_skill(skill_id, StartCaster::Unit, target).await;
            }
            Command::UseDoodadSkill { skill_id, doodad_obj_id } => {
                let target = if skill_id == 0 { Err("interaction skill ID must be nonzero") } else { SkillTarget::doodad(doodad_obj_id) };
                match target {
                    Ok(target) => self.start_skill(skill_id, StartCaster::Unit, target).await,
                    Err(message) => self.emit(Event::DoodadSkillRejected { skill_id, doodad_obj_id, message: message.into() }),
                }
            }
            Command::ExecuteCraft { craft_id, doodad_obj_id, count } => {
                let error = if craft_id == 0 {
                    Some("recipe ID must be nonzero")
                } else if doodad_obj_id == 0 || doodad_obj_id > 0xffffff {
                    Some("crafting doodad ID must be nonzero and fit in 24 bits")
                } else if count <= 0 {
                    Some("craft batch count must be positive")
                } else {
                    None
                };
                if let Some(message) = error {
                    self.emit(Event::CraftRejected { craft_id, doodad_obj_id, count, message: message.into() });
                } else if self.phase != Phase::InWorld {
                    self.emit(Event::CraftRejected { craft_id, doodad_obj_id, count, message: "crafting requires an active character".into() });
                } else if self.send_game(game::ClientMsg::ExecuteCraft { craft_id, doodad_obj_id, count }).await {
                    self.emit(Event::CraftRequested { craft_id, doodad_obj_id, count });
                } else {
                    self.emit(Event::CraftRejected { craft_id, doodad_obj_id, count, message: "crafting request was not sent".into() });
                }
            }
            Command::UseMateSkill { skill_id, mate_obj_id, mount_skill_id, target_obj_id } => {
                let error = if skill_id == 0 || mount_skill_id == 0 {
                    Some("combat skill and mount skill IDs must be nonzero")
                } else if mate_obj_id == 0 || mate_obj_id > 0xffffff || target_obj_id == 0 || target_obj_id > 0xffffff {
                    Some("mate caster and skill target must be nonzero 24-bit object IDs")
                } else {
                    None
                };
                if let Some(message) = error {
                    self.emit(Event::Mate(crate::mate::MateEvent::SkillRejected { skill_id, mate_obj_id, mount_skill_id, target_obj_id, message: message.into() }));
                } else if self.phase == Phase::InWorld && self.send_game(game::ClientMsg::StartSkill {
                    skill_id, self_obj_id: self.self_obj, caster: StartCaster::Mount { obj_id: mate_obj_id, mount_skill_id }, target: SkillTarget::unit(target_obj_id),
                }).await {
                    self.emit(Event::Mate(crate::mate::MateEvent::SkillRequested { skill_id, mate_obj_id, mount_skill_id, target_obj_id }));
                }
            }
            Command::StopCasting { tl, plot_tl } => {
                let obj_id = self.self_obj;
                self.send_game(game::ClientMsg::StopCasting { tl, plot_tl, obj_id }).await;
            }
            Command::UseSkillAt { skill_id, pos } => {
                self.start_skill(skill_id, StartCaster::Unit, SkillTarget::position(pos)).await;
            }
            Command::UseItem { item_id, template_id, skill_id, target_obj_id } => {
                let target = SkillTarget::unit(if target_obj_id == 0 { self.self_obj } else { target_obj_id });
                self.start_skill(skill_id, StartCaster::Item { item_id, template_id }, target).await;
            }
            Command::UseItemAt { item_id, template_id, skill_id, pos } => {
                self.start_skill(skill_id, StartCaster::Item { item_id, template_id }, SkillTarget::position(pos)).await;
            }
            Command::LeaveWorld(target) => {
                self.send_game(game::ClientMsg::LeaveWorld(target)).await;
            }
            Command::CancelLeave => {
                self.send_game(game::ClientMsg::CancelLeaveWorld).await;
            }
            Command::ReturnToWorldSelect => {
                self.wait_for("the reconnect token");
                self.send_game(game::ClientMsg::LeaveWorld(LeaveTarget::ServerSelect)).await;
            }
            Command::Disconnect => {
                self.close_all().await;
                self.waiting = None;
                if self.phase != Phase::Idle {
                    self.set_phase(Phase::Idle);
                    self.emit(Event::Disconnected { message: "disconnected".into() });
                }
            }
        }
    }

    async fn start_login(&mut self, addr: String, user: String, password: Secret) {
        self.close_all().await;
        self.account = None;
        self.cookie = None;
        self.afs = None;
        self.login_addr = addr.clone();
        self.set_phase(Phase::Authenticating);
        self.wait_for("the login server");
        match LoginConn::connect(&addr).await {
            Ok(c) => self.login = Some(c),
            Err(e) => {
                self.waiting = None;
                self.set_phase(Phase::Idle);
                self.emit(Event::LoginFailed {
                    message: format!("cannot reach the login server {addr}: {e}"),
                });
                return;
            }
        }
        self.send_login(login::ClientMsg::auth(&user, password.expose())).await;
    }

    async fn connect_world(&mut self) {
        let (Some((_, cookie)), Some(account)) = (self.cookie, self.account) else {
            tracing::warn!("ConnectToWorld without a world cookie");
            return;
        };
        self.set_phase(Phase::Connecting);
        self.wait_for("the game server");
        let addr = cookie.addr.to_string();
        match GameConn::connect(&addr).await {
            Ok(c) => {
                self.game = Some(c);
                self.next_ping = Some(Instant::now() + self.options.first_ping_delay);
                self.keep_alive().await;
            }
            Err(e) => {
                self.waiting = None;
                self.set_phase(Phase::ReadyToConnect);
                self.emit(Event::ConnectFailed {
                    message: format!("cannot reach the game server {addr}: {e}"),
                });
                return;
            }
        }
        self.send_game(game::ClientMsg::EnterWorld {
            account_id: account.account_id,
            cookie: cookie.cookie,
        })
        .await;
    }

    async fn send_move(&mut self, obj_id: u32, m: MoveCommand) {
        if self.self_obj == 0 || obj_id == 0 || self.phase != Phase::InWorld {
            tracing::debug!("ignoring Move: the player is not in the world");
            return;
        }
        let movement = m.to_movement(self.started.elapsed().as_millis() as u32);
        self.send_game(game::ClientMsg::Move { obj_id, movement }).await;
    }

    // ---- login server -------------------------------------------------------------------------

    async fn login_msg(&mut self, r: net::Result<login::ServerMsg>) {
        let msg = match r {
            Ok(m) => m,
            Err(NetError::Wire(e)) => {
                tracing::warn!("undecodable login packet: {e}");
                return;
            }
            Err(e) => return self.login_lost(e),
        };
        use login::ServerMsg as M;
        match msg {
            M::JoinResponse { reason, afs } => {
                if reason == 0 {
                    self.afs = Some(afs);
                } else {
                    self.fail_login(format!("join refused (reason {reason})")).await;
                }
            }
            M::AuthResponse { account_id, slot_count } => {
                let afs = self.afs.unwrap_or(0);
                let info = AccountInfo {
                    account_id,
                    max_characters: (afs & 0xFF) as u8,
                    extra_characters_per_world: (afs >> 32) as u32,
                    pre_select_period: afs & 0x100 != 0,
                    slot_count,
                };
                self.account = Some(info);
                self.waiting = None;
                self.set_phase(Phase::WorldSelect);
                self.emit(Event::LoginSucceeded(info));
                self.send_login(login::ClientMsg::ListWorlds).await;
            }
            M::WorldList(list) => self.emit(Event::WorldList(list)),
            M::WorldQueue(q) => {
                self.set_phase(Phase::Queued);
                self.emit(Event::WorldQueue(q));
            }
            M::WorldCookie(cookie) => {
                let world_id = self.pending_world;
                self.cookie = Some((world_id, cookie));
                self.set_phase(Phase::ReadyToConnect);
                self.emit(Event::ReadyToConnect { world_id, cookie });
            }
            M::EnterWorldDenied { reason } => {
                self.set_phase(Phase::WorldSelect);
                self.emit(Event::EnterWorldDenied { reason });
            }
            M::LoginDenied { reason, message } => {
                let text = if message.is_empty() {
                    login::login_denied_text(reason).to_string()
                } else {
                    message
                };
                self.fail_login(format!("login denied (reason {reason}): {text}")).await;
            }
            M::AccountWarned { source, message } => {
                tracing::warn!("account warning (source {source}): {message}");
            }
            M::Unsupported { opcode } => {
                tracing::warn!("unsupported login packet {}", crate::opcodes::ac::name(opcode));
            }
        }
    }

    async fn fail_login(&mut self, message: String) {
        self.waiting = None;
        if let Some(mut c) = self.login.take() {
            c.shutdown().await;
        }
        self.set_phase(Phase::Idle);
        self.emit(Event::LoginFailed { message });
    }

    // ---- game server --------------------------------------------------------------------------

    async fn game_msg(&mut self, r: net::Result<game::ServerMsg>) {
        let received_at=Instant::now();
        let local_receive_ms=received_at.duration_since(self.started).as_millis() as u32;
        self.advance_battlefield_clock();
        let msg = match r {
            Ok(m) => m,
            Err(NetError::Wire(e)) => {
                tracing::warn!("undecodable game packet: {e}");
                self.emit(Event::GameDecodeFailed{message:e.to_string()});
                return;
            }
            Err(e) => return self.game_lost(e),
        };
        match msg {
            // A batch holds packets that would each have arrived alone.
            game::ServerMsg::Batch(list) => {
                for m in list {
                    self.game_one_at(m,received_at,local_receive_ms).await;
                }
            }
            msg => self.game_one_at(msg,received_at,local_receive_ms).await,
        }
    }

    #[cfg(test)]
    async fn game_one(&mut self,msg:game::ServerMsg) {
        let received_at=Instant::now();let local_receive_ms=received_at.duration_since(self.started).as_millis() as u32;
        self.game_one_at(msg,received_at,local_receive_ms).await;
    }
    async fn game_one_at(&mut self, msg: game::ServerMsg,received_at:Instant,local_receive_ms:u32) {
        self.advance_battlefield_clock();
        use game::ServerMsg as M;
        match msg {
            M::ChangeState(state) => {
                // The client acknowledges every CryNetwork state; state 7 completes the lobby.
                if self.phase != Phase::InWorld {
                    self.send_game(game::ClientMsg::FinishState(state)).await;
                }
            }
            M::EnterWorldResponse { reason, stream_token, stream_port, .. } => {
                if reason != 0 {
                    self.waiting = None;
                    self.game = None;
                    self.set_phase(Phase::ReadyToConnect);
                    self.emit(Event::ConnectFailed {
                        message: format!("the game server refused us (reason {reason})"),
                    });
                    return;
                }
                self.stream_token = stream_token;
                self.stream_port = stream_port;
            }
            // State 7 of the handshake is answered with the premium points: the lobby is ready.
            M::PremiumPoint { point, old_grade, grade } => {
                self.emit(Event::PremiumPoints(crate::account::PremiumPoints { point, old_grade, grade }));
                if self.phase == Phase::Connecting {
                    self.lobby_ready().await;
                }
            }
            M::RaceCongestion(c) => self.emit(Event::RaceCongestion(c)),
            M::CharacterList { last, characters } => {
                self.chars_buf.extend(characters);
                if last {
                    self.waiting = None;
                    let list = std::mem::take(&mut self.chars_buf);
                    self.emit(Event::CharacterList(list));
                }
            }
            M::CharacterCreated(c) => {
                self.emit(Event::CharacterCreated(c));
                self.command(Command::RefreshCharacters).await;
            }
            M::CharacterCreationFailed { reason } => {
                self.emit(Event::CharacterCreateFailed { reason });
            }
            M::CharacterDeleteResponse { id, status, delete_request_time, delete_time } => {
                self.emit(Event::CharacterDeleteResponse { id, status, delete_request_time, delete_time });
                self.command(Command::RefreshCharacters).await;
            }
            // status 3: the delete was cancelled; 4: no such character
            M::CancelCharacterDeleteResponse { id, status } => {
                self.emit(Event::CharacterDeleteCancelled { character_id: id, restored: status == 3 });
                self.command(Command::RefreshCharacters).await;
            }
            M::CharacterDeleted { id, name } => self.emit(Event::CharacterDeleted { id, name }),
            M::CharacterState(state) => {
                self.wait_for("the player spawn");
                self.selected_state = Some((*state).clone());
                self.emit(Event::CharacterSelected(state));
                self.send_game(game::ClientMsg::SpawnCharacter(VisualOptions::default())).await;
            }
            M::UnitState(unit) => self.unit_state(*unit).await,
            M::UnitsRemoved(ids) => self.emit(Event::UnitsRemoved(ids)),
            M::UnitMoved { obj_id, movement } => {
                // A ship request has no transform; raw-event consumers must not treat its
                // default position as a movement report either.
                if movement.move_type != 4 {
                    self.emit(Event::UnitMoved { obj_id, movement });
                }
            }
            M::UnitMovements(list) => {
                for (obj_id, movement) in list {
                    self.emit(Event::UnitMoved { obj_id, movement });
                }
            }
            M::UnitAttached { obj_id, mount, point, reason } => self.emit(Event::UnitAttached { obj_id, mount, point, reason }),
            M::UnitDetached { obj_id, reason } => self.emit(Event::UnitDetached { obj_id, reason }),
            M::UnitPosture { obj_id, posture } => {
                self.emit(Event::UnitPosture { obj_id, kind: posture.kind, anim_action: posture.anim_action, activate: posture.activate });
                if let Some(house) = posture.house {
                    self.emit(Event::Housing(crate::housing::HousingEvent::UnitPosture { obj_id, posture: house }));
                }
            }
            M::CombatFirstHit { victim, hitter, hit_type } => self.emit(Event::CombatFirstHit { victim, hitter, hit_type }),
            M::AiAggro { npc, hostiles } => self.emit(Event::AiAggro { npc, hostiles }),
            M::AggroTargetChanged { npc, target } => self.emit(Event::AggroTargetChanged { npc, target }),
            M::UnitPoints { obj_id, hp, mp } => self.emit(Event::UnitPoints { obj_id, hp, mp }),
            M::UnitDeath { obj_id, kill_reason, resurrect_wait_ms, lost_exp, killer, .. } => {
                self.emit(Event::UnitDeath { obj_id });
                self.emit(Event::DeathDetails { obj_id, kill_reason, resurrect_wait_ms, lost_exp, killer });
            }
            M::ResurrectionOffered { caster } => self.emit(Event::ResurrectionOffered { caster }),
            M::CharacterResurrected { obj_id, pos, yaw } => self.emit(Event::Resurrected { obj_id, pos, yaw }),
            M::Lootable { owner_type, obj_id, has_loot } => self.emit(Event::Lootable { owner_type, obj_id, has_loot }),
            M::UnitLooting { obj_id, looting } => self.emit(Event::UnitLooting { obj_id, looting }),
            M::LootBag { items, loot_all } => self.emit(Event::LootBag { items, loot_all }),
            M::LootItemTook { template_id, item_index, owner_type, owner_obj_id, count } => {
                self.emit(Event::LootItemTook { template_id, item_index, owner_type, owner_obj_id, count });
            }
            M::LootItemFailed { error, item_index, owner_type, owner_obj_id, template_id } => {
                self.emit(Event::LootItemFailed { error, item_index, owner_type, owner_obj_id, item_template_id: template_id });
            }
            M::ExpChanged { obj_id, delta, add_ability_exp } => {
                self.emit(Event::ExpChanged { obj_id, delta, add_ability_exp });
            }
            M::AbilityExpChanged { obj_id, ability, exp } => {
                self.emit(Event::AbilityExpChanged { obj_id, ability, exp });
            }
            M::RecoverableExp { obj_id, recoverable, penaltied, reason } => {
                self.emit(Event::RecoverableExp { obj_id, recoverable, penaltied, reason });
            }
            M::LevelChanged { obj_id, level } => self.emit(Event::LevelChanged { obj_id, level }),
            M::AbilitySwapped { obj_id, old_ability, ability } => {
                self.emit(Event::AbilitySwapped { obj_id, old_ability, ability });
            }
            M::LaborPowerChanged { amount, action, point, step } => {
                self.emit(Event::LaborChanged { amount, action, point, step });
            }
            M::GamePoints { honor, vocation, other } => self.emit(Event::GamePoints { honor, vocation, other }),
            M::GamePointChanged { kind, amount } => self.emit(Event::GamePointChanged { kind, amount }),
            M::BmPoint(bm_point) => self.emit(Event::LoyaltyChanged { bm_point }),
            M::CashPoint(point) => self.emit(Event::CashPoint { point }),
            M::CombatEngaged { obj_id } => self.emit(Event::CombatChanged { obj_id, engaged: true }),
            M::CombatCleared { obj_id } => self.emit(Event::CombatChanged { obj_id, engaged: false }),
            M::UnitNameChanged { obj_id, name } => self.emit(Event::UnitNameChanged { obj_id, name }),
            M::UnitFactionChanged { obj_id, name, faction_id, faction_id2, temporary } => {
                self.emit(Event::UnitFactionChanged { obj_id, name, faction_id, faction_id2, temporary });
            }
            M::TimeOfDay { hour, speed } => self.emit(Event::TimeOfDay { hour, speed }),
            M::Snow(on) => self.emit(Event::Snow(on)),
            M::ChatJoined { chat_type, sub_type, faction_id, name } => {
                for chat_type in channel_types(chat_type) {
                    self.emit(Event::ChatChannelJoined { chat_type, sub_type, faction_id, name: name.clone() });
                }
            }
            M::ChatLeft { chat_type, sub_type, faction_id } => {
                for chat_type in channel_types(chat_type) {
                    self.emit(Event::ChatChannelLeft { chat_type, sub_type, faction_id });
                }
            }
            M::TargetChanged { obj_id, target_obj_id } => self.emit(Event::TargetChanged { obj_id, target_obj_id }),
            M::ActionSlots(slots) => self.emit(Event::ActionSlots(slots)),
            M::InvenInit { bag_slots, bank_slots } => self.emit(Event::InventoryInit { bag_slots, bank_slots }),
            M::InvenContents(chunk) => self.emit(Event::InventoryChunk(chunk)),
            M::ItemTasks(tasks) => self.emit(Event::ItemsChanged(tasks)),
            M::EquipmentChanged { obj_id, items } => self.emit(Event::EquipmentChanged { obj_id, items }),
            M::EquipmentIdsChanged { obj_id, ids } => self.emit(Event::EquipmentIdsChanged { obj_id, ids }),
            M::SpecialtyRatio(ratio) => self.emit(Event::SpecialtyRatio(ratio)),
            M::InvenExpanded { container, slots } => self.emit(Event::InventoryExpanded { container, slots }),
            M::SkillStarted(s) => self.emit(Event::SkillStarted(s)),
            M::SkillFired(s) => self.emit(Event::SkillFired(s)),
            M::SkillEnded { tl } => self.emit(Event::SkillEnded { tl }),
            M::SkillStopped { obj_id, skill_id } => self.emit(Event::SkillStopped { obj_id, skill_id }),
            M::CastingStopped { tl, duration_ms } => self.emit(Event::CastingStopped { tl, duration_ms }),
            M::Cooldowns(cooldowns) => self.emit(Event::Cooldowns(cooldowns)),
            M::SkillCooldownReset { obj_id, skill_id, tag_id, gcd } => {
                self.emit(Event::SkillCooldownReset { obj_id, skill_id, tag_id, gcd });
            }
            M::Quests(list) => self.emit(Event::QuestsLoaded(list)),
            M::CompletedQuests(list) => self.emit(Event::CompletedQuests(list)),
            M::QuestStarted(q) => self.emit(Event::QuestStarted(q)),
            M::QuestUpdated(q) => self.emit(Event::QuestUpdated(q)),
            M::QuestCompleted { quest_id, block } => self.emit(Event::QuestCompleted { quest_id, block }),
            M::QuestReset { quest_id, block } => self.emit(Event::QuestReset { quest_id, block }),
            M::Interaction(e) => self.emit(Event::Interaction(e)),
            M::Shop(e) => self.emit(Event::Shop(e)),
            M::Party(e) => self.emit(Event::Party(e)),
            M::LootRoll(e) => self.emit(Event::LootRoll(e)),
            M::Battlefield(e) => self.emit(Event::Battlefield(e)),
            M::Dominion(value) => self.emit(Event::Dominion(value)),
            M::Trial(e) => {
                let summon=if let crate::TrialEvent::SummonJury{trial_id,court,jury_number}=&e {Some((*trial_id,*court,*jury_number))}else{None};
                self.emit(Event::Trial(e));
                if let Some((trial_id,court,jury_number))=summon {
                    let character_id=self.selected;
                    let invalid=self.phase!=Phase::InWorld || character_id==0 || !self.selected_state.as_ref().is_some_and(|s|s.character.id==character_id);
                    if invalid {
                        self.emit(Event::JurySummonReplyRejected{character_id,trial_id,court,jury_number,message:"jury summon reply requires the actual active selected character header".into()});
                    } else if self.send_game(game::ClientMsg::JurySummoned{trial_id,court:court as i32,jury_number}).await {
                        self.emit(Event::JurySummonReplyRequested{character_id,trial_id,court,jury_number});
                    } else {
                        self.emit(Event::JurySummonReplyRejected{character_id,trial_id,court,jury_number,message:"jury summon reply was not sent".into()});
                    }
                }
            },
            M::Crime(e) => self.emit(Event::Crime(e)),
            M::Appellation(e) => self.emit(Event::Appellation(e)),
            M::Portal(e) => {
                // `SCNaviTeleport` (a navi doodad): the 1.2 client moves the player as for `SCTeleportUnit` (reason 0, yaw 0)
                // and answers `CSTeleportEnded`, the server holds movement until then
                if let crate::portal::PortalEvent::NaviTeleport { pos } = e {
                    self.send_game(game::ClientMsg::TeleportEnded { pos }).await;
                    self.emit(Event::Teleported { pos, yaw: 0.0, reason: 0 });
                }
                self.emit(Event::Portal(e));
            }
            M::Duel(e) => self.emit(Event::Duel(e)),
            M::Emote(e) => self.emit(Event::Emote(e)),
            M::Salon(e) => self.emit(Event::Salon(e)),
            M::Notice(e) => self.emit(Event::Notice(e)),
            M::UnitFx(e) => self.emit(Event::UnitFx(e)),
            M::Account(e) => self.emit(Event::Account(e)),
            M::CashShop(e) => self.emit(Event::CashShop(e)),
            M::Dice(e) => self.emit(Event::Dice(e)),
            M::ChatSpamDelay(e) => self.emit(Event::ChatSpamDelay(e)),
            M::GameType(e) => self.emit(Event::GameType(e)),
            M::Bounty(e) => self.emit(Event::Bounty(e)),
            M::Ranking(e) => self.emit(Event::Ranking(e)),
            M::Achievement(e) => self.emit(Event::Achievement(e)),
            M::Expedition(e) => self.emit(Event::Expedition(e)),
            M::Family(e) => self.emit(Event::Family(e)),
            M::UnitGmModeChanged(e) => self.emit(Event::UnitGmModeChanged(e)),
            M::UnitPvpPointsChanged(e) => self.emit(Event::UnitPvpPointsChanged(e)),
            M::ScheduledEventStarted(e) => self.emit(Event::ScheduledEventStarted(e)),
            M::Actability(e) => self.emit(Event::Actability(e)),
            M::InitialConfig(config) => self.emit(Event::InitialConfig(config)),
            M::CraftItemUnlocked(e) => self.emit(Event::CraftItemUnlocked(e)),
            M::CraftFailureNotice(e) => self.emit(Event::CraftFailureNotice(e)),
            M::LoadInstance(e) => self.emit(Event::LoadInstance(e)),
            M::BlinkUnit(e) => self.emit(Event::BlinkUnit(e)),
            M::ItemLifespan(e) => self.emit(Event::ItemLifespan(e)),
            M::ItemDetailUpdated(e) => self.emit(Event::ItemDetailUpdated(e)),
            M::ItemOperationError(e) => self.emit(Event::ItemOperationError(e)),
            M::Gimmick(e) => self.emit(Event::Gimmick(e)),
            M::Housing(e) => self.emit(Event::Housing(e)),
            M::Farm(e) => self.emit(Event::Farm(e)),
            M::Coffer(e) => self.emit(Event::Coffer(e)),
            M::Radar(e) => self.emit(Event::Radar(e)),
            M::UiData(e) => self.emit(Event::UiData(crate::ui_data::UiDataEvent::Response(e))),
            M::EnvDamage(e) => self.emit(Event::EnvDamage(e)),
            M::SetBreath { breath } => self.emit(Event::SetBreath { breath }),
            M::UnderWater { start } => self.emit(Event::UnderWater { start }),
            M::ConflictZoneState(state) => self.emit(Event::ConflictZoneState(state)),
            M::ForceAttackSet { obj_id, on } => self.emit(Event::ForceAttackSet { obj_id, on }),
            M::UnitInvisible { obj_id, invisible } => self.emit(Event::UnitInvisible { obj_id, invisible }),
            M::Mate(e) => self.emit(Event::Mate(e)),
            M::DoodadBond(e) => {
                if self.phase == Phase::InWorld { self.emit(Event::DoodadBond(e)); }
            }
            M::Slave(e) => self.emit(Event::Slave(e)),
            M::Transport(e) => self.emit(Event::Transport(e)),
            M::Auction(e) => self.emit(Event::Auction(e)),
            M::Trade(e) => {
                if let crate::trade::TradeEvent::Made(tasks) = &e {
                    self.emit(Event::ItemsChanged(tasks.clone()));
                }
                self.emit(Event::Trade(e));
            }
            M::Mail(e) => self.emit(Event::Mail(e)),
            M::Friend(e) => self.emit(Event::Friend(e)),
            M::SkillLearned { skill_id, level } => self.emit(Event::SkillLearned { skill_id, level }),
            M::SkillUpgraded { skill_id, level } => self.emit(Event::SkillUpgraded { skill_id, level }),
            M::SkillsReset { obj_id, ability } => self.emit(Event::SkillsReset { obj_id, ability }),
            M::UnitDamaged(d) => self.emit(Event::UnitDamaged(d)),
            M::UnitHealed(h) => self.emit(Event::UnitHealed(h)),
            M::CombatText { source, target, text_type } => {
                self.emit(Event::CombatText { source, target, text_type });
            }
            M::PlotEvent(e) => self.emit(Event::PlotEvent(e)),
            M::PlotEnded { tl } => self.emit(Event::PlotEnded { tl }),
            M::PlotStopped { tl, duration_ms, last_event, channeling } => {
                self.emit(Event::PlotStopped { tl, duration_ms, last_event, channeling });
            }
            M::Batch(_) => {} // flattened by `game_msg`
            M::BuffCreated { obj_id, buff } => self.emit(Event::BuffCreated { obj_id, buff }),
            M::BuffRemoved { obj_id, index } => self.emit(Event::BuffRemoved { obj_id, index }),
            M::BuffUpdated { obj_id, index, stacks, charged } => self.emit(Event::BuffUpdated { obj_id, index, stacks, charged }),
            M::ScheduleItems {declared_count,items} => self.emit(Event::ScheduleItems{declared_count,items}),
            M::ScheduleItemSent {schedule_type,raw_by_mail} => self.emit(Event::ScheduleItemSent{schedule_type,raw_by_mail}),
            M::ItemUccDataChanged {ucc_id,player_id,target_item_id} => self.emit(Event::ItemUccDataChanged{ucc_id,player_id,target_item_id}),
            M::GradeEnchantResult(result) => self.emit(Event::GradeEnchantResult(result)),
            M::GradeEnchantBroadcast(result) => self.emit(Event::GradeEnchantBroadcast(result)),
            M::LunagemSocketingResult { raw_result, item_id, template_id, raw_install } => self.emit(Event::LunagemSocketingResult { raw_result, item_id, template_id, raw_install }),
            M::LunastoneSocketingResult { raw_result, item_id, template_id } => self.emit(Event::LunastoneSocketingResult { raw_result, item_id, template_id }),
            M::BuffLearned { obj_id, buff_id } => self.emit(Event::PassiveBuffLearned { obj_id, buff_id }),
            M::Teleport { pos, yaw, reason, .. } => {
                self.send_game(game::ClientMsg::TeleportEnded { pos }).await;
                self.emit(Event::Teleported { pos, yaw, reason });
            }
            M::DoodadsCreated(v) => self.emit(Event::DoodadsSpawned(v)),
            M::DoodadsRemoved(v) => self.emit(Event::DoodadsRemoved(v)),
            M::DoodadChanged { obj_id, data } => self.emit(Event::DoodadChanged { obj_id, data }),
            M::DoodadOriginator { obj_id, new_owner_id, new_faction } => self.emit(Event::DoodadOriginator { obj_id, new_owner_id, new_faction }),
            M::DoodadSound { obj_id, sound_id } => self.emit(Event::DoodadSound { obj_id, sound_id }),
            M::VegetationCutdown { unit_obj_id, doodad_obj_id } => self.emit(Event::VegetationCutdown { unit_obj_id, doodad_obj_id }),
            M::DoodadPhaseChanged { obj_id, func_group_id, time_left, puzzle_group, item_template_id } => {
                self.emit(Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, puzzle_group, item_template_id });
            }
            M::Chat(m) => self.emit(Event::Chat(m)),
            M::ErrorMsg { code, .. } => self.emit(Event::ServerError { code }),
            M::Kicked { reason, message } => self.emit(Event::Kicked { reason, message }),
            M::PrepareLeaveWorld { time_ms, target, .. } => {
                self.set_phase(Phase::Leaving);
                self.emit(Event::LeavePending { time_ms, target });
            }
            M::LeaveWorldCanceled => {
                self.set_phase(Phase::InWorld);
                self.emit(Event::LeaveCancelled);
            }
            M::LeaveWorldGranted { target } => self.left_world(target).await,
            M::ReconnectAuth { token } => self.reconnect(token).await,
            M::Pong(pong) => {
                self.emit(Event::ClockSample(crate::clock::ClockSample{pong,local_receive_ms,received_at,local_origin:self.started}));
                self.emit(Event::Pong(pong));
            },
            M::SlotCount(count) => self.emit(Event::CharacterSlotCount(count)),
            M::AccountInfo { pay_method, pay_location, pay_start, pay_end, real_pay_time } => {
                self.emit(Event::AccountPayment(crate::account::AccountPayment {
                    pay_method, pay_location, pay_start, pay_end, real_pay_time,
                }));
            }
            M::DecodeFailed {message} => self.emit(Event::GameDecodeFailed{message}),
            M::Unhandled { level, opcode, len } => {
                tracing::trace!("unhandled game packet level {level} opcode {opcode:#05x} ({len} bytes)");
                self.emit(Event::UnhandledGamePacket {level,opcode,len});
            }
        }
    }

    async fn send_auction(&mut self,command:crate::auction::AuctionCommand){
        if let Err(message)=command.validate(){self.emit(Event::Auction(crate::auction::AuctionEvent::RequestRejected{command,message:message.into()}));}
        else if self.send_game(game::ClientMsg::Auction(command.clone())).await{self.emit(Event::Auction(crate::auction::AuctionEvent::Requested(command)));}
        else{self.emit(Event::Auction(crate::auction::AuctionEvent::RequestRejected{command,message:"auction request was not sent".into()}));}
    }

    async fn start_skill(&mut self, skill_id: u32, caster: StartCaster, target: SkillTarget) {
        if self.phase != Phase::InWorld {
            tracing::warn!("skill {skill_id} ignored: not in the world");
            return;
        }
        let self_obj_id = self.self_obj;
        self.send_game(game::ClientMsg::StartSkill { skill_id, self_obj_id, caster, target }).await;
    }

    /// The CryNetwork handshake finished: the lobby is usable.
    async fn lobby_ready(&mut self) {
        self.waiting = None;
        self.set_phase(Phase::CharacterSelect);
        self.emit(Event::LobbyReady);
        if self.stream.is_none() {
            self.join_stream().await;
        }
        self.command(Command::RefreshCharacters).await;
    }

    async fn join_stream(&mut self) {
        let (Some((_, cookie)), Some(account)) = (self.cookie, self.account) else { return };
        let addr = format!("{}:{}", cookie.addr.ip(), self.stream_port);
        match StreamConn::connect(&addr).await {
            Ok(mut c) => {
                let join = stream::ClientMsg::Join {
                    account_id: account.account_id,
                    token: self.stream_token,
                };
                if let Err(e) = c.send(&join).await {
                    tracing::warn!("stream join failed: {e}");
                } else {
                    self.stream = Some(c);
                }
            }
            Err(e) => tracing::warn!("cannot reach the stream server {addr}: {e}"),
        }
    }

    fn stream_msg(&mut self, r: net::Result<stream::ServerMsg>) {
        match r {
            Ok(stream::ServerMsg::CharacterName(row)) => {if row.name.len()<=128 {self.pending_names.remove(&row.character_id);}self.emit(Event::CharacterName(row));}
            Ok(stream::ServerMsg::JoinResponse { code }) => {
                self.stream_joined=code==0;
                if code != 0 {
                    self.pending_names.clear();self.emit(Event::CharacterNameStreamClosed);
                    tracing::warn!("stream server refused the join (code {code})");
                }
            }
            Ok(stream::ServerMsg::Other { opcode, len }) => {
                tracing::trace!("stream packet {opcode:#05x} ({len} bytes)");
            }
            Err(NetError::Wire(e)) => tracing::warn!("undecodable stream packet: {e}"),
            Err(e) => {
                tracing::debug!("stream connection closed: {e}");
                self.clear_character_name_stream();self.stream = None;
            }
        }
    }

    async fn unit_state(&mut self, unit: Unit) {
        let is_self = self.phase == Phase::EnteringWorld
            && self.self_obj == 0
            && matches!(unit.kind, UnitKind::Character { char_id } if char_id == self.selected);
        if !is_self {
            // the server announces the player to itself only when it spawns; a later copy must not become a second figure
            if self.self_obj != 0 && unit.obj_id == self.self_obj {
                if self.phase == Phase::InWorld && self.selected != 0
                    && matches!(unit.kind, UnitKind::Character { char_id } if char_id == self.selected)
                    && self.selected_state.as_ref().is_some_and(|s| s.character.id == self.selected) {
                    self.emit(Event::OwnUnitSnapshot(Box::new(unit)));
                }
                return;
            }
            self.emit(Event::UnitSpawned(Box::new(unit)));
            return;
        }
        let Some(state) = self.selected_state.clone() else { return };
        self.waiting = None;
        self.self_obj = unit.obj_id;
        self.set_phase(Phase::InWorld);
        self.emit(Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: unit.obj_id,
            state,
            unit,
        })));
        self.send_game(game::ClientMsg::NotifyInGame).await;
        self.send_game(game::ClientMsg::NotifyInGameCompleted).await;
        self.emit(Event::EnteredWorld);
    }

    async fn left_world(&mut self, target: u8) {
        self.self_obj = 0;
        self.selected = 0;
        self.selected_state = None;
        self.emit(Event::LeftWorld { target });
        match LeaveTarget::from_u8(target) {
            Some(LeaveTarget::CharacterSelect) => {
                // The server follows with ChangeState(0); the handshake runs again.
                self.set_phase(Phase::Connecting);
                self.wait_for("the lobby");
            }
            _ => {
                self.close_all().await;
                self.waiting = None;
                self.set_phase(Phase::Idle);
                self.emit(Event::Disconnected { message: "logged out".into() });
            }
        }
    }

    /// `SCReconnectAuth`: continue on a fresh login connection and show the world list again.
    async fn reconnect(&mut self, token: u32) {
        let (Some(account), Some((world_id, _))) = (self.account, self.cookie) else { return };
        if let Some(mut c) = self.game.take() {
            c.shutdown().await;
        }
        self.clear_character_name_stream();
        if let Some(mut c) = self.stream.take() {
            c.shutdown().await;
        }
        if let Some(mut c) = self.login.take() {
            c.shutdown().await;
        }
        self.cookie = None;
        self.afs = None;
        self.set_phase(Phase::Authenticating);
        self.wait_for("the login server");
        match LoginConn::connect(&self.login_addr).await {
            Ok(c) => self.login = Some(c),
            Err(e) => {
                self.waiting = None;
                self.set_phase(Phase::Idle);
                self.emit(Event::LoginFailed { message: format!("cannot reach the login server: {e}") });
                return;
            }
        }
        self.send_login(login::ClientMsg::Reconnect {
            account_id: account.account_id,
            world_id,
            token,
        })
        .await;
    }
}

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{alertness, FALL_VELOCITY_PER_MPS};

    #[tokio::test]
    async fn force_attack_requests_wait_for_authoritative_echo_and_never_toggle_sent_state() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session.self_obj = 7;
        session.command(Command::SetForceAttack { on: true }).await;
        assert!(
            events.try_recv().is_err(),
            "without a socket no Requested event is possible"
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        session.game = Some(connection.unwrap());
        for (phase, self_obj) in [
            (Phase::CharacterSelect, 7),
            (Phase::EnteringWorld, 7),
            (Phase::Leaving, 7),
            (Phase::InWorld, 0),
        ] {
            session.phase = phase;
            session.self_obj = self_obj;
            session.command(Command::SetForceAttack { on: true }).await;
            assert!(
                matches!(events.try_recv(), Ok(Event::ForceAttackRejected { on: true, message }) if !message.is_empty())
            );
        }
        session.phase = Phase::InWorld;
        session.self_obj = 7;
        let mut state = crate::state::WorldState::default();
        let mut character = CharacterState::default();
        character.character.id = 17;
        state.apply(&Event::CharacterSelected(Box::new(character.clone())));
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: character,
            unit: crate::state::tests::unit(7),
        })));
        assert_eq!(
            state.combat_status.get(7).unwrap().force_attack,
            Some(false)
        );
        for (counter, on) in [true, true, false].into_iter().enumerate() {
            // Repeated native toggles before an echo read the same old byte. Explicit requests
            // neither alternate a guessed local value nor optimistically update this cache.
            session.command(Command::SetForceAttack { on }).await;
            let event = events.try_recv().unwrap();
            assert!(matches!(event, Event::ForceAttackRequested { on: actual } if actual == on));
            state.apply(&event);
            let mut length = [0; 2];
            peer.read_exact(&mut length).await.unwrap();
            let mut received = vec![0; u16::from_le_bytes(length) as usize];
            peer.read_exact(&mut received).await.unwrap();
            assert_eq!(
                received,
                [0xdd, 1, 0, counter as u8, 0x4f, 0, u8::from(on)],
                "no rejected request reaches the socket"
            );
            assert_eq!(
                state.combat_status.get(7).unwrap().force_attack,
                Some(false)
            );
            assert!(!state.stats.in_combat);
            assert!(
                events.try_recv().is_err(),
                "no synthetic ForceAttackSet or Lua acknowledgement"
            );
        }
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::ForceAttackSet {
                    obj_id: 7,
                    on: true,
                },
                game::ServerMsg::ForceAttackSet {
                    obj_id: 7,
                    on: true,
                },
                game::ServerMsg::ForceAttackSet {
                    obj_id: 7,
                    on: false,
                },
                game::ServerMsg::Snow(true),
            ])))
            .await;
        for on in [true, true, false] {
            let event = events.try_recv().unwrap();
            assert!(
                matches!(event, Event::ForceAttackSet { obj_id: 7, on: actual } if actual == on)
            );
            state.apply(&event);
            assert_eq!(state.combat_status.get(7).unwrap().force_attack, Some(on));
        }
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn combat_status_session_preserves_reports_and_batch_tail_without_predicting_points() {
        use crate::combat_status::EnvironmentDamage;
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        let mut state = crate::state::WorldState::default();
        let mut character = CharacterState::default();
        character.character.id = 17;
        state.apply(&Event::CharacterSelected(Box::new(character.clone())));
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: character,
            unit: crate::state::tests::unit(7),
        })));
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))));
        let damage = EnvironmentDamage {
            source: 2,
            target_obj_id: 7,
            amount: u32::MAX,
            gimmick_id: None,
        };
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::EnvDamage(damage),
                game::ServerMsg::ForceAttackSet {
                    obj_id: 99,
                    on: true,
                },
                game::ServerMsg::UnitInvisible {
                    obj_id: 8,
                    invisible: true,
                },
                game::ServerMsg::SetBreath { breath: 999 },
                game::ServerMsg::ForceAttackSet {
                    obj_id: 7,
                    on: false,
                },
                game::ServerMsg::UnitPoints {
                    obj_id: 7,
                    hp: 6,
                    mp: 2,
                },
                game::ServerMsg::Snow(true),
            ])))
            .await;
        for index in 0..7 {
            let event = events.try_recv().unwrap();
            match (index, &event) {
                (0, Event::EnvDamage(actual)) => assert_eq!(*actual, damage),
                (
                    1,
                    Event::ForceAttackSet {
                        obj_id: 99,
                        on: true,
                    },
                ) => {}
                (
                    2,
                    Event::UnitInvisible {
                        obj_id: 8,
                        invisible: true,
                    },
                ) => {}
                (3, Event::SetBreath { breath: 999 }) => {}
                (
                    4,
                    Event::ForceAttackSet {
                        obj_id: 7,
                        on: false,
                    },
                ) => {}
                (
                    5,
                    Event::UnitPoints {
                        obj_id: 7,
                        hp: 6,
                        mp: 2,
                    },
                ) => {}
                (6, Event::Snow(true)) => {}
                _ => panic!("unexpected event at {index}: {event:?}"),
            }
            state.apply(&event);
            assert_eq!(state.stats.hp, if index < 5 { 10 } else { 6 });
        }
        assert!(events.try_recv().is_err());
        assert_eq!(state.combat_status.breath, Some(999));
        assert_eq!(
            state.combat_status.get(7).unwrap().force_attack,
            Some(false)
        );
        assert_eq!(state.combat_status.get(8).unwrap().invisible, Some(true));
        assert!(state.combat_status.get(99).is_none());
        assert!(state.units.get(8).is_some());
    }

    #[tokio::test]
    async fn ui_data_session_validates_save_identity_and_only_caches_received_responses() {
        use crate::ui_data::{UiDataCommand, UiDataEvent, UiDataResponse};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session.selected = 7;
        let save = UiDataCommand::Save {
            character_id: 7,
            data_type: 5,
            payload: "x\u{00e9}\u{1f600}".into(),
        };
        session.command(Command::UiData(save.clone())).await;
        assert!(
            events.try_recv().is_err(),
            "no connection cannot produce Requested"
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&address), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        session.game = Some(connection.unwrap());
        for (phase, command) in [
            (
                Phase::Idle,
                UiDataCommand::Request {
                    character_id: 7,
                    data_type: 5,
                },
            ),
            (
                Phase::InWorld,
                UiDataCommand::Request {
                    character_id: 0,
                    data_type: 5,
                },
            ),
            (Phase::CharacterSelect, save.clone()),
            (
                Phase::InWorld,
                UiDataCommand::Save {
                    character_id: 8,
                    data_type: 5,
                    payload: "other".into(),
                },
            ),
            (
                Phase::InWorld,
                UiDataCommand::Save {
                    character_id: 7,
                    data_type: 5,
                    payload: "a\0b".into(),
                },
            ),
            (
                Phase::InWorld,
                UiDataCommand::Save {
                    character_id: 7,
                    data_type: 5,
                    payload: "a".repeat(8192),
                },
            ),
        ] {
            session.phase = phase;
            session.command(Command::UiData(command.clone())).await;
            assert!(
                matches!(events.try_recv(), Ok(Event::UiData(UiDataEvent::RequestRejected { command: actual, message })) if actual == command && !message.is_empty())
            );
        }
        let mut state = crate::state::WorldState::default();
        // Requests may name a lobby character. The lane alone checks connection ownership and
        // silently ignores an unowned ID; a local socket write must never fabricate its response.
        for (index, command) in [
            UiDataCommand::Request {
                character_id: 7,
                data_type: 0xffff,
            },
            UiDataCommand::Request {
                character_id: 8,
                data_type: 2,
            },
            save.clone(),
        ]
        .into_iter()
        .enumerate()
        {
            session.phase = if index == 2 {
                Phase::InWorld
            } else {
                Phase::CharacterSelect
            };
            session.command(Command::UiData(command.clone())).await;
            let event = events.try_recv().unwrap();
            assert!(
                matches!(&event, Event::UiData(UiDataEvent::Requested(actual)) if *actual == command)
            );
            state.apply(&event);
            let mut length = [0; 2];
            peer.read_exact(&mut length).await.unwrap();
            let mut frame = vec![0; u16::from_le_bytes(length) as usize];
            peer.read_exact(&mut frame).await.unwrap();
            assert_eq!(
                frame,
                game::ClientMsg::UiData(command).encode(index as u8),
                "rejected commands must not reach the socket"
            );
            assert!(
                state.ui_data.entries().is_empty(),
                "a save has no ACK or optimistic cache write"
            );
            assert!(events.try_recv().is_err());
        }
        let values = [
            UiDataResponse {
                character_id: 7,
                data_type: 5,
                payload: "old".into(),
                raw_length: 4,
            },
            UiDataResponse {
                character_id: 8,
                data_type: 2,
                payload: "other".into(),
                raw_length: 6,
            },
            UiDataResponse {
                character_id: 7,
                data_type: 5,
                payload: "received".into(),
                raw_length: -1,
            },
        ];
        session
            .game_msg(Ok(game::ServerMsg::Batch(
                values
                    .iter()
                    .cloned()
                    .map(game::ServerMsg::UiData)
                    .chain([game::ServerMsg::UnitPoints {
                        obj_id: 99,
                        hp: 5,
                        mp: 6,
                    }])
                    .collect(),
            )))
            .await;
        for value in &values {
            let event = events.try_recv().unwrap();
            assert!(
                matches!(&event, Event::UiData(UiDataEvent::Response(actual)) if actual == value)
            );
            state.apply(&event);
        }
        assert_eq!(state.ui_data.get(7, 5), Some(&values[2]));
        assert_eq!(state.ui_data.get(8, 2), Some(&values[1]));
        assert!(matches!(
            events.try_recv(),
            Ok(Event::UnitPoints {
                obj_id: 99,
                hp: 5,
                mp: 6
            })
        ));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn transport_equipment_notices_forward_raw_without_completion_events() {
        use crate::mate::MateEvent;
        use crate::slave::SlaveEvent;
        let mate=MateEvent::EquipmentExpired{mate_tl_id:7,container:252,slot:3};
        let mut raw=crate::wire::Writer::new();raw.u32(1).u16(7).u32(2).bool(false).u32(0).u32(0).raw(&[242,1,2,3]).bool(true);
        let slave=SlaveEvent::read(crate::opcodes::sc::SCSlaveEquipmentChangedPacket,&mut crate::wire::Reader::new(&raw.finish())).unwrap();
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::Mate(mate.clone()),game::ServerMsg::Mate(mate.clone()),game::ServerMsg::Slave(slave.clone()),game::ServerMsg::Snow(true)]))).await;
        for _ in 0..2 { assert!(matches!(events.try_recv(),Ok(Event::Mate(actual)) if actual==mate)); }
        assert!(matches!(events.try_recv(),Ok(Event::Slave(actual)) if actual==slave));
        assert!(matches!(events.try_recv(),Ok(Event::Snow(true))));assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn npc_status_receipts_forward_once_in_order_without_synthetic_updates() {
        use crate::interaction::InteractEvent;
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session.selected = 7;
        session.self_obj = 0x445566;
        let mut state = crate::state::WorldState::default();
        let original_unit = crate::state::tests::unit(0x112233);
        state.units.by_id.insert(original_unit.obj_id, original_unit.clone());
        let original_bonds = session.received_doodad_bonds.borrow().clone();
        let fixture = include_bytes!("../tests/fixtures/npc-status/sc080-four-statuses-then-points.dd04.frame.bin");
        session.game_msg(game::ServerMsg::decode(&fixture[2..]).map_err(Into::into)).await;
        for raw in [0, 1, 2, 255] {
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::Interaction(InteractEvent::NpcStatusChanged {
                npc_obj_id: 0x112233, status_wire_byte,
            }) if *status_wire_byte == raw));
            state.apply(&event);
            assert_eq!(state.units.get(0x112233), Some(&original_unit));
            assert_eq!(state.units.by_id.len(), 1);
            assert!(state.player.spawn.is_none());
            assert!(state.doodads.by_id.is_empty());
            assert!(state.slaves.created.is_empty());
        }
        assert!(matches!(events.try_recv(), Ok(Event::UnitPoints { obj_id: 0xaabbcc, hp: 123, mp: -456 })));
        assert!(events.try_recv().is_err(), "each receipt is forwarded once, with no synthetic events");
        assert_eq!(session.phase, Phase::InWorld);
        assert_eq!(session.selected, 7);
        assert_eq!(session.self_obj, 0x445566);
        assert!(session.selected_state.is_none());
        assert_eq!(*session.received_doodad_bonds.borrow(), original_bonds);

        // An existing SC07b false skill-list receipt remains distinct from SC080.
        let list = InteractEvent::SkillList {
            npc_obj_id: 0x112233, obj_id: 0x445566, extra_info: 0, pick_id: 0,
            mouse_button: 0, skills: Vec::new(), interactable: false, modifier_keys: 0,
        };
        session.game_msg(Ok(game::ServerMsg::Interaction(list.clone()))).await;
        assert!(matches!(events.try_recv(), Ok(Event::Interaction(actual)) if actual == list));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn world_interaction_reports_forward_without_opening_or_casting() {
        use crate::interaction::{InteractEvent,WorldInteractionSkillList};
        let list=InteractEvent::WorldSkillList(WorldInteractionSkillList {
            outer_obj_ids:[1,2],context_obj_id:3,doodad_obj_id:4,interaction_type:5,pick_obj_id:6,
            raw_count:1,extra_info:7,skills:vec![22678],mouse_button:1,modifier_keys:0,
        });
        let cancel=InteractEvent::WorldCanceled{unit_obj_id:0,doodad_obj_id:4};
        let (evt,mut events)=unbounded_channel();
        let mut session=Session::new(evt,ClientOptions::default());
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::Interaction(list.clone()),game::ServerMsg::Interaction(list.clone()),game::ServerMsg::Interaction(cancel.clone()),game::ServerMsg::Snow(true)]))).await;
        for expected in [list.clone(),list,cancel] { assert!(matches!(events.try_recv(),Ok(Event::Interaction(actual)) if actual==expected)); }
        assert!(matches!(events.try_recv(),Ok(Event::Snow(true))));assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn native_house_posture_reports_preserve_the_old_event_and_raw_housing_event() {
        use crate::unit_msg::Posture;
        use crate::housing::HousingEvent;
        let raw=[1,0,1,0,2,3,4,5,6,7];
        let posture=Posture::read(&mut crate::wire::Reader::new(&raw)).unwrap();
        let (evt,mut events)=unbounded_channel();
        let mut session=Session::new(evt,ClientOptions::default());
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::UnitPosture{obj_id:99,posture},game::ServerMsg::Snow(true)]))).await;
        assert!(matches!(events.try_recv(),Ok(Event::UnitPosture{obj_id:99,kind:1,anim_action:0,activate:false})));
        assert!(matches!(events.try_recv(),Ok(Event::Housing(HousingEvent::UnitPosture{obj_id:99,posture:house})) if house == posture.house.unwrap()));
        assert!(matches!(events.try_recv(),Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn transfer_radar_reports_forward_repeated_raw_chunks_without_creating_units() {
        use crate::radar::{RadarEvent, RadarToggle};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let toggle = RadarEvent::TransferToggled(RadarToggle { on: true, range: 500.0 });
        let chunk = RadarEvent::TransferContacts { last: true, contacts: vec![] };
        session.game_msg(Ok(game::ServerMsg::Batch(vec![
            game::ServerMsg::Radar(toggle.clone()), game::ServerMsg::Radar(chunk.clone()),
            game::ServerMsg::Radar(chunk.clone()),
            game::ServerMsg::Radar(RadarEvent::ShipToggled(RadarToggle { on: false, range: 15.0 })),
            game::ServerMsg::Radar(RadarEvent::ShipContacts { last: true, contacts: vec![] }),
            game::ServerMsg::Radar(RadarEvent::FishToggled(RadarToggle { on: false, range: 15.0 })),
            game::ServerMsg::Radar(RadarEvent::FishContacts { last: true, contacts: vec![] }),
            game::ServerMsg::Snow(true),
        ]))).await;
        for expected in [toggle, chunk.clone(), chunk] {
            assert!(matches!(events.try_recv(), Ok(Event::Radar(actual)) if actual == expected));
        }
        assert!(matches!(events.try_recv(), Ok(Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: false, range: 15.0 })))));
        assert!(matches!(events.try_recv(), Ok(Event::Radar(RadarEvent::ShipContacts { last: true, contacts })) if contacts.is_empty()));
        assert!(matches!(events.try_recv(), Ok(Event::Radar(RadarEvent::FishToggled(RadarToggle { on: false, range: 15.0 })))));
        assert!(matches!(events.try_recv(), Ok(Event::Radar(RadarEvent::FishContacts { last: true, contacts })) if contacts.is_empty()));
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn battlefield_commands_and_receipts_preserve_write_outcomes_and_event_order() {
        use crate::battlefield::{BattlefieldCommand as C, BattlefieldEvent as E, BattlefieldQueue};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let commands = [C::Apply { battlefield_id: 7, corps: 2 }, C::Cancel];
        for phase in [Phase::Idle, Phase::CharacterSelect, Phase::InWorld] {
            session.phase = phase;
            for command in commands {
                session.command(Command::Battlefield(command)).await;
                assert!(matches!(events.try_recv(), Ok(Event::Battlefield(E::RequestRejected { command: actual, .. })) if actual==command));
            }
        }
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr=listener.local_addr().unwrap().to_string();
        let (connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());
        let (mut peer,_)=accepted.unwrap(); session.game=Some(connection.unwrap());
        for command in commands {
            session.command(Command::Battlefield(command)).await;
            assert!(matches!(events.try_recv(), Ok(Event::Battlefield(E::Requested(actual))) if actual==command));
            let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();
            let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
            assert_eq!(&frame[6..],game::ClientMsg::Battlefield(command).encode_body());
        }
        let expected=[E::Applied { battlefield_id:7,corps:2,error:0 },E::QueueState(BattlefieldQueue { battlefield_id:7,corps_size:5,counts:[1,2,3] }),E::Cancelled{raw:0x12345678}];
        session.game_msg(Ok(game::ServerMsg::Batch(expected.iter().cloned().map(game::ServerMsg::Battlefield).collect()))).await;
        for e in expected {assert!(matches!(events.try_recv(),Ok(Event::Battlefield(actual)) if actual==e));}
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn battlefield_reply_is_checked_again_at_socket_write_and_does_not_invent_join() {
        use crate::battlefield::{BattlefieldCommand as C, BattlefieldEvent as E, BattlefieldInvitation};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        let invite=BattlefieldInvitation {zone_id:8,instance_id:9,ruleset_id:10,corps:1,qualified_id:55};
        let reply=C::RespondToInvitation {invitation:invite,accept:true};
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr=listener.local_addr().unwrap().to_string();
        let (connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());
        let (mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        session.command(Command::Battlefield(reply)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::RequestRejected{..}))));
        session.emit(Event::Battlefield(E::Requested(C::Apply{battlefield_id:7,corps:2})));
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::Applied{battlefield_id:7,corps:2,error:0}))).await;
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::Invited(invite)))).await;
        while events.try_recv().is_ok() {}
        // A UI-captured reply can become stale before it reaches the network task.
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::InviteTimedOut{zone_id:8,instance_id:9,ruleset_id:10,corps:1}))).await;
        events.try_recv().unwrap();
        session.command(Command::Battlefield(reply)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::RequestRejected{..}))));
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::Invited(invite)))).await;
        events.try_recv().unwrap();
        session.command(Command::Battlefield(reply)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::Requested(c))) if c==reply));
        let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();
        let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[6..],game::ClientMsg::Battlefield(reply).encode_body());
        assert!(session.battlefield.borrow().joined.is_none());
        let ready=session.battlefield.borrow().world_ready(invite).unwrap();
        session.command(Command::Battlefield(ready)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::Requested(c))) if c==ready));
        let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();
        let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[6..],game::ClientMsg::Battlefield(ready).encode_body());
        assert!(session.battlefield.borrow().joined.is_none());
        session.command(Command::Battlefield(reply)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::RequestRejected{..}))));
        assert!(tokio::time::timeout(Duration::from_millis(30),peer.read_u8()).await.is_err());
    }

    #[tokio::test]
    async fn battlefield_result_receipt_clock_gate_and_leave_reset_reach_session_in_order() {
        use crate::battlefield::{BattlefieldCommand as C, BattlefieldEvent as E, BattlefieldInvitation, BattlefieldJoined, BattlefieldStart, BattlefieldEnd, BattlefieldTeamResult};
        let (evt, mut events)=unbounded_channel(); let mut session=Session::new(evt,ClientOptions::default()); session.phase=Phase::InWorld;
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap(); let addr=listener.local_addr().unwrap().to_string();
        let (connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());
        let (mut peer,_)=accepted.unwrap(); session.game=Some(connection.unwrap());
        let i=BattlefieldInvitation {zone_id:8,instance_id:9,ruleset_id:10,corps:1,qualified_id:55};
        for e in [E::Requested(C::Apply{battlefield_id:7,corps:2}),E::Applied{battlefield_id:7,corps:2,error:0},E::Invited(i),E::Requested(C::RespondToInvitation{invitation:i,accept:true}),
            E::Joined(BattlefieldJoined{zone_id:8,instance_id:9,corps:1,victory_score:1,victory_kill_count:2,victory_kill_corps_head:3,phase_times_ms:[1,2,3],factions:[4,5],killstreak_skills:[0;24]}),
            E::Started(BattlefieldStart{zone_id:8,instance_id:9,now_ms:20000,start_ms:20000})] {
            session.game_msg(Ok(game::ServerMsg::Battlefield(e))).await;
        }
        let end=BattlefieldEnd{zone_id:8,instance_id:9,reason:0,teams:std::array::from_fn(|_|BattlefieldTeamResult{state:0,faction_id:1,score:0,total_kills:0,total_deaths:0,members:vec![]})};
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::Ended(end.clone())))).await;
        assert_eq!(session.battlefield.borrow().last_end,Some(end.clone()));
        assert!(session.battlefield.borrow().result.is_none()); // Packet now is not the native mapped clock.
        while events.try_recv().is_ok() {}
        session.command(Command::Battlefield(C::Leave)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::RequestRejected{command:C::Leave,..}))));
        assert!(tokio::time::timeout(Duration::from_millis(30),peer.read_u8()).await.is_err());
        // A genuine Pong is captured and mapped by the backend before subsequent packets/actions.
        session.game_msg(Ok(game::ServerMsg::Pong(crate::proxy::ProxyPong{tm:0,when:0,elapsed:0,remote:14000000,local:0,world:14000}))).await;
        assert!(matches!(events.try_recv(),Ok(Event::ClockSample(_))));
        assert!(matches!(events.try_recv(),Ok(Event::Pong(_))));
        assert_eq!(session.battlefield.borrow().phase,Some(crate::battlefield::BattlefieldPhase::Playing));
        session.game_msg(Ok(game::ServerMsg::Battlefield(E::Ended(end)))).await;
        events.try_recv().unwrap(); session.command(Command::Battlefield(C::Leave)).await;
        assert!(matches!(events.try_recv(),Ok(Event::Battlefield(E::Requested(C::Leave)))));
        let mut length=[0;2]; peer.read_exact(&mut length).await.unwrap(); let mut frame=vec![0;u16::from_le_bytes(length) as usize]; peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[4..6],&0xe5u16.to_le_bytes()); assert!(frame[6..].is_empty());
        assert!(session.battlefield.borrow().in_instant_game());
        let load=crate::world_msg::InstanceLoad{raw_id:1,zone_id:2,pos:Default::default(),angles:[0.;3]};
        session.game_msg(Ok(game::ServerMsg::LoadInstance(load))).await;
        assert_eq!(*session.battlefield.borrow(),crate::battlefield::Battlefield::default());
    }

    #[tokio::test]
    async fn pong_forwards_raw_clock_fields_without_mutating_received_battlefield_state() {
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        let pong=crate::proxy::ProxyPong{tm:-1,when:2,elapsed:3,remote:4000,local:u32::MAX,world:4};
        let before=session.battlefield.borrow().clone();
        session.game_msg(Ok(game::ServerMsg::Pong(pong))).await;
        let Event::ClockSample(sample)=events.try_recv().unwrap() else {panic!("missing backend sample")};
        assert_eq!(sample.pong,pong);assert_eq!(sample.local_origin,session.started);
        assert_eq!(sample.local_receive_ms,sample.received_at.duration_since(sample.local_origin).as_millis() as u32);
        assert!(matches!(events.try_recv(),Ok(Event::Pong(p)) if p==pong));
        assert!(session.server_clock.borrow().mapped_now().is_some());
        assert_eq!(*session.battlefield.borrow(),before);assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn unpack_checks_current_received_identity_at_write_and_waits_for_security_receipt() {
        use crate::{UnpackItemRequest,items::{ItemTask,ItemTasks},model::Item};
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        let request=UnpackItemRequest::new(2,0,55).unwrap();
        session.command(Command::UnpackItem(request)).await;
        assert!(matches!(events.try_recv(),Ok(Event::UnpackItemRequestRejected{request:actual,..}) if actual==request));
        session.phase=Phase::InWorld;session.selected=42;
        session.emit(Event::InventoryInit{bag_slots:1,bank_slots:0});
        let add=|id|ItemTasks{tasks:vec![ItemTask::Add{ raw_detail_block: None,action:1,container:2,slot:0,item:Item{id,template_id:99,flags:0x80,unpack_time:7,..Default::default()}}],..Default::default()};
        session.emit(Event::ItemsChanged(add(55)));while events.try_recv().is_ok(){}
        // Valid snapshot, missing transport: reject with no local unpack success.
        session.command(Command::UnpackItem(request)).await;
        assert!(matches!(events.try_recv(),Ok(Event::UnpackItemRequestRejected{..})));
        assert_eq!(session.received_inventory.borrow().bag[0].as_ref().unwrap().flags,0x80);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        session.emit(Event::ItemsChanged(add(56)));while events.try_recv().is_ok(){}
        session.command(Command::UnpackItem(request)).await;
        assert!(matches!(events.try_recv(),Ok(Event::UnpackItemRequestRejected{..})));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.emit(Event::ItemsChanged(add(55)));while events.try_recv().is_ok(){}
        session.command(Command::UnpackItem(request)).await;
        assert!(matches!(events.try_recv(),Ok(Event::UnpackItemRequested(actual)) if actual==request));
        let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[6..],&[2,0,55,0,0,0,0,0,0,0]);
        assert_eq!(session.received_inventory.borrow().bag[0].as_ref().unwrap().unpack_time,7);
        let update=ItemTasks{reason:114,tasks:vec![ItemTask::Security{container:2,slot:0,item_id:55,bits:0x11,is_unsecure_excess:0,is_unsecure_set:0,is_unpack:1,unsecure_time:0,unpack_time:99}],..Default::default()};
        session.game_msg(Ok(game::ServerMsg::ItemTasks(update.clone()))).await;
        assert!(matches!(events.try_recv(),Ok(Event::ItemsChanged(actual)) if actual==update));
        assert_eq!(session.received_inventory.borrow().bag[0].as_ref().unwrap().unpack_time,99);
        // The unpack acknowledgement never emits an automatic equipment request.
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.game.as_mut().unwrap().shutdown().await;
        session.command(Command::UnpackItem(request)).await;
        let mut rejected=false;while let Ok(e)=events.try_recv(){if matches!(e,Event::UnpackItemRequestRejected{..}){rejected=true}assert!(!matches!(e,Event::UnpackItemRequested(_)));}assert!(rejected);
    }

    #[tokio::test]
    async fn family_receipts_forward_repeated_and_unknown_family_notices_without_local_success() {
        use crate::family::{FamilyEvent,FamilyRoster};
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        let receipt=FamilyEvent::Described(FamilyRoster{family_id:7,members:vec![]});
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::Family(receipt.clone()))).await;
            assert!(matches!(events.try_recv(),Ok(Event::Family(actual)) if actual==receipt));
        }
        let notice=FamilyEvent::MemberOnline{family_id:99,character_id:7,online:true};
        session.game_msg(Ok(game::ServerMsg::Family(notice.clone()))).await;
        assert!(matches!(events.try_recv(),Ok(Event::Family(actual)) if actual==notice));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn complete_scheduled_receipt_updates_only_its_received_ratio_and_forwards_all_fields() {
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        let start=crate::scheduled::ScheduledEventStarted{id:3,values:[-7;21],changes:u32::MAX,priority:0,start_date:1,period_date:2,hour:3,minute:4,period_minutes:5,week_day:6,open_message:b"notice".to_vec()};
        session.game_msg(Ok(game::ServerMsg::ScheduledEventStarted(start.clone()))).await;
        assert!(matches!(events.try_recv(),Ok(Event::ScheduledEventStarted(actual)) if actual==start));
        assert!(events.try_recv().is_err());assert_eq!(session.battlefield.borrow().my_corps_kill_death_ratio,-7);
        assert!(!session.battlefield.borrow().in_instant_game());assert_eq!(session.server_clock.borrow().mapped_now(),None);
    }

    #[tokio::test]
    async fn expert_control_receipts_forward_exact_ack_order_without_synthetic_points() {
        use crate::actability::{ActabilityEvent, ActabilityValue};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let expected = [
            ActabilityEvent::Snapshot { last: true, entries: vec![ActabilityValue { id: 5, point: 30000, step: 2 }] },
            ActabilityEvent::TierChanged { is_upgrade: false, id: 5, step: 1 },
            ActabilityEvent::Expanded { count: 2 },
            ActabilityEvent::TierChanged { is_upgrade: false, id: 5, step: 1 },
        ];
        let mut messages: Vec<_> = expected.iter().cloned().map(game::ServerMsg::Actability).collect();
        messages.push(game::ServerMsg::Snow(true));
        session.game_msg(Ok(game::ServerMsg::Batch(messages))).await;
        for expected in expected {
            assert!(matches!(events.try_recv(), Ok(Event::Actability(actual)) if actual == expected));
        }
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn housing_reports_forward_every_received_action_in_order() {
        use crate::housing::HousingEvent;
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let expected = HousingEvent::State(crate::housing::tests::snapshot());
        session.game_msg(Ok(game::ServerMsg::Batch(vec![
            game::ServerMsg::Housing(expected.clone()),
            game::ServerMsg::Housing(expected.clone()),
            game::ServerMsg::Housing(HousingEvent::Demolished { tl_id: 7 }),
            game::ServerMsg::Snow(true),
        ]))).await;
        for _ in 0..2 { assert!(matches!(events.try_recv(), Ok(Event::Housing(actual)) if actual == expected)); }
        assert!(matches!(events.try_recv(), Ok(Event::Housing(HousingEvent::Demolished { tl_id: 7 }))));
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn received_initial_configs_reach_app_before_following_packets_without_defaulting_short_features() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut raw = crate::wire::Writer::new();
        raw.blob(b"host").blob(&[0x80]).i32(0).i32(50).raw(&[1; 7]).u8(2).u8(0).u16(0);
        let raw = raw.finish();
        let config = crate::initial_config::InitialConfig::read(&mut crate::wire::Reader::new(&raw)).unwrap();
        session.game_msg(Ok(game::ServerMsg::Batch(vec![
            game::ServerMsg::InitialConfig(config.clone()),
            game::ServerMsg::InitialConfig(config.clone()),
            game::ServerMsg::Snow(true),
        ]))).await;
        let mut state = crate::state::WorldState::default();
        for _ in 0..2 {
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::InitialConfig(actual) if actual == &config));
            state.apply(&event);
            assert_eq!(state.game_config.received.as_ref().unwrap().mate_level_limit(), None);
        }
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn mate_rename_sends_exact_native_name_and_waits_for_received_name_change() {
        use crate::mate::{MateCommand, MateEvent};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session.game = Some(connection.unwrap());
        let command = MateCommand::Rename { tl_id: 0x102, name: "A\u{e9}".into() };
        session.command(Command::Mate(command.clone())).await;
        assert!(matches!(events.try_recv(), Ok(Event::Mate(MateEvent::Requested(actual))) if actual == command));
        assert!(events.try_recv().is_err(), "local send cannot invent a received name change");
        let mut length = [0; 2];
        peer.read_exact(&mut length).await.unwrap();
        let mut received = vec![0; u16::from_le_bytes(length) as usize];
        peer.read_exact(&mut received).await.unwrap();
        assert_eq!(received, [0xdd, 1, 0, 0, 0xa6, 0, 2, 1, 3, 0, 65, 0xc3, 0xa9]);
        session.game_msg(Ok(game::ServerMsg::UnitNameChanged { obj_id: 51, name: "Received".into() })).await;
        assert!(matches!(events.try_recv(), Ok(Event::UnitNameChanged { obj_id: 51, name }) if name == "Received"));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn craft_unlock_and_repeated_setting_actions_reach_app_in_received_order() {
        use crate::craft::CraftItemUnlock;
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let changes = crate::items::ItemTasks {
            reason: 0, tasks: vec![crate::items::ItemTask::AutoUseAaPoint(1)],
            force_remove: vec![], truncated: false,
        };
        let unlock = CraftItemUnlock { kind: 0xf0, slot: 0 };
        session.game_msg(Ok(game::ServerMsg::Batch(vec![
            game::ServerMsg::ItemTasks(changes.clone()),
            game::ServerMsg::ItemTasks(changes.clone()),
            game::ServerMsg::CraftItemUnlocked(unlock),
            game::ServerMsg::CraftItemUnlocked(unlock),
        ]))).await;
        for _ in 0..2 {
            assert!(matches!(events.try_recv(), Ok(Event::ItemsChanged(actual)) if actual == changes));
        }
        for _ in 0..2 {
            assert!(matches!(events.try_recv(), Ok(Event::CraftItemUnlocked(actual)) if actual == unlock));
        }
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn aa_point_request_without_successful_send_never_latches_pending() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.command(Command::SetAutoUseAaPoint(true)).await;
        assert!(matches!(events.try_recv(), Ok(Event::AutoUseAaPointRequestRejected { value: true, .. })));
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        session.command(Command::SetAutoUseAaPoint(true)).await;
        assert!(matches!(events.try_recv(), Ok(Event::AutoUseAaPointRequestRejected { value: true, .. })));
        assert!(events.try_recv().is_err(), "no connection cannot mark a request pending");
    }

    #[tokio::test]
    async fn repair_bank_and_mate_unsent_paths_report_existing_rejections_without_pending_success() {
        use crate::bank::{BankCommand, BankEvent};
        use crate::mate::{MateCommand, MateEvent};
        use crate::repair::{RepairCommand, RepairEvent};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut state = crate::state::WorldState::default();
        let bank = BankCommand::Expand { container: 2, auto_use_aa_point: false };
        let repair = RepairCommand::All { auto_use_aa_point: false };
        let mate = MateCommand::ReviveItems { npc_obj_id: 7 };
        for phase in [Phase::Idle, Phase::CharacterSelect, Phase::InWorld] {
            session.phase = phase;
            for command in [Command::Bank(bank), Command::Repair(repair), Command::Mate(mate.clone())] {
                session.command(command.clone()).await;
                let event = events.try_recv().unwrap();
                match command {
                    Command::Bank(expected) => assert!(matches!(&event, Event::Bank(BankEvent::RequestRejected { command, message })
                        if *command == expected && !message.is_empty())),
                    Command::Repair(expected) => assert!(matches!(&event, Event::Repair(RepairEvent::RequestRejected { command, message })
                        if *command == expected && !message.is_empty())),
                    Command::Mate(expected) => assert!(matches!(&event, Event::Mate(MateEvent::RequestRejected { command, message })
                        if *command == expected && !message.is_empty())),
                    _ => unreachable!(),
                }
                state.apply(&event);
                assert!(state.repair.pending.is_empty());
                assert!(state.repair.last_report.is_none());
                assert!(state.mates.own.is_none());
                assert_eq!(state.inventory.bag_slots, 0);
                assert!(events.try_recv().is_err());
            }
        }
    }

    #[tokio::test]
    async fn repair_and_bank_services_validate_locally_and_wait_for_complete_authoritative_acks() {
        use crate::bank::{BankCommand, BankEvent};
        use crate::repair::{RepairCommand, RepairEvent};
        use crate::wire::Writer;
        use tokio::io::AsyncWriteExt;
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        let expand = BankCommand::Expand {
            container: 2,
            auto_use_aa_point: true,
        };
        session.command(Command::Bank(expand)).await;
        assert!(matches!(events.try_recv(), Ok(Event::Bank(BankEvent::RequestRejected { command, .. })) if command == expand));
        assert!(events.try_recv().is_err());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        session.game = Some(connection.unwrap());
        session.phase = Phase::CharacterSelect;
        session.command(Command::Bank(expand)).await;
        assert!(
            matches!(events.try_recv(), Ok(Event::Bank(BankEvent::RequestRejected { command, .. })) if command == expand)
        );
        let single = RepairCommand::Single {
            container: 1,
            slot: 15,
            auto_use_aa_point: true,
        };
        session.command(Command::Repair(single)).await;
        assert!(
            matches!(events.try_recv(), Ok(Event::Repair(RepairEvent::RequestRejected { command, .. })) if command == single)
        );
        session.phase = Phase::InWorld;
        for command in [
            BankCommand::Expand {
                container: 1,
                auto_use_aa_point: false,
            },
            BankCommand::ExpandWithContext {
                container: 3,
                auto_use_aa_point: false,
                raw_context_obj_id: 0x1000000,
            },
            BankCommand::DepositMoney {
                amount: -1,
                aa_point: 0,
            },
        ] {
            session.command(Command::Bank(command)).await;
            assert!(
                matches!(events.try_recv(), Ok(Event::Bank(BankEvent::RequestRejected { command: actual, .. })) if actual == command)
            );
        }
        let bad_repair = RepairCommand::Single {
            container: 3,
            slot: 0,
            auto_use_aa_point: false,
        };
        session.command(Command::Repair(bad_repair)).await;
        assert!(
            matches!(events.try_recv(), Ok(Event::Repair(RepairEvent::RequestRejected { command, .. })) if command == bad_repair)
        );
        let mut state = crate::state::WorldState::default();
        state.apply(&Event::InventoryInit {
            bag_slots: 50,
            bank_slots: 50,
        });
        state.stats.money = 777;
        state.inventory.bag[0] = Some(crate::model::Item {
            id: 7,
            template_id: 8000025,
            count: 2,
            ..Default::default()
        });
        let before_bag = state.inventory.bag.clone();
        let contextual = BankCommand::ExpandWithContext {
            container: 3,
            auto_use_aa_point: false,
            raw_context_obj_id: 0x123456,
        };
        let all = RepairCommand::All {
            auto_use_aa_point: false,
        };
        let requests = [
            (
                Command::Bank(expand),
                vec![0xdd, 1, 0, 0, 0x41, 0, 2, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                Command::Bank(contextual),
                vec![0xdd, 1, 0, 1, 0x41, 0, 3, 0, 0, 0, 0, 0x56, 0x34, 0x12],
            ),
            (
                Command::Repair(single),
                vec![0xdd, 1, 0, 2, 0x3c, 0, 1, 15, 1],
            ),
            (Command::Repair(all), vec![0xdd, 1, 0, 3, 0x3d, 0, 0]),
        ];
        for (request, expected_frame) in requests {
            session.command(request.clone()).await;
            let event = events.try_recv().unwrap();
            match request {
                Command::Bank(expected) => assert!(
                    matches!(&event, Event::Bank(BankEvent::Requested(actual)) if *actual == expected)
                ),
                Command::Repair(expected) => assert!(
                    matches!(&event, Event::Repair(RepairEvent::Requested(actual)) if *actual == expected)
                ),
                _ => unreachable!(),
            }
            state.apply(&event);
            let mut length = [0; 2];
            peer.read_exact(&mut length).await.unwrap();
            let mut received = vec![0; u16::from_le_bytes(length) as usize];
            peer.read_exact(&mut received).await.unwrap();
            assert_eq!(
                received, expected_frame,
                "rejected requests must not reach the socket"
            );
        }
        assert_eq!(
            (state.inventory.bag_slots, state.inventory.bank_slots),
            (50, 50)
        );
        assert_eq!(state.inventory.bag, before_bag);
        assert_eq!(state.stats.money, 777);
        assert!(
            state.shop.last_error.is_none(),
            "bank validation must not report a shop rejection"
        );
        assert_eq!(state.repair.pending, vec![single, all]);
        assert!(state.repair.last_report.is_none());
        let mut body = Writer::new();
        body.u16(0)
            .u16(crate::opcodes::sc::SCInvenExpandedPacket)
            .u8(2)
            .u8(60);
        body.u16(0)
            .u16(crate::opcodes::sc::SCItemTaskSuccessPacket)
            .raw(&[3, 0, 0, 0, 0, 0, 0]);
        body.u16(0)
            .u16(crate::opcodes::sc::SCOnOffSnowPacket)
            .bool(true);
        let body = body.finish();
        let len = body.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(3)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&body);
        let frame = frame.finish();
        peer.write_all(&(frame.len() as u16).to_le_bytes())
            .await
            .unwrap();
        peer.write_all(&frame).await.unwrap();
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;
        let capacity = events.try_recv().unwrap();
        assert!(matches!(
            &capacity,
            Event::InventoryExpanded {
                container: 2,
                slots: 60
            }
        ));
        state.apply(&capacity);
        assert_eq!(
            state.repair.pending,
            vec![single, all],
            "capacity ACK is not a repair ACK"
        );
        let repair_ack = events.try_recv().unwrap();
        assert!(
            matches!(&repair_ack, Event::ItemsChanged(tasks) if tasks.reason == 3 && !tasks.truncated)
        );
        state.apply(&repair_ack);
        assert_eq!(
            state.repair.pending,
            vec![all],
            "one complete ACK completes only one request"
        );
        assert!(state.repair.last_report.as_ref().unwrap().items.is_empty());
        let snow = events.try_recv().unwrap();
        assert!(matches!(&snow, Event::Snow(true)));
        state.apply(&snow);
        assert!(state.world.snow);
        assert_eq!(
            (state.inventory.bag_slots, state.inventory.bank_slots),
            (60, 50)
        );
        assert_eq!(state.inventory.bag[0], before_bag[0]);
        assert_eq!(
            state.stats.money, 777,
            "capacity ACK alone cannot invent certificate or money settlement"
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn gimmick_batch_reaches_metadata_in_order_without_phantom_motion_or_reply() {
        use crate::gimmick::{GimmickEvent, GimmickMovement};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.game = Some(connection.unwrap());
        session.phase = Phase::InWorld;
        let mut spawn = crate::gimmick::tests::spawn();
        spawn.obj_id = 0x10203;
        let motion = GimmickMovement {
            obj_id: spawn.obj_id,
            ..crate::gimmick::tests::movement()
        };
        let created = GimmickEvent::Created {
            raw_count: 1,
            gimmicks: vec![spawn.clone()],
        };
        let joints = GimmickEvent::JointsBroken {
            raw_count: 0,
            entries: vec![],
        };
        let moved = GimmickEvent::Moved(motion);
        let removed = GimmickEvent::Removed {
            raw_count: 1,
            obj_ids: vec![spawn.obj_id],
        };
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::Gimmick(created.clone()),
                game::ServerMsg::Gimmick(joints.clone()),
                game::ServerMsg::Gimmick(moved.clone()),
                game::ServerMsg::UnitPoints {
                    obj_id: 8,
                    hp: 123,
                    mp: 456,
                },
                game::ServerMsg::Gimmick(removed.clone()),
                game::ServerMsg::Snow(true),
            ])))
            .await;
        let mut state = crate::state::WorldState::default();
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))));
        let before = state.units.by_id.clone();
        for (index, expected) in [created, joints, moved].into_iter().enumerate() {
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::Gimmick(actual) if actual == &expected));
            state.apply(&event);
            assert_eq!(state.gimmicks.by_id[&spawn.obj_id].spawn, spawn);
            assert_eq!(
                state.gimmicks.by_id[&spawn.obj_id].last_movement,
                if index == 2 { Some(motion) } else { None }
            );
            assert_eq!(state.units.by_id, before);
        }
        let points = events.try_recv().unwrap();
        assert!(matches!(
            &points,
            Event::UnitPoints {
                obj_id: 8,
                hp: 123,
                mp: 456
            }
        ));
        state.apply(&points);
        assert_eq!(state.units.by_id[&8].hp, 123);
        let event = events.try_recv().unwrap();
        assert!(matches!(&event, Event::Gimmick(actual) if actual == &removed));
        state.apply(&event);
        assert!(state.gimmicks.by_id.is_empty());
        let snow = events.try_recv().unwrap();
        assert!(matches!(&snow, Event::Snow(true)));
        state.apply(&snow);
        assert!(state.world.snow);
        assert!(state.player.spawn.is_none());
        assert!(events.try_recv().is_err());
        let mut extra = [0];
        assert!(
            tokio::time::timeout(Duration::from_millis(30), peer.read(&mut extra))
                .await
                .is_err(),
            "gimmick announcements/reports must not fabricate any movement response"
        );
    }

    #[tokio::test]
    async fn raw_instance_blink_and_item_lifespan_keep_batch_order_without_ack_or_fake_state() {
        use crate::items::ItemLifespan;
        use crate::wire::Writer;
        use crate::world_msg::{BlinkUnit, InstanceLoad};
        use tokio::io::AsyncWriteExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.game = Some(connection.unwrap());
        session.phase = Phase::InWorld;
        session.self_obj = 7;
        let load = InstanceLoad {
            raw_id: 9001,
            zone_id: 33,
            pos: Pos::new(50.0, 60.0, 70.0),
            angles: [1.0, 2.0, 3.0],
        };
        let blink = BlinkUnit {
            obj_id: 8,
            distance: 12.5,
            degree: -45.0,
            x: crate::wire::coord_to_long(100.0),
            y: crate::wire::coord_to_long(200.0),
            z: 300.0,
        };
        let timer = ItemLifespan {
            added: true,
            item_id: 55,
            template_id: 123,
            raw_time: -7,
        };
        let cleared = ItemLifespan {
            added: false,
            ..timer
        };
        let mut raw = Writer::new();
        raw.u16(0)
            .u16(0x197)
            .u32(load.raw_id)
            .u32(load.zone_id)
            .f32(load.pos.x)
            .f32(load.pos.y)
            .f32(load.pos.z);
        for angle in load.angles {
            raw.f32(angle);
        }
        raw.u16(0)
            .u16(0x073)
            .bc(blink.obj_id)
            .f32(blink.distance)
            .f32(blink.degree)
            .i64(blink.x)
            .i64(blink.y)
            .f32(blink.z);
        for value in [timer, cleared] {
            raw.u16(0)
                .u16(0x098)
                .bool(value.added)
                .u64(value.item_id)
                .u32(value.template_id)
                .i64(value.raw_time);
        }
        raw.u16(0)
            .u16(crate::opcodes::sc::SCUnitPointsPacket)
            .bc(8)
            .i32(12300)
            .i32(45600);
        raw.u16(0)
            .u16(crate::opcodes::sc::SCOnOffSnowPacket)
            .bool(true);
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(6)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
        let frame = frame.finish();
        peer.write_all(&(frame.len() as u16).to_le_bytes())
            .await
            .unwrap();
        peer.write_all(&frame).await.unwrap();
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;

        let mut state = crate::state::WorldState::default();
        let mut selected = CharacterState::default();
        selected.character.id = 42;
        selected.character.zone_id = 19;
        selected.instance_id = 77;
        state.apply(&Event::CharacterSelected(Box::new(selected.clone())));
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: selected,
            unit: crate::state::tests::unit(7),
        })));
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))));
        state.inventory.bag.push(Some(crate::model::Item {
            id: 55,
            template_id: 123,
            count: 5,
            unpack_time: 321,
            lifespan_mins: 60,
            ..Default::default()
        }));
        let before = state.clone();
        for expected in 0..4 {
            let event = events.try_recv().unwrap();
            match expected {
                0 => assert!(matches!(&event, Event::LoadInstance(got) if *got == load)),
                1 => assert!(matches!(&event, Event::BlinkUnit(got) if *got == blink)),
                2 => assert!(matches!(&event, Event::ItemLifespan(got) if *got == timer)),
                _ => assert!(matches!(&event, Event::ItemLifespan(got) if *got == cleared)),
            }
            state.apply(&event);
        }
        assert_eq!(state.world, before.world);
        assert_eq!(state.player.spawn, before.player.spawn);
        assert_eq!(state.units.by_id, before.units.by_id);
        assert_eq!(state.inventory.bag, before.inventory.bag);
        let points = events.try_recv().unwrap();
        assert!(matches!(
            &points,
            Event::UnitPoints {
                obj_id: 8,
                hp: 123,
                mp: 456
            }
        ));
        state.apply(&points);
        let snow = events.try_recv().unwrap();
        assert!(matches!(&snow, Event::Snow(true)));
        state.apply(&snow);
        assert_eq!(state.units.by_id[&8].hp, 123);
        assert!(state.world.snow);
        assert!(events.try_recv().is_err());
        assert_eq!(session.phase, Phase::InWorld);
        assert_eq!(session.self_obj, 7);
        let mut extra = [0];
        assert!(
            tokio::time::timeout(Duration::from_millis(30), peer.read(&mut extra))
                .await
                .is_err(),
            "receive directives must not fabricate a teleport or instance-loaded response"
        );
    }

    #[tokio::test]
    async fn slave_controls_validate_and_emit_requests_without_authoritative_mutation() {
        use crate::slave::{SlaveCreated, SlaveEvent};
        use crate::slave_control::SlaveCommand;
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stub = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            for expected in [
                vec![0xdd, 1, 0, 0, 0x31, 0, 2, 1],
                vec![0xdd, 1, 0, 1, 0x34, 0, 2, 1, 4, 0, b'B', b'o', b'a', b't'],
                vec![0xdd, 1, 0, 2, 0x30, 0, 2, 1],
            ] {
                let mut length = [0; 2];
                tokio::time::timeout(Duration::from_secs(2), sock.read_exact(&mut length))
                    .await
                    .unwrap()
                    .unwrap();
                let mut raw = vec![0; usize::from(u16::from_le_bytes(length))];
                sock.read_exact(&mut raw).await.unwrap();
                assert_eq!(
                    raw, expected,
                    "invalid/out-of-world requests must not reach the socket"
                );
            }
            let reply = vec![
                0xdd, 1, 0, 0, 0x70, 0, 81, 0, 0, 4, 0, b'B', b'o', b'a', b't',
            ];
            sock.write_all(&(reply.len() as u16).to_le_bytes())
                .await
                .unwrap();
            sock.write_all(&reply).await.unwrap();
            let mut extra = [0];
            assert!(
                tokio::time::timeout(Duration::from_millis(30), sock.read(&mut extra))
                    .await
                    .is_err()
            );
        });
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session
            .command(Command::Slave(SlaveCommand::Bind { tl_id: 0x102 }))
            .await;
        assert!(
            events.try_recv().is_err(),
            "no connection is not a successful local send"
        );
        session.game = Some(GameConn::connect(&addr).await.unwrap());
        session.phase = Phase::CharacterSelect;
        session
            .command(Command::Slave(SlaveCommand::Bind { tl_id: 0x102 }))
            .await;
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        let mut state = crate::state::WorldState::default();
        state.slaves.own = Some(SlaveCreated {
            owner_obj_id: 7,
            tl_id: 0x102,
            obj_id: 81,
            hide_spawn_effect: false,
            raw_id: 0,
            creator_name: "Owner".into(),
        });
        let mut unit = crate::state::tests::unit(81);
        unit.name = "OldBoat".into();
        unit.kind = crate::model::UnitKind::Slave {
            id: 1,
            tl_id: 0x102,
            template_id: 68,
            summoner_id: 42,
        };
        state.units.by_id.insert(81, unit);
        let units_before = state.units.by_id.clone();
        let before = state.slaves.clone();
        for command in [
            SlaveCommand::Bind { tl_id: 0 },
            SlaveCommand::Despawn { obj_id: 0x1000000 },
            SlaveCommand::Rename {
                tl_id: 1,
                name: " ".into(),
            },
        ] {
            session.command(Command::Slave(command.clone())).await;
            let event = events.try_recv().unwrap();
            assert!(
                matches!(&event, Event::Slave(SlaveEvent::RequestRejected { command: actual, .. }) if actual == &command)
            );
            state.apply(&event);
        }
        for command in [
            SlaveCommand::Bind { tl_id: 0x102 },
            SlaveCommand::Rename {
                tl_id: 0x102,
                name: "Boat".into(),
            },
            SlaveCommand::Dismiss { tl_id: 0x102 },
        ] {
            session.command(Command::Slave(command.clone())).await;
            let event = events.try_recv().unwrap();
            assert!(
                matches!(&event, Event::Slave(SlaveEvent::Requested(actual)) if actual == &command)
            );
            state.apply(&event);
        }
        assert_eq!(state.slaves, before);
        assert_eq!(state.units.by_id, units_before);
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            session.game.as_mut().unwrap().recv(),
        )
        .await
        .unwrap();
        session.game_msg(response).await;
        let renamed = events.try_recv().unwrap();
        assert!(matches!(&renamed, Event::UnitNameChanged { obj_id: 81, name } if name == "Boat"));
        state.apply(&renamed);
        assert_eq!(state.units.by_id[&81].name, "Boat");
        assert_eq!(state.slaves, before);
        assert!(
            events.try_recv().is_err(),
            "server silence cannot fabricate control success"
        );
        stub.await.unwrap();
    }

    #[tokio::test]
    async fn mate_controls_and_explicit_cast_wait_for_real_server_events() {
        use crate::mate::{MateCommand, MateEvent};
        use tokio::io::AsyncWriteExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stub = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let requests = [
                vec![0xdd, 1, 0, 0, 0xa7, 0, 2, 1, 1, 4],
                vec![
                    0xdd, 1, 0, 1, 0x52, 0, 0x21, 0x4e, 0, 0, 3, 0x66, 0x55, 0x44, 77, 0, 0, 0, 0,
                    0x66, 0x55, 0x44, 0,
                ],
            ];
            for (index, expected) in requests.into_iter().enumerate() {
                let mut length = [0u8; 2];
                tokio::time::timeout(Duration::from_secs(2), sock.read_exact(&mut length))
                    .await
                    .unwrap()
                    .unwrap();
                let mut request = vec![0u8; u16::from_le_bytes(length) as usize];
                sock.read_exact(&mut request).await.unwrap();
                assert_eq!(
                    request, expected,
                    "only the validated explicit control/cast may reach the socket"
                );
                let mut response = crate::wire::Writer::new();
                response.raw(&[0xdd, 1, 0, 0]);
                if index == 0 {
                    response
                        .u16(crate::opcodes::sc::SCErrorMsgPacket)
                        .i16(95)
                        .i16(95)
                        .u32(0)
                        .bool(false);
                } else {
                    response
                        .u16(crate::opcodes::sc::SCSkillStartedPacket)
                        .u32(20001)
                        .u16(7)
                        .u8(3)
                        .bc(0x445566)
                        .u32(77)
                        .u8(0)
                        .bc(0x445566)
                        .u8(0)
                        .u16(150)
                        .u16(150)
                        .u8(0)
                        .u8(0);
                }
                let response = response.finish();
                sock.write_all(&(response.len() as u16).to_le_bytes())
                    .await
                    .unwrap();
                sock.write_all(&response).await.unwrap();
            }
        });
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        let mount = MateCommand::Mount {
            tl_id: 0x102,
            point: 1,
            reason: 4,
        };
        session.command(Command::Mate(mount.clone())).await;
        assert!(matches!(events.try_recv(), Ok(Event::Mate(MateEvent::RequestRejected { command, .. })) if command == mount));
        assert!(events.try_recv().is_err());
        session.game = Some(GameConn::connect(&addr).await.unwrap());
        session.self_obj = 0x112233;
        for command in [
            MateCommand::Dismiss { tl_id: 0 },
            MateCommand::UserState { tl_id: 1, state: 4 },
            MateCommand::Target {
                tl_id: 1,
                target_obj_id: 0x1000000,
            },
        ] {
            session.command(Command::Mate(command.clone())).await;
            assert!(
                matches!(events.try_recv(), Ok(Event::Mate(MateEvent::RequestRejected { command: rejected, .. })) if rejected == command)
            );
        }
        session.phase = Phase::CharacterSelect;
        session.command(Command::Mate(mount.clone())).await;
        assert!(matches!(events.try_recv(), Ok(Event::Mate(MateEvent::RequestRejected { command, .. })) if command == mount));
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        session.command(Command::Mate(mount.clone())).await;
        assert!(
            matches!(events.try_recv(), Ok(Event::Mate(MateEvent::Requested(command))) if command == mount)
        );
        assert!(
            events.try_recv().is_err(),
            "no invented UnitAttached or mode ACK"
        );
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::ServerError { code: 95 })
        ));
        for (skill_id, mate_obj_id, mount_skill_id, target_obj_id) in [
            (0, 1, 1, 1),
            (1, 1, 0, 1),
            (1, 0, 1, 1),
            (1, 0x1000000, 1, 1),
            (1, u32::MAX, 1, 1),
            (1, 1, 1, 0),
            (1, 1, 1, 0x1000000),
            (1, 1, 1, u32::MAX),
        ] {
            session
                .command(Command::UseMateSkill {
                    skill_id,
                    mate_obj_id,
                    mount_skill_id,
                    target_obj_id,
                })
                .await;
            assert!(
                matches!(events.try_recv(), Ok(Event::Mate(MateEvent::SkillRejected { skill_id: sid, mate_obj_id: mid, mount_skill_id: msid, target_obj_id: tid, .. })) if (sid,mid,msid,tid) == (skill_id,mate_obj_id,mount_skill_id,target_obj_id))
            );
        }
        let command = Command::UseMateSkill {
            skill_id: 20001,
            mate_obj_id: 0x445566,
            mount_skill_id: 77,
            target_obj_id: 0x445566,
        };
        session.phase = Phase::CharacterSelect;
        session.command(command.clone()).await;
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        session.command(command).await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::Mate(MateEvent::SkillRequested {
                skill_id: 20001,
                mate_obj_id: 0x445566,
                mount_skill_id: 77,
                target_obj_id: 0x445566
            }))
        ));
        assert!(
            events.try_recv().is_err(),
            "sending does not create a cast or charge the pet"
        );
        let mut state = crate::state::WorldState::default();
        state.skills.obj_id = 0x112233;
        assert!(
            state.skills.known.is_empty(),
            "pet casts do not require the player's learned list"
        );
        assert!(state.skills.casting_of(0x445566).is_none());
        stub.await.unwrap();
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;
        let event = events.try_recv().unwrap();
        assert!(
            matches!(&event, Event::SkillStarted(skill) if skill.skill_id == 20001 && skill.caster.obj_id == 0x445566 && skill.caster.kind == 3)
        );
        state.apply(&event);
        assert_eq!(
            state.skills.casting_of(0x445566).unwrap().target.obj_id,
            0x445566
        );
        assert!(state.skills.casting_of(0x112233).is_none());
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn owned_mate_and_slave_metadata_reach_resources_before_following_points() {
        use crate::mate::{DynamicUnitState, MateEvent, MateSpawn, SlaveState};
        let spawn = MateSpawn {
            tl_id: 12,
            id: 17,
            item_id: 0x1122334455667788,
            user_state: 1,
            experience: 300,
            mileage: 5,
            spawn_delay_time: 1000,
            skills: [11, 0, 12, 0, 0, 0, 0, 0, 0, 13],
        };
        let dynamic = DynamicUnitState {
            skills: vec![[u32::MAX, 2, 3]],
            tags: vec![[4, 5, 0x80000000]],
        };
        let slave = SlaveState {
            obj_id: 81,
            tl_id: 9,
            raw_type: u64::MAX,
            dynamic: dynamic.clone(),
            creator_name: "Someone".into(),
            owner_obj_id: 0xf1234567,
            db_id: 55,
        };
        let packets = vec![
            MateEvent::Spawned(spawn.clone()),
            MateEvent::State {
                obj_id: 71,
                dynamic: dynamic.clone(),
            },
            MateEvent::SlaveState(slave),
            MateEvent::EquipmentChanged(crate::mate_equipment::MateEquipmentChange {
                character_id: 42,
                mate_tl_id: 12,
                passenger_id: 0,
                bts: false,
                entries: vec![],
                success: false,
            }),
        ];
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut batch: Vec<_> = packets.iter().cloned().map(game::ServerMsg::Mate).collect();
        let created = crate::slave::SlaveCreated {
            owner_obj_id: 0x112233,
            tl_id: 9,
            obj_id: 81,
            hide_spawn_effect: false,
            raw_id: i64::MIN,
            creator_name: "Someone".into(),
        };
        batch.push(game::ServerMsg::Slave(crate::slave::SlaveEvent::Created(
            created.clone(),
        )));
        let craft_notice = crate::craft::CraftFailureNotice {
            raw_id: 0xfedcba98,
            raw_count: 1,
            entries: vec![1234],
        };
        batch.push(game::ServerMsg::CraftFailureNotice(craft_notice.clone()));
        batch.push(game::ServerMsg::UnitPoints {
            obj_id: 71,
            hp: 123,
            mp: 456,
        });
        session.game_msg(Ok(game::ServerMsg::Batch(batch))).await;
        let mut state = crate::state::WorldState::default();
        let mut selected = CharacterState::default();
        selected.character.id = 42;
        state.apply(&Event::CharacterSelected(Box::new(selected)));
        let mut mate = crate::state::tests::unit(71);
        mate.kind = crate::model::UnitKind::Mate {
            tl_id: 12,
            template_id: 100,
            owner_id: 42,
        };
        state.apply(&Event::UnitSpawned(Box::new(mate)));
        for expected in packets {
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::Mate(actual) if actual == &expected));
            state.apply(&event);
        }
        let creation_event = events.try_recv().unwrap();
        assert!(
            matches!(&creation_event, Event::Slave(crate::slave::SlaveEvent::Created(actual)) if actual == &created)
        );
        state.apply(&creation_event);
        assert_eq!(state.slaves.created.get(&81), Some(&created));
        assert!(state.slaves.own.is_none());
        assert_eq!(
            state
                .mates
                .last_equipment_change
                .as_ref()
                .map(|change| change.success),
            Some(false)
        );
        let failure_event = events.try_recv().unwrap();
        assert!(matches!(&failure_event, Event::CraftFailureNotice(actual) if actual == &craft_notice));
        state.apply(&failure_event);
        assert!(state.inventory.bag.iter().all(Option::is_none));
        let points = events.try_recv().unwrap();
        assert!(matches!(
            &points,
            Event::UnitPoints {
                obj_id: 71,
                hp: 123,
                mp: 456
            }
        ));
        state.apply(&points);
        let own = state.mates.own.as_ref().unwrap();
        assert_eq!(own.spawn, spawn);
        assert_eq!(own.obj_id, Some(71));
        assert_eq!(own.dynamic, Some(dynamic));
        assert_eq!(
            (state.units.by_id[&71].hp, state.units.by_id[&71].mp),
            (123, 456)
        );
        assert!(
            !state.units.by_id.contains_key(&81),
            "slave metadata is not a UnitSpawned or local ownership proof"
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn doodad_mutation_events_preserve_signed_data_stored_owner_faction_and_sound() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::DoodadChanged {
                    obj_id: 8,
                    data: i32::MIN,
                },
                game::ServerMsg::DoodadOriginator {
                    obj_id: 9,
                    new_owner_id: 0xf1234567,
                    new_faction: 0x80000000,
                },
                game::ServerMsg::DoodadSound {
                    obj_id: 7,
                    sound_id: u32::MAX,
                },
            ])))
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::DoodadChanged {
                obj_id: 8,
                data: i32::MIN
            })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(Event::DoodadOriginator {
                obj_id: 9,
                new_owner_id: 0xf1234567,
                new_faction: 0x80000000
            })
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(Event::DoodadSound {
                obj_id: 7,
                sound_id: u32::MAX
            })
        ));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn proficiency_requests_reject_inactive_and_missing_transport_without_latching_pending() {
        use crate::actability::{ActabilityCommand, ActabilityEvent};
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut state = crate::state::WorldState::default();
        let commands = [
            ActabilityCommand::Upgrade { id: 5, auto_use_aa_point: true },
            ActabilityCommand::Downgrade { id: 5, auto_use_aa_point: false },
            ActabilityCommand::Expand,
        ];
        for phase in [Phase::Idle, Phase::CharacterSelect, Phase::InWorld] {
            session.phase = phase;
            for command in commands {
                session.command(Command::Actability(command)).await;
                let event = events.try_recv().unwrap();
                assert!(matches!(&event, Event::Actability(ActabilityEvent::RequestRejected { command: actual, message })
                    if *actual == command && !message.is_empty()));
                state.apply(&event);
                assert!(state.actabilities.pending.is_empty());
                assert!(state.actabilities.entries.is_empty());
                assert!(events.try_recv().is_err());
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        session.game = Some(connection.unwrap());
        session.phase = Phase::CharacterSelect;
        session.command(Command::Actability(commands[0])).await;
        assert!(matches!(events.try_recv(), Ok(Event::Actability(ActabilityEvent::RequestRejected { .. }))));
        session.phase = Phase::InWorld;
        for (index, command) in commands.into_iter().enumerate() {
            session.command(Command::Actability(command)).await;
            let event = events.try_recv().unwrap();
            assert!(matches!(&event, Event::Actability(ActabilityEvent::Requested(actual)) if *actual == command));
            state.apply(&event);
            let mut length = [0; 2];
            peer.read_exact(&mut length).await.unwrap();
            let mut received = vec![0; u16::from_le_bytes(length) as usize];
            peer.read_exact(&mut received).await.unwrap();
            let mut expected = vec![0xdd, 1, 0, index as u8];
            expected.extend(command.opcode().to_le_bytes());
            let mut body = crate::wire::Writer::new();
            command.write(&mut body);
            expected.extend(body.finish());
            assert_eq!(received, expected, "inactive requests must not reach the socket");
        }
        assert_eq!(state.actabilities.pending, commands);
        assert!(state.actabilities.entries.is_empty(), "sending does not invent proficiency");
        assert_eq!(state.actabilities.expanded_expert, None);
    }

    #[tokio::test]
    async fn craft_command_validates_before_sending_and_reports_only_local_request_state() {
        use tokio::io::AsyncWriteExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stub = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let expected = [
                vec![
                    0xdd, 1, 0, 0, 0xf8, 0, 0x44, 0x33, 0x22, 0x11, 0x66, 0x55, 0x44, 3, 0, 0, 0,
                ],
                vec![
                    0xdd, 1, 0, 1, 0xf8, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 127,
                ],
            ];
            for (index, expected) in expected.into_iter().enumerate() {
                let mut length = [0u8; 2];
                tokio::time::timeout(Duration::from_secs(2), sock.read_exact(&mut length))
                    .await
                    .unwrap()
                    .unwrap();
                let mut request = vec![0u8; u16::from_le_bytes(length) as usize];
                sock.read_exact(&mut request).await.unwrap();
                assert_eq!(
                    request, expected,
                    "invalid/out-of-world requests or an extra skill packet must not precede the batch"
                );
                if index == 0 {
                    let mut response = crate::wire::Writer::new();
                    response
                        .raw(&[0xdd, 1, 0, 0])
                        .u16(crate::opcodes::sc::SCErrorMsgPacket)
                        .i16(95)
                        .i16(95)
                        .u32(0)
                        .bool(false);
                    let response = response.finish();
                    sock.write_all(&(response.len() as u16).to_le_bytes())
                        .await
                        .unwrap();
                    sock.write_all(&response).await.unwrap();
                }
            }
        });
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.phase = Phase::InWorld;
        session
            .command(Command::ExecuteCraft {
                craft_id: 1,
                doodad_obj_id: 1,
                count: 1,
            })
            .await;
        assert!(matches!(events.try_recv(), Ok(Event::CraftRejected { craft_id: 1, doodad_obj_id: 1, count: 1, .. })));
        assert!(events.try_recv().is_err());
        session.game = Some(GameConn::connect(&addr).await.unwrap());
        for (craft_id, doodad_obj_id, count) in [
            (0, 1, 1),
            (1, 0, 1),
            (1, 0x1000000, 1),
            (1, u32::MAX, 1),
            (1, 1, 0),
            (1, 1, -1),
            (1, 1, i32::MIN),
        ] {
            session
                .command(Command::ExecuteCraft {
                    craft_id,
                    doodad_obj_id,
                    count,
                })
                .await;
            assert!(
                matches!(events.try_recv(), Ok(Event::CraftRejected { craft_id: id, doodad_obj_id: obj, count: n, message }) if (id,obj,n)==(craft_id,doodad_obj_id,count) && !message.is_empty())
            );
        }
        session.phase = Phase::CharacterSelect;
        session
            .command(Command::ExecuteCraft {
                craft_id: 1,
                doodad_obj_id: 1,
                count: 1,
            })
            .await;
        assert!(matches!(events.try_recv(), Ok(Event::CraftRejected { craft_id: 1, doodad_obj_id: 1, count: 1, .. })));
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        session
            .command(Command::ExecuteCraft {
                craft_id: 0x11223344,
                doodad_obj_id: 0x445566,
                count: 3,
            })
            .await;
        let requested = events.try_recv().unwrap();
        assert!(matches!(
            &requested,
            Event::CraftRequested {
                craft_id: 0x11223344,
                doodad_obj_id: 0x445566,
                count: 3
            }
        ));
        let mut state = crate::state::WorldState::default();
        state.stats.labor = 123;
        state.stats.money = 456;
        state.inventory.bag.push(None);
        state.apply(&requested);
        assert!(
            events.try_recv().is_err(),
            "send is not a SkillStarted or completed product"
        );
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;
        let rejected = events.try_recv().unwrap();
        assert!(matches!(&rejected, Event::ServerError { code: 95 }));
        state.apply(&rejected);
        assert!(
            events.try_recv().is_err(),
            "generic server error is not a fabricated batch ACK"
        );
        session
            .command(Command::ExecuteCraft {
                craft_id: u32::MAX,
                doodad_obj_id: 0xffffff,
                count: i32::MAX,
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::CraftRequested {
                craft_id: u32::MAX,
                doodad_obj_id: 0xffffff,
                count: i32::MAX
            })
        ));
        stub.await.unwrap();
        assert!(
            events.try_recv().is_err(),
            "silent server leaves the actual result unknown"
        );
        assert!(state.skills.casting.is_empty());
        assert!(state.doodads.by_id.is_empty());
        assert_eq!((state.stats.labor, state.stats.money), (123, 456));
        assert_eq!(state.inventory.bag, vec![None]);
    }

    #[tokio::test]
    async fn doodad_skill_command_rejects_truncation_and_waits_for_real_skill_started() {
        use tokio::io::AsyncWriteExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let stub = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut length = [0u8; 2];
            tokio::time::timeout(Duration::from_secs(2), sock.read_exact(&mut length))
                .await
                .unwrap()
                .unwrap();
            let mut request = vec![0u8; u16::from_le_bytes(length) as usize];
            sock.read_exact(&mut request).await.unwrap();
            assert_eq!(
                request,
                [
                    0xdd, 1, 0, 0, 0x52, 0, 0xd4, 0x46, 0, 0, 0, 0x33, 0x22, 0x11, 4, 0x66, 0x55, 0x44,
                    0
                ]
            );
            let mut response = crate::wire::Writer::new();
            response
                .raw(&[0xdd, 1, 0, 0, 0xa1, 0])
                .u32(18132)
                .u16(7)
                .u8(0)
                .bc(0x112233)
                .u8(4)
                .bc(0x445566)
                .u8(0)
                .u16(150)
                .u16(150)
                .u8(0)
                .u8(0);
            let response = response.finish();
            sock.write_all(&(response.len() as u16).to_le_bytes())
                .await
                .unwrap();
            sock.write_all(&response).await.unwrap();
        });
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        session.game = Some(GameConn::connect(&addr).await.unwrap());
        session.self_obj = 0x112233;
        session.phase = Phase::InWorld;
        for doodad_obj_id in [0, 0x100_0000, u32::MAX] {
            session
                .command(Command::UseDoodadSkill {
                    skill_id: 18132,
                    doodad_obj_id,
                })
                .await;
            assert!(
                matches!(events.try_recv(),Ok(Event::DoodadSkillRejected{skill_id:18132,doodad_obj_id:id,..}) if id==doodad_obj_id)
            );
        }
        session
            .command(Command::UseDoodadSkill {
                skill_id: 0,
                doodad_obj_id: 1,
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Ok(Event::DoodadSkillRejected { skill_id: 0, .. })
        ));
        session.phase = Phase::CharacterSelect;
        session
            .command(Command::UseDoodadSkill {
                skill_id: 123,
                doodad_obj_id: 1,
            })
            .await;
        assert!(events.try_recv().is_err());
        session.phase = Phase::InWorld;
        session
            .command(Command::UseDoodadSkill {
                skill_id: 18132,
                doodad_obj_id: 0x445566,
            })
            .await;
        assert!(
            events.try_recv().is_err(),
            "sending must not fabricate cast or interaction success"
        );
        let mut state = crate::state::WorldState::default();
        state.skills.obj_id = 0x112233;
        assert!(state.skills.known.is_empty());
        assert!(state.skills.casting_of(0x112233).is_none());
        stub.await.unwrap();
        let response = session.game.as_mut().unwrap().recv().await;
        session.game_msg(response).await;
        let event = events.try_recv().unwrap();
        assert!(
            matches!(&event,Event::SkillStarted(s) if s.skill_id==18132 && s.target.kind==4 && s.target.obj_id==0x445566)
        );
        state.apply(&event);
        assert_eq!(
            state.skills.casting_of(0x112233).unwrap().target,
            SkillTarget::doodad(0x445566).unwrap()
        );
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn independent_mixed_deflate_batch_reaches_world_state_in_wire_order() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut frame = vec![game::MAGIC, game::LEVEL_COMPRESSED];
        frame.extend(260u16.to_le_bytes());
        frame.extend_from_slice(include_bytes!("../tests/fixtures/deflate/mixed.deflate"));
        session.game_msg(game::ServerMsg::decode(&frame).map_err(Into::into)).await;
        let mut state = crate::state::WorldState::default();
        state.stats.obj_id = 256;
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            state.apply(&event);
            seen.push(event);
        }
        assert_eq!(seen.len(), 260);
        assert!(matches!(seen[0], Event::UnitPoints { obj_id: 1, hp: 300, mp: 20 }));
        assert!(matches!(seen[1], Event::Snow(true)));
        assert!(matches!(seen[2], Event::UnitPoints { obj_id: 2, hp: 331, mp: 21 }));
        for (i, event) in seen[3..259].iter().enumerate() {
            assert!(matches!(event, Event::UnitPoints { obj_id, hp, mp }
                if *obj_id == i as u32 + 1 && *hp == 300 + i as i32 * 31 && *mp == 20 + i as i32 % 71));
        }
        assert!(matches!(seen[259], Event::Snow(true)));
        assert_eq!((state.stats.hp, state.stats.mp), (8205, 62));
        assert!(state.world.snow);
    }

    #[tokio::test]
    async fn invalid_compression_emits_no_partial_events_and_the_next_frame_still_decodes() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut frame = vec![game::MAGIC, game::LEVEL_COMPRESSED, 3, 0];
        frame.extend_from_slice(include_bytes!("../tests/fixtures/deflate/fixed.deflate"));
        frame.push(0); // Trailing data invalidates the complete stream, including its prefix.
        session.game_msg(game::ServerMsg::decode(&frame).map_err(Into::into)).await;
        assert!(matches!(events.try_recv(),Ok(Event::GameDecodeFailed{..})));
        assert!(events.try_recv().is_err());
        frame.pop();
        session.game_msg(game::ServerMsg::decode(&frame).map_err(Into::into)).await;
        assert!(matches!(events.try_recv(), Ok(Event::UnitPoints { obj_id: 1, hp: 300, mp: 20 })));
        assert!(matches!(events.try_recv(), Ok(Event::Snow(true))));
        assert!(matches!(events.try_recv(), Ok(Event::UnitPoints { obj_id: 2, hp: 331, mp: 21 })));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn trade_settlement_changes_items_before_the_completion_notification() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let tasks = crate::items::ItemTasks { reason: crate::trade::TRADE_TASK,
            tasks: vec![crate::items::ItemTask::Money(5)], ..Default::default() };
        session.game_one(game::ServerMsg::Trade(crate::trade::TradeEvent::Made(tasks.clone()))).await;
        assert!(matches!(events.try_recv(), Ok(Event::ItemsChanged(got)) if got == tasks));
        assert!(matches!(events.try_recv(), Ok(Event::Trade(crate::trade::TradeEvent::Made(got))) if got == tasks));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn resurrection_offer_is_emitted_before_the_following_death_details() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let caster = BuffCaster { kind: crate::buff::caster_kind::UNIT, obj_id: 0 };
        session.game_one(game::ServerMsg::ResurrectionOffered { caster }).await;
        session.game_one(game::ServerMsg::UnitDeath {
            obj_id: 5, kill_reason: 0, resurrect_wait_ms: 10_000, lost_exp: 0, durability_loss_rate: 0, killer: None,
        }).await;
        assert!(matches!(events.try_recv(), Ok(Event::ResurrectionOffered { caster: got }) if got == caster));
        assert!(matches!(events.try_recv(), Ok(Event::UnitDeath { obj_id: 5 })));
        assert!(matches!(events.try_recv(), Ok(Event::DeathDetails { obj_id: 5, .. })));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn ship_requests_are_not_announced_as_position_updates() {
        let (evt, mut events) = unbounded_channel();
        let mut session = Session::new(evt, ClientOptions::default());
        let mut movement = Movement::unit(Pos::default(), 0.0, 1);
        movement.move_type = 4;
        session.game_one(game::ServerMsg::UnitMoved { obj_id: 5, movement }).await;
        assert!(events.try_recv().is_err());
        let movement = Movement::unit(Pos::new(500.0, 600.0, 7.0), 0.0, 2);
        session.game_one(game::ServerMsg::UnitMoved { obj_id: 5, movement }).await;
        assert!(matches!(events.try_recv(), Ok(Event::UnitMoved { obj_id: 5, movement }) if movement.pos.x == 500.0));
    }

    #[test]
    fn the_zone_channel_is_shout_trade_and_party_search() {
        assert_eq!(channel_types(1), vec![1, 2, 3]);
        assert_eq!(channel_types(6), vec![6]);
        assert_eq!(channel_types(14), vec![14]);
    }
    use crate::opcodes::pp;
    use std::net::SocketAddrV4;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    fn at(x: f32, y: f32, z: f32) -> Pos {
        Pos::new(x, y, z)
    }

    #[test]
    fn move_command_defaults_are_a_grounded_run() {
        let m = MoveCommand::new(at(1.0, 2.0, 3.0), 0.0).to_movement(42);
        assert_eq!((m.move_type, m.time, m.flags), (1, 42, move_flags::MOVING));
        assert_eq!(m.actor_flags, actor_flags::STANDING_ON_SOLID);
        assert_eq!((m.stance, m.alertness, m.fall_vel), (stance::RELAXED, 0, 0));
        assert_eq!(m.delta_movement, [0; 3]);
    }

    #[test]
    fn move_command_states_map_to_flags() {
        let base = MoveCommand::new(at(0.0, 0.0, 0.0), 0.0);
        // Stopping: the "stopping" flag replaces "moving" (the server's own stop packets do this).
        assert_eq!(MoveCommand::stopped(base.pos, 0.0).to_movement(0).flags, move_flags::STOPPING);
        // Jump: moving + stopping, actor flag "jumping", no ground.
        let jump = base.with_state(MoveState::Jump).to_movement(0);
        assert_eq!((jump.flags, jump.actor_flags), (move_flags::JUMPING, actor_flags::JUMPING));
        // Fall: no ground flag at all.
        let fall = base.with_state(MoveState::Fall).to_movement(0);
        assert_eq!((fall.flags, fall.actor_flags), (move_flags::MOVING, 0));
        // Landing at 25 m/s: grounded again, with the fall velocity block.
        let land = base.with_state(MoveState::Fall).landing(25.0).to_movement(0);
        assert_eq!(land.actor_flags, actor_flags::STANDING_ON_SOLID | actor_flags::FALL_VELOCITY);
        assert_eq!(land.fall_vel, (25.0 * FALL_VELOCITY_PER_MPS) as u16);
        // Combat.
        let fight = MoveCommand { in_combat: true, alertness: alertness::COMBAT, stance: stance::COMBAT, ..base };
        let m = fight.to_movement(0);
        assert_eq!(m.flags, move_flags::MOVING | move_flags::IN_COMBAT);
        assert_eq!((m.alertness, m.stance), (alertness::COMBAT, stance::COMBAT));
    }

    #[test]
    fn running_moves_along_the_facing() {
        // yaw 0 faces +y; a positive yaw turns left, towards -x.
        let north = MoveCommand::running(at(0.0, 0.0, 0.0), 0.0, 5.0);
        assert_eq!(north.velocity, [0, 5 * 1024, 0]);
        assert_eq!(north.delta_movement, [0, 127, 0]);
        let west = MoveCommand::running(at(0.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2, 5.0);
        assert_eq!(west.velocity[0], -5 * 1024);
        assert!(west.velocity[1].abs() <= 1);
    }

    /// A scripted game server: accepts one connection and reports (level, opcode) of the first
    /// frames, giving up when nothing arrives for a while.
    async fn game_stub(frames: usize) -> (SocketAddrV4, tokio::task::JoinHandle<Vec<(u8, u16)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let std::net::SocketAddr::V4(addr) = listener.local_addr().unwrap() else { unreachable!() };
        let task = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            for _ in 0..frames {
                let mut len = [0u8; 2];
                match tokio::time::timeout(Duration::from_millis(300), sock.read_exact(&mut len)).await {
                    Ok(Ok(_)) => {}
                    _ => break,
                }
                let mut payload = vec![0u8; usize::from(u16::from_le_bytes(len))];
                sock.read_exact(&mut payload).await.unwrap();
                // [0xDD][level], then the proxy opcode directly or hash / count / opcode.
                let opcode = match payload[1] {
                    game::LEVEL_PROXY => u16::from_le_bytes([payload[2], payload[3]]),
                    _ => u16::from_le_bytes([payload[4], payload[5]]),
                };
                seen.push((payload[1], opcode));
            }
            seen
        });
        (addr, task)
    }

    fn session_for(addr: SocketAddrV4, options: ClientOptions) -> Session {
        let (evt, _events) = unbounded_channel();
        let mut s = Session::new(evt, options);
        s.account = Some(AccountInfo { account_id: 7, ..AccountInfo::default() });
        s.cookie = Some((1, WorldCookie { cookie: 9, addr }));
        s
    }

    async fn connect_with(options: ClientOptions) -> Vec<(u8, u16)> {
        let (addr, stub) = game_stub(3).await;
        session_for(addr, options).connect_world().await;
        stub.await.unwrap()
    }

    /// The server's dead connection sweep closes game connections that have not pinged from the
    /// start, so the first packet of a session is the ping, ahead of the enter world request.
    #[tokio::test]
    async fn the_first_ping_goes_out_as_soon_as_the_game_connection_opens() {
        let seen = connect_with(ClientOptions::default()).await;
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0], (game::LEVEL_PROXY, pp::PingPacket));
        assert_eq!(seen[1].0, game::LEVEL_GAME);
    }

    #[tokio::test]
    async fn the_ping_can_be_delayed_or_switched_off_for_tests() {
        let delayed = ClientOptions { first_ping_delay: Duration::from_secs(60), ..ClientOptions::default() };
        let seen = connect_with(delayed).await;
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].0, game::LEVEL_GAME);
        let off = ClientOptions { ping_interval: None, ..ClientOptions::default() };
        assert_eq!(connect_with(off).await.len(), 1);
    }

    #[tokio::test]
    async fn pings_repeat_at_the_interval() {
        let (addr, stub) = game_stub(5).await;
        let options = ClientOptions { ping_interval: Some(Duration::from_millis(20)), ..ClientOptions::default() };
        let mut s = session_for(addr, options);
        s.connect_world().await;
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            s.keep_alive().await;
        }
        let pings = stub.await.unwrap().iter().filter(|f| **f == (game::LEVEL_PROXY, pp::PingPacket)).count();
        assert!(pings >= 4, "{pings} pings");
    }
    #[tokio::test]
    async fn received_cooldowns_and_reset_preserve_session_order_without_cast_events() {
        use crate::skill::{Cooldown, SkillCooldowns};
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        session.phase = Phase::InWorld;
        session.self_obj = 5;
        let mut state = crate::state::WorldState::default();
        state.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 5,
            state: Default::default(),
            unit: crate::state::tests::unit(5),
        })));
        let entries = SkillCooldowns {
            skills: vec![Cooldown {
                id: 18132,
                total_ms: 60_000,
                remaining_ms: 30_000,
            }],
            tags: vec![],
        };
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::Cooldowns(entries.clone()),
                game::ServerMsg::Cooldowns(SkillCooldowns::default()),
                game::ServerMsg::SkillCooldownReset {
                    obj_id: 5,
                    skill_id: 18132,
                    tag_id: 0,
                    gcd: true,
                },
                game::ServerMsg::Cooldowns(entries.clone()),
                game::ServerMsg::SetBreath { breath: 123 },
            ])))
            .await;
        for index in 0..5 {
            let event = events.try_recv().unwrap();
            match (index, &event) {
                (0 | 3, Event::Cooldowns(actual)) => assert_eq!(*actual, entries),
                (1, Event::Cooldowns(actual)) => assert_eq!(*actual, SkillCooldowns::default()),
                (
                    2,
                    Event::SkillCooldownReset {
                        obj_id: 5,
                        skill_id: 18132,
                        gcd: true,
                        ..
                    },
                ) => {}
                (4, Event::SetBreath { breath: 123 }) => {}
                other => panic!("wrong event order: {other:?}"),
            }
            state.apply(&event);
            if index == 1 {
                assert!(
                    state.skills.cooldown_left(18132, 0, 0).is_some(),
                    "empty is not reset"
                );
            }
            if index == 2 {
                assert!(state.skills.cooldown_left(18132, 0, 0).is_none());
            }
        }
        assert!(events.try_recv().is_err());
        assert_eq!(state.skills.received_skill_cooldowns.len(), 2);
        assert!(
            state.skills.received_skill_cooldowns[0].reset
                && !state.skills.received_skill_cooldowns[1].reset
        );
        assert!(state.skills.cooldown_left(18132, 0, 0).is_some());
        assert!(
            state.skills.last_used.is_empty()
                && state.skills.last_cast.is_none()
                && state.skills.casting.is_empty()
        );
        assert_eq!(state.combat_status.breath, Some(123));
    }

    #[tokio::test]
    async fn conflict_reports_forward_in_wire_order_before_character_and_spawn() {
        use crate::conflict::ConflictZoneState;
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        // No player, selected character or game socket. World reports still remain meaningful.
        assert_eq!(session.self_obj, 0);
        let initial = ConflictZoneState {
            zone_group_id: 14,
            state: 6,
            end_time: 1,
        };
        let update = ConflictZoneState {
            state: 255,
            end_time: u64::MAX,
            ..initial
        };
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::ConflictZoneState(initial),
                game::ServerMsg::SetBreath { breath: 77 },
                game::ServerMsg::ConflictZoneState(initial),
                game::ServerMsg::ConflictZoneState(update),
            ])))
            .await;
        let mut state = crate::state::WorldState::default();
        for index in 0..4 {
            let event = events.try_recv().unwrap();
            match (index, &event) {
                (0 | 2, Event::ConflictZoneState(actual)) => assert_eq!(*actual, initial),
                (1, Event::SetBreath { breath: 77 }) => {}
                (3, Event::ConflictZoneState(actual)) => assert_eq!(*actual, update),
                other => panic!("wrong event order: {other:?}"),
            }
            state.apply(&event);
        }
        assert!(
            events.try_recv().is_err(),
            "no synthetic entry/transition/permission events"
        );
        assert_eq!(state.conflict_zones.get(14), Some(&update));
        assert_eq!(state.combat_status.breath, Some(77));
        assert!(state.player.spawn.is_none() && state.units.by_id.is_empty());
        assert_eq!(session.self_obj, 0);
    }

    #[tokio::test]
    async fn underwater_reports_preserve_repeats_without_synthetic_lua_or_capacity_events() {
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        session
            .game_msg(Ok(game::ServerMsg::Batch(vec![
                game::ServerMsg::UnderWater { start: false },
                game::ServerMsg::UnderWater { start: true },
                game::ServerMsg::UnderWater { start: true },
                game::ServerMsg::SetBreath { breath: 12345 },
                game::ServerMsg::UnderWater { start: false },
                game::ServerMsg::SetBreath { breath: 777 },
            ])))
            .await;
        let mut state = crate::state::WorldState::default();
        for index in 0..6 {
            let event = events.try_recv().unwrap();
            match (index, &event) {
                (0 | 4, Event::UnderWater { start: false }) => {}
                (1 | 2, Event::UnderWater { start: true }) => {}
                (3, Event::SetBreath { breath: 12345 }) => {}
                (5, Event::SetBreath { breath: 777 }) => {}
                other => panic!("wrong event order: {other:?}"),
            }
            state.apply(&event);
            if index < 3 {
                assert_eq!(state.combat_status.breath, None);
            }
        }
        assert!(events.try_recv().is_err());
        assert_eq!(state.combat_status.under_water, Some(false));
        assert_eq!(
            state.combat_status.breath,
            Some(777),
            "raw counter does not fabricate a DIVE_START"
        );
        assert!(state.combat_status.by_unit.is_empty() && state.player.spawn.is_none());
    }

    #[tokio::test]
    async fn fired_final_wire_flag_survives_repeated_session_forwarding() {
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        let mut wire = crate::wire::Writer::new();
        wire.u32(18132).u16(3).u8(0).bc(1).u8(0).bc(2).u8(0);
        wire.i16(10).i16(10).u8(0).u32(296).u8(255);
        let bytes = wire.finish();
        let fired = SkillFired::read(&mut crate::wire::Reader::new(&bytes)).unwrap();
        session.game_msg(Ok(game::ServerMsg::Batch(vec![
            game::ServerMsg::SkillFired(fired.clone()),
            game::ServerMsg::SkillFired(fired),
        ]))).await;
        for _ in 0..2 {
            assert!(matches!(events.try_recv(), Ok(Event::SkillFired(f)) if f.raw_final_flag == 255));
        }
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn navi_teleport_moves_the_player_and_answers_teleport_ended() {
        use crate::portal::PortalEvent;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connection, accepted) = tokio::join!(GameConn::connect(&addr), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        session.game = Some(connection.unwrap());
        session.phase = Phase::InWorld;
        let pos = Pos::new(1000.5, 2000.0, 50.0);
        session.game_msg(Ok(game::ServerMsg::Portal(PortalEvent::NaviTeleport { pos }))).await;
        assert!(matches!(events.try_recv(), Ok(Event::Teleported { pos: p, yaw, reason: 0 }) if p == pos && yaw == 0.0));
        assert!(matches!(events.try_recv(), Ok(Event::Portal(PortalEvent::NaviTeleport { .. }))));
        let n = peer.read_u16_le().await.unwrap();
        let mut frame = vec![0; n as usize];
        peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, game::ClientMsg::TeleportEnded { pos }.encode(0));
    }

    #[tokio::test]
    async fn bounty_controls_exact_loopback_identity_and_failed_send_no_optimism(){
        use crate::BountyCommand;
        async fn frame(peer:&mut tokio::net::TcpStream)->Vec<u8>{let n=peer.read_u16_le().await.unwrap();let mut out=vec![0;n as usize];peer.read_exact(&mut out).await.unwrap();out[4..].to_vec()}
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());session.phase=Phase::InWorld;session.selected=42;
        let mut header=crate::model::CharacterState::default();header.character.id=42;header.bounty_money=123;session.selected_state=Some(header);
        let request=BountyCommand::request_set_money(b"N\0tail").unwrap();
        session.command(Command::Bounty{character_id:42,command:request.clone()}).await;
        let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::BountyRequested{..}|Event::Bounty(_)));rejected|=matches!(e,Event::BountyRequestRejected{..});}assert!(rejected);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        for command in [request.clone(),BountyCommand::Update{kind:u32::MAX,name:vec![255],money:i32::MIN},BountyCommand::OpenBulletin{doodad_obj_id:0x123456}]{
            let mut expected=command.opcode().to_le_bytes().to_vec();expected.extend(game::ClientMsg::Bounty(command.clone()).encode_body());
            session.command(Command::Bounty{character_id:42,command:command.clone()}).await;assert_eq!(frame(&mut peer).await,expected);
            assert!(matches!(events.try_recv(),Ok(Event::BountyRequested{character_id:42,command:actual}) if actual==command));assert!(events.try_recv().is_err());assert_eq!(session.selected_state.as_ref().unwrap().bounty_money,123);
        }
        session.command(Command::Bounty{character_id:41,command:request.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::BountyRequestRejected{..})));
        session.selected_state.as_mut().unwrap().character.id=41;session.command(Command::Bounty{character_id:42,command:request.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::BountyRequestRejected{..})));session.selected_state.as_mut().unwrap().character.id=42;
        session.command(Command::Bounty{character_id:42,command:BountyCommand::RequestSetMoney{name:vec![b'X';129]}}).await;assert!(matches!(events.try_recv(),Ok(Event::BountyRequestRejected{..})));
        session.command(Command::Bounty{character_id:42,command:BountyCommand::OpenBulletin{doodad_obj_id:0x1000000}}).await;assert!(matches!(events.try_recv(),Ok(Event::BountyRequestRejected{..})));
        session.phase=Phase::WorldSelect;session.command(Command::Bounty{character_id:42,command:request.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::BountyRequestRejected{..})));session.phase=Phase::InWorld;
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.game.as_mut().unwrap().shutdown().await;session.command(Command::Bounty{character_id:42,command:request}).await;
        let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::BountyRequested{..}|Event::Bounty(_)));rejected|=matches!(e,Event::BountyRequestRejected{..});}assert!(rejected);
    }

    #[tokio::test]
    async fn trial_controls_use_actual_receipts_and_send_no_optimistic_confirmation_or_cancel(){
        use crate::{TrialCommand,TrialEvent};
        async fn frame(peer:&mut tokio::net::TcpStream)->Vec<u8>{let n=peer.read_u16_le().await.unwrap();let mut out=vec![0;n as usize];peer.read_exact(&mut out).await.unwrap();out[4..].to_vec()}
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());session.phase=Phase::InWorld;session.selected=42;
        let mut header=crate::model::CharacterState::default();header.character.id=42;session.selected_state=Some(header);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        session.game_msg(Ok(game::ServerMsg::Trial(TrialEvent::SummonJury{trial_id:7,court:1,jury_number:-1}))).await;
        assert_eq!(&frame(&mut peer).await[..2],&[0x72,0]);assert!(matches!(events.try_recv(),Ok(Event::Trial(_))));assert!(matches!(events.try_recv(),Ok(Event::JurySummonReplyRequested{..})));
        let confirm=TrialCommand::ConfirmCrimeRecords{trial_id:7,jury_number:-1};
        for _ in 0..2{
            let before=session.received_trials.borrow().clone();session.command(Command::Trial{character_id:42,command:confirm}).await;
            assert_eq!(frame(&mut peer).await,[0x73,0,7,0,0,0,255,255,255,255]);assert!(matches!(events.try_recv(),Ok(Event::TrialRequested{character_id:42,command}) if command==confirm));
            assert_eq!(*session.received_trials.borrow(),before,"transport writes never add an OK count or pending receipt");
        }
        // Verdict uses the captured native seat but does not invent a role/phase gate.
        session.received_trials.borrow_mut().is_jury=false;
        let verdict=TrialCommand::ChooseVerdict{trial_id:7,jury_number:-1,choice:6};
        for _ in 0..2 {
            let before=session.received_trials.borrow().clone();session.command(Command::Trial{character_id:42,command:verdict}).await;
            assert_eq!(frame(&mut peer).await,[0x75,0,7,0,0,0,255,255,255,255,6]);assert!(matches!(events.try_recv(),Ok(Event::TrialRequested{command,..}) if command==verdict));assert_eq!(*session.received_trials.borrow(),before);
        }
        session.command(Command::Trial{character_id:42,command:TrialCommand::ChooseVerdict{trial_id:7,jury_number:-1,choice:0}}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        session.received_trials.borrow_mut().is_jury=true;
        let cancel=TrialCommand::CancelTrial{trial_id:7};session.command(Command::Trial{character_id:42,command:cancel}).await;
        assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.game_msg(Ok(game::ServerMsg::Trial(TrialEvent::Waiting{order:1,sentence_time_ms:0}))).await;assert!(matches!(events.try_recv(),Ok(Event::Trial(_))));
        let before=session.received_trials.borrow().clone();session.command(Command::Trial{character_id:42,command:cancel}).await;
        assert_eq!(frame(&mut peer).await,[0x74,0,7,0,0,0]);assert!(matches!(events.try_recv(),Ok(Event::TrialRequested{character_id:42,command}) if command==cancel));assert_eq!(*session.received_trials.borrow(),before);
        session.command(Command::Trial{character_id:41,command:confirm}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        session.command(Command::Trial{character_id:42,command:TrialCommand::ConfirmCrimeRecords{trial_id:8,jury_number:-1}}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        session.command(Command::Trial{character_id:42,command:TrialCommand::ConfirmCrimeRecords{trial_id:7,jury_number:0}}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.game.as_mut().unwrap().shutdown().await;session.command(Command::Trial{character_id:42,command:confirm}).await;
        let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::TrialRequested{..}));rejected|=matches!(e,Event::TrialRequestRejected{..});}assert!(rejected);
    }
    #[tokio::test]
    async fn initial_trial_task_receipts_never_reply_automatically_and_commands_use_captured_task_context(){
        use crate::{TrialCommand,TrialEvent};
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());session.phase=Phase::InWorld;session.selected=42;
        let mut header=crate::model::CharacterState::default();header.character.id=42;session.selected_state=Some(header);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        for receipt in [TrialEvent::ImprisonOrTrialOffered{crime_value:-1,jail_minutes:60},TrialEvent::JuryInvited{defendant_name:b"A\0tail".to_vec(),trial_id:999},TrialEvent::JurySeated{raw_is_west:255,trial_id:999,court:-1,jury_number:4}] {
            session.game_msg(Ok(game::ServerMsg::Trial(receipt.clone()))).await;assert!(matches!(events.try_recv(),Ok(Event::Trial(e))if e==receipt));assert!(events.try_recv().is_err());
        }
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());assert_eq!(session.received_trials.borrow().trial_id,0);
        for(command,body)in [
            (TrialCommand::ReplyImprisonOrTrial{request_trial:false},vec![0x6f,0,0]),
            (TrialCommand::ReplyJuryInvitation{accept:true,trial_id:999},vec![0x71,0,1,231,3,0,0]),
            (TrialCommand::SkipFinalStatement{trial_id:999},vec![0x70,0,231,3,0,0])]{
            let before=session.received_trials.borrow().clone();session.command(Command::Trial{character_id:42,command}).await;
            let n=peer.read_u16_le().await.unwrap();let mut actual=vec![0;n as usize];peer.read_exact(&mut actual).await.unwrap();assert_eq!(actual[4..],body);
            assert!(matches!(events.try_recv(),Ok(Event::TrialRequested{command:c,..})if c==command));assert_eq!(*session.received_trials.borrow(),before);
            session.command(Command::Trial{character_id:41,command}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        }
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
    }

    #[tokio::test]
    async fn trial_control_identity_header_missing_send_and_lifecycle_are_checked_at_write(){
        use crate::{TrialCommand,TrialEvent};let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let command=TrialCommand::CancelTrial{trial_id:0};
        session.command(Command::Trial{character_id:42,command}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        session.phase=Phase::InWorld;session.selected=42;session.emit(Event::Trial(TrialEvent::Waiting{order:0,sentence_time_ms:0}));events.try_recv().unwrap();
        let mut header=crate::model::CharacterState::default();header.character.id=43;session.selected_state=Some(header.clone());
        session.command(Command::Trial{character_id:42,command}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));
        header.character.id=42;session.selected_state=Some(header);let before=session.received_trials.borrow().clone();
        session.command(Command::Trial{character_id:42,command}).await;assert!(matches!(events.try_recv(),Ok(Event::TrialRequestRejected{..})));assert_eq!(*session.received_trials.borrow(),before);
        session.emit(Event::CharacterSelected(Box::default()));events.try_recv().unwrap();assert_eq!(*session.received_trials.borrow(),crate::Trials::default());assert!(command.validate_received(&session.received_trials.borrow()).is_err());assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn genuine_jury_summons_reply_exactly_once_each_without_choices_and_keep_raw_receipts() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        session.phase=Phase::InWorld;session.selected=42;
        let mut header=crate::model::CharacterState::default();header.character.id=42;session.selected_state=Some(header);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        let receipt=crate::TrialEvent::SummonJury{trial_id:0,court:u32::MAX,jury_number:i32::MIN};
        let expected=game::ClientMsg::JurySummoned{trial_id:0,court:-1,jury_number:i32::MIN}.encode_body();
        let mut received=crate::Trials::default();
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::Trial(receipt.clone()))).await;
            let raw=events.try_recv().unwrap();assert!(matches!(&raw,Event::Trial(actual) if *actual==receipt));received.apply(&raw);
            let requested=events.try_recv().unwrap();assert!(matches!(requested,Event::JurySummonReplyRequested{character_id:42,trial_id:0,court:u32::MAX,jury_number:i32::MIN}));
            let before=received.clone();received.apply(&requested);assert_eq!(received,before,"transport notification never changes received trial state");
            let n=peer.read_u16_le().await.unwrap();let mut frame=vec![0;n as usize];peer.read_exact(&mut frame).await.unwrap();
            assert_eq!(&frame[4..6],&[0x72,0]);assert_eq!(&frame[6..],expected);assert!(events.try_recv().is_err());
        }
        assert!(received.is_jury);assert_eq!(received.trial_id,0);
        session.game.as_mut().unwrap().shutdown().await;
        session.game_msg(Ok(game::ServerMsg::Trial(receipt.clone()))).await;
        assert!(matches!(events.try_recv(),Ok(Event::Trial(actual)) if actual==receipt));
        let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::JurySummonReplyRequested{..}));rejected|=matches!(e,Event::JurySummonReplyRejected{character_id:42,..});}assert!(rejected);
    }
    #[tokio::test]
    async fn jury_summon_automatic_reply_refuses_inactive_stale_header_and_missing_connection() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let receipt=crate::TrialEvent::SummonJury{trial_id:7,court:1,jury_number:-1};
        for (phase,selected,header_id) in [(Phase::CharacterSelect,42,42),(Phase::InWorld,0,0),(Phase::InWorld,42,43),(Phase::InWorld,42,42)] {
            session.phase=phase;session.selected=selected;let mut header=crate::model::CharacterState::default();header.character.id=header_id;session.selected_state=Some(header);
            session.game_msg(Ok(game::ServerMsg::Trial(receipt.clone()))).await;
            assert!(matches!(events.try_recv(),Ok(Event::Trial(actual)) if actual==receipt));
            assert!(matches!(events.try_recv(),Ok(Event::JurySummonReplyRejected{character_id,..}) if character_id==selected));assert!(events.try_recv().is_err());
        }
        // With a live socket, a stale actual header still sends no bytes.
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        session.selected_state=None;session.game_msg(Ok(game::ServerMsg::Trial(receipt))).await;
        assert!(matches!(events.try_recv(),Ok(Event::Trial(_))));assert!(matches!(events.try_recv(),Ok(Event::JurySummonReplyRejected{..})));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
    }

    #[tokio::test]
    async fn jury_number_request_is_empty_and_rechecks_active_identity_and_failed_writes() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let request=||Command::RequestJuryWaitingNumber{character_id:42};
        session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::JuryWaitingNumberRequestRejected{character_id:42,..})));
        session.phase=Phase::InWorld;session.selected=42;session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::JuryWaitingNumberRequestRejected{..})));
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        session.selected=43;session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::JuryWaitingNumberRequestRejected{..})));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.selected=42;session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::JuryWaitingNumberRequested{character_id:42})));
        let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();let mut frame=vec![0;u16::from_le_bytes(length)as usize];peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[4..],&[0x79,0]);assert!(events.try_recv().is_err(),"request is not a queue receipt");
        session.game.as_mut().unwrap().shutdown().await;session.command(request()).await;
        let mut rejected=false;while let Ok(event)=events.try_recv(){assert!(!matches!(event,Event::JuryWaitingNumberRequested{..}));rejected|=matches!(event,Event::JuryWaitingNumberRequestRejected{..});}assert!(rejected);
    }

    #[tokio::test]
    async fn family_commands_recheck_captured_character_and_only_notify_actual_writes() {
        use crate::FamilyCommand;
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        let request = || Command::Family { character_id: 42, command: FamilyCommand::Leave };
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::FamilyRequestRejected { character_id:42,.. })));
        session.phase=Phase::InWorld; session.selected=42;
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::FamilyRequestRejected { .. })));
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());
        let(mut peer,_)=accepted.unwrap(); session.game=Some(connection.unwrap());
        session.selected=43; // A queued request from the prior character cannot cross relogin.
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::FamilyRequestRejected { .. })));
        session.selected=42;
        session.command(Command::Family { character_id:42,command:FamilyCommand::Kick { member_id:0 } }).await;
        assert!(matches!(events.try_recv(), Ok(Event::FamilyRequestRejected { .. })));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::FamilyRequested {character_id:42,command:FamilyCommand::Leave})));
        let mut length=[0;2]; peer.read_exact(&mut length).await.unwrap();
        let mut frame=vec![0;u16::from_le_bytes(length) as usize]; peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[4..], &[0x1c,0]);
        assert!(events.try_recv().is_err(), "sending never creates a Family receipt");
        session.game.as_mut().unwrap().shutdown().await;
        session.command(request()).await;
        let mut rejected=false;
        while let Ok(event)=events.try_recv() {
            assert!(!matches!(event,Event::FamilyRequested {..}));
            rejected |= matches!(event,Event::FamilyRequestRejected {..});
        }
        assert!(rejected);
    }

    #[tokio::test]
    async fn expedition_commands_recheck_captured_character_and_only_notify_actual_writes() {
        use crate::ExpeditionCommand;
        let (tx, mut events) = unbounded_channel();
        let mut session = Session::new(tx, ClientOptions::default());
        let request = || Command::Expedition { character_id: 42, command: ExpeditionCommand::Leave };
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::ExpeditionRequestRejected { character_id:42,.. })));
        session.phase=Phase::InWorld; session.selected=42;
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::ExpeditionRequestRejected { .. })));
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());
        let(mut peer,_)=accepted.unwrap(); session.game=Some(connection.unwrap());
        session.selected=43; // A queued request from the prior character cannot cross relogin.
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::ExpeditionRequestRejected { .. })));
        session.selected=42;
        session.command(Command::Expedition { character_id:42,command:ExpeditionCommand::Kick { member_id:0 } }).await;
        assert!(matches!(events.try_recv(), Ok(Event::ExpeditionRequestRejected { .. })));
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.command(request()).await;
        assert!(matches!(events.try_recv(), Ok(Event::ExpeditionRequested {character_id:42,command:ExpeditionCommand::Leave})));
        let mut length=[0;2]; peer.read_exact(&mut length).await.unwrap();
        let mut frame=vec![0;u16::from_le_bytes(length) as usize]; peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[4..], &[0x0e,0]);
        assert!(events.try_recv().is_err(), "sending never creates an Expedition receipt");
        for command in [ExpeditionCommand::Create{name:vec![255,0],sponsor_id:0},ExpeditionCommand::Rename{expedition_id:9,name:vec![128,0]}] {
            let mut expected=crate::wire::Writer::new();command.write(&mut expected);let body=expected.finish();let opcode=command.opcode();
            session.command(Command::Expedition{character_id:42,command:command.clone()}).await;
            assert!(matches!(events.try_recv(),Ok(Event::ExpeditionRequested{character_id:42,command:sent}) if sent==command));
            peer.read_exact(&mut length).await.unwrap();let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
            assert_eq!(u16::from_le_bytes([frame[4],frame[5]]),opcode);assert_eq!(&frame[6..],body);
            assert!(events.try_recv().is_err(),"creation/rename sends cannot fabricate settlement");
        }

        session.game.as_mut().unwrap().shutdown().await;
        session.command(request()).await;
        let mut rejected=false;
        while let Ok(event)=events.try_recv() {
            assert!(!matches!(event,Event::ExpeditionRequested {..}));
            rejected |= matches!(event,Event::ExpeditionRequestRejected {..});
        }
        assert!(rejected);
    }

    #[tokio::test]
    async fn gm_mode_reports_preserve_repeats_unknown_indices_and_raw_values() {
        let (tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let raw=crate::unit_msg::UnitGmModeChanged{obj_id:7,mode:-1,value:255};
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::UnitGmModeChanged(raw),game::ServerMsg::UnitGmModeChanged(raw)]))).await;
        for _ in 0..2 {assert!(matches!(events.try_recv(),Ok(Event::UnitGmModeChanged(value)) if value==raw));}
        assert!(events.try_recv().is_err());assert_eq!(session.self_obj,0);
    }

    #[tokio::test]
    async fn expedition_receipts_forward_empty_and_repeated_batches() {
        let (evt,mut events)=unbounded_channel();let mut session=Session::new(evt,ClientOptions::default());
        let receipt=crate::ExpeditionEvent::Members{total:21,expedition_id:9,members:vec![]};
        for value in [receipt.clone(),receipt,crate::ExpeditionEvent::Policies(vec![]),
            crate::ExpeditionEvent::RoleChanged{character_id:7,role:2,name:vec![255,0]},
            crate::ExpeditionEvent::PolicyChanged{policy:crate::ExpeditionRolePolicy::default(),success:2},
            crate::ExpeditionEvent::OwnerChanged{old_owner_id:7,new_owner_id:8,name:vec![0]},
            crate::ExpeditionEvent::Dismissed{expedition_id:9,success:255},
            crate::ExpeditionEvent::MembershipChanged{obj_id:77,character_id:7,kicker:vec![255],name:vec![0],old_expedition_id:9,new_expedition_id:0,expel:2}] {
            session.game_msg(Ok(game::ServerMsg::Expedition(value.clone()))).await;
            assert!(matches!(events.try_recv(),Ok(Event::Expedition(actual)) if actual==value));
        }
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn trial_receipts_forward_order_even_when_summon_reply_is_locally_refused() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let value=crate::TrialEvent::SummonJury{trial_id:7,court:u32::MAX,jury_number:-1};
        for receipt in [value.clone(),value,crate::TrialEvent::JuryWaiting{count:-1,total:0,sentence_time_ms:i32::MIN}] {
            session.game_msg(Ok(game::ServerMsg::Trial(receipt.clone()))).await;
            assert!(matches!(events.try_recv(),Ok(Event::Trial(actual)) if actual==receipt));
            if matches!(receipt,crate::TrialEvent::SummonJury{..}) {assert!(matches!(events.try_recv(),Ok(Event::JurySummonReplyRejected{character_id:0,..})));}
        }
        assert!(events.try_recv().is_err(),"no inferred jury choice or seat response");
    }

    #[tokio::test]
    async fn ranking_receipts_forward_every_duplicate_without_rewards_or_name_fabrication(){
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let value=crate::RankingEvent::Alarm{rank_id:7,raw_kind:255};for receipt in [value.clone(),value,crate::RankingEvent::RewardMail{rank_id:7}]{session.game_msg(Ok(game::ServerMsg::Ranking(receipt.clone()))).await;assert!(matches!(events.try_recv(),Ok(Event::Ranking(actual))if actual==receipt));}assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn bounty_receipts_forward_duplicates_without_automatic_payments() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let receipt=crate::BountyEvent::Paid{hunter:b"H\0tail".to_vec(),arrested:vec![255],money:i64::MIN};
        for _ in 0..2 {session.game_msg(Ok(game::ServerMsg::Bounty(receipt.clone()))).await;assert!(matches!(events.try_recv(),Ok(Event::Bounty(actual)) if actual==receipt));}
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn crime_receipts_forward_every_duplicate_without_requests_or_fake_choices() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        for receipt in [crate::CrimeEvent::JuryPointChanged{jury_point:i32::MIN},crate::CrimeEvent::JuryPointChanged{jury_point:i32::MIN},crate::CrimeEvent::JuryWaitingNumber{waiting_number:-1}] {
            session.game_msg(Ok(game::ServerMsg::Crime(receipt))).await;
            assert!(matches!(events.try_recv(),Ok(Event::Crime(actual)) if actual==receipt));
        }
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn achievement_receipts_forward_duplicates_and_raw_values_without_state_success() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let receipts=[crate::AchievementEvent::Completed{id:7,completed_at:i64::MIN},crate::AchievementEvent::Completed{id:7,completed_at:i64::MIN},crate::AchievementEvent::ItemSent{id:7,by_mail:255},crate::AchievementEvent::List{raw_count:-1,rows:vec![]}];
        for receipt in receipts {session.game_msg(Ok(game::ServerMsg::Achievement(receipt.clone()))).await;assert!(matches!(events.try_recv(),Ok(Event::Achievement(actual)) if actual==receipt));}
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn dominion_receipts_and_unhandled_tail_metadata_reach_consumers_in_order() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let receipt=crate::DominionEvent::Deleted{id:25};
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::Dominion(receipt.clone()),
            game::ServerMsg::Unhandled{level:game::LEVEL_COMPRESSED,opcode:0x3ff,len:42},game::ServerMsg::Dominion(receipt.clone())]))).await;
        assert!(matches!(events.try_recv(),Ok(Event::Dominion(value)) if value==receipt));
        assert!(matches!(events.try_recv(),Ok(Event::UnhandledGamePacket{level:4,opcode:0x3ff,len:42})));
        assert!(matches!(events.try_recv(),Ok(Event::Dominion(value)) if value==receipt));assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn actual_game_decode_failures_are_visible_instead_of_silent_coverage_loss() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        session.game_msg(Err(NetError::Wire(crate::wire::WireError::Invalid("truncated".into())))).await;
        assert!(matches!(events.try_recv(),Ok(Event::GameDecodeFailed{message}) if message.contains("truncated")));
        session.game_msg(Ok(game::ServerMsg::Batch(vec![game::ServerMsg::DecodeFailed{message:"bad compressed tail".into()}]))).await;
        assert!(matches!(events.try_recv(),Ok(Event::GameDecodeFailed{message}) if message=="bad compressed tail"));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn character_names_stream_join_identity_duplicates_and_failed_write(){
      let(tx,mut events)=unbounded_channel();let mut s=Session::new(tx,ClientOptions::default());s.phase=Phase::InWorld;s.selected=42;let mut header=CharacterState::default();header.character.id=42;s.selected_state=Some(header);
      s.clear_character_name_stream();assert!(events.try_recv().is_err());
      let command=||Command::QueryCharacterName{character_id:42,requested_id:7};
      s.command(command()).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));
      let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();let(connection,accepted)=tokio::join!(StreamConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();s.stream=Some(connection.unwrap());
      s.command(command()).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));
      s.stream_msg(Ok(stream::ServerMsg::JoinResponse{code:0}));
      for bad in [Command::QueryCharacterName{character_id:41,requested_id:7},Command::QueryCharacterName{character_id:42,requested_id:0}]{s.command(bad).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));}
      s.selected_state.as_mut().unwrap().character.id=41;s.command(command()).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));s.selected_state.as_mut().unwrap().character.id=42;
      s.command(command()).await;let n=peer.read_u16_le().await.unwrap();let mut bytes=vec![0;n as usize];peer.read_exact(&mut bytes).await.unwrap();assert_eq!(bytes,[10,0,7,0,0,0]);assert!(s.pending_names.contains(&7));assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequested{requested_id:7,..})));
      s.command(command()).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));assert!(s.pending_names.contains(&7));
      for name in [b"A".to_vec(),b"B".to_vec(),b"B".to_vec()]{let row=crate::CharacterName{character_id:7,name};s.stream_msg(Ok(stream::ServerMsg::CharacterName(row.clone())));assert!(matches!(events.try_recv(),Ok(Event::CharacterName(actual)) if actual==row));assert!(!s.pending_names.contains(&7));}
      s.stream.as_mut().unwrap().shutdown().await;s.command(command()).await;assert!(matches!(events.try_recv(),Ok(Event::CharacterNameRequestRejected{..})));assert!(!s.pending_names.contains(&7));
    }

    #[tokio::test]
    async fn ranking_requests_received_board_identity_loopback_and_no_optimism(){
      use crate::{RankingCommand as C,RankingEvent,RankCharacterRow};
      async fn frame(peer:&mut tokio::net::TcpStream)->Vec<u8>{let n=peer.read_u16_le().await.unwrap();let mut bytes=vec![0;n as usize];peer.read_exact(&mut bytes).await.unwrap();bytes[4..].to_vec()}
      let(tx,mut events)=unbounded_channel();let mut s=Session::new(tx,ClientOptions::default());s.phase=Phase::InWorld;s.selected=42;let mut header=CharacterState::default();header.character.id=42;s.selected_state=Some(header.clone());s.emit(Event::CharacterSelected(Box::new(header)));while events.try_recv().is_ok(){}
      s.command(Command::Ranking{character_id:42,command:C::RequestPlayerRecords}).await;let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::RankingRequested{..}|Event::Ranking(_)));rejected|=matches!(e,Event::RankingRequestRejected{..});}assert!(rejected);
      let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();s.game=Some(connection.unwrap());
      s.command(Command::Ranking{character_id:42,command:C::RequestSnapshots{rank_id:7}}).await;assert!(matches!(events.try_recv(),Ok(Event::RankingRequestRejected{..})));
      let row=RankCharacterRow{account_id:1,account:vec![],character_id:42,rank_id:7,world_id:1,v1:0,raw_sum_v1:0,v2:0,raw_sum_v2:0,recorded_raw:0};s.emit(Event::Ranking(RankingEvent::Personal{type_id:1,error:0,ack_time_raw:123,count:1,rows:vec![row]}));assert!(matches!(events.try_recv(),Ok(Event::Ranking(_))));
      for command in [C::RequestPlayerRecords,C::RequestSnapshots{rank_id:7}]{let before=s.received_rankings.borrow().clone();s.command(Command::Ranking{character_id:42,command}).await;let mut expected=command.opcode().to_le_bytes().to_vec();expected.extend(game::ClientMsg::Ranking(command).encode_body());assert_eq!(frame(&mut peer).await,expected);assert!(matches!(events.try_recv(),Ok(Event::RankingRequested{command:c,..})if c==command));assert_eq!(*s.received_rankings.borrow(),before);}
      s.command(Command::Ranking{character_id:41,command:C::RequestPlayerRecords}).await;assert!(matches!(events.try_recv(),Ok(Event::RankingRequestRejected{..})));
      s.selected_state.as_mut().unwrap().character.id=41;s.command(Command::Ranking{character_id:42,command:C::RequestPlayerRecords}).await;assert!(matches!(events.try_recv(),Ok(Event::RankingRequestRejected{..})));s.selected_state.as_mut().unwrap().character.id=42;
      s.phase=Phase::CharacterSelect;s.command(Command::Ranking{character_id:42,command:C::RequestPlayerRecords}).await;assert!(matches!(events.try_recv(),Ok(Event::RankingRequestRejected{..})));s.phase=Phase::InWorld;
      s.received_rankings.borrow_mut().character_id=Some(41);s.command(Command::Ranking{character_id:42,command:C::RequestSnapshots{rank_id:7}}).await;assert!(matches!(events.try_recv(),Ok(Event::RankingRequestRejected{..})));
      assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());s.game.as_mut().unwrap().shutdown().await;s.command(Command::Ranking{character_id:42,command:C::RequestPlayerRecords}).await;let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::RankingRequested{..}|Event::Ranking(_)));rejected|=matches!(e,Event::RankingRequestRejected{..});}assert!(rejected);
    }

    #[tokio::test]
    async fn auction_captured_character_loopback_and_complete_send_rejection(){
        use crate::auction::{AuctionCommand as C,AuctionSearch,AuctioneerIds,AuctionEvent};
        let(tx,mut events)=unbounded_channel();let mut s=Session::new(tx,ClientOptions::default());s.phase=Phase::InWorld;s.selected=42;let mut header=CharacterState::default();header.character.id=42;s.selected_state=Some(header);
        let command=C::Search{auctioneer:AuctioneerIds::default(),query:AuctionSearch::default()};
        s.command(Command::AuctionForCharacter{character_id:42,command:command.clone()}).await;let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::Auction(AuctionEvent::Requested(_))));rejected|=matches!(e,Event::Auction(AuctionEvent::RequestRejected{..}));}assert!(rejected);
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();s.game=Some(connection.unwrap());
        s.command(Command::AuctionForCharacter{character_id:41,command:command.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::Auction(AuctionEvent::RequestRejected{..}))));
        s.selected_state.as_mut().unwrap().character.id=41;s.command(Command::AuctionForCharacter{character_id:42,command:command.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::Auction(AuctionEvent::RequestRejected{..}))));s.selected_state.as_mut().unwrap().character.id=42;
        s.phase=Phase::CharacterSelect;s.command(Command::AuctionForCharacter{character_id:42,command:command.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::Auction(AuctionEvent::RequestRejected{..}))));s.phase=Phase::InWorld;
        s.command(Command::AuctionForCharacter{character_id:42,command:command.clone()}).await;let n=peer.read_u16_le().await.unwrap();let mut raw=vec![0;n as usize];peer.read_exact(&mut raw).await.unwrap();let mut expected=command.opcode().to_le_bytes().to_vec();expected.extend(game::ClientMsg::Auction(command.clone()).encode_body());assert_eq!(&raw[4..],expected);assert!(matches!(events.try_recv(),Ok(Event::Auction(AuctionEvent::Requested(actual)))if actual==command));assert!(events.try_recv().is_err());
        s.game.as_mut().unwrap().shutdown().await;s.command(Command::AuctionForCharacter{character_id:42,command:command.clone()}).await;let mut rejected=false;while let Ok(e)=events.try_recv(){assert!(!matches!(e,Event::Auction(AuctionEvent::Requested(_))));rejected|=matches!(e,Event::Auction(AuctionEvent::RequestRejected{..}));}assert!(rejected);
        s.command(Command::Auction(command)).await;assert!(matches!(events.try_recv(),Ok(Event::Auction(AuctionEvent::RequestRejected{..}))));
    }

    #[tokio::test]
    async fn remove_buff_captures_character_actor_and_only_reports_actual_transport() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        session.phase=Phase::InWorld;session.selected=42;session.self_obj=0x010203;
        let mut header=crate::model::CharacterState::default();header.character.id=42;session.selected_state=Some(header);
        let request=||Command::RemoveBuff{character_id:42,obj_id:0x010203,buff_index:7};
        session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::BuffRemovalRequestRejected{..})));
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();
        let(connection,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(connection.unwrap());
        for (character_id,obj_id) in [(0,0x010203),(41,0x010203),(42,0),(42,0x010204),(42,0x1000000)] {
            session.command(Command::RemoveBuff{character_id,obj_id,buff_index:7}).await;
            assert!(matches!(events.try_recv(),Ok(Event::BuffRemovalRequestRejected{..})));
        }
        session.selected_state.as_mut().unwrap().character.id=41;session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::BuffRemovalRequestRejected{..})));
        session.selected_state.as_mut().unwrap().character.id=42;
        session.phase=Phase::Idle;session.command(request()).await;
        assert!(matches!(events.try_recv(),Ok(Event::BuffRemovalRequestRejected{..})));session.phase=Phase::InWorld;
        assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        for buff_index in [0,7,u32::MAX] {
            session.command(Command::RemoveBuff{character_id:42,obj_id:0x010203,buff_index}).await;
            let n=peer.read_u16_le().await.unwrap();let mut frame=vec![0;n as usize];peer.read_exact(&mut frame).await.unwrap();
            assert_eq!(&frame[4..6],&[0x55,0]);assert_eq!(&frame[6..],game::ClientMsg::RemoveBuff{obj_id:0x010203,buff_index,reason:0}.encode_body());
            assert!(matches!(events.try_recv(),Ok(Event::BuffRemovalRequested{character_id:42,obj_id:0x010203,buff_index:i})if i==buff_index));
            assert!(events.try_recv().is_err(),"no fabricated BuffRemoved acknowledgement");
        }
        session.game.as_mut().unwrap().shutdown().await;session.command(request()).await;
        let mut rejected=false;while let Ok(event)=events.try_recv(){assert!(!matches!(event,Event::BuffRemovalRequested{..}));rejected|=matches!(event,Event::BuffRemovalRequestRejected{..});}assert!(rejected);
    }

    #[tokio::test]
    async fn buff_updated_forwards_each_raw_receipt_without_actor_or_success_fabrication() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::BuffUpdated{obj_id:99,index:u32::MAX,stacks:i32::MIN,charged:-1})).await;
            assert!(matches!(events.try_recv(),Ok(Event::BuffUpdated{obj_id:99,index:u32::MAX,stacks:i32::MIN,charged:-1})));
            assert!(events.try_recv().is_err());
        }
        assert_eq!(session.self_obj,0);assert!(session.selected_state.is_none());assert!(session.game.is_none());
    }

    #[tokio::test]
    async fn socket_results_forward_raw_repeats_without_inventory_or_ack_fabrication() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let mut world=crate::state::WorldState::default();
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::LunagemSocketingResult{raw_result:255,item_id:u64::MAX,template_id:u32::MAX,raw_install:254})).await;
            let event=events.try_recv().unwrap();assert!(matches!(event,Event::LunagemSocketingResult{raw_result:255,item_id:u64::MAX,template_id:u32::MAX,raw_install:254}));world.apply(&event);
            session.game_msg(Ok(game::ServerMsg::LunastoneSocketingResult{raw_result:0,item_id:0,template_id:0})).await;
            let event=events.try_recv().unwrap();assert!(matches!(event,Event::LunastoneSocketingResult{raw_result:0,item_id:0,template_id:0}));world.apply(&event);
            assert!(events.try_recv().is_err());
        }
        assert_eq!(session.self_obj,0);assert!(session.selected_state.is_none());assert!(session.game.is_none());
        assert!(world.player.spawn.is_none());assert!(world.inventory.bag.is_empty());assert!(world.inventory.bank.is_empty());
    }

    #[tokio::test]
    async fn grade_results_forward_raw_repeats_without_item_mutation() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());let mut world=crate::state::WorldState::default();
        let result=crate::item_effect::GradeEnchantResult{raw_result:255,item:None,item_wire:vec![0;4],before_grade:255,after_grade:0};
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::GradeEnchantResult(result.clone()))).await;
            let event=events.try_recv().unwrap();assert!(matches!(&event,Event::GradeEnchantResult(v) if v==&result));world.apply(&event);
            let announced=crate::item_effect::GradeEnchantBroadcast{raw_name:vec![255,0,1],result:result.clone()};
            session.game_msg(Ok(game::ServerMsg::GradeEnchantBroadcast(announced.clone()))).await;
            let event=events.try_recv().unwrap();assert!(matches!(&event,Event::GradeEnchantBroadcast(v) if v==&announced));world.apply(&event);
            assert!(events.try_recv().is_err());
        }
        assert!(session.selected_state.is_none());assert!(session.game.is_none());assert!(world.inventory.bag.is_empty());assert!(world.player.spawn.is_none());
    }

    #[tokio::test]
    async fn ucc_results_forward_every_receipt_without_appearance_mutation() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());let mut world=crate::state::WorldState::default();
        for ucc_id in [u64::MAX,u64::MAX,0] {
            session.game_msg(Ok(game::ServerMsg::ItemUccDataChanged{ucc_id,player_id:u32::MAX,target_item_id:u64::MAX})).await;
            let e=events.try_recv().unwrap();assert!(matches!(&e,Event::ItemUccDataChanged{ucc_id:id,player_id:u32::MAX,target_item_id:u64::MAX} if *id==ucc_id));world.apply(&e);assert!(events.try_recv().is_err());
        }
        assert!(session.selected_state.is_none());assert!(session.game.is_none());assert!(world.inventory.bag.is_empty());assert!(world.inventory.bank.is_empty());assert!(world.player.spawn.is_none());
    }

    #[tokio::test]
    async fn schedule_receipts_preserve_duplicates_without_claim_or_cache_success() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());let mut world=crate::state::WorldState::default();
        let row=crate::schedule::ScheduleItem{schedule_type:7,raw_gave:255,accumulated:i32::MIN,raw_updated:i64::MAX};
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::ScheduleItems{declared_count:2,items:vec![row.clone();2]})).await;
            let e=events.try_recv().unwrap();assert!(matches!(&e,Event::ScheduleItems{declared_count:2,items} if items==&vec![row.clone();2]));world.apply(&e);
            session.game_msg(Ok(game::ServerMsg::ScheduleItemSent{schedule_type:7,raw_by_mail:255})).await;
            let e=events.try_recv().unwrap();assert!(matches!(&e,Event::ScheduleItemSent{schedule_type:7,raw_by_mail:255}));world.apply(&e);assert!(events.try_recv().is_err());
        }
        assert!(session.game.is_none());assert!(world.inventory.bag.is_empty());assert!(world.inventory.bank.is_empty());assert!(world.player.spawn.is_none());
    }

    fn seed_item_effect_records(session:&mut Session,items:Vec<Option<Item>>) -> InvenChunk {
        assert_eq!(items.len(),10);
        let mut spawn=crate::native_item_records::tests::spawn();
        spawn.obj_id=99;spawn.unit.obj_id=99;spawn.unit.kind=crate::model::UnitKind::Character{char_id:42};spawn.state.character.id=42;
        session.phase=Phase::InWorld;session.selected=42;session.self_obj=99;session.selected_state=Some(spawn.state.clone());
        session.emit(Event::CharacterSelected(Box::new(spawn.state.clone())));session.emit(Event::PlayerSpawn(Box::new(spawn)));
        session.emit(Event::InventoryInit{bag_slots:10,bank_slots:0});
        let chunk=InvenChunk{container:2,chunks:1,start_chunk:0,items,truncated:false};session.emit(Event::InventoryChunk(chunk.clone()));chunk
    }
    fn native_item_effect_request(session:&Session,mode:crate::item_effect::ItemEffectMode)->crate::item_effect::ItemEffectRequest {
        use crate::item_effect::{CapturedBagItem,ItemEffectMode,ItemEffectRequest};
        let records=session.received_native_records.borrow();
        ItemEffectRequest::new(mode,if matches!(mode,ItemEffectMode::Grade{..}){22520}else{11285},
            CapturedBagItem{slot:0,item:records.item(2,0).unwrap().clone()},
            CapturedBagItem{slot:1,item:records.item(2,1).unwrap().clone()},None).unwrap()
    }
    async fn read_item_effect_write(peer:&mut tokio::net::TcpStream,request:&crate::item_effect::ItemEffectRequest) {
        let mut length=[0;2];peer.read_exact(&mut length).await.unwrap();let mut frame=vec![0;u16::from_le_bytes(length) as usize];peer.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame[4..6],&0x52u16.to_le_bytes());assert_eq!(&frame[6..],&game::ClientMsg::ItemEffect{self_obj_id:99,request:request.clone()}.encode_body());
    }
    #[tokio::test]
    async fn item_effect_native_records_retry_supported_tasks_and_detail_without_snapshot() {
        use crate::item_effect::{CapturedBagItem,ItemEffectMode,ItemEffectRequest};
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let source=Item{id:11,template_id:101,count:2,grade:6,made_unit_id:9,..Default::default()};let target=Item{id:22,template_id:202,count:1,grade:7,..Default::default()};
        let request=ItemEffectRequest::new(ItemEffectMode::Gem,11285,CapturedBagItem{slot:0,item:source.clone()},CapturedBagItem{slot:1,item:target.clone()},None).unwrap();
        let command=|request:&ItemEffectRequest|Command::ItemEffect{character_id:42,request:request.clone()};
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));
        let mut items=vec![Some(source),Some(target)];items.resize(10,None);let chunk=seed_item_effect_records(&mut session,items);while events.try_recv().is_ok(){}
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..}))); // no transport
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();let(conn,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(conn.unwrap());
        session.emit(Event::UnhandledGamePacket{level:1,opcode:0xfff,len:20});while events.try_recv().is_ok(){}
        assert!(session.received_native_records.borrow().complete_bag());
        session.command(Command::ItemEffect{character_id:41,request:request.clone()}).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRequested{..})));read_item_effect_write(&mut peer,&request).await;
        // Native Count ignores packet ID; the legacy reducer refuses this row. The
        // retry must read native PRE-row projection rather than the old legacy copy.
        session.emit(Event::ItemsChanged(crate::items::ItemTasks{tasks:vec![crate::items::ItemTask::Count{container:2,slot:1,item_id:999,template_id:999,delta:1}],..Default::default()}));while events.try_recv().is_ok(){}
        assert_eq!(session.received_inventory.borrow().bag[1].as_ref().unwrap().count,1);
        assert_eq!(session.received_native_records.borrow().item(2,1).unwrap().count,2);
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        let after_count=native_item_effect_request(&session,ItemEffectMode::Gem);session.command(command(&after_count)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRequested{..})));read_item_effect_write(&mut peer,&after_count).await;
        session.emit(Event::ItemDetailUpdated(crate::items::ItemDetailUpdate{item_id:11,container:2,slot:0,raw_detail:vec![3,17,0,0,0,1,28]}));while events.try_recv().is_ok(){}
        assert_eq!(session.received_inventory.borrow().bag[0].as_ref().unwrap().detail,crate::model::ItemDetail::None);
        session.command(command(&after_count)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));
        let after_detail=native_item_effect_request(&session,ItemEffectMode::Gem);session.command(command(&after_detail)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRequested{..})));read_item_effect_write(&mut peer,&after_detail).await;
        // Unknown padded task detail retires only that row. Legacy post-state is
        // never adopted, and a partial093 cannot reconstruct its missing metadata.
        session.emit(Event::ItemsChanged(crate::items::ItemTasks{tasks:vec![crate::items::ItemTask::Update{container:2,slot:0,item_id:11,detail:crate::model::ItemDetail::None,detail_payload:None,raw_detail_block:None}],..Default::default()}));while events.try_recv().is_ok(){}
        assert!(session.received_native_records.borrow().item(2,0).is_none());assert!(session.received_native_records.borrow().item(2,1).is_some());
        session.command(command(&after_detail)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.emit(Event::InventoryChunk(chunk.clone()));while events.try_recv().is_ok(){}session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRequested{..})));read_item_effect_write(&mut peer,&request).await;
        session.emit(Event::GameDecodeFailed{message:"loss".into()});session.emit(Event::InventoryChunk(chunk.clone()));while events.try_recv().is_ok(){}
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        // Identical full items from a different actual record owner cannot satisfy
        // an old captured-character request, even while the selected header is old.
        let mut replacement=crate::native_item_records::tests::spawn();replacement.obj_id=99;replacement.unit.obj_id=99;
        session.emit(Event::PlayerSpawn(Box::new(replacement)));session.emit(Event::InventoryInit{bag_slots:10,bank_slots:0});session.emit(Event::InventoryChunk(chunk));while events.try_recv().is_ok(){}
        assert_eq!(session.received_native_records.borrow().owner(),Some(47));assert!(request.recheck_native(&session.received_native_records.borrow(),0).is_ok());
        session.command(command(&request)).await;assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRejected{..})));assert!(tokio::time::timeout(Duration::from_millis(15),peer.read_u8()).await.is_err());
        session.game.as_mut().unwrap().shutdown().await;while events.try_recv().is_ok(){}
    }

    #[tokio::test]
    async fn detail_update_forwards_duplicates_and_changes_only_shared_native_provider() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let before=Item{id:123,template_id:8001,count:7,grade:6,..Default::default()};
        let mut items=vec![Some(before.clone()),Some(before.clone())];items.resize(10,None);let chunk=seed_item_effect_records(&mut session,items);while events.try_recv().is_ok(){}
        let update=crate::items::ItemDetailUpdate{item_id:123,container:2,slot:0,raw_detail:vec![3,17,0,0,0,1,28]};
        for _ in 0..2 {session.game_msg(Ok(game::ServerMsg::ItemDetailUpdated(update.clone()))).await;assert!(matches!(events.try_recv(),Ok(Event::ItemDetailUpdated(got)) if got==update));assert!(events.try_recv().is_err());}
        let mut after=before.clone();after.detail=crate::model::ItemDetail::Mate{exp:17,dead:1,level:28};after.detail_payload=Some(update.raw_detail[1..].to_vec());
        assert_eq!(session.received_native_records.borrow().item(2,0),Some(&after));assert_eq!(session.received_inventory.borrow().bag[0],Some(before.clone()));
        let mut wrong=update.clone();wrong.item_id=999;session.emit(Event::ItemDetailUpdated(wrong));assert_eq!(session.received_native_records.borrow().item(2,0),Some(&after));
        let mut malformed=update.clone();malformed.raw_detail.pop();session.emit(Event::ItemDetailUpdated(malformed));assert!(session.received_native_records.borrow().item(2,0).is_none());assert!(session.received_native_records.borrow().item(2,1).is_some());
        session.emit(Event::ItemDetailUpdated(update));assert!(session.received_native_records.borrow().item(2,0).is_none());
        session.emit(Event::InventoryChunk(chunk));assert_eq!(session.received_native_records.borrow().item(2,0),Some(&before));
    }

    #[test]
    fn item_effect_native_recheck_preserves_settings_and_refuses_changed_owner_or_unknown_rows() {
        use crate::item_effect::ItemEffectMode;
        let(tx,_)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let mut items=vec![Some(Item{id:11,template_id:101,count:2,..Default::default()}),Some(Item{id:22,template_id:202,count:1,..Default::default()})];items.resize(10,None);seed_item_effect_records(&mut session,items);
        let request=native_item_effect_request(&session,ItemEffectMode::Grade{auto_use_aa_point:false});assert!(request.recheck_native(&session.received_native_records.borrow(),0).is_ok());assert!(request.recheck_native(&session.received_native_records.borrow(),1).is_err());assert!(request.recheck_native(&session.received_native_records.borrow(),255).is_ok());
        session.emit(Event::ItemsChanged(crate::items::ItemTasks{tasks:vec![crate::items::ItemTask::AutoUseAaPoint(1)],..Default::default()}));assert!(session.received_native_records.borrow().complete_bag());assert_eq!(session.received_inventory.borrow().auto_use_aa_point_raw,1);
        assert!(request.recheck_native(&session.received_native_records.borrow(),session.received_inventory.borrow().auto_use_aa_point_raw).is_err());
        session.emit(Event::UnitsRemoved(vec![99]));assert!(request.recheck_native(&session.received_native_records.borrow(),0).is_err());assert!(session.received_native_records.borrow().item(2,1).is_none());
    }

    #[tokio::test]
    async fn item_operation_error_decodes_and_forwards_without_automatic_unlock_or_content_change() {
        use crate::items::ItemOperationError;
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let before=Item{id:11,template_id:101,count:8,..Default::default()};session.received_inventory.borrow_mut().bag=vec![Some(before.clone())];session.received_inventory.borrow_mut().mark_item_slot_pending(2,0);
        for error in [0,-1,i16::MIN,i16::MAX] {
            let expected=ItemOperationError{container:2,slot:0,second_container:2,second_slot:0,error_code:error};
            let mut frame=crate::wire::Writer::new();frame.raw(&[0xdd,1,0,0]).u16(0xa0).raw(&[2,0,2,0]).i16(error);
            let raw=frame.finish();
            for _ in 0..2 {
                session.game_msg(Ok(game::ServerMsg::decode(&raw).unwrap())).await;
                assert!(matches!(events.try_recv(),Ok(Event::ItemOperationError(got)) if got==expected));assert!(events.try_recv().is_err());
                assert_eq!(session.received_inventory.borrow().bag,vec![Some(before.clone())]);assert!(session.received_inventory.borrow().item_slot_pending(2,0));
            }
        }
        assert!(session.game.is_none());assert!(session.selected_state.is_none());
    }

    #[tokio::test]
    async fn equipment_ids_forward_exact_raw_receipts_without_legacy_promotion() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        let mut items=vec![Some(Item{id:11,template_id:101,..Default::default()})];items.resize(10,None);
        seed_item_effect_records(&mut session,items);while events.try_recv().is_ok(){}
        let legacy=session.received_inventory.borrow().clone();
        let ids=[u64::MAX;28];
        for _ in 0..2 {
            session.game_msg(Ok(game::ServerMsg::EquipmentIdsChanged{obj_id:99,ids})).await;
            assert!(matches!(events.try_recv(),Ok(Event::EquipmentIdsChanged{obj_id:99,ids:got}) if got==ids));
            assert!(events.try_recv().is_err());
        }
        assert_eq!(session.received_inventory.borrow().bag,legacy.bag);assert_eq!(session.received_inventory.borrow().equipment,legacy.equipment);
        assert_eq!(session.received_native_records.borrow().item(1,0).unwrap().id,u64::MAX);
    }

    #[tokio::test]
    async fn session_catalog_seeds_native_unpack_retry_and_survives_selected_reset() {
        use crate::item_effect::ItemEffectMode;
        let catalog=std::sync::Arc::new(std::collections::HashMap::from([(26390,4874)]));
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions{item_recharge_catalog:Some(catalog),..Default::default()});
        let mut payload=vec![0;55];payload[11..19].copy_from_slice(&102i64.to_le_bytes());
        let source=Item{id:11,template_id:26390,count:2,detail:crate::model::ItemDetail::Equip(crate::model::EquipDetail{charge_time:102,..Default::default()}),detail_payload:Some(payload),..Default::default()};
        let mut items=vec![Some(source),Some(Item{id:22,template_id:202,count:1,..Default::default()})];items.resize(10,None);
        seed_item_effect_records(&mut session,items.clone());
        session.emit(Event::LeftWorld{target:0});seed_item_effect_records(&mut session,items);
        let old=native_item_effect_request(&session,ItemEffectMode::Gem);
        session.emit(Event::ItemsChanged(ItemTasks{tasks:vec![crate::items::ItemTask::Security{container:2,slot:0,item_id:999,bits:4,is_unsecure_excess:0,is_unsecure_set:0,is_unpack:1,unsecure_time:-7,unpack_time:-9}],..Default::default()}));
        assert!(matches!(&session.received_native_records.borrow().item(2,0).unwrap().detail,crate::model::ItemDetail::Equip(e) if e.charge_time == -9));
        assert!(old.recheck_native(&session.received_native_records.borrow(),0).is_err());
        let refreshed=native_item_effect_request(&session,ItemEffectMode::Gem);while events.try_recv().is_ok(){}
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap().to_string();let(conn,accepted)=tokio::join!(GameConn::connect(&addr),listener.accept());let(mut peer,_)=accepted.unwrap();session.game=Some(conn.unwrap());
        session.command(Command::ItemEffect{character_id:42,request:refreshed.clone()}).await;
        assert!(matches!(events.try_recv(),Ok(Event::ItemEffectRequested{..})));read_item_effect_write(&mut peer,&refreshed).await;
        session.game.as_mut().unwrap().shutdown().await;
    }

    #[tokio::test]
    async fn specialty_ratio_forwards_raw_duplicates_without_game_gate_or_optimism() {
        let(tx,mut events)=unbounded_channel();let mut session=Session::new(tx,ClientOptions::default());
        for ratio in [0,117,-1,117,i32::MIN,i32::MAX] {
            session.game_msg(Ok(game::ServerMsg::SpecialtyRatio(ratio))).await;
            assert!(matches!(events.try_recv(),Ok(Event::SpecialtyRatio(value)) if value==ratio));
            assert!(events.try_recv().is_err());assert!(session.game.is_none());assert_eq!(session.self_obj,0);
        }
    }

}
