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

use std::time::{Duration, Instant};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::buff::{Buff, BuffCaster};
use crate::game::{self, LeaveTarget, VisualOptions};
use crate::items::{ActionSlot, InvenChunk, ItemMove, ItemTasks};
use crate::quest::{CompletedBlock, Quest};
use crate::skill::{PlotEvent, SkillFired, SkillStarted, SkillTarget, StartCaster, UnitDamage, UnitHeal};
use crate::unit_msg::AggroEntry;
use crate::login::{self, AccountInfo, Secret, WorldCookie, WorldList, WorldQueue};
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
pub struct ClientOptions {
    /// Interval of the keep-alive `PingPacket`, `None` for no pings at all.
    ///
    /// The server drops a game connection whose last ping is older than 30 s. It checks every 30 s
    /// and treats a connection that never pinged as dead, so a client must ping soon after it
    /// connects and then at least every few seconds.
    pub ping_interval: Option<Duration>,
    /// How long after the game connection opens the first ping goes out (default: at once).
    pub first_ping_delay: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { ping_interval: Some(PING_INTERVAL), first_ping_delay: Duration::ZERO }
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
    /// Cast `skill_id` (a skill the character knows, `Unit::skills`) at the unit `target_obj_id`
    /// (0 = the player itself, for self buffs and heals). The answer is `SkillStarted`: with a
    /// non zero `result` when the server refuses.
    UseSkill { skill_id: u32, target_obj_id: u32 },
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
    Bank(crate::bank::BankCommand),
    /// Party invitations, replies, leaving, kicking and leadership (`crate::party`).
    Party(crate::party::PartyCommand),
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
    /// The game connection is established and in the lobby.
    LobbyReady,
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
    /// A unit entered or left combat.
    CombatChanged { obj_id: u32, engaged: bool },
    /// `hitter` struck `victim` first: the fight starts (`hit_type` as in [`UnitDamage`]).
    CombatFirstHit { victim: u32, hitter: u32, hit_type: u32 },
    /// The hostile units on an NPC's aggro list; empty when the NPC left combat.
    AiAggro { npc: u32, hostiles: Vec<AggroEntry> },
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
    /// The cooldown of `skill_id` (or of all skills with `tag_id`) was reset.
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
    Mail(crate::mail::MailEvent),
    Friend(crate::friend::FriendEvent),
    Repair(crate::repair::RepairEvent),
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
    /// A passive buff was learned.
    PassiveBuffLearned { obj_id: u32, buff_id: u32 },
    DoodadsSpawned(Vec<Doodad>),
    DoodadsRemoved(Vec<u32>),
    DoodadPhaseChanged { obj_id: u32, func_group_id: u32 },
    /// The server moved the local player (answered automatically).
    Teleported { pos: Pos },
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
    pub fn spawn_on(handle: &tokio::runtime::Handle) -> Self {
        Self::spawn_on_with(handle, ClientOptions::default())
    }

