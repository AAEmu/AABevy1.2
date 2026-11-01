//! Vehicle damage states (S15) on the real rowboat prefabs: a row of the five state looks and one
//! live boat stepped through them by `FUN_3989f8b0`'s thresholds.
//!
//! The example builds the `smallboat.smallboat_a` prefab for 100 %, 74 %, 50 %, 25 % and 0 % of the
//! template's `unit_modifiers` maximum (template 15, 1420 HP), then drives a sixth boat through
//! 74 -> 50 -> 25 -> 0 % through `vehicle::update_damage` (the `SCUnitPoints` path is in the unit
//! tests). It asserts the state and the prefab ref of every step (2 `damege50`, 3 `damege50`,
//! 4 `damege75`, 6 `dead`) and exits non-zero on a mismatch. `--shot` keeps it open for the picture.
//!
//! `cargo run -p aa-client --example vehicle_damage -- --mute --offscreen-shot shots/aa-client/vehicle_damage.png --shot-after 5`

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

/// The rowboat's `models` row (`ShipModel` 4) and slave template 15 (1420 HP).
const MODEL: u32 = 129;
const TEMPLATE: u32 = 15;
/// The live boat's object id.
const LIVE: u32 = 900;
const GAP: f32 = 9.0;
/// HP percentages of the static row, from the state's own definition.
const ROW: [(i32, u8); 5] = [(100, 1), (74, 2), (50, 3), (25, 4), (0, 6)];
/// The live steps after the initial 100 %: (HP %, expected state, expected prefab suffix).
const STEPS: [(i32, u8, &str); 4] = [
    (74, 2, "smallboat.smallboat_damege50"),
    (50, 3, "smallboat.smallboat_damege50"),
    (25, 4, "smallboat.smallboat_damege75"),
    (0, 6, "smallboat.smallboat_dead"),
];

#[derive(Resource)]
struct Check {
    stage: usize,
    timer: Timer,
    pending: Option<(u8, &'static str)>,
    max_hp: i32,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            stage: 0,
            timer: Timer::from_seconds(0.6, TimerMode::Repeating),
            pending: None,
            max_hp: 1,
        }
    }
}

#[derive(Component)]
struct Live;

fn ship_unit(obj_id: u32, hp: i32) -> aa_net::model::Unit {
    aa_net::model::Unit {
        obj_id,
        name: format!("rowboat {obj_id}"),
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
    if !aa_cry::pak_available() {
        eprintln!(
            "vehicle_damage: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "vehicle_damage: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("vehicle_damage: skipped, the game database cannot be opened");
        return AppExit::Success;
    };
    let max_hp = db
        .slave_max_hp(TEMPLATE)
        .expect("rowboat template 15's MaxHealth");
    assert!(max_hp > 0);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Vehicle damage states"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .insert_resource(Check {
            max_hp,
            ..default()
        })
        .init_resource::<aa_net::state::Units>()
        .add_systems(Startup, (stage, scene).chain())
        .add_systems(
            Update,
            (ingame::vehicle::update_damage, check_states).chain(),
        )
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let width = (ROW.len() as f32 + 1.0) * GAP;
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(width * 0.5, width * 0.9, width * 1.1)
            .looking_at(Vec3::new(width * 0.5 - GAP * 0.5, 1.0, 0.0), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(width + 40.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
        Transform::from_xyz(width * 0.5, 0.0, 0.0),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<ingame::data::Db>,
    mut units: ResMut<aa_net::state::Units>,
    check: Res<Check>,
) {
    let look = db.vehicle_look(MODEL).expect("the rowboat");
    let variants = db
        .vehicle_variants(MODEL)
        .expect("the rowboat's state rows");
    let loader = cry.loader();
    let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    // the five state looks side by side
    for (i, (percent, state)) in ROW.into_iter().enumerate() {
        let variant = variants.for_state(state).to_string();
        let pieces = ingame::vehicle::prepare(
            &loader,
            &ingame::data::VehicleLook {
                normal: variant.clone(),
                wheels: look.wheels.clone(),
            },
        );
        assert!(!pieces.is_empty(), "rowboat {variant} has no parts");
        let transform = Transform::from_xyz(i as f32 * GAP, 0.0, 0.0).with_rotation(rotation);
        let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
        commands
            .entity(root)
            .insert(ingame::vehicle::VehicleDamage {
                state,
                variant: variant.clone(),
            });
        ingame::vehicle::spawn_state_effects(&mut commands, root, &variant, false);
        info!(
            "vehicle_damage: row {} % -> state {state} {variant}",
            percent
        );
    }
    // the live boat starts healthy and is stepped down by `check_states`
    let unit = ship_unit(LIVE, check.max_hp);
    let variant = variants.for_state(1).to_string();
    let pieces = ingame::vehicle::prepare(
        &loader,
        &ingame::data::VehicleLook {
            normal: variant.clone(),
            wheels: look.wheels,
        },
    );
    let transform = Transform::from_xyz(ROW.len() as f32 * GAP, 0.0, 0.0).with_rotation(rotation);
    let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
    commands.entity(root).insert((
        Live,
        ingame::units::RemoteUnit::announced(&unit),
        ingame::vehicle::VehicleDamage { state: 1, variant },
    ));
    ingame::vehicle::spawn_state_effects(&mut commands, root, variants.for_state(1), false);
    units.by_id.insert(LIVE, unit);
}

fn check_states(
    time: Res<Time>,
    db: Res<ingame::data::Db>,
    mut units: ResMut<aa_net::state::Units>,
    mut check: ResMut<Check>,
    live: Query<&ingame::vehicle::VehicleDamage, With<Live>>,
    mut exit: MessageWriter<AppExit>,
) {
    check.timer.tick(time.delta());
    if !check.timer.just_finished() {
        return;
    }
    if let Some((state, variant)) = check.pending.take() {
        let damage = live.single().expect("the live boat");
        assert_eq!(damage.state, state, "state after step {}", check.stage);
        assert!(
            damage.variant.ends_with(variant),
            "{} does not end with {variant}",
            damage.variant
        );
        info!("vehicle_damage: verified state {state} {}", damage.variant);
        if check.stage == STEPS.len() {
            info!("vehicle_damage: ok");
            if !aa_dev::args()
                .iter()
                .any(|a| matches!(a.as_str(), "--shot" | "--offscreen-shot" | "--exit-after"))
            {
                exit.write(AppExit::Success);
            }
            return;
        }
    }
    if check.stage >= STEPS.len() {
        return;
    }
    let (percent, state, suffix) = STEPS[check.stage];
    let Some(unit) = units.by_id.get_mut(&LIVE) else {
        return;
    };
    unit.hp = check.max_hp * percent / 100;
    check.pending = Some((state, suffix));
    check.stage += 1;
    let variants = db
        .vehicle_variants(MODEL)
        .expect("the rowboat's state rows");
    assert!(
        variants.for_state(state).ends_with(suffix),
        "state {state} prefab"
    );
}
