//! Vehicle damage states (S15) on the real prefabs: a row of the rowboat's state looks and one live
//! Farm Wagon stepped through the HP thresholds and then through a death.
//!
//! The example builds the `smallboat.smallboat_a` prefab for 100 %, 74 %, 50 %, 25 % and 0 % of the
//! rowboat's attribute-6 maximum (template 15: 4796 formula base + 1420 `unit_modifiers` = 6216 HP), then
//! drives a Farm Wagon (model 1008, template 60: 7296 HP) through 74 -> 50 -> 25 -> 0 % and finally
//! `SCUnitDeath` through `vehicle::update_damage` (the packet path is in the unit tests). It asserts the
//! state and the prefab ref of every step (2 normal, 3 normal, 4 `tractor_body_damage75`, 6
//! `tractor_body_dead`, then the death: 5 `tractor_body_dying` for `models.dying_time` 3.3 s, then 6
//! `tractor_body_dead`) and exits non-zero on a mismatch. `--shot` keeps it open for the picture.
//!
//! `cargo run -p aa-client --example vehicle_damage -- --mute --offscreen-shot shots/aa-client/vehicle_damage.png --shot-after 8`

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

/// The rowboat's static row: `models` row (`ShipModel` 4) and slave template 15 (1420 HP).
const MODEL: u32 = 129;
const TEMPLATE: u32 = 15;
/// The live Farm Wagon: `models` row (`VehicleModel` 38), slave template 60 (2500 HP) and its dying window.
const LIVE_MODEL: u32 = 1008;
const LIVE_TEMPLATE: u32 = 60;
const LIVE_DYING_TIME: f32 = 3.3;
/// The live wagon's object id.
const LIVE: u32 = 900;
const GAP: f32 = 9.0;
/// HP percentages of the static row, from the state's own definition.
const ROW: [(i32, u8); 5] = [(100, 1), (74, 2), (50, 3), (25, 4), (0, 6)];
/// The live steps after the initial 100 %: (HP %, expected state, expected prefab suffix).
const STEPS: [(i32, u8, &str); 4] = [
    (74, 2, "vehicle_tractor.tractor_body"),
    (50, 3, "vehicle_tractor.tractor_body"),
    (25, 4, "vehicle_tractor.tractor_body_damage75"),
    (0, 6, "vehicle_tractor.tractor_body_dead"),
];

