//! Clicking a wagon anywhere on its model targets it, as in 1.2 (`captures/2026-10-28b/summary.txt:36`,
//! case e `bound`: a right click on the slave you ride sends `CSChangeTarget` 0x02c). The example draws the Farm
//! Wagon (slave template 60, model 1008), then sends view rays through independently chosen world
//! points on its front, middle and rear (a 4 m body, not points read back from the pick box) and
//! requires the shared pick to return the wagon each time. A ray well beside it must pick nothing.
//!
//! It also stands a camera inside the wagon's box with a passenger on the body: the wagon must not
//! be picked from inside its own box, and the passenger must be (the deck case). A camera outside
//! the box looking at the same passenger must pick the passenger, not the nearer wagon box; a click
//! on the rear side with a merchant 3 m behind the wagon must pick the wagon, not the merchant; and
//! a wagon root moved and turned after its box was measured must be picked at the new place, not
//! the old one.
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
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
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
const DECK_NPC: u32 = 21;
const MERCHANT: u32 = 22;
/// Wait this long for the model and its bounds, then give up.
const READY_TIMEOUT: f32 = 30.0;
/// With no screenshot asked for, leave once the check passed.
const HEADLESS_EXIT: f32 = 2.5;

#[derive(Resource)]
struct WagonRoot(Entity);

#[derive(Resource, Default)]
struct MoveWagon {
    pending: bool,
    old_point: Vec3,
    new_point: Vec3,
}

#[derive(Resource, Default)]
struct Checked {
    done: bool,
    passed: bool,
    phase: u8,
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
        .init_resource::<MoveWagon>()
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .add_systems(Startup, (stage, scene))
        .add_systems(Update, (move_wagon, check, finish).chain())
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
    commands.insert_resource(WagonRoot(wagon));
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
        turret: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
        dead: false,
    }
}

/// The shared pick, with the current world pose and model box of each unit.
fn pick(
    ray: &Ray3d,
    units: &Units,
    poses: &hud::pick::UnitPoses,
    boxes: &mut hud::pick::ModelBoxes,
) -> Option<u32> {
    hud::pick::pick_unit(
        ray,
        units,
        0,
        None,
        |_| true,
        |unit, ray| {
            (
                poses.get(unit, units).map(|(pos, _)| pos),
                boxes.get_near(unit, ray),
            )
        },
    )
}

