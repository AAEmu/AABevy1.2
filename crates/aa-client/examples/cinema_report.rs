//! Quest 1065's report cinema through the actual dialog hook and cinema systems: the directing
//! window opens (0x0d0), the hand-in confirms (0x0cf), `fly_east` drives the world camera from
//! its Scene camera track for 21 s, then 0x0ce and `CSCompleteQuestContext` go out. Fails when the
//! camera does not follow the track, when the hook sends the quest request directly, or when the
//! component has no cinema. `cargo run -p aa-client --example cinema_report -- --offscreen-shot
//! shots/aa-client/cinema_report.png --shot-after 40`; needs AA_PAK and AA_DB.

#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;

use aa_net::interaction::Interact;
use aa_net::state::Units;
use aa_net::{Command, MoveCommand, NetCommand, NetEvent, Pos};
use bevy::app::AppExit;
use bevy::prelude::*;
use hud::GameData;
use hud::SharedGameData;
use hud::api::{HudBridge, HudData, Shared};
use hud::cinema::{CinemaPlayer, block_cinema_input, cinema_ui, drive_cinema_camera, tick_cinema};
use hud::dialog::{Req, handle_dialog};
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

const QUEST: u32 = 1065;
const COMPONENT: u32 = 5336;
const NPC: u32 = 3443;
const CINEMA: &str = "fly_east";

#[derive(Resource, Default)]
struct Seen {
    packets: Vec<Interact>,
    moves: usize,
    casts: usize,
}

#[derive(Resource, Default)]
struct Check {
    t1: Option<Vec3>,
    t2: Option<Vec3>,
    input_sent: bool,
    input_at: Option<f32>,
    input_checked: bool,
    done: bool,
}

#[derive(Resource, Default)]
struct Visual(bool);

fn main() -> AppExit {
    if !aa_world::pak_available() {
        eprintln!("cinema_report: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let Ok(game) = GameData::load(&db) else {
        eprintln!("cinema_report: skipped, the game database cannot be read (AA_DB)");
        return AppExit::Success;
    };
    let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
    {
        let mut data = shared.borrow_mut();
        data.game = SharedGameData::default();
        let _ = data.game.0.set(game);
        data.dialog.directing = true;
        data.dialog.npc = Some((NPC, NPC));
        data.dialog.requests.push(Req::CinemaPermission {
            quest: QUEST,
            kind: 6,
        });
        data.dialog.requests.push(Req::Complete(QUEST, 0));
    }
    let xml = aa_pak::Pak::global()
        .read_string(&format!("worlds/main_world/cinemas/{CINEMA}.xml"))
        .expect("fly_east sequence");
    let first = aa_world::cinema::CinemaCamera::parse(&xml)
        .expect("fly_east first camera")
        .pose(0.0);
    let mut world = aa_world::AaWorldPlugin::new("main_world");
    world.config.load_radius = 160.0;
    world.config.unload_radius = 200.0;

    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("aa-client: quest report cinema"),
        aa_dev::DevPlugin,
    ))
    .add_plugins((aa_fx::FxPlugin, world))
    .insert_resource(aa_world::WorldClock {
        hour: 10.0,
        ..default()
    })
    .insert_resource(Seen::default())
    .insert_resource(Visual(
        aa_dev::args()
            .iter()
            .any(|a| a == "--shot" || a == "--offscreen-shot"),
    ))
    .init_resource::<Check>()
    .init_resource::<CinemaPlayer>()
    .init_resource::<Units>()
    .insert_non_send(HudBridge(shared.clone()))
    .add_message::<NetEvent>()
    .add_message::<NetCommand>()
    .add_systems(
        PreUpdate,
        block_cinema_input.after(bevy::input::InputSystems),
    )
    .add_systems(
        Startup,
        move |mut commands: Commands, mut time: ResMut<Time<Virtual>>| {
            time.set_max_delta(std::time::Duration::from_secs_f32(1.0));
            time.set_relative_speed(12.0);
            commands.spawn((
                Camera3d::default(),
                aa_world::WorldFocus,
                Projection::Perspective(PerspectiveProjection {
                    fov: first.fov_deg.to_radians(),
                    near: 0.3,
                    far: 20000.0,
                    ..default()
                }),
                Transform::from_translation(aa_world::cry_to_bevy(first.position))
                    .looking_to(aa_world::cry_to_bevy(first.forward()), Vec3::Y),
            ));
        },
    )
    .add_systems(
        Update,
        (
            handle_dialog,
            tick_cinema,
            cinema_ui,
            drive_cinema_camera,
            movement_probe.run_if(hud::cinema::idle),
            collect,
            check,
        )
            .chain(),
    );
    app.run();
    AppExit::Success
}

fn movement_probe(keys: Res<ButtonInput<KeyCode>>, mut net: MessageWriter<NetCommand>) {
    if keys.just_pressed(KeyCode::KeyW) {
        net.write(NetCommand(Command::Move(MoveCommand::stopped(
            Pos::new(0.0, 0.0, 0.0),
            0.0,
        ))));
    }
    if keys.just_pressed(KeyCode::Digit1) {
        net.write(NetCommand(Command::UseSkill {
            skill_id: 1,
            target_obj_id: 0,
        }));
    }
}

