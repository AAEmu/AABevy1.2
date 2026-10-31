//! Clicking a wagon anywhere on its model targets it, as in 1.2 (the server's own capture case e:
//! a right click on the slave you ride sends `CSChangeTarget` 0x02c). The example draws the Farm
//! Wagon (slave template 60, model 1008), then sends view rays through the middle, the front and
//! the rear of its drawn model box and requires the shared pick to return the wagon each time. A
//! ray beside it must pick nothing.
//!
//! The run exits non-zero on a failed check or a staging timeout. Needs `AA_PAK` and `AA_DB`;
//! without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example slave_pick -- --mute --offscreen-shot shots/aa-client/slave_pick.png`
//! `cargo run -p aa-client --example slave_pick -- --mute --headless`

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

use aa_net::Pos;
use aa_net::model::{Equipment, ModelParams, Unit, UnitKind};
use aa_net::state::Units;
use bevy::prelude::*;
use ingame::data::Db;
use ingame::player::placement;
use ingame::units::{RemoteUnit, Views};
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

const WAGON_OBJ: u32 = 20;
const WAGON_TEMPLATE: u32 = 60;
const WAGON_MODEL: u32 = 1008;
const WAGON_POS: Vec3 = Vec3::new(100.0, 200.0, 5.0);
const WAGON_YAW: f32 = 0.4;
/// Wait this long for the model and its bounds, then give up.
const READY_TIMEOUT: f32 = 30.0;
/// With no screenshot asked for, leave once the check passed.
const HEADLESS_EXIT: f32 = 2.5;

#[derive(Resource, Default)]
struct Checked {
    done: bool,
    passed: bool,
}

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("slave_pick: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "slave_pick: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = Db::open() else {
        eprintln!("slave_pick: skipped, the database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("Slave pick"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .init_resource::<Units>()
        .init_resource::<Views>()
        .init_resource::<Checked>()
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .add_systems(Startup, (stage, scene))
        .add_systems(Update, (check, finish))
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let wagon = aa_world::cry_to_bevy(WAGON_POS);
    let eye = wagon + Vec3::new(-7.0, 5.0, 7.0);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(wagon + Vec3::Y, Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(60.0, 60.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.3, 0.16))),
        Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(100.0, 200.0, 0.0))),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<Db>,
    mut views: ResMut<Views>,
    mut units: ResMut<Units>,
) {
    let look = db
        .vehicle_look(WAGON_MODEL)
        .expect("the Farm Wagon's vehicle look");
    let pieces = ingame::vehicle::prepare(&cry.loader(), &look);
    let wagon = ingame::vehicle::spawn(
        &mut commands,
        &mut cry,
        &clips,
        pieces,
        placement(WAGON_POS, WAGON_YAW, 1.0),
    );
    let unit = fixture_unit(
        WAGON_OBJ,
        UnitKind::Slave {
            id: WAGON_TEMPLATE,
            tl_id: WAGON_TEMPLATE as u16,
            template_id: WAGON_TEMPLATE,
            summoner_id: 0,
        },
        "Farm Wagon",
        WAGON_MODEL,
        WAGON_POS,
    );
    commands.entity(wagon).insert(RemoteUnit::announced(&unit));
    views.track_unit_entity(WAGON_OBJ, wagon);
    units.by_id.insert(WAGON_OBJ, unit);
}

fn fixture_unit(obj_id: u32, kind: UnitKind, name: &str, model_id: u32, pos: Vec3) -> Unit {
    Unit {
        obj_id,
        name: name.into(),
        kind,
        owner_name: String::new(),
        pos: Pos::new(pos.x, pos.y, pos.z),
        rotation: [0.0, 0.0, WAGON_YAW],
        scale: 1.0,
        level: 1,
        model_id,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 1,
        mp: 0,
        max_hp: 1,
        max_mp: 0,
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
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
        dead: false,
    }
}

