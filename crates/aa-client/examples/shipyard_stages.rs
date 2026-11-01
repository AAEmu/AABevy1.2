//! A shipyard's construction stages, through the game's own systems; no server connection.
//!
//! `SCUnitState` announces the drydock (template 5, the real `shipyards` row) and `SCShipyardState` 0x1aa moves it
//! through its stages. The example checks the rule the 1.2 client implements: the unit's model follows the packet's
//! `step` (`FUN_394d5140`), `step == steps.len()` is the finished hull (`main_model_id`) and step 1000 the launch
//! ceremony (`ceremony_model_id`), and `GetShipyardBaseProgress` / `GetShipyardRemainProgress` are the template's
//! total `num_actions` and what the packet's `actions` leaves.
//!
//! Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example shipyard_stages -- --mute --offscreen-shot shots/aa-client/shipyard_stages.png --shot-after 18`
//! takes the picture at 18 s; the checks run at 16 s. Without a shot the run ends after the checks.
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
use bevy::prelude::*;
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

use aa_net::model::{Equipment, ModelParams, Unit, UnitKind};
use aa_net::state::Units;
use aa_net::{Event, Pos, ShipyardEvent, ShipyardState, Shipyards};
use ingame::shipyard::{DrawnShipyards, ShipyardModel};

/// The real Iznan small sailboat drydock (`shipyards` id 5): seven stages, main hull 680, ceremony 683.
const TEMPLATE: u32 = 5;
const YARD_ID: u64 = 0x0000_0000_0000_0005;
const OBJ: u32 = 4_001;
/// `shipyard_steps.num_actions` 2+3+1+2+2+3+2.
const ALL_ACTIONS: i32 = 15;
const MAIN_MODEL: u32 = 680;
const CEREMONY_MODEL: u32 = 683;

/// The stage swaps: (time, step, actions, model, path suffix). Step 7 is `steps.len()` (the appended main
/// model); step 1000 is the ceremony model.
const SWAPS: [(f32, i32, i32, u32, &str); 5] = [
    (0.0, 0, 0, 673, "galleon.step_01"),
    (4.0, 1, 2, 674, "galleon.step_02"),
    (7.0, 3, 5, 676, "galleon.step_04"),
    (10.0, 7, 15, MAIN_MODEL, "galleon.step_08"),
    (12.0, 1000, 15, CEREMONY_MODEL, "galleon.step_09ceremony"),
];
/// What each sample should show: (sample time, swap index).
const SAMPLES: [(f32, usize); 5] = [(3.0, 0), (6.0, 1), (9.0, 2), (11.0, 3), (14.0, 4)];
const CHECK_AT: f32 = 16.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!(
            "shipyard_stages: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "shipyard_stages: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("shipyard_stages: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    let Some(template) = db.shipyard_template(TEMPLATE) else {
        eprintln!("shipyard_stages: skipped, the database has no shipyard {TEMPLATE}");
        return AppExit::Success;
    };
    if (template.main_model_id, template.ceremony_model_id) != (MAIN_MODEL, CEREMONY_MODEL)
        || template.steps.len() != 7
    {
        eprintln!(
            "shipyard_stages: template {TEMPLATE} is main {} / ceremony {} with {} steps, expected {MAIN_MODEL} / {CEREMONY_MODEL} / 7",
            template.main_model_id,
            template.ceremony_model_id,
            template.steps.len()
        );
        return AppExit::error();
    }
    println!(
        "shipyard-stages: template {TEMPLATE}: {} steps, main {MAIN_MODEL}, ceremony {CEREMONY_MODEL} ({:?})",
        template.steps.len(),
        template.ceremony_anim_key
    );
    if shot_mode() && shot_after() <= CHECK_AT {
        eprintln!(
            "shipyard_stages: --shot-after {} is not after the check at {CHECK_AT} s; raise it",
            shot_after()
        );
        return AppExit::error();
    }
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: shipyard stages"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .init_resource::<Units>()
        .init_resource::<Shipyards>()
        .init_resource::<Inbox>()
        .init_resource::<Swapped>()
        .init_resource::<Observed>()
        .add_plugins(ingame::shipyard::plugin)
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .add_systems(Startup, (stage, viewer, yard))
        // where `NetPlugin` runs `receive_events`: before everything that reads the state
        .add_systems(PreUpdate, (drive, deliver).chain())
        .add_systems(
            Update,
            (observe.after(ingame::shipyard::sync_shipyards), report).chain(),
        )
        .run()
}

/// What a live session would hand over in this frame.
#[derive(Resource, Default)]
struct Inbox(Vec<Event>);

/// Stands in for `aa_net`'s `receive_events`: every event is folded into the state.
fn deliver(mut inbox: ResMut<Inbox>, mut units: ResMut<Units>, mut yards: ResMut<Shipyards>) {
    for event in inbox.0.drain(..) {
        units.apply(&event);
        yards.apply(&event);
    }
}

fn state(step: i32, actions: i32) -> ShipyardState {
    ShipyardState {
        id: YARD_ID,
        template_id: TEMPLATE,
        z: 0.0,
        z_rot: 0.0,
        money_amount: 0,
        actions,
        kind: 0,
        kind2: 47,
        owner_name: "Tester".into(),
        kind3: 0,
        spawned: 1_700_000_000,
        obj_id: OBJ,
        hp: 30_000,
        step,
        ..Default::default()
    }
}