fn collect(mut events: MessageReader<NetCommand>, mut seen: ResMut<Seen>) {
    for NetCommand(command) in events.read() {
        match command {
            Command::Interact(packet) => seen.packets.push(packet.clone()),
            Command::Move(_) | Command::MoveUnit { .. } => seen.moves += 1,
            Command::UseSkill { .. }
            | Command::UseDoodadSkill { .. }
            | Command::UseSkillAt { .. }
            | Command::UseMateSkill { .. } => seen.casts += 1,
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    time: Res<Time>,
    real: Res<Time<Real>>,
    player: Res<CinemaPlayer>,
    seen: Res<Seen>,
    bridge: NonSend<HudBridge>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut check: ResMut<Check>,
    visual: Res<Visual>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    mut xml: Local<Option<String>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !check.done && real.elapsed_secs() > 3.0 {
        panic!("the report cinema did not finish (saw {:?})", seen.packets);
    }
    let Ok((transform, projection)) = cameras.single() else {
        return;
    };
    if let Some(t) = player.elapsed() {
        if t >= 0.5 && !check.input_sent {
            keys.press(KeyCode::Escape);
            keys.press(KeyCode::KeyW);
            keys.press(KeyCode::Digit1);
            check.input_sent = true;
            check.input_at = Some(t);
        }
        if check.input_at.is_some_and(|at| t >= at + 0.2) && !check.input_checked {
            assert!(
                !keys.pressed(KeyCode::Escape),
                "C1: Esc was not consumed during the movie"
            );
            assert!(
                !keys.pressed(KeyCode::KeyW) && !keys.pressed(KeyCode::Digit1),
                "C4: movement/cast input was not blocked"
            );
            assert!(player.active(), "C1: Esc cancelled the movie");
            assert!(
                bridge.0.borrow().dialog.cinema_hidden,
                "C5: the talk window was not hidden"
            );
            assert_eq!(
                (seen.moves, seen.casts),
                (0, 0),
                "C4: movement or a cast left the client during the movie"
            );
            assert!(
                !seen.packets.contains(&Interact::Cinema { completed: true }),
                "C1: Esc ended the movie early"
            );
            check.input_checked = true;
        }
        let checkpoint = (t >= 0.5 && check.t1.is_none())
            .then_some(0)
            .or_else(|| (t >= 2.0 && check.t2.is_none()).then_some(1));
        if let Some(which) = checkpoint {
            let xml = xml.get_or_insert_with(|| {
                aa_pak::Pak::global()
                    .read_string(&format!("worlds/main_world/cinemas/{CINEMA}.xml"))
                    .unwrap()
            });
            let expected = aa_world::cinema::CinemaCamera::parse(xml).unwrap().pose(t);
            let want = aa_world::cry_to_bevy(expected.position);
            assert!(
                transform.translation.distance(want) < 0.05,
                "camera at {:.2?}, track says {:.2?} at t={t:.2}",
                transform.translation,
                want
            );
            if let Projection::Perspective(perspective) = projection {
                assert!(
                    (perspective.fov - expected.fov_deg.to_radians()).abs() < 0.01,
                    "fov {} != {}",
                    perspective.fov,
                    expected.fov_deg.to_radians()
                );
            }
            if which == 0 {
                check.t1 = Some(transform.translation);
            } else {
                check.t2 = Some(transform.translation);
                let moved = check.t1.unwrap().distance(check.t2.unwrap());
                assert!(
                    moved > 1.0,
                    "the camera did not follow the track ({moved:.2} m between t=0.5 and t=2)"
                );
            }
        }
    }
    if check.done || time.elapsed_secs() < 1.0 {
        return;
    }
    let ended = seen
        .packets
        .iter()
        .position(|p| *p == Interact::Cinema { completed: true });
    let Some(ended) = ended else { return };
    let expected = [
        Interact::CinemaPermission {
            quest_id: QUEST,
            npc_obj_id: NPC,
            doodad_obj_id: 0,
        },
        Interact::Cinema { completed: false },
    ];
    assert_eq!(
        &seen.packets[..2],
        &expected,
        "0x0d0 then 0x0cf expected, saw {:?}",
        seen.packets
    );
    assert_eq!(
        seen.packets.get(ended + 1),
        Some(&Interact::QuestComplete {
            quest_id: QUEST,
            npc_obj_id: NPC,
            doodad_obj_id: 0,
            selected: 0
        }),
        "CSCompleteQuestContext must follow 0x0ce: {:?}",
        seen.packets
    );
    assert!(
        !seen.packets[..ended]
            .iter()
            .any(|p| matches!(p, Interact::QuestComplete { .. })),
        "quest request went out before the movie ended: {:?}",
        seen.packets
    );
    assert!(
        !bridge.0.borrow().dialog.cinema_hidden,
        "C5: the talk window stayed hidden after the movie"
    );
    assert!(check.input_checked, "C1/C4/C5 checks did not run");
    check.done = true;
    println!(
        "CINEMA REPORT: PASS {COMPONENT} {CINEMA}; camera followed the track; 0x0d0 -> 0x0cf -> 0x0ce -> CSCompleteQuestContext"
    );
    if !visual.0 {
        exit.write(AppExit::Success);
    }
}