/// A view ray the way a click makes one (a screen point back through the camera).
fn click_ray(camera: &Camera, transform: &GlobalTransform, at: Vec3) -> Result<Ray3d, String> {
    let screen = camera
        .world_to_viewport(transform, at)
        .map_err(|e| format!("{at:?} is off screen: {e}"))?;
    camera
        .viewport_to_world(transform, screen)
        .map_err(|e| format!("no ray at {screen:?}: {e}"))
}

fn check(
    time: Res<Time>,
    boxes: hud::pick::ModelBoxes,
    units: Res<Units>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut checked: ResMut<Checked>,
    mut exit: MessageWriter<AppExit>,
) {
    if checked.done || time.elapsed_secs() < 0.5 {
        return;
    }
    let Some(wagon) = units.get(WAGON_OBJ) else {
        if time.elapsed_secs() > READY_TIMEOUT {
            error!("slave_pick: the wagon unit was never announced");
            exit.write(AppExit::error());
            checked.done = true;
        }
        return;
    };
    let Some(model) = boxes.get(WAGON_OBJ) else {
        if time.elapsed_secs() > READY_TIMEOUT {
            error!("slave_pick: the wagon was never drawn with bounds");
            exit.write(AppExit::error());
            checked.done = true;
        }
        return;
    };
    let Some((camera, transform)) = cameras.iter().next() else {
        error!("slave_pick: no world camera");
        exit.write(AppExit::error());
        checked.done = true;
        return;
    };
    // the long horizontal axis of the drawn model (front and rear), and the side across it
    let along_x = model.half.x >= model.half.z;
    let (long, half_long, side, half_side) = if along_x {
        (Vec3::X, model.half.x, Vec3::Z, model.half.z)
    } else {
        (Vec3::Z, model.half.z, Vec3::X, model.half.x)
    };
    let front = model.centre + model.rotation * long * (half_long - 0.25);
    let rear = model.centre - model.rotation * long * (half_long - 0.25);
    let mut failures = Vec::new();
    for (what, at) in [("front", front), ("middle", model.centre), ("rear", rear)] {
        let picked = match click_ray(camera, transform, at) {
            Ok(ray) => hud::pick::pick_unit(&ray, &units, 0, |_| true, |obj_id| boxes.get(obj_id)),
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        info!("slave_pick: {what} click at {at:?} -> {picked:?}");
        if picked != Some(WAGON_OBJ) {
            failures.push(format!(
                "the {what} click picked {picked:?}, not the wagon {}",
                wagon.obj_id
            ));
        }
    }
    // a straight-down ray well beside the wagon must miss it
    let beside = model.centre + model.rotation * side * (half_side + 2.0);
    let beside_ray = Ray3d::new(
        beside + Vec3::Y * 10.0,
        Dir3::new(Vec3::NEG_Y).expect("down"),
    );
    let picked = hud::pick::pick_unit(&beside_ray, &units, 0, |_| true, |obj_id| boxes.get(obj_id));
    info!("slave_pick: beside click at {beside:?} -> {picked:?}");
    if picked.is_some() {
        failures.push(format!("the beside click picked {picked:?}, not nothing"));
    }
    checked.done = true;
    if failures.is_empty() {
        info!("slave_pick: the wagon is picked at its front, middle and rear");
        checked.passed = true;
    } else {
        for failure in failures {
            error!("slave_pick: {failure}");
        }
        exit.write(AppExit::error());
    }
}

/// Headless without `--shot` / `--offscreen-shot` (which `aa-dev` takes and exits on): leave after
/// the check passed, once the screenshot had its time.
fn finish(
    time: Res<Time>,
    args: Res<aa_dev::DevArgs>,
    checked: Res<Checked>,
    mut exit: MessageWriter<AppExit>,
) {
    if !checked.passed {
        return;
    }
    let at = if args.shot.is_some() || args.offscreen_shot.is_some() {
        args.shot_after + 1.0
    } else {
        HEADLESS_EXIT
    };
    if time.elapsed_secs() >= at {
        exit.write(AppExit::Success);
    }
}
