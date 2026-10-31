//! Quest cinema rules through the real dialog, cinema and input routes: report 1065 (C1/C4/C5),
//! before-bubble 1352 (C2) and progress 1061/5320 (C6). The report case opens the dialog in one
//! frame and confirms later, so 0x0d0-at-open is told apart from 0x0d0-at-confirm. Fails when the
//! camera leaves its track, an input event reaches a consumer, the talk window is not hidden, or
//! the 0x0d0/0x0cf/0x0ce order changes. `--offscreen-shot shots/aa-client/cinema_report.png
//! --shot-after 80`; needs AA_PAK and AA_DB.

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
use aa_net::quest::{Quest, status};
use aa_net::state::Units;
use aa_net::{Command, MoveCommand, NetCommand, NetEvent, Pos};
use bevy::app::AppExit;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::MouseButtonInput;
use bevy::prelude::*;
use hud::api::{HudBridge, HudData, Shared};
use hud::cinema::{
    CinemaInputGate, CinemaInputRead, CinemaPlayer, block_cinema_input, cinema_ui,
    drive_cinema_camera, tick_cinema, track,
};
use hud::dialog::{Req, handle_dialog};
use hud::{GameData, SharedGameData};
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
const C2_QUEST: u32 = 1352;
const C2_COMPONENT: u32 = 6758;
const C6_COMPONENT: u32 = 5320;

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Report,
    BeforeBubble,
    Progress,
    Done,
}

#[derive(Resource, Default)]
struct Seen {
    packets: Vec<Interact>,
    moves: usize,
    casts: usize,
}

#[derive(Resource, Default)]
struct InputSeen {
    keys: usize,
    clicks: usize,
}

#[derive(Resource, Default)]
struct AimSeen(bool);

#[derive(Resource, Default)]
struct Injected(bool);

