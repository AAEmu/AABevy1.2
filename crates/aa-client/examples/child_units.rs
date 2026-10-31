//! Child slave units draw at their parent's attach point.
//!
//! A child slave's `SCUnitState` carries the parent object id, the attach point and a parent-LOCAL
//! offset (`aaemu-game` `slaves.rs:4256-4261`, `unit_state.rs:473-495`). The example spawns a ship
//! at (12000, 9000) and a cannon at local (2, 3, 1), checks that the child is in range, is a
//! `ChildOf` the ship and its `GlobalTransform` is the ship placement times the local offset, and
//! that a second ship yawed 90 degrees turns that offset. It exits non-zero when a check fails.
//!
//! `cargo run -p aa-client --example child_units -- --mute --headless`
//! `cargo run -p aa-client --example child_units -- --mute --offscreen-shot shots/aa-client/child_units.png --shot-after 2`

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
use aa_net::state::Units;
use aa_world::{bevy_to_cry, cry_to_bevy};
use bevy::app::AppExit;
use bevy::prelude::*;
use ingame::player::placement;
use ingame::units::{
    RemoteUnit, Views, attach_unit_root, unit_local_transform, unit_parent, unit_world,
};
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
/// The check runs after the transform hierarchy has propagated once.
const CHECK_AT: f32 = 0.5;

#[derive(Resource)]
struct Scene {
    ship_a: Entity,
    child_a: Entity,
    ship_b: Entity,
    child_b: Entity,
}

fn slave(id: u32, pos: Vec3, yaw: f32) -> Unit {
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
        model_id: 1,
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
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

fn child(id: u32, parent: u32, local: Vec3) -> Unit {
    let mut unit = slave(id, local, 0.0);
    unit.state_attachments = Some(UnitStateAttachments {
        first_point: 3,
        first_obj_id: Some(parent),
        bonding_obj_id: None,
    });
    unit
}

fn visual_transform(unit: &Unit) -> Transform {
    if unit_parent(unit).is_some() {
        unit_local_transform(unit, 1.0)
    } else {
        placement(
            Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
            unit.yaw(),
            1.0,
        )
    }
}

fn spawn_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    unit: &Unit,
    size: Vec3,
    color: Color,
) -> Entity {
    commands
        .spawn((
            Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                ..default()
            })),
            visual_transform(unit),
            RemoteUnit::announced(unit),
        ))
        .id()
}

fn setup(
    mut commands: Commands,
    mut units: ResMut<Units>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ship_a_unit = slave(SHIP, SHIP_AT, 0.0);
    let child_a_unit = child(CHILD, SHIP, LOCAL);
    let ship_b_unit = slave(SHIP_B, SHIP_AT, YAW_B);
    let child_b_unit = child(CHILD_B, SHIP_B, LOCAL);
    for unit in [&ship_a_unit, &child_a_unit, &ship_b_unit, &child_b_unit] {
        units.by_id.insert(unit.obj_id, unit.clone());
    }

    let ship_a = spawn_visual(
        &mut commands,
        &mut meshes,
        &mut materials,
        &ship_a_unit,
        Vec3::new(6.0, 2.0, 18.0),
        Color::srgb(0.45, 0.30, 0.18),
    );
    let child_a = spawn_visual(
        &mut commands,
        &mut meshes,
        &mut materials,
        &child_a_unit,
        Vec3::new(1.0, 1.0, 2.0),
        Color::srgb(0.2, 0.2, 0.22),
    );
    attach_unit_root(&mut commands, child_a, ship_a);
    let ship_b = spawn_visual(
        &mut commands,
        &mut meshes,
        &mut materials,
        &ship_b_unit,
        Vec3::new(6.0, 2.0, 18.0),
        Color::srgb(0.20, 0.30, 0.45),
    );
    let child_b = spawn_visual(
        &mut commands,
        &mut meshes,
        &mut materials,
        &child_b_unit,
        Vec3::new(1.0, 1.0, 2.0),
        Color::srgb(0.2, 0.2, 0.22),
    );
    attach_unit_root(&mut commands, child_b, ship_b);

    commands.insert_resource(Scene {
        ship_a,
        child_a,
        ship_b,
        child_b,
    });
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

fn check(
    scene: Option<Res<Scene>>,
    units: Res<Units>,
    views: Res<Views>,
    transforms: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    time: Res<Time>,
    mut checked: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(scene) = scene else { return };
    if *checked || time.elapsed_secs() < CHECK_AT {
        return;
    }
    *checked = true;
    let mut failures = Vec::new();

    let child_a = units.by_id.get(&CHILD).expect("child unit");
    let child_b = units.by_id.get(&CHILD_B).expect("second child unit");
    let expected_a = placement(SHIP_AT, 0.0, 1.0).transform_point(cry_to_bevy(LOCAL));
    let expected_b = placement(SHIP_AT, YAW_B, 1.0).transform_point(cry_to_bevy(LOCAL));

    let global_a = transforms
        .get(scene.child_a)
        .map(GlobalTransform::translation);
    match global_a {
        Ok(point) if point.distance(expected_a) < 1e-3 => {}
        other => failures.push(format!("child A global {other:?} != {expected_a:?}")),
    }
    let global_b = transforms
        .get(scene.child_b)
        .map(GlobalTransform::translation);
    match global_b {
        Ok(point) if point.distance(expected_b) < 1e-3 => {}
        other => failures.push(format!("child B global {other:?} != {expected_b:?}")),
    }
    if expected_b.distance(expected_a) < 1.0 {
        failures.push("the 90 degree ship yaw did not turn the local offset".into());
    }
    for (name, child, ship) in [
        ("A", scene.child_a, scene.ship_a),
        ("B", scene.child_b, scene.ship_b),
    ] {
        if parents.get(child).map(ChildOf::parent).ok() != Some(ship) {
            failures.push(format!("child {name} is not ChildOf its ship"));
        }
    }
    let world_a = unit_world(child_a, &units).unwrap_or(Vec3::splat(f32::NAN));
    if world_a.distance(bevy_to_cry(expected_a)) > 1e-3 {
        failures.push(format!(
            "unit_world A {world_a:?} != {:?}",
            bevy_to_cry(expected_a)
        ));
    }
    let world_b = unit_world(child_b, &units).unwrap_or(Vec3::splat(f32::NAN));
    if world_b.distance(bevy_to_cry(expected_b)) > 1e-3 {
        failures.push(format!(
            "unit_world B {world_b:?} != {:?}",
            bevy_to_cry(expected_b)
        ));
    }
    let (far, failed, _, _) = views.census(&units, Vec3::new(12001.7, 8991.4, 0.0));
    if far != 0 || failed != 0 {
        failures.push(format!("census says {far} far / {failed} failed, not 0/0"));
    }

    if failures.is_empty() {
        info!(
            "child_units: PASS: the children are at the ship placements, ChildOf their ships, and the yawed ship turns the offset"
        );
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
    } else {
        error!("child_units: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
}

fn shot_mode() -> bool {
    aa_dev::args()
        .iter()
        .any(|a| a == "--shot" || a == "--offscreen-shot")
}

fn main() -> AppExit {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: child units"),
            aa_dev::DevPlugin,
        ))
        .init_resource::<Units>()
        .init_resource::<Views>()
        .add_systems(Startup, setup)
        .add_systems(
            PostUpdate,
            check.after(bevy::transform::TransformSystems::Propagate),
        )
        .run()
}
