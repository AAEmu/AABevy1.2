//! A ship's helm under a held steering (`ShipUnitModel::UpdateRudder` / `UpdateSteeringWheel` /
//! `UpdateThrottleAnim`, `ingame::ship::helm`): the Eznan Cutter's `rudder` brush turns about its own up axis by
//! the steering as radians, and its `$helms` doodad (`galleon_helms`, attach point 35) turns about its axle to
//! the 45 degree step of `steering * 7` at 3.5 rad/s. The helm doodad is placed at the hull's `$helms` helper
//! here; in game the server announces it attached to the ship. There is no server: the steering and throttle
//! are what a type-3 echo would carry. `--model 1287 --view duck --throttle 1` shows the duck boat, whose feet
//! play the throttle clips (`run_f` at speed 1).
//!
//! `cargo run -p aa-client --example ship_helm -- --mute --shot shots/aa-client/ship_helm.png --shot-after 8`
//! `cargo run -p aa-client --example ship_helm -- --steer 0 --mute --shot shots/aa-client/ship_helm_still.png --shot-after 8`
//! `cargo run -p aa-client --example ship_helm -- --view helm --mute --shot shots/aa-client/ship_helm_wheel.png --shot-after 8`
//!
//! At rest an 8-spoke wheel looks the same on every 45 degree step; `--steer-after <s>` holds the helm until then,
//! so a shot 0.6 s later catches the wheel mid-turn:
//! `cargo run -p aa-client --example ship_helm -- --view helm --steer-after 6 --mute --shot shots/aa-client/ship_helm_wheel_turning.png --shot-after 6.6`
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
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::prelude::*;
use ingame::ship::helm;
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

/// The Eznan Cutter (`ShipModel` 393, the row named `galleon`).
const GALLEON: u32 = 393;
/// The galleon helm doodad (template 2374, `slave_doodad_bindings` attach point 35).
const HELMS: &str = "prefab://Prefabs/galleon.xml/galleon.galleon_helms";

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn number(name: &str, default: f32) -> f32 {
    arg(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The ship root, waiting for its `$helms` helper (the parts' helpers land with the spawn commands).
#[derive(Component)]
struct NeedsHelm;

fn main() -> AppExit {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("ship_helm: skipped, the game pak or database is missing (AA_PAK, AA_DB)");
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("ship_helm: skipped, the game database cannot be opened");
        return AppExit::Success;
    };
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Ship helm"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_fx::FxPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.68, 0.82)))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 500.0, ..default() })
        .insert_resource(db)
        .add_systems(Startup, scene)
        .add_systems(
            Update,
            (place_helm, hold_helm, helm::turn_rudders, helm::turn_helm_wheels, helm::play_throttle_anims, report).chain(),
        )
        .run()
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<ingame::data::Db>,
) {
    let model = number("--model", GALLEON as f32) as u32;
    // the galleon's `$helms` is at cry (0, -14.0, 6.9), its rudder hinge at (0, -15.4, 3.7): Bevy z 14 / 15.4 aft
    let (eye, target) = match arg("--view").as_deref() {
        // the wheel on the stern deck, from behind and above
        Some("helm") => (Vec3::new(4.0, 11.0, 20.0), Vec3::new(0.0, 7.5, 14.0)),
        // the duck boat (`--model 1287`) and its feet from astern
        Some("duck") => (Vec3::new(5.0, 2.5, 9.0), Vec3::new(0.0, 0.5, 1.5)),
        // the stern and its rudder from astern
        _ => (Vec3::new(8.0, 1.0, 42.0), Vec3::new(0.0, 3.0, 15.0)),
    };
    commands.spawn((Camera3d::default(), Transform::from_translation(eye).looking_at(target, Vec3::Y)));
    commands.spawn((
        DirectionalLight { illuminance: 10000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    let look = db.vehicle_look(model).expect("a ship look");
    let pieces = ingame::vehicle::prepare(&cry.loader(), &look);
    assert!(!pieces.is_empty(), "no part of the ship could be loaded");
    let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, Transform::default());
    commands.entity(root).insert((helm::ShipHelm::new(&look.normal), NeedsHelm));
}

/// The helm a type-3 echo would carry: `--steer` (default full right) and `--throttle`, from `--steer-after` on.
fn hold_helm(time: Res<Time>, mut ships: Query<&mut helm::ShipHelm>) {
    let held = time.elapsed_secs() >= number("--steer-after", 0.0);
    for mut ship in &mut ships {
        ship.steering = if held { number("--steer", 1.0).clamp(-1.0, 1.0) } else { 0.0 };
        ship.throttle = number("--throttle", 0.0).clamp(-1.0, 1.0);
    }
}

/// Hangs the galleon helm doodad on the hull's `$helms` helper, as the server's attached doodad would sit, and marks
/// its model the ship's wheel. A model without a wheel (the duck boat) gets none.
fn place_helm(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    roots: Query<(Entity, &ingame::vehicle::SeatHelpers, &helm::ShipHelm), With<NeedsHelm>>,
) {
    for (root, helpers, ship) in &roots {
        commands.entity(root).remove::<NeedsHelm>();
        if !ship.wheel_turns {
            continue;
        }
        let Some(point) = helpers.0.get("helms") else {
            warn!("ship_helm: the hull has no $helms helper");
            continue;
        };
        let Some(prepared) = ingame::doodad_model::prepare(&cry.loader(), HELMS) else {
            warn!("ship_helm: {HELMS} cannot be read");
            continue;
        };
        let (_, rotation, translation) = point.to_scale_rotation_translation();
        info!("ship_helm: $helms at {translation} (cry)");
        let spawned = ingame::doodad_model::spawn(&mut commands, &mut cry, prepared);
        let doodad = commands
            .spawn((
                Transform::from_translation(cry_to_bevy(translation.into())).with_rotation(cry_rot_to_bevy(rotation)),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        commands.entity(spawned.entity).insert((
            ChildOf(doodad),
            helm::HelmWheel { ship: root, base: Transform::IDENTITY },
        ));
    }
}

/// Logs the helm once a second: the angles the shot shows.
fn report(time: Res<Time>, ships: Query<&helm::ShipHelm>, mut last: Local<f32>) {
    if time.elapsed_secs() - *last < 1.0 {
        return;
    }
    *last = time.elapsed_secs();
    for ship in &ships {
        info!(
            "ship_helm: steering {:.2} (rudder {:.1} deg), wheel {:.1} deg, state {:?}, throttle {:.2}",
            ship.steering,
            ship.steering.to_degrees(),
            ship.wheel.to_degrees(),
            ship.state,
            ship.throttle
        );
    }
}
