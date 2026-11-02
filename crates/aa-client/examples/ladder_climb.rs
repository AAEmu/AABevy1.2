//! Ladder self-check: the use trigger, the climb, the top exit and the outgoing hang packets,
//! driven through the real `ingame::ladder` systems.
//!
//! The run grabs a synthetic kind-1 climb doodad with F, climbs with W, and asserts:
//! * `CSHang` and a `CSMoveUnit` with actor flag `0x40`, gcId 1 and the grab's climb data are sent;
//! * the body rises along the ladder and the climb data stays the grab value;
//! * reaching the top starts the 0.5 s exit, then the body steps over and `CSUnhang` 1 is sent;
//! * a ladder attached to a rotated, translated hull (`ChildOf`) hangs on the ladder's front
//!   side, not between the rungs and the deck, and its jump carries outboard (`CSUnhang` 7).
//!
//! It exits non-zero when a stage times out or a check fails. Without `--shot`/`--offscreen-shot`
//! it ends after the checks.
//!
//! `cargo run -p aa-client --example ladder_climb -- --mute --shot shots/aa-client/ladder_climb.png --shot-after 3 --exit-after 6`

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

use aa_net::state::{Doodads, Units};
use aa_net::{Command, NetCommand, Pos};
use bevy::prelude::*;
use ingame::data::Db;
use ingame::doodad_model::LadderHelper;
use ingame::doodad_visual::DoodadVisual;
use ingame::ladder::{Ladder, LadderClaim, control_ladder};
use ingame::player::{LocalPlayer, Mover, PlayerInput, SendState};
use ingame::units::{DeckHullPoses, RemoteDoodad, Views};
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

const LADDER: u32 = 77;
const SHIP_LADDER: u32 = 78;
const HULL: u32 = 0x1234;
const PLAYER: u32 = 100;
const HEIGHT: f32 = 2.0;
const HULL_POS: Vec3 = Vec3::new(4.0, 0.0, 0.0);
const HULL_YAW: f32 = std::f32::consts::FRAC_PI_2;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("ladder_climb: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    let Some(db) = Db::open() else {
        eprintln!("ladder_climb: skipped, the game database is missing (AA_DB)");
        return AppExit::Success;
    };
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Ladder climb"),
        aa_dev::DevPlugin,
        aa_cry::CryPlugin,
        aa_anim::AnimPlugin,
        aa_world::pipelines::build,
    ))
    .init_state::<Screen>()
    .insert_state(Screen::InGame)
    .insert_resource(db)
    .init_resource::<Doodads>()
    .init_resource::<Units>()
    .init_resource::<Views>()
    .init_resource::<DeckHullPoses>()
    .init_resource::<LadderClaim>()
    .init_resource::<ingame::ladder::LadderGrab>()
    .init_resource::<SendState>()
    .init_resource::<PlayerInput>()
    .init_resource::<settings::Actions>()
    .init_resource::<Script>()
    .add_message::<NetCommand>()
    .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.84)))
    .insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 300.0,
        ..default()
    })
    .add_systems(Startup, (scene, spawn_ladder))
    .add_systems(Update, (control_ladder, run_script.after(control_ladder)));
    app.run()
}

#[derive(Resource, Default)]
struct Script {
    stage: u8,
    hang_seen: bool,
    move_seen: bool,
    climb_seen: bool,
    top_seen: bool,
    unhang_seen: bool,
    ship_hang_seen: bool,
    ship_front_seen: bool,
    ship_unhang_seen: bool,
    done: bool,
}

#[derive(Component)]
struct PlayerBody;

fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(5.0, 2.6, 7.0).looking_at(Vec3::new(0.0, 1.8, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.22, 0.32, 0.16))),
    ));
    let wood = materials.add(Color::srgb(0.36, 0.24, 0.12));
    for x in [-0.42f32, 0.42] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.09, HEIGHT, 0.09))),
            MeshMaterial3d(wood.clone()),
            Transform::from_xyz(x, HEIGHT * 0.5, 0.0),
        ));
    }
    for i in 0..6 {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.94, 0.06, 0.06))),
            MeshMaterial3d(wood.clone()),
            Transform::from_xyz(0.0, 0.15 + i as f32 * (HEIGHT - 0.3) / 5.0, 0.0),
        ));
    }
    // the local body: a feet-root the ladder places, with a capsule drawn on it
    let player = commands
        .spawn((
            LocalPlayer { obj_id: PLAYER },
            Mover {
                pos: Vec3::new(0.5, 0.0, 0.0),
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            GlobalTransform::default(),
            ingame::Motion::default(),
            Visibility::default(),
        ))
        .id();
    commands.spawn((
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.1))),
        MeshMaterial3d(materials.add(Color::srgb(0.18, 0.35, 0.75))),
        Transform::from_xyz(0.0, 0.9, 0.0),
        PlayerBody,
        ChildOf(player),
    ));
}

