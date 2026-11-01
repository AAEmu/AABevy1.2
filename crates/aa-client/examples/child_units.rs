//! Child slave units draw at their parent's attach point through the real receive/sync systems.
//!
//! A child slave's `SCUnitState` carries the parent object id, the attach point and a parent-LOCAL
//! offset (`aaemu-game` `slaves.rs:4256-4261`, `unit_state.rs:473-495`). The example announces a
//! ship at (12000, 9000) and a cannon at local (2, 3, 1) through `receive_moves` and lets
//! `sync_views` draw both from the real game models. It checks that the child is in range, is a
//! `ChildOf` the ship and its `GlobalTransform` is the ship placement times the local offset, and
//! that a second ship yawed 90 degrees turns that offset. It exits non-zero when a check fails.
//!
//! `cargo run -p aa-client --example child_units -- --mute --headless`
//! `cargo run -p aa-client --example child_units -- --mute --offscreen-shot shots/aa-client/child_units.png --shot-after 10`

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

use aa_net::model::{Equipment, ModelParams, Unit, UnitKind, UnitStateAttachments};
use aa_net::state::{Doodads, Units};
use aa_net::{Event, NetEvent};
use aa_world::{bevy_to_cry, cry_to_bevy};
use bevy::app::AppExit;
use bevy::prelude::*;
use ingame::data::Db;
use ingame::player::placement;
use ingame::units::{Views, unit_world};
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

const SHIP: u32 = 8911;
const SHIP_B: u32 = 8912;
const CHILD: u32 = 9001;
const CHILD_B: u32 = 9002;
const SHIP_AT: Vec3 = Vec3::new(12000.0, 9000.0, 0.0);
const LOCAL: Vec3 = Vec3::new(2.0, 3.0, 1.0);
const YAW_B: f32 = std::f32::consts::FRAC_PI_2;
const SHIP_MODEL: u32 = 129;
const CANNON_MODEL: u32 = 117;
const PLAYER: u32 = 1;
/// The real models are prepared on the workers; leave time for that before the check.
const CHECK_AT: f32 = 2.0;
const GIVE_UP_AT: f32 = 20.0;

#[derive(Resource)]
struct Scene {
    ship_a: Entity,
    child_a: Entity,
    ship_b: Entity,
    child_b: Entity,
}

