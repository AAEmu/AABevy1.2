//! The login stage: the original Lua UI screens backed by the server and the 3D characters.
//!
//! * [`login_screen`]: the custom login screen in front of world select (account, password, server; see
//!   its docs and `docs/aa-client-login.md`); dev flags skip it
//! * [`world_select`]: server list (`X2World` from aa-net), connect
//! * [`character_select`]: character list (`X2LoginCharacter`), 3D character, start / delete
//! * [`create`]: character creation (race, appearance with live preview, skillset and name)
//! * [`loading`]: the loading screen while the world streams in, then `Screen::InGame`
//! * [`gameapi`], [`host`], [`stage3d`]: the game API the scripts call, the per-stage UI runtime and
//!   the shared 3D stage (camera, lighting, character slot)
//!
//! ## Flow
//!
//! `cargo run -p aa-client` shows [`login_screen`], which logs in and then shows `Screen::WorldSelect`. With
//! a dev flag (`--account`, `--goto`, `--shot`, ...: see [`login_screen::enabled`]) it logs in at startup instead
//! (`--account`, default `bevytest`) and shows `Screen::WorldSelect` as soon as the world list is there. Connect goes to the lobby; the character list decides the
//! next screen: no characters = `Screen::CharacterCreate` (like the real client), else
//! `Screen::CharacterSelect`. Start sends `EnterGame` and shows `Screen::Loading`; the loading screen
//! waits for the spawn and the terrain around it, then `Screen::InGame` builds the player the same
//! way `--enter-world` does. `--enter-world` (see `ingame::enter`) skips all of this.
//!
//! ## Hand-off to the create screen (`create`)
//!
//! * Entering `Screen::CharacterCreate`: the player clicked an empty slot (`X2LoginCharacter:CreateCharacter`)
//!   or the account has no characters. [`ensure_ui`] leaves the UI alone on that screen: the create
//!   plugin builds its own stage with [`host::switch_stage`] (fresh Lua state, game API, scripts) and
//!   its 3D with [`stage3d`].
//! * Leaving it: set `Screen::CharacterSelect` (cancel, or after `Event::CharacterCreated`; the
//!   character list is refreshed by the session) and [`ensure_ui`] rebuilds character select.
//! * Race congestion is in `aa_net::state::Characters::race_congestion`, the creation itself is
//!   `Command::CreateCharacter`.
//!
//! ## Flags
//!
//! `--account NAME`, `--login HOST:PORT`, `--password-file PATH` (see `ingame::enter::Credentials`);
//! `--goto world-select|character-select|loading` drives the screens through the original UI
//! scripts up to that screen (for `--shot`); `--hold-loading PCT` keeps the loading screen at that
//! progress instead of entering the game; `--lua SCREEN:CODE` (repeatable) runs Lua in the UI of a
//! screen (`world-select`, `character-select`, `loading`, `ingame`) once it has been up for a moment;
//! `--stage-clear R,G,B` shows what the stage camera draws on its own, `AA_BG_GRADE=tonemap,ev,sat`
//! overrides the background grading of [`stage3d::Grade`].

pub mod character_select;
#[cfg(not(feature = "no-create"))]
pub mod create;
#[cfg(feature = "no-create")]
#[path = "create_stub.rs"]
pub mod create;
pub mod gameapi;
pub mod host;
pub mod curtain;
pub mod loading;
pub mod login_screen;
pub(crate) mod name_policy;
pub mod stage3d;
pub mod world_select;

use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::camera::Camera;

use crate::Screen;
use crate::ingame::enter::{Credentials, flag};
use gameapi::{Request, Shared};

/// Where `--goto` takes the screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goto {
    WorldSelect,
    CharacterSelect,
    Loading,
}

#[derive(Resource, Clone, Debug)]
pub struct LoginArgs {
    pub enter_world: bool,
    pub goto: Option<Goto>,
    /// Keep the loading screen at this progress (percent) instead of entering the game.
    pub hold_loading: Option<f32>,
    /// `--lua SCREEN:CODE`: Lua run in the UI of that screen once it has been up for a moment.
    pub lua: Vec<(Screen, String)>,
}

