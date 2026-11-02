//! The opened Experimental Glider on a real character through `glide::show_glider` and `glide::drive_glide`, with a
//! self-check: the wing plane must stay within 10 degrees of horizontal (the 2026-11-01 owner report was a glider
//! standing vertical on the character's `item_glider` bone).
//!
//! `AA_EQUIP_VIEW=back|side|front|top` picks the camera.
//! `cargo run -p aa-client --example glider_look -- --mute --offscreen-shot shots/aa-client/glider_look.png --shot-after 3`
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

use aa_anim::{AnimPlugin, GameDb, Gender, Race, facial::FacialPlugin};
use aa_cry::scene::{CryAssets, CryPlugin};
use aa_net::model::{Equipment, Item, ModelParams, slot};
use aa_net::state::Inventory;
use bevy::prelude::*;
use ingame::glide::{Active, Flight, GlideClips, GlideParams, Gliding, Pose};
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

const GLIDER: u32 = 14677;

#[derive(Resource, Default)]
struct WingChecked(bool);

fn item(template_id: u32) -> Item {
    Item {
        id: u64::from(template_id),
        template_id,
        count: 1,
        ..default()
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: glider look"),
            aa_dev::DevPlugin,
            CryPlugin,
            AnimPlugin,
            FacialPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight {
            brightness: 600.,
            ..default()
        })
        .init_resource::<WingChecked>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (ingame::glide::drive_glide, ingame::glide::show_glider).chain(),
        )
        .add_systems(Update, check_wing.after(ingame::glide::show_glider))
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<aa_anim::ClipLibrary>) {
    let db = GameDb::open().expect("read-only original database");
    let client_db = ingame::data::Db::open().expect("game database");
    let mut equipment = Equipment::default();
    equipment.slots[slot::BACKPACK] = Some(item(GLIDER));
    let race = match std::env::var("AA_GLIDER_RACE").as_deref() {
        Ok("el") => Race::Elf,
        Ok("ha") => Race::Hariharan,
        Ok("fe") => Race::Ferre,
        _ => Race::Nuian,
    };
    let gender = if std::env::var("AA_GLIDER_GENDER").as_deref() == Ok("f") {
        Gender::Female
    } else {
        Gender::Male
    };
    let desc = ingame::appearance::describe(&db, race, gender, &equipment, &ModelParams::default())
        .expect("describe");
    let assets = ingame::appearance::resolve(&db, &desc, &equipment).expect("resolve");
    let carried = ingame::weapons::carried(&client_db, &equipment);
    let root = ingame::player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        assets,
        Transform::IDENTITY,
        carried,
    )
    .expect("player spawn");
    commands.entity(root).insert((
        ingame::player::LocalPlayer { obj_id: 7 },
        GlideClips::default(),
    ));
    let mut gliding = Gliding::default();
    let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
    flight.elapsed = 3.0;
    gliding.active = Some(Active {
        flight,
        buff_id: 1029,
        buff_index: 1,
        pose: Pose::Idle,
        touched: false,
        touched_for: 0.0,
        missing: 0.0,
        water: false,
        deck: None,
    });
    commands.insert_resource(gliding);
    let mut inventory = Inventory::default();
    inventory.equipment = equipment;
    commands.insert_resource(inventory);
    commands.insert_resource(client_db);
    let view = std::env::var("AA_EQUIP_VIEW").unwrap_or_else(|_| "back".into());
    let eye = match view.as_str() {
        "side" => Vec3::new(10.0, 2.6, 0.0),
        "front" => Vec3::new(0.0, 2.6, -10.0),
        "top" => Vec3::new(0.0, 14.0, 0.01),
        _ => Vec3::new(0.0, 2.6, 10.0),
    };
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(Vec3::new(0.0, 1.8, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
}

/// Waits for the glider to be drawn, then measures the wing plane from the skinned mesh: a vertical wing means the
/// attachment added a second CryEngine-to-Bevy conversion under the `item_glider` bone.
fn check_wing(world: &mut World) {
    if world.resource::<WingChecked>().0 || world.resource::<Time>().elapsed_secs() < 2.0 {
        return;
    }
    let Some(normal) = ingame::glide::glider_wing_normal(world) else {
        return;
    };
    let degrees = normal.dot(Vec3::Y).abs().acos().to_degrees();
    assert!(
        degrees < 10.0,
        "glider_look: the wing stands up: its plane is {degrees:.1} deg from horizontal"
    );
    println!("glider_look: PASS wing plane {degrees:.1} deg from horizontal");
    world.resource_mut::<WingChecked>().0 = true;
}