/// The drydock's `SCUnitState` and its first `SCShipyardState`.
fn yard(mut inbox: ResMut<Inbox>) {
    let unit = Unit {
        obj_id: OBJ,
        name: "Drydock".into(),
        kind: UnitKind::Shipyard {
            id: YARD_ID,
            template_id: TEMPLATE,
        },
        owner_name: "Tester".into(),
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id: 673,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 300,
        mp: 0,
        max_hp: 300,
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
    };
    inbox.0.push(Event::UnitSpawned(Box::new(unit)));
    inbox
        .0
        .push(Event::Shipyard(ShipyardEvent::State(state(0, 0))));
}

#[derive(Resource, Default)]
struct Swapped {
    next: usize,
}

/// Delivers the stage swaps at their times.
fn drive(time: Res<Time>, mut swapped: ResMut<Swapped>, mut inbox: ResMut<Inbox>) {
    let now = time.elapsed_secs();
    while swapped.next < SWAPS.len() && now >= SWAPS[swapped.next].0 {
        let (at, step, actions, _, path) = SWAPS[swapped.next];
        swapped.next += 1;
        if at == 0.0 {
            continue;
        }
        inbox
            .0
            .push(Event::Shipyard(ShipyardEvent::State(state(step, actions))));
        info!("shipyard-stages: SCShipyardState step {step}, actions {actions} at {at} s ({path})");
    }
}

#[derive(Resource, Default)]
struct Observed {
    first: Option<(f32, u32, String)>,
    samples: [Option<(u32, String, i32, i32)>; 5],
    blank_at: Option<f32>,
    seen: bool,
}

/// Samples the drawn model and the progress every frame.
fn observe(
    time: Res<Time>,
    models: Query<&ShipyardModel>,
    shipyards: Res<Shipyards>,
    db: Res<ingame::data::Db>,
    mut observed: ResMut<Observed>,
) {
    let now = time.elapsed_secs();
    let Some(model) = models.iter().find(|m| m.obj_id == OBJ) else {
        if observed.seen && observed.blank_at.is_none() {
            observed.blank_at = Some(now);
            error!("shipyard-stages: the drydock has no model at {now:.2} s (blank frame)");
        }
        return;
    };
    observed.seen = true;
    if observed.first.is_none() {
        observed.first = Some((now, model.model_id, model.path.clone()));
        info!(
            "shipyard-stages: first drawn at {now:.2} s: model {} ({})",
            model.model_id, model.path
        );
    }
    let progress = shipyards.of(YARD_ID).and_then(|state| {
        let template = db.shipyard_template(TEMPLATE)?;
        Some(ingame::shipyard::progress(&template, state))
    });
    for (sample_at, index) in SAMPLES {
        if now >= sample_at && observed.samples[index].is_none() {
            let (base, remain) = progress.unwrap_or((0, 0));
            observed.samples[index] = Some((model.model_id, model.path.clone(), base, remain));
        }
    }
}

fn report(
    time: Res<Time>,
    observed: Res<Observed>,
    drawn: Res<DrawnShipyards>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let now = time.elapsed_secs();
    if *done || now < CHECK_AT {
        return;
    }
    *done = true;
    let mut failures = Vec::new();
    let Some((first_at, first_model, first_path)) = observed.first.clone() else {
        error!("shipyard-stages: the drydock was never drawn");
        exit.write(AppExit::error());
        return;
    };
    info!(
        "shipyard-stages: at {now:.2} s: first {first_model} ({first_path}) at {first_at:.2} s, samples {:?}",
        observed.samples
    );
    if drawn.entity(OBJ).is_none() {
        failures.push("the drydock has no drawn entity".into());
    }
    if (
        first_model,
        first_path.to_ascii_lowercase().ends_with(SWAPS[0].4),
    ) != (SWAPS[0].3, true)
    {
        failures.push(format!(
            "the first model is {first_model} ({first_path}), not stage 0"
        ));
    }
    for (sample_at, index) in SAMPLES {
        let (_, step, actions, model, suffix) = SWAPS[index];
        let Some((drawn_model, path, base, remain)) = observed.samples[index].clone() else {
            failures.push(format!("no frame was sampled at {sample_at} s"));
            continue;
        };
        if drawn_model != model || !path.to_ascii_lowercase().ends_with(suffix) {
            failures.push(format!(
                "at {sample_at} s the drydock is {drawn_model} ({path}), expected {model} ({suffix})"
            ));
        }
        let expected_remain = (ALL_ACTIONS - actions).max(0);
        if (base, remain) != (ALL_ACTIONS, expected_remain) {
            failures.push(format!(
                "at {sample_at} s (step {step}) progress is {base}/{remain}, expected {ALL_ACTIONS}/{expected_remain}"
            ));
        }
    }
    if let Some(blank) = observed.blank_at {
        failures.push(format!("a frame at {blank:.2} s had no model"));
    }
    if failures.is_empty() {
        info!("shipyard-stages: PASSED");
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
    } else {
        error!("shipyard-stages: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
}

/// Where `sync_shipyards` thinks the player is.
fn viewer(mut commands: Commands) {
    commands.spawn((
        Name::new("player viewer"),
        ingame::player::LocalPlayer { obj_id: 0 },
        ingame::player::Mover {
            pos: Vec3::ZERO,
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
    ));
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(30.0, 20.0, 35.0).looking_at(Vec3::new(0.0, 4.0, 0.0), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(60.0, 60.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.24, 0.35, 0.17))),
    ));
}

/// When the run is unattended (`--shot` / `--offscreen-shot`).
fn shot_mode() -> bool {
    aa_dev::args()
        .iter()
        .any(|a| a == "--shot" || a == "--offscreen-shot")
}

fn shot_after() -> f32 {
    let args = aa_dev::args();
    args.iter()
        .position(|a| a == "--shot-after")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(20.0)
}