impl LoginArgs {
    pub fn from_env() -> LoginArgs {
        let args: Vec<String> = std::env::args().collect();
        let goto = flag(&args, "--goto").and_then(|g| match g.as_str() {
            "world-select" => Some(Goto::WorldSelect),
            "character-select" => Some(Goto::CharacterSelect),
            "loading" => Some(Goto::Loading),
            // the create screen drives itself there (`create::auto_drive`, see its docs)
            "create" => None,
            other => {
                warn!("--goto: unknown screen {other:?} (world-select, character-select, create, loading)");
                None
            }
        });
        LoginArgs {
            enter_world: args.iter().any(|a| a == "--enter-world" || a == "--script"),
            goto,
            hold_loading: flag(&args, "--hold-loading").and_then(|v| v.parse().ok()),
            lua: args
                .windows(2)
                .filter(|w| w[0] == "--lua")
                .filter_map(|w| {
                    let (screen, code) = w[1].split_once(':')?;
                    let screen = match screen {
                        "world-select" => Screen::WorldSelect,
                        "character-select" => Screen::CharacterSelect,
                        "create" => Screen::CharacterCreate,
                        "loading" => Screen::Loading,
                        "ingame" => Screen::InGame,
                        other => {
                            warn!("--lua: unknown screen {other:?}");
                            return None;
                        }
                    };
                    Some((screen, code.to_string()))
                })
                .collect(),
        }
    }
}

/// The game data and request queue shared with the scripts of every stage (non-send: Lua).
pub struct LoginBridge(pub Shared);

/// What the UI runtime currently is the UI of.
#[derive(Resource, Default)]
pub struct UiBuilt(pub Option<Screen>);

/// A request a script queued (`X2World:EnterWorld`, `X2LoginCharacter:SelectCharacter`, ...) that the
/// screens act on: the generic ones are carried out by this module, the rest forwarded here.
#[derive(Message, Clone, Debug)]
pub struct UiRequest(pub Request);

pub struct LoginPlugin;

/// Native gEnv+0x96 is set before ConnectToServer attempts the login. It is
/// independent of login success and is also consumed by the jury Task6 predicate.
#[derive(Resource, Default)]
pub(crate) struct NativeLoginStarted(pub bool);

impl Plugin for LoginPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeLoginStarted>();
        let args = LoginArgs::from_env();
        if args.enter_world {
            // dev shortcut: `ingame::enter` logs in and enters the game without the screens
            return;
        }
        if !app.is_plugin_added::<aa_net::NetPlugin>() {
            app.add_plugins(aa_net::NetPlugin);
        }
        if !app.is_plugin_added::<aa_cry::CryPlugin>() {
            app.add_plugins(aa_cry::CryPlugin);
        }
        if !app.is_plugin_added::<aa_anim::AnimPlugin>() {
            app.add_plugins(aa_anim::AnimPlugin);
        }
        if !app.is_plugin_added::<aa_anim::facial::FacialPlugin>() {
            app.add_plugins(aa_anim::facial::FacialPlugin);
        }
        app.add_plugins(aa_ui::UiPlugin { config: Some(host::ui_config()) })
            .insert_resource(args)
            .insert_resource(Credentials::from_args())
            .init_resource::<UiBuilt>()
            .insert_non_send(LoginBridge(gameapi::new_shared()))
            .add_message::<UiRequest>()
            .add_systems(Startup, (start_login.run_if(|| login_screen::skipped()), spawn_world_anchor))
            .add_systems(Update, (sync_bridge, route_events, handle_requests, ensure_ui, run_lua).chain())
            .add_systems(OnEnter(Screen::InGame), release_world_anchor)
            .add_systems(OnEnter(Screen::CharacterCreate), |mut grade: ResMut<stage3d::BackgroundGrade>| grade.0 = None)
            .add_plugins((stage3d::Stage3dPlugin, login_screen::plugin, world_select::plugin, character_select::plugin, create::plugin, loading::plugin));
    }
}

/// `--lua SCREEN:CODE` (repeatable): run Lua in the UI of a screen, one step a second, after the
/// screen has been up for two seconds (drives the original scripts for unattended checks).
fn run_lua(
    rt: Option<NonSend<UiRuntime>>,
    built: Res<UiBuilt>,
    args: Res<LoginArgs>,
    time: Res<Time>,
    screen_state: Res<State<Screen>>,
    mut state: Local<(usize, f32, Option<Screen>)>,
) {
    let (next, wait, seen) = &mut *state;
    let (Some(rt), Some((screen, code))) = (rt, args.lua.get(*next)) else { return };
    let in_game = *screen == Screen::InGame && *screen_state.get() == Screen::InGame
        && rt.lua.globals().raw_get::<Option<mlua::Table>>("HUD").ok().flatten().is_some();
    if built.0 != Some(*screen) && !in_game {
        *wait = 0.0;
        *seen = None;
        return;
    }
    if seen.is_none() {
        *seen = Some(*screen);
        *wait = 1.0;
    }
    *wait += time.delta_secs();
    if *wait < 3.0 {
        return;
    }
    *next += 1;
    *wait = 2.0;
    info!("login: --lua {code}");
    if let Err(e) = rt.exec("lua", code) {
        warn!("login: --lua: {e}");
    }
}