#[derive(Resource, Default)]
struct Check {
    t1: Option<Vec3>,
    t2: Option<Vec3>,
    input_sent: bool,
    input_at: Option<f32>,
    input_checked: bool,
    confirmed: bool,
    open_checked: bool,
    started_before_confirm: bool,
    loaded_sent: bool,
    updated_sent: bool,
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
    }
    let rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0).expect("ui runtime");
    rt.lua
        .load(r#"QUEST_W=UIParent:CreateWidget("emptywidget","questDirecting",UIParent,1);QUEST_W:Show(true);OTHER=UIParent:CreateWidget("emptywidget","otherHud",UIParent,1);OTHER:Show(true)"#)
        .exec()
        .expect("cinema widgets");
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
    .insert_resource(Phase::Report)
    .insert_resource(Seen::default())
    .insert_resource(InputSeen::default())
    .insert_resource(AimSeen::default())
    .insert_resource(Injected::default())
    .insert_resource(Visual(
        aa_dev::args()
            .iter()
            .any(|a| a == "--shot" || a == "--offscreen-shot"),
    ))
    .init_resource::<Check>()
    .init_resource::<CinemaPlayer>()
    .init_resource::<Units>()
    .init_resource::<hud::HudKeyboardFocus>()
    .insert_resource(settings::Actions::default())
    .insert_non_send(HudBridge(shared.clone()))
    .insert_non_send(rt)
    .add_message::<NetEvent>()
    .add_message::<NetCommand>()
    .configure_sets(
        PreUpdate,
        (
            CinemaInputGate.after(bevy::input::InputSystems),
            CinemaInputRead.after(CinemaInputGate),
        ),
    )
    .configure_sets(Update, ingame::CameraFinal.after(lens_override))
    .add_systems(
        PreUpdate,
        (
            inject_input.before(CinemaInputGate),
            block_cinema_input.in_set(CinemaInputGate),
            aim_probe.in_set(CinemaInputRead),
        ),
    )
    .add_systems(
        Startup,
        move |mut commands: Commands, mut time: ResMut<Time<Virtual>>| {
            time.set_max_delta(std::time::Duration::from_secs_f32(1.0));
            time.set_relative_speed(12.0);
            commands.spawn((
                Camera3d::default(),
                ingame::OrbitCamera::default(),
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
    .add_systems(Update, lens_override.before(check))
    .add_systems(
        Update,
        (
            handle_dialog,
            track,
            tick_cinema,
            cinema_ui,
            drive_cinema_camera.in_set(ingame::CameraFinal),
            movement_probe,
            input_probe,
            collect,
            check,
        )
            .chain(),
    );
    app.run();
    AppExit::Success
}

fn show_widgets(rt: &aa_ui::UiRuntime) {
    rt.lua
        .load("QUEST_W:Show(true);OTHER:Show(true)")
        .exec()
        .unwrap();
}

fn visible(rt: &aa_ui::UiRuntime, name: &str) -> bool {
    rt.lua
        .load(format!("return {name}:IsVisible()"))
        .eval()
        .unwrap()
}

fn inject_input(
    time: Res<Time>,
    phase: Res<Phase>,
    player: Res<CinemaPlayer>,
    mut done: ResMut<Injected>,
    mut keys: MessageWriter<KeyboardInput>,
    mut mouse: MessageWriter<MouseButtonInput>,
) {
    if *phase != Phase::Report || done.0 || time.elapsed_secs() < 0.5 || !player.active() {
        return;
    }
    done.0 = true;
    for key in [KeyCode::Escape, KeyCode::KeyW, KeyCode::Digit1] {
        keys.write(KeyboardInput {
            key_code: key,
            logical_key: Key::Character("x".into()),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    }
    mouse.write(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window: Entity::PLACEHOLDER,
    });
}

fn aim_probe(mut buttons: MessageReader<MouseButtonInput>, mut seen: ResMut<AimSeen>) {
    if buttons
        .read()
        .any(|b| b.button == MouseButton::Left && b.state == ButtonState::Pressed)
    {
        seen.0 = true;
    }
}

fn lens_override(
    phase: Res<Phase>,
    player: Res<CinemaPlayer>,
    mut cameras: Query<&mut Projection, With<Camera3d>>,
) {
    if *phase != Phase::Report || !player.active() {
        return;
    }
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = 0.5;
        }
    }
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

fn input_probe(
    mut keys: MessageReader<KeyboardInput>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut seen: ResMut<InputSeen>,
) {
    seen.keys += keys.read().count();
    seen.clicks += buttons.read().count();
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

fn reset_phase(
    player: &mut CinemaPlayer,
    seen: &mut Seen,
    input: &mut InputSeen,
    check: &mut Check,
) {
    *player = CinemaPlayer::default();
    *seen = Seen::default();
    *input = InputSeen::default();
    *check = Check::default();
}

#[allow(clippy::too_many_arguments)]
fn check(
    time: Res<Time>,
    mut phase: ResMut<Phase>,
    mut player: ResMut<CinemaPlayer>,
    mut seen: ResMut<Seen>,
    mut input: ResMut<InputSeen>,
    aim: Res<AimSeen>,
    mut check: ResMut<Check>,
    visual: Res<Visual>,
    cameras: Query<(&Transform, &Projection), With<Camera3d>>,
    mut xml: Local<Option<String>>,
    bridge: NonSend<HudBridge>,
    rt: NonSend<aa_ui::UiRuntime>,
    mut net_events: MessageWriter<NetEvent>,
    mut exit: MessageWriter<AppExit>,
) {
    if *phase == Phase::Done {
        return;
    }
    if time.elapsed_secs() > 60.0 && *phase != Phase::Done {
        panic!(
            "cinema report phase {phase:?} did not finish (saw {:?})",
            seen.packets
        );
    }
    if !check.confirmed && time.elapsed_secs() > 36.0 && *phase == Phase::Report {
        panic!("the report cinema did not finish (saw {:?})", seen.packets);
    }
    if *phase == Phase::Report && !check.open_checked && !seen.packets.is_empty() {
        assert_eq!(
            seen.packets[0],
            Interact::CinemaPermission {
                component_id: COMPONENT,
                npc_obj_id: NPC,
                doodad_obj_id: 0
            },
            "0x0d0 must carry component 5336"
        );
        assert!(
            !seen
                .packets
                .contains(&Interact::Cinema { completed: false }),
            "0x0cf arrived before the later confirm"
        );
        check.open_checked = true;
    }
    if *phase == Phase::Report && !check.confirmed && time.elapsed_secs() >= 0.2 {
        check.started_before_confirm = seen
            .packets
            .contains(&Interact::Cinema { completed: false });
        bridge
            .0
            .borrow_mut()
            .dialog
            .requests
            .push(Req::Complete(QUEST, 0));
        check.confirmed = true;
    }
    if *phase == Phase::Progress {
        if !check.loaded_sent {
            let quest = Quest {
                id: 1,
                template_id: 1061,
                status: status::PROGRESS,
                component_id: C6_COMPONENT,
                ..Quest::default()
            };
            net_events.write(NetEvent(aa_net::Event::QuestsLoaded(vec![quest])));
            check.loaded_sent = true;
        } else if !check.updated_sent {
            assert!(
                seen.packets.is_empty(),
                "C6: a relog QuestsLoaded replayed the progress cinema"
            );
            let quest = Quest {
                id: 1,
                template_id: 1061,
                status: status::PROGRESS,
                component_id: C6_COMPONENT,
                ..Quest::default()
            };
            net_events.write(NetEvent(aa_net::Event::QuestUpdated(quest)));
            check.updated_sent = true;
        }
    }
    let Ok((transform, projection)) = cameras.single() else {
        return;
    };
    let Some(t) = player.elapsed() else {
        if seen.packets.is_empty() {
            return;
        }
        let ended = seen
            .packets
            .iter()
            .position(|p| *p == Interact::Cinema { completed: true });
        let Some(ended) = ended else { return };
        match *phase {
            Phase::Report => {
                let expected = [
                    Interact::CinemaPermission {
                        component_id: COMPONENT,
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
                    !visible(&rt, "QUEST_W"),
                    "C5: the talk window was restored instead of closed"
                );
                assert!(
                    visible(&rt, "OTHER"),
                    "C5: the rest of the HUD was not restored"
                );
                assert!(check.input_checked, "C1/C4/C5 checks did not run");
                println!(
                    "CINEMA REPORT: PASS report 1065; C1/C4/C5; 0x0d0 at open, 0x0cf at confirm, 0x0ce then the hand-in"
                );
                *phase = Phase::BeforeBubble;
                reset_phase(&mut player, &mut seen, &mut input, &mut check);
                show_widgets(&rt);
                let mut data = bridge.0.borrow_mut();
                data.dialog.directing = true;
                data.dialog.npc = Some((NPC, NPC));
                data.dialog.requests.push(Req::CinemaPermission {
                    quest: C2_QUEST,
                    kind: 2,
                });
            }
            Phase::BeforeBubble => {
                assert_eq!(
                    seen.packets,
                    [
                        Interact::CinemaPermission {
                            component_id: C2_COMPONENT,
                            npc_obj_id: NPC,
                            doodad_obj_id: 0
                        },
                        Interact::Cinema { completed: false },
                        Interact::Cinema { completed: true },
                        Interact::QuestStart {
                            quest_id: C2_QUEST,
                            npc_obj_id: NPC,
                            doodad_obj_id: 0,
                            sphere_id: 0
                        },
                    ],
                    "C2: 0x0d0+0x0cf at open, then 0x0ce and the accept"
                );
                println!(
                    "CINEMA REPORT: PASS before-bubble 1352; the panel stayed visible and the accept followed 0x0ce"
                );
                *phase = Phase::Progress;
                reset_phase(&mut player, &mut seen, &mut input, &mut check);
                let mut data = bridge.0.borrow_mut();
                data.dialog.directing = false;
                data.dialog.requests.clear();
            }
            Phase::Progress => {
                assert_eq!(
                    seen.packets,
                    [
                        Interact::Cinema { completed: false },
                        Interact::Cinema { completed: true }
                    ],
                    "C6: no 0x0d0 and one movie"
                );
                println!("CINEMA REPORT: PASS progress 1061/5320; update only, no 0x0d0");
                *phase = Phase::Done;
                println!("CINEMA REPORT: PASS all cases");
                if !visual.0 {
                    exit.write(AppExit::Success);
                }
            }
            Phase::Done => {}
        }
        return;
    };
    if *phase == Phase::Report {
        if t >= 1.5 && !check.input_checked {
            assert_eq!(
                (input.keys, input.clicks),
                (0, 0),
                "C4: keyboard/mouse events reached their consumers"
            );
            assert_eq!(
                (seen.moves, seen.casts),
                (0, 0),
                "C4: movement or a cast left the client during the movie"
            );
            assert!(!aim.0, "C4: a click while aiming acted during the movie");
            assert!(player.active(), "C1: Esc cancelled the movie");
            assert!(bridge.0.borrow().dialog.cinema_hidden, "C5: hidden flag");
            assert!(
                !visible(&rt, "QUEST_W") && !visible(&rt, "OTHER"),
                "C5: the HUD was not hidden"
            );
            assert!(
                !seen.packets.contains(&Interact::Cinema { completed: true }),
                "C1: Esc ended the movie early"
            );
            assert!(
                !check.started_before_confirm,
                "0x0cf started before the later confirm"
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
                assert!(
                    check.t1.unwrap().distance(check.t2.unwrap()) > 1.0,
                    "the camera did not follow the track"
                );
            }
        }
    } else if *phase == Phase::BeforeBubble {
        if t >= 0.2 && !check.confirmed {
            check.started_before_confirm = seen
                .packets
                .contains(&Interact::Cinema { completed: false });
            bridge
                .0
                .borrow_mut()
                .dialog
                .requests
                .push(Req::Accept(C2_QUEST));
            check.confirmed = true;
        }
        if t >= 0.5 && !check.input_checked {
            assert!(
                visible(&rt, "QUEST_W"),
                "C2: the accept panel must stay visible during its movie"
            );
            assert_eq!(
                &seen.packets[..2],
                &[
                    Interact::CinemaPermission {
                        component_id: C2_COMPONENT,
                        npc_obj_id: NPC,
                        doodad_obj_id: 0
                    },
                    Interact::Cinema { completed: false },
                ],
                "C2: 0x0d0 and 0x0cf must arrive together at the open"
            );
            assert!(
                !seen
                    .packets
                    .iter()
                    .any(|p| matches!(p, Interact::QuestStart { .. })),
                "C2: the accept went out during the movie"
            );
            check.input_checked = true;
        }
    }
}