fn down(at: Vec3) -> Ray3d {
    Ray3d::new(at, Dir3::new(Vec3::NEG_Y).expect("down"))
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

/// Drives the wagon after its first box was measured; the check asks for it in a later frame so
/// the pick reads the moved current transform, not last frame's `GlobalTransform`.
fn move_wagon(
    mut moved: ResMut<MoveWagon>,
    wagon: Res<WagonRoot>,
    mut roots: Query<&mut Transform>,
) {
    if !moved.pending {
        return;
    }
    let Some(root) = roots.get(wagon.0).copied().ok() else {
        return;
    };
    let local_rear = Vec3::new(0.0, 1.2, 1.8);
    moved.old_point = root.transform_point(local_rear);
    let moved_root = Transform {
        translation: root.translation + Vec3::new(20.0, 0.0, 0.0),
        rotation: root.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        ..root
    };
    moved.new_point = moved_root.transform_point(local_rear);
    if let Ok(mut root) = roots.get_mut(wagon.0) {
        *root = moved_root;
    }
    moved.pending = false;
}

fn check(
    time: Res<Time>,
    mut boxes: hud::pick::ModelBoxes,
    poses: hud::pick::UnitPoses,
    mut units: ResMut<Units>,
    mut moved: ResMut<MoveWagon>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut checked: ResMut<Checked>,
    mut exit: MessageWriter<AppExit>,
) {
    if checked.done {
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
    let Some(model) = boxes.get(wagon) else {
        // the box is not ready until every mesh of the model has its bounds
        if time.elapsed_secs() > READY_TIMEOUT {
            error!("slave_pick: the wagon model was never fully built");
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
    let mut failures = Vec::new();
    if checked.phase == 0 {
        // independently chosen points on the 4 m wagon body: 1.8 m to the front and rear of its
        // origin, 1.2 m up; not read back from the pick box, so the click is not the box checking itself
        let base = aa_world::cry_to_bevy(WAGON_POS);
        let turn = Quat::from_rotation_y(WAGON_YAW);
        let point = |local: Vec3| base + turn * local;
        for (what, at) in [
            ("front", point(Vec3::new(0.0, 1.2, -1.8))),
            ("middle", point(Vec3::new(0.0, 1.2, 0.0))),
            ("rear", point(Vec3::new(0.0, 1.2, 1.8))),
        ] {
            let ray = match click_ray(camera, transform, at) {
                Ok(ray) => ray,
                Err(e) => {
                    failures.push(e);
                    continue;
                }
            };
            let picked = pick(&ray, &units, &poses, &mut boxes);
            info!("slave_pick: {what} click at {at:?} -> {picked:?}");
            if picked != Some(WAGON_OBJ) {
                failures.push(format!(
                    "the {what} click picked {picked:?}, not the wagon {}",
                    wagon.obj_id
                ));
            }
        }
        // a straight-down ray 4 m beside the wagon must miss it
        let beside = point(Vec3::new(4.0, 1.2, 0.0));
        let beside_ray = down(beside);
        let picked = pick(&beside_ray, &units, &poses, &mut boxes);
        info!("slave_pick: beside click at {beside:?} -> {picked:?}");
        if picked.is_some() {
            failures.push(format!("the beside click picked {picked:?}, not nothing"));
        }
        // the deck case: a camera inside the wagon's box must not pick the wagon; a passenger on the
        // body must be picked instead. The camera sits at a horizontal near-corner of the box (inside,
        // and more than the 1.1 m pick sphere from the wagon's middle); the passenger's middle is the
        // independently chosen point 0.5 m above the wagon's middle.
        let wagon_middle = base + Vec3::Y * 0.9;
        let passenger_middle = wagon_middle + Vec3::Y * 0.5;
        let passenger = fixture_unit(
            DECK_NPC,
            UnitKind::Npc {
                template_id: 1,
                owner_id: 0,
            },
            "Deck Hand",
            0,
            aa_world::bevy_to_cry(passenger_middle - Vec3::Y * 0.9),
        );
        units.by_id.insert(DECK_NPC, passenger);
        let inside =
            model.centre + model.rotation * Vec3::new(model.half.x * 0.9, 0.0, model.half.z * 0.9);
        if !model.contains(inside) {
            failures.push("the deck camera is not inside the wagon box".into());
        }
        let deck_ray = Ray3d::new(
            inside,
            Dir3::new(passenger_middle - inside).expect("the passenger direction"),
        );
        let picked = pick(&deck_ray, &units, &poses, &mut boxes);
        info!("slave_pick: deck click from {inside:?} -> {picked:?}");
        if picked != Some(DECK_NPC) {
            failures.push(format!(
                "the on-deck click picked {picked:?}, not the passenger {DECK_NPC}"
            ));
        }
        // a camera outside the box looking at the same passenger: the ray meets the wagon's box
        // before the passenger's sphere, and the passenger must still win
        let shore = passenger_middle + model.rotation * Vec3::new(0.0, 0.0, -(model.half.z + 6.0));
        if model.contains(shore) {
            failures.push("the shore camera is inside the wagon box".into());
        }
        let shore_ray = Ray3d::new(
            shore,
            Dir3::new(passenger_middle - shore).expect("the passenger direction"),
        );
        let picked = pick(&shore_ray, &units, &poses, &mut boxes);
        info!("slave_pick: shore click from {shore:?} -> {picked:?}");
        if picked != Some(DECK_NPC) {
            failures.push(format!(
                "the shore click picked {picked:?}, not the passenger {DECK_NPC}"
            ));
        }
        // a merchant 3 m behind the wagon on the same ray: a click on the rear side, outside the
        // wagon's own sphere, must pick the wagon (the nearer box entry), not the merchant
        let behind_camera = base + turn * Vec3::new(8.0, 3.0, 0.0);
        let behind_aim = base + turn * Vec3::new(0.5, 1.2, 1.8);
        let behind_ray = Ray3d::new(
            behind_camera,
            Dir3::new(behind_aim - behind_camera).expect("the wagon rear side"),
        );
        let merchant_middle = behind_ray.origin + *behind_ray.direction * 12.0;
        let merchant = fixture_unit(
            MERCHANT,
            UnitKind::Npc {
                template_id: 1,
                owner_id: 0,
            },
            "Wagon-side Merchant",
            0,
            aa_world::bevy_to_cry(merchant_middle - Vec3::Y * 0.9),
        );
        units.by_id.insert(MERCHANT, merchant);
        let picked = pick(&behind_ray, &units, &poses, &mut boxes);
        info!("slave_pick: behind-the-wagon click -> {picked:?}");
        if picked != Some(WAGON_OBJ) {
            failures.push(format!(
                "the rear-side click picked {picked:?}, not the wagon {WAGON_OBJ}"
            ));
        }
        if !failures.is_empty() {
            checked.done = true;
            for failure in failures {
                error!("slave_pick: {failure}");
            }
            exit.write(AppExit::error());
            return;
        }
        moved.pending = true;
        checked.phase = 1;
        return;
    }
    // the wagon drove on after its box was measured: the click follows it to the new place and
    // no longer picks the old one
    // the ray starts above the body, as a click ray from a camera outside the box would
    let above = Vec3::Y * 5.0;
    let picked = pick(&down(moved.new_point + above), &units, &poses, &mut boxes);
    info!(
        "slave_pick: moved wagon click at {:?} -> {picked:?}",
        moved.new_point
    );
    if picked != Some(WAGON_OBJ) {
        failures.push(format!(
            "the moved wagon click at {:?} picked {picked:?}, not the wagon",
            moved.new_point
        ));
    }
    let picked = pick(&down(moved.old_point + above), &units, &poses, &mut boxes);
    info!(
        "slave_pick: old wagon click at {:?} -> {picked:?}",
        moved.old_point
    );
    if picked.is_some() {
        failures.push(format!(
            "the old wagon place picked {picked:?}, not nothing"
        ));
    }
    checked.done = true;
    if failures.is_empty() {
        info!(
            "slave_pick: the wagon is picked at its front, middle and rear, on deck and after moving, \
             and not from inside its box or at its old place"
        );
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