fn slave(id: u32, pos: Vec3, yaw: f32, model_id: u32) -> Unit {
    Unit {
        obj_id: id,
        name: format!("slave {id}"),
        kind: UnitKind::Slave {
            id,
            tl_id: id as u16,
            template_id: 1,
            summoner_id: 0,
        },
        owner_name: String::new(),
        pos: aa_net::Pos::new(pos.x, pos.y, pos.z),
        rotation: [0.0, 0.0, yaw],
        scale: 1.0,
        level: 1,
        model_id,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 1,
        mp: 0,
        max_hp: 1,
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

fn child(id: u32, parent: u32, local: Vec3) -> Unit {
    let mut unit = slave(id, local, 0.0, CANNON_MODEL);
    unit.state_attachments = Some(UnitStateAttachments {
        first_point: 3,
        first_obj_id: Some(parent),
        bonding_obj_id: None,
    });
    unit
}

fn stage(mut commands: Commands, mut units: ResMut<Units>) {
    let ship_a = slave(SHIP, SHIP_AT, 0.0, SHIP_MODEL);
    let child_a = child(CHILD, SHIP, LOCAL);
    let ship_b = slave(SHIP_B, SHIP_AT, YAW_B, SHIP_MODEL);
    let child_b = child(CHILD_B, SHIP_B, LOCAL);
    for unit in [&ship_a, &child_a, &ship_b, &child_b] {
        units.by_id.insert(unit.obj_id, unit.clone());
    }
    commands.spawn((
        ingame::player::LocalPlayer { obj_id: PLAYER },
        ingame::player::Mover {
            pos: Vec3::new(12001.7, 8991.4, 0.0),
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(12000.0, 30.0, -9000.0 + 55.0)
            .looking_at(Vec3::new(12000.0, 0.0, -9000.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            ..default()
        },
        Transform::from_xyz(1.0, 2.0, 1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.85)));
}

fn announce(mut announced: Local<bool>, mut out: MessageWriter<NetEvent>, units: Res<Units>) {
    if *announced {
        return;
    }
    *announced = true;
    for id in [SHIP, CHILD, SHIP_B, CHILD_B] {
        if let Some(unit) = units.by_id.get(&id) {
            out.write(NetEvent(Event::UnitSpawned(Box::new(unit.clone()))));
        }
    }
}

fn check(
    scene: Option<Res<Scene>>,
    units: Res<Units>,
    views: Res<Views>,
    transforms: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    time: Res<Time>,
    mut started: Local<Option<f32>>,
    mut done: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(scene) = scene else { return };
    let now = time.elapsed_secs();
    let started = *started.get_or_insert(now);
    if *done || now - started < CHECK_AT {
        return;
    }
    if views.unit_entities() < 4 {
        if now - started > GIVE_UP_AT {
            error!(
                "child_units: FAILED: the real sync drew {} of 4 units",
                views.unit_entities()
            );
            exit.write(AppExit::error());
        }
        return;
    }
    *done = true;
    let mut failures = Vec::new();

    let child_a = units.by_id.get(&CHILD).expect("child unit");
    let child_b = units.by_id.get(&CHILD_B).expect("second child unit");
    let expected_a = placement(SHIP_AT, 0.0, 1.0).transform_point(cry_to_bevy(LOCAL));
    let expected_b = placement(SHIP_AT, YAW_B, 1.0).transform_point(cry_to_bevy(LOCAL));

    for (name, child, ship, expected) in [
        ("A", scene.child_a, scene.ship_a, expected_a),
        ("B", scene.child_b, scene.ship_b, expected_b),
    ] {
        let actual = transforms.get(child).map(GlobalTransform::translation);
        match actual {
            Ok(point) if point.distance(expected) < 1e-3 => {}
            other => failures.push(format!("child {name} global {other:?} != {expected:?}")),
        }
        if parents.get(child).map(ChildOf::parent).ok() != Some(ship) {
            failures.push(format!("child {name} is not ChildOf its ship"));
        }
    }
    if expected_b.distance(expected_a) < 1.0 {
        failures.push("the 90 degree ship yaw did not turn the local offset".into());
    }
    for (name, child, expected) in [("A", child_a, expected_a), ("B", child_b, expected_b)] {
        let world = unit_world(child, &units).unwrap_or(Vec3::splat(f32::NAN));
        if world.distance(bevy_to_cry(expected)) > 1e-3 {
            failures.push(format!(
                "unit_world {name} {world:?} != {:?}",
                bevy_to_cry(expected)
            ));
        }
    }
    let (far, failed, _, _) = views.census(&units, Vec3::new(12001.7, 8991.4, 0.0));
    if far != 0 || failed != 0 {
        failures.push(format!("census says {far} far / {failed} failed, not 0/0"));
    }

    if failures.is_empty() {
        info!(
            "child_units: PASS: the real sync drew both children under their ships at the parent-composed points"
        );
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
    } else {
        error!("child_units: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
}

fn remember_scene(mut commands: Commands, views: Res<Views>) {
    if views.unit_entities() < 4 {
        return;
    }
    let (Some(ship_a), Some(child_a), Some(ship_b), Some(child_b)) = (
        views.unit_entity(SHIP),
        views.unit_entity(CHILD),
        views.unit_entity(SHIP_B),
        views.unit_entity(CHILD_B),
    ) else {
        return;
    };
    commands.insert_resource(Scene {
        ship_a,
        child_a,
        ship_b,
        child_b,
    });
}

fn shot_mode() -> bool {
    aa_dev::args()
        .iter()
        .any(|a| a == "--shot" || a == "--offscreen-shot")
}

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("child_units: skipped, no game_pak (set AA_PAK)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!("child_units: skipped, no game database (set AA_DB)");
        return AppExit::Success;
    }
    let Some(db) = Db::open() else {
        eprintln!("child_units: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: child units"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
        ))
        .insert_resource(db)
        .init_resource::<Units>()
        .init_resource::<Doodads>()
        .init_resource::<Views>()
        .add_message::<NetEvent>()
        .add_systems(Startup, stage)
        .add_systems(PreUpdate, announce)
        .add_systems(
            Update,
            (
                ingame::units::receive_moves,
                ingame::units::sync_views,
                remember_scene,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            check.after(bevy::transform::TransformSystems::Propagate),
        )
        .run()
}
