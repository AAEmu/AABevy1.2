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

use crate::game::{self, LeaveTarget, VisualOptions};
use crate::login::{self, AccountInfo, WorldCookie, WorldList, WorldQueue};
use crate::model::{
    ChatMessage, Character, CharacterState, CreateCharacter, Doodad, Movement, Unit, UnitKind,
    stance,
};
use crate::net::{self, GameConn, LoginConn, NetError, StreamConn};
use crate::stream;
use crate::wire::Pos;

/// How long a request may stay unanswered before the session reports a failure.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Interval of the keep-alive ping; the server drops game connections silent for 30 s.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// A movement the app wants to send for the local player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveCommand {
    pub pos: Pos,
    /// Facing in radians (0 faces +y, positive turns left).
    pub yaw: f32,
    pub moving: bool,
    /// [`stance`] byte.
    pub stance: u8,
    /// Velocity in 1/1024 m/s per axis.
    pub velocity: [i16; 3],
}

impl MoveCommand {
    pub fn new(pos: Pos, yaw: f32) -> Self {
        Self {
            pos,
            yaw,
            moving: true,
            stance: stance::RELAXED,
            velocity: [0; 3],
        }
    }
}

/// What the app can ask the session to do.
#[derive(Clone, Debug)]
pub enum Command {
    /// Connect to the login server at `addr` (`host:port`) and authenticate. On success the world
    /// list follows.
    Login { addr: String, user: String, password: String },
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
    CharacterDeleted { id: u32, name: String },
    /// The server accepted `EnterGame`; the character is loading.
    CharacterSelected(Box<CharacterState>),
    /// The local player exists in the world.
    PlayerSpawn(Box<PlayerSpawn>),
    /// The enter-world sequence is complete; units and doodads around start to arrive.
    EnteredWorld,
    UnitSpawned(Box<Unit>),
    UnitMoved { obj_id: u32, movement: Movement },
    UnitsRemoved(Vec<u32>),
    UnitPoints { obj_id: u32, hp: i32, mp: i32 },
    UnitDeath { obj_id: u32 },
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
        let (commands, cmd_rx) = unbounded_channel();
        let (evt_tx, events) = unbounded_channel();
        handle.spawn(Session::new(evt_tx).run(cmd_rx));
        Self { commands, events }
    }

    /// Starts the session task on its own thread with a private single-threaded runtime. This is
    /// what apps without tokio (Bevy) use.
    pub fn spawn_thread() -> std::io::Result<Self> {
        let (commands, cmd_rx) = unbounded_channel();
        let (evt_tx, events) = unbounded_channel();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("aa-net".into())
            .spawn(move || rt.block_on(Session::new(evt_tx).run(cmd_rx)))?;
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
    fn new(evt: UnboundedSender<Event>) -> Self {
        Self {
            evt,
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
        let mut last_ping = Instant::now();
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
                    if self.game.is_some() && last_ping.elapsed() >= PING_INTERVAL {
                        last_ping = Instant::now();
                        let time_ms = self.started.elapsed().as_millis() as i64;
                        self.send_game(game::ClientMsg::Ping { time_ms }).await;
                    }
                }
            }
        }
        self.close_all().await;
    }

    // ---- helpers ------------------------------------------------------------------------------

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

    async fn start_login(&mut self, addr: String, user: String, password: String) {
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
        self.send_login(login::ClientMsg::auth(&user, &password)).await;
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
            Ok(c) => self.game = Some(c),
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
        let mut movement = Movement::unit(m.pos, m.yaw, self.started.elapsed().as_millis() as u32);
        movement.stance = m.stance;
        movement.velocity = m.velocity;
        if !m.moving {
            movement.flags = 0;
        }
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
            M::CancelCharacterDeleteResponse { .. } => {
                self.command(Command::RefreshCharacters).await;
            }
            M::CharacterDeleted { id, name } => self.emit(Event::CharacterDeleted { id, name }),
            M::CharacterState(state) => {
                self.waiting = None;
                self.selected_state = Some((*state).clone());
                self.emit(Event::CharacterSelected(state));
                self.send_game(game::ClientMsg::SpawnCharacter(VisualOptions::default())).await;
            }
            M::UnitState(unit) => self.unit_state(*unit).await,
            M::UnitsRemoved(ids) => self.emit(Event::UnitsRemoved(ids)),
            M::UnitMoved { obj_id, movement } => self.emit(Event::UnitMoved { obj_id, movement }),
            M::UnitPoints { obj_id, hp, mp } => self.emit(Event::UnitPoints { obj_id, hp, mp }),
            M::UnitDeath { obj_id } => self.emit(Event::UnitDeath { obj_id }),
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
            self.emit(Event::UnitSpawned(Box::new(unit)));
            return;
        }
        let Some(state) = self.selected_state.clone() else { return };
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
