//! The same flow as `walkthrough`, driven through [`NetPlugin`] in a headless Bevy app: commands go
//! out as `NetCommand` messages, results come back as `NetEvent` messages and as resources.
//!
//! ```text
//! cargo run -p aa-net --features bevy --example bevy_walkthrough -- [--login HOST:PORT]
//!     [--user NAME] [--password-file PATH]
//! ```

use std::time::Duration;

use aa_net::model::{gender, race};
use aa_net::state::{
    ActionBar, Characters, Chat, Doodads, Inventory, Player, PlayerStats, Skills, Status, Units, WorldInfo, Worlds,
};
use aa_net::{Command, CreateCharacter, Event, LeaveTarget, NetCommand, NetEvent, NetPlugin};
use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::prelude::*;

#[derive(Resource)]
struct Script {
    login: String,
    user: String,
    password: String,
    character: String,
    world_id: Option<u8>,
    in_world_since: Option<f32>,
    quit_sent: bool,
}

fn main() {
    let mut login = aa_net::default_login();
    let mut user = "bevytest".to_string();
    let mut password_file = std::env::var("AA_PASSWORD_FILE").ok();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--login" => login = it.next().expect("--login needs a value"),
            "--user" => user = it.next().expect("--user needs a value"),
            "--password-file" => password_file = it.next(),
            other => panic!("unknown argument {other}"),
        }
    }
    let password = password_file
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| user.clone());

    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))))
        .add_plugins(NetPlugin)
        .insert_resource(Script {
            login,
            user,
            password,
            character: "Bevytest".into(),
            world_id: None,
            in_world_since: None,
            quit_sent: false,
        })
        .add_systems(Startup, start)
        .add_systems(Update, (drive, report))
        .run();
}

fn start(script: Res<Script>, mut out: MessageWriter<NetCommand>) {
    out.write(NetCommand(Command::Login {
        addr: script.login.clone(),
        user: script.user.clone(),
        password: script.password.as_str().into(),
    }));
}

fn drive(
    mut events: MessageReader<NetEvent>,
    mut out: MessageWriter<NetCommand>,
    mut exit: MessageWriter<AppExit>,
    mut script: ResMut<Script>,
    time: Res<Time>,
    worlds: Res<Worlds>,
    units: Res<Units>,
    doodads: Res<Doodads>,
    hud: Hud,
) {
    for NetEvent(event) in events.read() {
        match event {
            Event::LoginSucceeded(a) => println!("logged in, account {}", a.account_id),
            Event::WorldList(list) => {
                let Some(w) = list.worlds.iter().find(|w| w.online) else {
                    println!("no online world");
                    exit.write(AppExit::error());
                    return;
                };
                println!("world list: {:?}", list.worlds.iter().map(|w| &w.name).collect::<Vec<_>>());
                script.world_id = Some(w.id);
                out.write(NetCommand(Command::EnterWorld { world_id: w.id }));
            }
            Event::ReadyToConnect { cookie, .. } => {
                println!("game server {}", cookie.addr);
                out.write(NetCommand(Command::ConnectToWorld));
            }
            Event::CharacterList(list) => match list.first() {
                None => {
                    println!("no characters, creating {}", script.character);
                    out.write(NetCommand(Command::CreateCharacter(CreateCharacter::new(
                        script.character.clone(),
                        race::NUIAN,
                        gender::MALE,
                    ))));
                }
                Some(c) => {
                    println!("entering as {} (id {})", c.name, c.id);
                    out.write(NetCommand(Command::EnterGame { character_id: c.id }));
                }
            },
            Event::PlayerSpawn(s) => {
                let p = s.position();
                println!("in world: obj {} zone {} at ({:.1}, {:.1}, {:.1})", s.obj_id, s.zone_id(), p.x, p.y, p.z);
                script.in_world_since = Some(time.elapsed_secs());
            }
            Event::LeftWorld { .. } => println!("left the world"),
            Event::Disconnected { message } => {
                println!("disconnected: {message}");
                exit.write(AppExit::Success);
            }
            Event::LoginFailed { message } | Event::ConnectFailed { message } => {
                println!("failed: {message}");
                exit.write(AppExit::error());
            }
            _ => {}
        }
    }
    if let Some(since) = script.in_world_since {
        if !script.quit_sent && time.elapsed_secs() - since > 5.0 {
            script.quit_sent = true;
            println!(
                "after 5 s: {} units and {} doodads in view; {} world(s) known; logging out",
                units.by_id.len(),
                doodads.by_id.len(),
                worlds.list.worlds.len()
            );
            // What the HUD reads: all of it plain resources.
            let (s, w) = (&hud.stats, &hud.world);
            println!(
                "HUD: {} lvl {} hp {}/{} mp {}/{} exp {:?} labor {}; zone group {:?}, hour {:?}; {} chat channels; \
                 {} action slots in use, {} bag items; skills {:?}",
                s.name,
                s.level,
                s.hp,
                s.max_hp,
                s.mp,
                s.max_mp,
                s.exp_progress(),
                s.labor,
                w.zone_group_id,
                w.hour_now(),
                hud.chat.channels.len(),
                hud.actions.slots.iter().filter(|a| !a.is_empty()).count(),
                hud.inventory.bag_items().count(),
                hud.skills.known
            );
            out.write(NetCommand(Command::LeaveWorld(LeaveTarget::Quit)));
        }
    }
    if time.elapsed_secs() > 120.0 {
        println!("giving up");
        exit.write(AppExit::error());
    }
}

/// The HUD resources in one system parameter.
#[derive(bevy::ecs::system::SystemParam)]
struct Hud<'w> {
    stats: Res<'w, PlayerStats>,
    world: Res<'w, WorldInfo>,
    chat: Res<'w, Chat>,
    actions: Res<'w, ActionBar>,
    inventory: Res<'w, Inventory>,
    skills: Res<'w, Skills>,
}

/// Resources are plain Bevy resources: this system only prints when they change.
fn report(status: Res<Status>, characters: Res<Characters>, player: Res<Player>) {
    if status.is_changed() {
        println!("phase: {:?}", status.phase);
    }
    if characters.is_changed() && !characters.list.is_empty() {
        println!("characters: {:?}", characters.list.iter().map(|c| &c.name).collect::<Vec<_>>());
    }
    if player.is_changed() {
        if let Some(p) = &player.spawn {
            println!("player resource: {} level {}", p.name(), p.unit.level);
        }
    }
}