    /// [`Client::spawn_on`] with explicit [`ClientOptions`].
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
        Self {
            evt,
            options,
            next_ping: None,
            started: Instant::now(),
            phase: Phase::Idle,
            login: None,
            game: None,
            stream: None,
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
            waiting: None,
        }
    }

    async fn run(mut self, mut cmd_rx: UnboundedReceiver<Command>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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

    fn emit(&self, e: Event) {
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
                self.stream = None;
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

    async fn close_all(&mut self) {
        if let Some(mut c) = self.login.take() {
            c.shutdown().await;
        }
        if let Some(mut c) = self.game.take() {
            c.shutdown().await;
        }
        if let Some(mut c) = self.stream.take() {
            c.shutdown().await;
        }
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
        self.stream = None;
        self.waiting = None;
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
            Command::Move(m) => self.send_move(m).await,
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
            Command::Target { obj_id } => {
                self.send_game(game::ClientMsg::ChangeTarget { obj_id }).await;
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
            Command::Bank(m) => {
                if let Err(message) = m.validate() {
                    self.emit(Event::Shop(crate::shop::ShopEvent::RequestRejected(message.into())));
                } else {
                    self.send_game(game::ClientMsg::Bank(m)).await;
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
                } else if self.send_game(game::ClientMsg::Repair(command)).await {
                    self.emit(Event::Repair(crate::repair::RepairEvent::Requested(command)));
                }
            }
            Command::UseSkill { skill_id, target_obj_id } => {
                let target = SkillTarget::unit(if target_obj_id == 0 { self.self_obj } else { target_obj_id });
                self.start_skill(skill_id, StartCaster::Unit, target).await;
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

    async fn send_move(&mut self, m: MoveCommand) {
        if self.self_obj == 0 || self.phase != Phase::InWorld {
            tracing::debug!("ignoring Move: the player is not in the world");
            return;
        }
        let movement = m.to_movement(self.started.elapsed().as_millis() as u32);
        let obj_id = self.self_obj;
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
        let msg = match r {
            Ok(m) => m,
            Err(NetError::Wire(e)) => {
                tracing::warn!("undecodable game packet: {e}");
                return;
            }
            Err(e) => return self.game_lost(e),
        };
        match msg {
            // A batch holds packets that would each have arrived alone.
            game::ServerMsg::Batch(list) => {
                for m in list {
                    self.game_one(m).await;
                }
            }
            msg => self.game_one(msg).await,
        }
    }

    async fn game_one(&mut self, msg: game::ServerMsg) {
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
            M::PremiumPoint { .. } => {
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
            }
            M::CombatFirstHit { victim, hitter, hit_type } => self.emit(Event::CombatFirstHit { victim, hitter, hit_type }),
            M::AiAggro { npc, hostiles } => self.emit(Event::AiAggro { npc, hostiles }),
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
            M::CombatEngaged { obj_id } => self.emit(Event::CombatChanged { obj_id, engaged: true }),
            M::CombatCleared { obj_id } => self.emit(Event::CombatChanged { obj_id, engaged: false }),
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
            M::InvenExpanded { container, slots } => self.emit(Event::InventoryExpanded { container, slots }),
            M::SkillStarted(s) => self.emit(Event::SkillStarted(s)),
            M::SkillFired(s) => self.emit(Event::SkillFired(s)),
            M::SkillEnded { tl } => self.emit(Event::SkillEnded { tl }),
            M::SkillStopped { obj_id, skill_id } => self.emit(Event::SkillStopped { obj_id, skill_id }),
            M::CastingStopped { tl, duration_ms } => self.emit(Event::CastingStopped { tl, duration_ms }),
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
            M::BuffLearned { obj_id, buff_id } => self.emit(Event::PassiveBuffLearned { obj_id, buff_id }),
            M::Teleport { pos, .. } => {
                self.send_game(game::ClientMsg::TeleportEnded { pos }).await;
                self.emit(Event::Teleported { pos });
            }
            M::DoodadsCreated(v) => self.emit(Event::DoodadsSpawned(v)),
            M::DoodadsRemoved(v) => self.emit(Event::DoodadsRemoved(v)),
            M::DoodadPhaseChanged { obj_id, func_group_id } => {
                self.emit(Event::DoodadPhaseChanged { obj_id, func_group_id });
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
            M::SlotCount(_) | M::AccountInfo { .. } | M::Pong | M::Compressed { .. } => {}
            M::Unhandled { level, opcode, len } => {
                tracing::trace!("unhandled game packet level {level} opcode {opcode:#05x} ({len} bytes)");
            }
        }
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
            Ok(stream::ServerMsg::JoinResponse { code }) => {
                if code != 0 {
                    tracing::warn!("stream server refused the join (code {code})");
                }
            }
            Ok(stream::ServerMsg::Other { opcode, len }) => {
                tracing::trace!("stream packet {opcode:#05x} ({len} bytes)");
            }
            Err(NetError::Wire(e)) => tracing::warn!("undecodable stream packet: {e}"),
            Err(e) => {
                tracing::debug!("stream connection closed: {e}");
                self.stream = None;
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
                tracing::debug!("the server announced the local player ({}) again: ignored", unit.obj_id);
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
mod tests {
    use super::*;
    use crate::model::{alertness, FALL_VELOCITY_PER_MPS};

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
}