fn start_login(credentials: Res<Credentials>, mut started: ResMut<NativeLoginStarted>, mut out: MessageWriter<NetCommand>) {
    started.0 = true;
    info!("login: logging in as {} at {}", credentials.user, credentials.login);
    out.write(NetCommand(credentials.command()));
}

/// The world (sky dome, sun, terrain streaming) follows "the first `Camera3d`" and gives it its
/// own post processing. During the login screens the only 3D camera is the stage camera, which
/// must stay untouched, so the world gets an inactive camera of its own, far outside the map:
/// nothing streams. The loading screen moves it to the spawn, the in-game camera replaces it.
#[derive(Component)]
pub struct WorldAnchor;

fn spawn_world_anchor(mut commands: Commands) {
    commands.spawn((
        Name::new("world anchor"),
        WorldAnchor,
        aa_world::WorldFocus,
        Camera3d::default(),
        Camera { is_active: false, ..default() },
        Transform::from_xyz(-1.0e6, 0.0, 1.0e6),
    ));
}

fn release_world_anchor(mut commands: Commands, anchor: Query<Entity, With<WorldAnchor>>) {
    for e in &anchor {
        commands.entity(e).despawn();
    }
}

/// Copy aa-net's resources into the bridge the scripts read.
fn sync_bridge(
    bridge: NonSend<LoginBridge>,
    worlds: Res<aa_net::state::Worlds>,
    characters: Res<aa_net::state::Characters>,
) {
    if worlds.is_changed() || characters.is_changed() {
        bridge.0.borrow_mut().sync(&worlds, &characters);
    }
}

/// Screen changes that follow from what the server says.
fn route_events(
    mut events: MessageReader<NetEvent>,
    screen: Res<State<Screen>>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
    args: Res<LoginArgs>,
    bridge: NonSend<LoginBridge>,
    mut notice: ResMut<login_screen::Notice>,
) {
    for NetEvent(event) in events.read() {
        match (event, *screen.get()) {
            // the login screen handles the session's answers itself
            (_, Screen::Login) => {}
            // a lost connection or a refused re-login goes back to the login screen
            (Event::LoginFailed { message } | Event::Disconnected { message } | Event::Kicked { message, .. }, screen)
                if login_screen::enabled() && screen != Screen::InGame =>
            {
                login_screen::back_to_login(message, &mut notice, &mut next);
            }
            (Event::CharacterList(list), Screen::WorldSelect) => {
                let force_select = args.goto.is_some_and(|g| g != Goto::WorldSelect);
                info!("login: {} character(s) on {}", list.len(), bridge.0.borrow().world_name);
                next.set(if list.is_empty() && !force_select { Screen::CharacterCreate } else { Screen::CharacterSelect });
            }
            // the new character is the one the list selects when it comes back (see `Bridge::prefer`)
            (Event::CharacterCreated(c), _) => bridge.0.borrow_mut().set_prefer(c.id),
            // back from character select (or create) to the server list
            (Event::WorldList(_), Screen::CharacterSelect | Screen::CharacterCreate) => next.set(Screen::WorldSelect),
            (Event::LoginFailed { message }, _) => {
                error!("login: {message}");
                exit.write(AppExit::error());
            }
            // in game the HUD shows the disconnect dialog (`hud::session`)
            (Event::Disconnected { message } | Event::Kicked { message, .. }, screen) if screen != Screen::WorldSelect && screen != Screen::InGame => {
                error!("login: {message}");
                exit.write(AppExit::error());
            }
            (Event::ConnectFailed { message }, _) => error!("login: {message}"),
            (Event::ServerError { code }, _) => warn!("login: server error {code}"),
            _ => {}
        }
    }
}