#[derive(Resource)]
struct Check {
    stage: usize,
    timer: Timer,
    pending: Option<(u8, &'static str)>,
    max_hp: i32,
    /// 0 before the death, 1 while dying (5) is expected, 2 while the dead (6) result is expected.
    death_phase: u8,
}

impl Default for Check {
    fn default() -> Self {
        Self {
            stage: 0,
            timer: Timer::from_seconds(0.6, TimerMode::Repeating),
            pending: None,
            max_hp: 1,
            death_phase: 0,
        }
    }
}

#[derive(Component)]
struct Live;

fn ship_unit(obj_id: u32, hp: i32) -> aa_net::model::Unit {
    aa_net::model::Unit {
        obj_id,
        name: format!("farm wagon {obj_id}"),
        kind: aa_net::model::UnitKind::Slave {
            id: LIVE_TEMPLATE,
            tl_id: obj_id as u16,
            template_id: LIVE_TEMPLATE,
            summoner_id: 1,
        },
        owner_name: String::new(),
        pos: aa_net::Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 50,
        model_id: LIVE_MODEL,
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
    let row_max = db
        .slave_max_hp(TEMPLATE, 50)
        .expect("rowboat template 15's MaxHealth");
    assert_eq!(row_max, 6_216, "4796 formula base + 1420 unit_modifiers");
    let max_hp = db
        .slave_max_hp(LIVE_TEMPLATE, 50)
        .expect("Farm Wagon template 60's MaxHealth");
    assert_eq!(max_hp, 7_296, "4796 formula base + 2500 unit_modifiers");
    assert_eq!(db.vehicle_dying_time(LIVE_MODEL), LIVE_DYING_TIME);
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
        .init_resource::<ingame::vehicle::VehicleSwaps>()
        .add_systems(Startup, (stage, scene).chain())
        .add_systems(
            Update,
            (ingame::vehicle::update_damage, ingame::vehicle::finish_damage_swaps, check_states).chain(),
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
                max_hp: check.max_hp,
                requested: None,
                dying_start: None,
                dying_time: 0.0,
                death_seen: false,
            });
        ingame::vehicle::spawn_state_effects(&mut commands, root, &variant, false);
        info!(
            "vehicle_damage: row {} % -> state {state} {variant}",
            percent
        );
    }
    // the live Farm Wagon starts healthy and is stepped down, then killed, by `check_states`
    let live_look = db.vehicle_look(LIVE_MODEL).expect("the Farm Wagon");
    let live_variants = db
        .vehicle_variants(LIVE_MODEL)
        .expect("the Farm Wagon's state rows");
    let unit = ship_unit(LIVE, check.max_hp);
    let variant = live_variants.for_state(1).to_string();
    let pieces = ingame::vehicle::prepare(
        &loader,
        &ingame::data::VehicleLook {
            normal: variant.clone(),
            wheels: live_look.wheels,
        },
    );
    let transform = Transform::from_xyz(ROW.len() as f32 * GAP, 0.0, 0.0).with_rotation(rotation);
    let root = ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
    commands.entity(root).insert((
        Live,
        ingame::units::RemoteUnit::announced(&unit),
        ingame::vehicle::VehicleDamage {
            state: 1,
            variant,
            max_hp: check.max_hp,
            requested: None,
            dying_start: None,
            dying_time: LIVE_DYING_TIME,
            death_seen: false,
        },
    ));
    ingame::vehicle::spawn_state_effects(&mut commands, root, live_variants.for_state(1), false);
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
        let damage = live.single().expect("the live wagon");
        assert_eq!(damage.state, state, "state after step {}", check.stage);
        assert!(
            damage.variant.ends_with(variant),
            "{} does not end with {variant}",
            damage.variant
        );
        info!("vehicle_damage: verified state {state} {}", damage.variant);
        match check.death_phase {
            2 => {
                info!("vehicle_damage: ok");
                if !aa_dev::args()
                    .iter()
                    .any(|a| matches!(a.as_str(), "--shot" | "--offscreen-shot" | "--exit-after"))
                {
                    exit.write(AppExit::Success);
                }
                return;
            }
            1 => {
                // the death entered dying (5): wait out `models.dying_time` before the dead prefab
                check.death_phase = 2;
                check.pending = Some((6, "tractor_body_dead"));
                check
                    .timer
                    .set_duration(std::time::Duration::from_secs_f32(LIVE_DYING_TIME + 0.5));
                return;
            }
            _ => {}
        }
    }
    if check.stage < STEPS.len() {
        let (percent, state, suffix) = STEPS[check.stage];
        let Some(unit) = units.by_id.get_mut(&LIVE) else {
            return;
        };
        unit.hp = check.max_hp * percent / 100;
        check.pending = Some((state, suffix));
        check.stage += 1;
        let variants = db
            .vehicle_variants(LIVE_MODEL)
            .expect("the Farm Wagon's state rows");
        assert!(
            variants.for_state(state).ends_with(suffix),
            "state {state} prefab"
        );
        return;
    }
    if check.death_phase == 0 {
        // `SCUnitDeath`: the wagon was already drawn dead from its 0 hp; the death still enters dying
        units.apply(&aa_net::Event::UnitDeath { obj_id: LIVE });
        check.death_phase = 1;
        check.pending = Some((5, "tractor_body_dying"));
        check.timer.set_duration(std::time::Duration::from_secs_f32(0.6));
    }
}
