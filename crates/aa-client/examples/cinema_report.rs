//! Quest 1065's report cinema through the actual dialog hook and cinema systems: the directing
//! window opens (0x0d0), the hand-in confirms (0x0cf), `fly_east` drives the world camera from
//! its Scene camera track for 21 s, then 0x0ce and `CSCompleteQuestContext` go out. Fails when the
//! camera does not follow the track, when the hook sends the quest request directly, or when the
//! component has no cinema. `cargo run -p aa-client --example cinema_report -- --offscreen-shot
//! shots/aa-client/cinema_report.png --shot-after 30`; needs AA_PAK and AA_DB.

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
use aa_net::{Command, NetCommand, NetEvent};
use bevy::app::AppExit;
use bevy::prelude::*;
use hud::GameData;
use hud::SharedGameData;
use hud::api::{HudBridge, HudData, Shared};
use hud::cinema::{CinemaPlayer, drive_cinema_camera, tick_cinema};
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
struct Seen(Vec<Interact>);

#[derive(Resource, Default)]
struct Check {
    t1: Option<Vec3>,
    t2: Option<Vec3>,
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
            drive_cinema_camera,
            collect,
            check,
        )
            .chain(),
    );
    app.run();
    AppExit::Success
}

fn collect(mut events: MessageReader<NetCommand>, mut seen: ResMut<Seen>) {
    for NetCommand(command) in events.read() {
        if let Command::Interact(packet) = command {
            seen.0.push(packet.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check(
    time: Res<Time>,
    real: Res<Time<Real>>,
    player: Res<CinemaPlayer>,
    seen: Res<Seen>,
    mut check: ResMut<Check>,
    visual: Res<Visual>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    mut xml: Local<Option<String>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !check.done && real.elapsed_secs() > 2.2 {
        panic!("the report cinema did not finish (saw {:?})", seen.0);
    }
    let Ok((transform, projection)) = cameras.single() else {
        return;
    };
    if let Some(t) = player.elapsed() {
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
        .0
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
        &seen.0[..2],
        &expected,
        "0x0d0 then 0x0cf expected, saw {:?}",
        seen.0
    );
    assert_eq!(
        seen.0.get(ended + 1),
        Some(&Interact::QuestComplete {
            quest_id: QUEST,
            npc_obj_id: NPC,
            doodad_obj_id: 0,
            selected: 0
        }),
        "CSCompleteQuestContext must follow 0x0ce: {:?}",
        seen.0
    );
    assert!(
        !seen.0[..ended]
            .iter()
            .any(|p| matches!(p, Interact::QuestComplete { .. })),
        "quest request went out before the movie ended: {:?}",
        seen.0
    );
    check.done = true;
    println!(
        "CINEMA REPORT: PASS {COMPONENT} {CINEMA}; camera followed the track; 0x0d0 -> 0x0cf -> 0x0ce -> CSCompleteQuestContext"
    );
    if !visual.0 {
        exit.write(AppExit::Success);
    }
}
