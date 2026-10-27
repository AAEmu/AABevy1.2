//! `--enter-world`: log in, pick the world and the first character, enter the game.
//!
//! Until the login-stage screens are wired up this drives the whole flow without a UI:
//! `Login -> EnterWorld -> ConnectToWorld -> EnterGame`, then switches [`Screen`] to `InGame`.
//!
//! Flags: `--account NAME` (alias `--user`, default `bevytest`; the server lets one connection per
//! account in), `--login HOST:PORT` (else `$AA_LOGIN`, else AAEmu's default `127.0.0.1:1237`),
//! `--password-file PATH` (else `$AA_PASSWORD_FILE`, else the account name; the password is never printed), `--character NAME` (default: the first). The account flags are
//! shared with the login screens ([`Credentials`]).

use aa_net::{Command, Event, NetCommand, NetEvent, Secret};
use bevy::app::AppExit;
use bevy::prelude::*;

use crate::Screen;

/// Which account logs in where (`--account`, `--login`, `--password-file`); used by the login
/// screens and by `--enter-world`.
#[derive(Resource, Clone)]
pub struct Credentials {
    pub login: String,
    pub user: String,
    pub password: Secret,
}

pub fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

impl Credentials {
    pub fn from_args() -> Credentials {
        let args: Vec<String> = aa_dev::args();
        let user = flag(&args, "--account").or_else(|| flag(&args, "--user")).unwrap_or_else(|| "bevytest".into());
        let path = flag(&args, "--password-file").or_else(|| std::env::var("AA_PASSWORD_FILE").ok());
        let password = path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| user.clone());
        Credentials { login: flag(&args, "--login").unwrap_or_else(aa_net::default_login), user, password: Secret::new(password) }
    }

    /// The `Login` command for these credentials.
    pub fn command(&self) -> Command {
        Command::Login { addr: self.login.clone(), user: self.user.clone(), password: self.password.clone() }
    }
}

/// Present when `--enter-world` was given.
#[derive(Resource)]
pub struct AutoEnter {
    credentials: Credentials,
    character: Option<String>,
}

impl AutoEnter {
    pub fn from_args() -> Option<AutoEnter> {
        let args: Vec<String> = aa_dev::args();
        if !args.iter().any(|a| a == "--enter-world" || a == "--script") {
            return None;
        }
        Some(AutoEnter { credentials: Credentials::from_args(), character: flag(&args, "--character") })
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<crate::login::NativeLoginStarted>()
        .add_systems(Startup, login.run_if(resource_exists::<AutoEnter>))
        .add_systems(Update, drive.run_if(resource_exists::<AutoEnter>));
}

fn login(auto: Res<AutoEnter>, mut started: ResMut<crate::login::NativeLoginStarted>, mut out: MessageWriter<NetCommand>) {
    started.0 = true;
    info!("enter-world: logging in as {} at {}", auto.credentials.user, auto.credentials.login);
    out.write(NetCommand(auto.credentials.command()));
}

fn drive(
    auto: Res<AutoEnter>,
    mut events: MessageReader<NetEvent>,
    mut out: MessageWriter<NetCommand>,
    mut next: ResMut<NextState<Screen>>,
    screen: Res<State<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let in_game = *screen.get() == Screen::InGame;
    for NetEvent(event) in events.read() {
        match event {
            Event::WorldList(list) => match list.worlds.iter().find(|w| w.online) {
                Some(w) => {
                    info!("enter-world: entering world {} ({})", w.id, w.name);
                    out.write(NetCommand(Command::EnterWorld { world_id: w.id }));
                }
                None => {
                    error!("enter-world: no online world");
                    exit.write(AppExit::error());
                }
            },
            Event::ReadyToConnect { .. } => {
                out.write(NetCommand(Command::ConnectToWorld));
            }
            Event::CharacterList(list) => {
                let chosen = match &auto.character {
                    Some(name) => list.iter().find(|c| c.name.eq_ignore_ascii_case(name)),
                    None => list.first(),
                };
                match chosen {
                    Some(c) => {
                        info!("enter-world: entering the game as {} (id {})", c.name, c.id);
                        out.write(NetCommand(Command::EnterGame { character_id: c.id }));
                    }
                    None => {
                        error!("enter-world: no matching character among {} on this world", list.len());
                        exit.write(AppExit::error());
                    }
                }
            }
            Event::PlayerSpawn(s) => {
                let p = s.position();
                info!("enter-world: spawned {} (obj {}) at ({:.1}, {:.1}, {:.1}) facing {:.2}", s.name(), s.obj_id, p.x, p.y, p.z, s.yaw());
                next.set(Screen::InGame);
            }
            Event::LoginFailed { message } | Event::ConnectFailed { message } => {
                error!("enter-world: {message}");
                exit.write(AppExit::error());
            }
            // in game the HUD owns a lost connection: it shows the disconnect dialog (and ends the run itself without a HUD,
            // `hud::session::route_session`); only a connection lost on the way in ends the run here
            Event::Disconnected { message } if in_game => info!("enter-world: disconnected in game ({message})"),
            Event::Disconnected { message } => {
                error!("enter-world: {message}");
                exit.write(AppExit::error());
            }
            Event::EnterWorldDenied { reason } => {
                error!("enter-world: world denied ({reason})");
                exit.write(AppExit::error());
            }
            Event::Kicked { reason, message } if in_game => info!("enter-world: kicked in game ({reason}): {message}"),
            Event::Kicked { reason, message } => {
                error!("enter-world: kicked ({reason}): {message}");
                exit.write(AppExit::error());
            }
            _ => {}
        }
    }
}