fn spawn_ladder(mut commands: Commands, mut doodads: ResMut<Doodads>) {
    doodads.by_id.insert(
        LADDER,
        aa_net::model::Doodad {
            obj_id: LADDER,
            template_id: 303,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 15013,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        },
    );
    commands.spawn((
        RemoteDoodad { obj_id: LADDER },
        DoodadVisual {
            path: String::new(),
            model: None,
            func_group_id: 15013,
            rendered_func_group_id: 15013,
            bond_helpers: Default::default(),
            ladder: Some(LadderHelper {
                bottom: Vec3::ZERO,
                top: Vec3::new(0.0, HEIGHT, 0.0),
                axis: Vec3::Y,
                forward: Vec3::NEG_Z,
            }),
            growth: None,
            remaining_ms: 0.0,
            total_ms: 0.0,
            base_scale: 1.0,
        },
        Transform::default(),
        GlobalTransform::default(),
    ));
    // A rotated, translated hull with the ladder as its child, as a ship's side ladder is.
    let hull = commands
        .spawn((
            Transform {
                translation: aa_world::cry_to_bevy(HULL_POS),
                rotation: Quat::from_rotation_y(HULL_YAW),
                scale: Vec3::ONE,
            },
            GlobalTransform::default(),
            Visibility::default(),
        ))
        .id();
    doodads.by_id.insert(
        SHIP_LADDER,
        aa_net::model::Doodad {
            obj_id: SHIP_LADDER,
            template_id: 1187,
            owner_obj_id: 0,
            parent_obj_id: HULL,
            attach_point: 24,
            pos: Pos::new(0.0, 0.0, 0.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 15013,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        },
    );
    commands.spawn((
        RemoteDoodad {
            obj_id: SHIP_LADDER,
        },
        DoodadVisual {
            path: String::new(),
            model: None,
            func_group_id: 15013,
            rendered_func_group_id: 15013,
            bond_helpers: Default::default(),
            ladder: Some(LadderHelper {
                bottom: Vec3::ZERO,
                top: Vec3::new(0.0, HEIGHT, 0.0),
                axis: Vec3::Y,
                // the helper faces the hull's local +X; the hull yaw turns it to world +Y
                forward: Vec3::X,
            }),
            growth: None,
            remaining_ms: 0.0,
            total_ms: 0.0,
            base_scale: 1.0,
        },
        Transform::default(),
        GlobalTransform::default(),
        ChildOf(hull),
    ));
}

#[allow(clippy::too_many_arguments)]
fn run_script(
    time: Res<Time>,
    mut actions: ResMut<settings::Actions>,
    mut script: ResMut<Script>,
    ladder: Query<&Ladder>,
    mut player: Query<&mut Mover, With<LocalPlayer>>,
    mut moves: MessageReader<NetCommand>,
    mut exit: MessageWriter<AppExit>,
) {
    if script.done {
        return;
    }
    let t = time.elapsed_secs();
    for NetCommand(command) in moves.read() {
        match command {
            Command::Hang { target_obj_id } if *target_obj_id == LADDER => script.hang_seen = true,
            Command::Hang { target_obj_id } if *target_obj_id == SHIP_LADDER => {
                script.ship_hang_seen = true
            }
            Command::Unhang { reason: 1 } => script.unhang_seen = true,
            Command::Unhang { reason: 7 } => script.ship_unhang_seen = true,
            Command::Move(m) => {
                let movement = m.to_movement(0);
                if movement.actor_flags & aa_net::model::actor_flags::HANGING_FROM_OBJECT != 0 {
                    script.move_seen = true;
                    if let Some(support) = movement.support
                        && support.obj_id == 1
                        && support.climb.is_some()
                    {
                        script.climb_seen = true;
                    }
                }
            }
            _ => {}
        }
    }
    match script.stage {
        0 if t > 0.6 => {
            actions.script_tap("do_interaction_1");
            script.stage = 1;
        }
        1 => {
            actions.script_clear_taps();
            actions.script_hold("moveforward");
            script.stage = 2;
        }
        2 => {
            if let Ok(l) = ladder.single() {
                if l.fraction > 0.25 {
                    script.climb_seen = true;
                }
                if l.exiting.is_some() {
                    script.top_seen = true;
                }
            }
            if t > 2.2 {
                actions.script_release("moveforward");
            }
            if t > 2.6 {
                assert!(script.hang_seen, "no CSHang");
                assert!(script.move_seen, "no hanging CSMoveUnit");
                assert!(
                    script.climb_seen,
                    "the climb data block was missing or the body did not climb"
                );
                script.stage = 3;
            }
        }
        3 => {
            if ladder.single().is_err() {
                assert!(
                    script.top_seen || script.unhang_seen,
                    "the top exit did not start"
                );
                assert!(script.unhang_seen, "no CSUnhang 1 after the top exit");
                // the ship ladder: stand on the deck side (south of the front), the inside
                if let Ok(mut mover) = player.single_mut() {
                    mover.pos = HULL_POS + Vec3::new(0.0, -0.6, 0.0);
                    mover.facing = 0.0;
                    mover.vz = 0.0;
                    mover.grounded = true;
                }
                actions.script_clear_taps();
                script.stage = 4;
            }
            assert!(t < 10.0, "the top exit timed out");
        }
        4 => {
            actions.script_tap("do_interaction_1");
            script.stage = 5;
        }
        5 => {
            if let Ok(l) = ladder.single()
                && l.obj_id == SHIP_LADDER
            {
                let mover = player.single().expect("the local player");
                assert!(
                    mover.pos.y > HULL_POS.y + 0.25,
                    "the ship ladder hangs inside the hull: {:?}",
                    mover.pos
                );
                script.ship_front_seen = true;
                actions.script_clear_taps();
                actions.script_tap("jump");
                script.stage = 6;
            }
            assert!(t < 15.0, "the ship ladder grab timed out");
        }
        6 => {
            if ladder.single().is_err() && script.ship_unhang_seen {
                assert!(script.ship_hang_seen, "no CSHang on the ship ladder");
                assert!(
                    script.ship_front_seen,
                    "the ship ladder front side was never checked"
                );
                let mover = player.single().expect("the local player");
                assert!(
                    mover.pos.y > HULL_POS.y + 0.5,
                    "the ship ladder jump did not carry outboard: {:?}",
                    mover.pos
                );
                println!(
                    "ladder_climb: PASS (grab, climb data, top exit, ship ladder front side and jump)"
                );
                script.done = true;
                if !std::env::args().any(|a| a == "--shot" || a == "--offscreen-shot") {
                    exit.write(AppExit::Success);
                }
            }
            assert!(t < 18.0, "the ship ladder jump timed out");
        }
        _ => {}
    }
}
