//! The ship's speed-driven wake (`ShipUnitModel::UpdateExhaustEffect`, `FUN_390c78d0`): the Eznan Cutter's
//! active `exhaust` emitters (bow wave, side wash, wake) are silent at rest and strengthen with the root's speed
//! (`ingame::vehicle::drive_speed_emitters`). Its `splash_l` / `splash_r` are authored inactive, so a turn shows no
//! splash, as in 1.2. Only the game database is used (no sound database), as in a muted run. The motion is
//! synthetic, there is no server.
//!
//! `cargo run -p aa-client --example ship_wake -- --mute --shot shots/aa-client/ship_wake_rest.png --shot-after 8`
//! `cargo run -p aa-client --example ship_wake -- --speed 12 --turn 0.2 --mute --shot shots/aa-client/ship_wake_speed.png --shot-after 8`
#![allow(dead_code)]
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/logging.rs"]
mod logging;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/priority.rs"]
mod priority;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
use bevy::prelude::*;
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

/// The Eznan Cutter: slave template 21, `ShipModel` 393.
const MODEL: u32 = 393;
const TEMPLATE: u32 = 21;

fn arg(name: &str) -> Option<f32> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

fn ship_unit(obj_id: u32, hp: i32) -> aa_net::model::Unit {
    aa_net::model::Unit {
        obj_id,
        name: format!("ship {obj_id}"),
        kind: aa_net::model::UnitKind::Slave {
            id: TEMPLATE,
            tl_id: obj_id as u16,
            template_id: TEMPLATE,
            summoner_id: 1,
        },
        owner_name: String::new(),
        pos: aa_net::Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 50,
        model_id: MODEL,
        equipment: aa_net::model::Equipment::default(),
        model_params: aa_net::model::ModelParams::default(),
        hp,
        mp: 0,
        max_hp: hp,
        max_mp: 0,
        dead: false,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        turret: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

fn main() -> AppExit {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("ship_wake: skipped, the game pak or database is missing (AA_PAK, AA_DB)");
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("ship_wake: skipped, the game database cannot be opened");
        return AppExit::Success;
    };
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Ship wake"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_fx::FxPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(db)
        .add_systems(Startup, (sea, scene))
        .add_systems(Update, (set_motion, ingame::vehicle::drive_speed_emitters).chain())
        .run()
}

fn sea(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(200.0, 200.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.06, 0.2, 0.3))),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<ingame::data::Db>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(28.0, 9.0, 30.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 10000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    let look = db.vehicle_look(MODEL).expect("the Eznan Cutter");
    let unit = ship_unit(7, 1000);
    let pieces = ingame::vehicle::prepare(&cry.loader(), &look);
    assert!(!pieces.is_empty(), "no part of the ship could be loaded");
    let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, Transform::from_xyz(0.0, 1.0, 0.0));
    commands.entity(root).insert(ingame::units::RemoteUnit::announced(&unit));
    ingame::vehicle::spawn_state_effects(&mut commands, root, &look.normal, false);
}

/// The synthetic motion: `--speed <m/s>` along the bow, `--turn <rad/s>` to the left (both default to rest).
fn set_motion(mut roots: Query<&mut ingame::vehicle::VehicleMotion>) {
    let (speed, turn) = (arg("--speed").unwrap_or(0.0), arg("--turn").unwrap_or(0.0));
    for mut motion in &mut roots {
        motion.velocity = Vec3::new(0.0, 0.0, -speed);
        motion.yaw_rate = turn;
        motion.forward_speed = speed;
    }
}
