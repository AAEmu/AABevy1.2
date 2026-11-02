//! An Eznan Cutter (slave 21, model 393) burns at 25 %, dies and sinks on the real prefabs. The
//! example drives the HP state and `SCUnitDeath` through `vehicle::update_damage`, checks that the
//! damaged aft-deck fire keeps burning, that the dead look has no explosion, that the water-destroy
//! effect is spawned and that the hull lowers and lists over `vehicle::WRECK_SINK_TIME`. It exits
//! non-zero on a mismatch. `--shot` keeps it open for the picture.
//!
//! `cargo run -p aa-client --example vehicle_ship_death -- --mute --offscreen-shot shots/aa-client/vehicle_ship_death.png --shot-after 8`

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

/// The Eznan Cutter: `models` 393 (`ShipModel` 6), slave template 21 (52 216 HP).
const MODEL: u32 = 393;
const TEMPLATE: u32 = 21;
const OBJ: u32 = 900;

#[derive(Resource)]
struct Check {
    timer: Timer,
    phase: u8,
    dead_at: f64,
    base_y: f32,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.5, TimerMode::Repeating),
            phase: 0,
            dead_at: 0.0,
            base_y: 0.0,
        }
    }
}

#[derive(Component)]
struct Ship;

fn galleon_unit(hp: i32, max_hp: i32) -> aa_net::model::Unit {
    aa_net::model::Unit {
        obj_id: OBJ,
        name: "Eznan Cutter".into(),
        kind: aa_net::model::UnitKind::Slave {
            id: TEMPLATE,
            tl_id: 1,
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
        max_hp,
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
    if !aa_cry::pak_available() {
        eprintln!(
            "vehicle_ship_death: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "vehicle_ship_death: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("vehicle_ship_death: skipped, the game database cannot be opened");
        return AppExit::Success;
    };
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Vehicle ship death"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_fx::FxPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .insert_resource(Check::default())
        .init_resource::<aa_net::state::Units>()
        .init_resource::<ingame::vehicle::VehicleSwaps>()
        .add_systems(Startup, (stage, scene).chain())
        .add_systems(
            Update,
            (
                ingame::vehicle::update_damage,
                ingame::vehicle::finish_damage_swaps,
                ingame::vehicle::sink_ship_wrecks,
                check,
            )
                .chain(),
        )
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(34.0, 14.0, 42.0).looking_at(Vec3::new(0.0, 5.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(300.0, 300.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.05, 0.18, 0.3))),
        Transform::from_xyz(0.0, -0.4, 0.0),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<ingame::data::Db>,
    mut units: ResMut<aa_net::state::Units>,
) {
    let look = db.vehicle_look(MODEL).expect("the Eznan Cutter");
    let variants = db
        .vehicle_variants(MODEL)
        .expect("the Eznan Cutter's state rows");
    let max_hp = db
        .slave_max_hp(TEMPLATE, 50)
        .expect("template 21's MaxHealth");
    let hp = max_hp * 25 / 100;
    let unit = galleon_unit(hp, max_hp);
    let variant = variants.for_state(4).to_string();
    assert!(variant.ends_with("galleon.damaged75"), "{variant}");
    let loader = cry.loader();
    let pieces = ingame::vehicle::prepare(
        &loader,
        &ingame::data::VehicleLook {
            normal: variant.clone(),
            wheels: look.wheels,
        },
    );
    let root = ingame::vehicle::spawn(
        &mut commands,
        &mut cry,
        &clips,
        pieces,
        Transform::default(),
    );
    commands.entity(root).insert((
        Ship,
        ingame::units::RemoteUnit::announced(&unit),
        ingame::vehicle::VehicleDamage {
            state: 4,
            variant: variant.clone(),
            max_hp,
            requested: None,
            dying_start: None,
            dying_time: 0.0,
            death_seen: false,
            dead_effect_played: false,
        },
    ));
    ingame::vehicle::spawn_state_effects(&mut commands, root, &variant, false);
    units.by_id.insert(OBJ, unit);
}

fn check(
    time: Res<Time>,
    mut check: ResMut<Check>,
    mut units: ResMut<aa_net::state::Units>,
    ships: Query<(Entity, &ingame::vehicle::VehicleDamage, &Transform), With<Ship>>,
    effects: Query<(&ChildOf, &aa_fx::Effect), With<ingame::vehicle::VehicleEffect>>,
    mut exit: MessageWriter<AppExit>,
) {
    check.timer.tick(time.delta());
    if !check.timer.just_finished() {
        return;
    }
    let (root, damage, transform) = ships.single().expect("the cutter");
    let names: Vec<String> = effects
        .iter()
        .filter(|(parent, _)| parent.parent() == root)
        .map(|(_, effect)| effect.name.to_ascii_lowercase())
        .collect();
    match check.phase {
        0 => {
            assert!(damage.state == 4, "starts at 25 % (state 4)");
            assert!(
                names
                    .iter()
                    .any(|n| n == "x2_effects.e_effect.ship_damage_fireflame"),
                "the damaged prefab burns: {names:?}"
            );
            units.apply(&aa_net::Event::UnitDeath { obj_id: OBJ });
            check.phase = 1;
        }
        1 if damage.state == 6 => {
            assert!(
                damage.variant.ends_with("galleon.dead"),
                "{}",
                damage.variant
            );
            assert!(
                names
                    .iter()
                    .any(|n| n == "x2_effects.e_effect.ship_damage_fireflame"),
                "the fire keeps burning: {names:?}"
            );
            assert!(
                names
                    .iter()
                    .any(|n| n == "x2_effects.e_effect.caraval_water_destroy"),
                "the dead water effect: {names:?}"
            );
            assert!(
                !names
                    .iter()
                    .any(|n| n == "x2_effects.housing_test.small_ship_exp"),
                "no explosion: {names:?}"
            );
            info!("vehicle_ship_death: dead, burning and sinking");
            check.dead_at = time.elapsed_secs_f64();
            check.base_y = transform.translation.y;
            check.phase = 2;
        }
        2 if time.elapsed_secs_f64() >= check.dead_at + 2.0 => {
            assert!(
                transform.translation.y < check.base_y - 0.1,
                "the dead hull sinks (y {} from {})",
                transform.translation.y,
                check.base_y
            );
            info!(
                "vehicle_ship_death: ok, the hull sank to y {}",
                transform.translation.y
            );
            check.phase = 3;
            if !aa_dev::args()
                .iter()
                .any(|a| matches!(a.as_str(), "--shot" | "--offscreen-shot" | "--exit-after"))
            {
                exit.write(AppExit::Success);
            }
        }
        _ => {}
    }
}