/// Carry out what the scripts asked for.
#[allow(clippy::too_many_arguments)]
fn handle_requests(
    bridge: NonSend<LoginBridge>,
    rt: Option<NonSend<UiRuntime>>,
    mut net: MessageWriter<NetCommand>,
    mut next: ResMut<NextState<Screen>>,
    screen: Res<State<Screen>>,
    mut exit: MessageWriter<AppExit>,
    mut forward: MessageWriter<UiRequest>,
) {
    let requests = bridge.0.borrow_mut().take_requests();
    for request in requests {
        debug!("login: request {request:?}");
        let character = |index: usize| bridge.0.borrow().characters.get(index.wrapping_sub(1)).cloned();
        match request {
            Request::Quit => {
                net.write(NetCommand(Command::Disconnect));
                // World Select's Exit Game goes back to the login screen (which has its own Exit)
                if login_screen::enabled() && *screen.get() == Screen::WorldSelect {
                    next.set(Screen::Login);
                } else {
                    exit.write(AppExit::Success);
                }
            }
            Request::EnterWorld { world_id } => {
                bridge.0.borrow_mut().set_world(world_id);
                net.write(NetCommand(Command::EnterWorld { world_id }));
            }
            Request::RefreshWorlds => {
                net.write(NetCommand(Command::RefreshWorldList));
            }
            Request::CancelQueue => {
                net.write(NetCommand(Command::CancelEnterWorld));
            }
            Request::Connect => {
                net.write(NetCommand(Command::ConnectToWorld));
            }
            Request::ToWorldSelect => {
                net.write(NetCommand(Command::ReturnToWorldSelect));
            }
            Request::RefreshCharacters => {
                net.write(NetCommand(Command::RefreshCharacters));
            }
            Request::StartGame(index) => match character(index) {
                Some(c) => {
                    info!("login: starting the game as {} (id {})", c.name, c.id);
                    bridge.0.borrow_mut().prefer = None;
                    net.write(NetCommand(Command::EnterGame { character_id: c.id }));
                    next.set(Screen::Loading);
                }
                None => warn!("login: no character {index} to start"),
            },
            Request::CreateCharacter => next.set(Screen::CharacterCreate),
            Request::DeleteCharacter { index, name } => {
                let Some(c) = character(index) else { continue };
                if c.name.eq_ignore_ascii_case(name.trim()) {
                    net.write(NetCommand(Command::DeleteCharacter { id: c.id }));
                } else if let Some(rt) = &rt {
                    rt.fire_event("LOGIN_CHARACTER_UPDATED", ("error_char_name_mismatch", index as i64));
                }
            }
            Request::CancelDelete(index) => {
                if let Some(c) = character(index) {
                    net.write(NetCommand(Command::CancelDeleteCharacter { id: c.id }));
                    // aa-net has no event for the answer, the refreshed list is it: the screen tells
                    // the scripts "delete_canceled" when it arrives
                    forward.write(UiRequest(Request::CancelDelete(index)));
                }
            }
            other => {
                forward.write(UiRequest(other));
            }
        }
    }
}

/// Builds the UI of the current screen once its data is there.
fn ensure_ui(world: &mut World) {
    let screen = *world.resource::<State<Screen>>().get();
    if world.resource::<UiBuilt>().0 == Some(screen) {
        return;
    }
    let built = match screen {
        Screen::WorldSelect => world_select::build(world),
        Screen::CharacterSelect => character_select::build(world),
        Screen::Loading => loading::build(world),
        // the login screen draws itself (`login_screen`)
        Screen::Login => true,
        // the create plugin builds its own UI (see the module docs)
        Screen::CharacterCreate => true,
        // the HUD builds its own stage once the player has spawned (`hud::build_hud`)
        Screen::InGame => true,
    };
    if built {
        world.resource_mut::<UiBuilt>().0 = Some(screen);
        // the painted background is graded like the client's post processing, per screen
        // (`stage3d::Grade`); the create screen sets its own when it builds
        if screen != Screen::CharacterCreate {
            world.resource_mut::<stage3d::BackgroundGrade>().0 = match screen {
                Screen::WorldSelect => Some(stage3d::Grade::WORLD_SELECT),
                Screen::CharacterSelect => Some(stage3d::Grade::CHARACTER_SELECT),
                _ => None,
            };
        }
    }
}

/// True while the UI runtime is the UI of `screen`.
pub fn ui_is(screen: Screen) -> impl Fn(Res<UiBuilt>, Res<State<Screen>>) -> bool + Clone {
    move |built: Res<UiBuilt>, state: Res<State<Screen>>| built.0 == Some(screen) && *state.get() == screen
}
