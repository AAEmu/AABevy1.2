//! A planted sapling's growth and its phase model change, through the game's own systems; no server connection.
//!
//! Fig Tree 380's start phase 1779 has `DoodadFuncGrowth` 0.8 -> 1.0; phase 1780 swaps its model to
//! `temp_tree_fig02no.cgf` with growth 0.65 -> 0.8 (the database rows). The example announces the tree with a short
//! 8,000 ms `time_left` (`SCDoodadCreated`), lets it grow, then sends the phase change (`SCDoodadPhaseChanged`) and
//! keeps drawing. It checks the rule the original client's `ClientDoodad::Update` (`0x393a4810`) implements: the
//! packet's `time_left` is the whole duration, `start_scale` at the announcement, linear growth to `end_scale`, and the
//! next phase starts again at its own `start_scale`. A phase change must never leave a frame without the doodad's model:
//! every drawn frame is sampled, and the new model is spawned before the old one is despawned.
//!
//! F18 (the original client's scale at spawn) was not sampled in the 2026-10-30 capture; the rule here is the decompile
//! plus `doodad_func_growths`, and this example is the client's own regression check.
//!
//! Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example farm_growth -- --mute --offscreen-shot shots/aa-client/farm_growth.png --shot-after 10`
//! takes the picture at 10 s; the checks run at 8 s. Without a shot the run ends after the checks.
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

use aa_net::state::Doodads;
use aa_net::{Doodad, Event, NetEvent, Pos};
use ingame::doodad_visual::DoodadVisual;
use ingame::units::{RemoteDoodad, Views};

/// Fig Tree (doodad 380): the sapling phase, the phase it changes into, and its object id.
const FIG_TEMPLATE: u32 = 380;
const FIG_PHASE: u32 = 1779;
const FIG_NEXT_PHASE: u32 = 1780;
const FIG_OBJ: u32 = 4_001;
/// The growth announced for both phases, milliseconds (`doodad_func_growths` has 514,286 / 2,057,143 ms; the packet's
/// `time_left` is the whole duration, as the original client reads it).
const TIME_LEFT_MS: u32 = 8_000;
/// At this second the phase change is delivered.
const SWAP_AT: f32 = 4.0;
/// The checks run here: 4 s after the phase change, halfway through its growth.
const CHECK_AT: f32 = 8.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("farm_growth: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "farm_growth: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("farm_growth: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    if shot_mode() && shot_after() <= CHECK_AT {
        eprintln!(
            "farm_growth: --shot-after {} is not after the check at {CHECK_AT} s; raise it",
            shot_after()
        );
        return AppExit::error();
    }
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("aa-client: farm growth"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .init_resource::<Views>()
        .init_resource::<Doodads>()
        .init_resource::<aa_net::state::Units>()
        .init_resource::<Inbox>()
        .init_resource::<Swapped>()
        .init_resource::<Observed>()
        .add_message::<NetEvent>()
        .add_plugins(ingame::doodad_visual::plugin)
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .add_systems(Startup, (stage, viewer, fig))
        // where `NetPlugin` runs `receive_events`: before everything that reads the events
        .add_systems(PreUpdate, (drive, deliver).chain())
        .add_systems(
            Update,
            (
                ingame::units::receive_moves,
                ingame::units::sync_views,
                // `phase_changes` sets the new phase's model and scale; the sampler must see that frame
                observe.after(ingame::doodad_visual::phase_changes),
                report,
            )
                .chain(),
        )
        .run()
}

/// What a live session would hand over in this frame.
#[derive(Resource, Default)]
struct Inbox(Vec<Event>);

/// Stands in for `aa_net`'s `receive_events`: every event is folded into the state, then written as a `NetEvent`.
fn deliver(
    mut inbox: ResMut<Inbox>,
    mut doodads: ResMut<Doodads>,
    mut out: MessageWriter<NetEvent>,
) {
    for event in inbox.0.drain(..) {
        doodads.apply(&event);
        out.write(NetEvent(event));
    }
}

/// The sapling at the origin, announced with the short growth time.
fn fig(mut inbox: ResMut<Inbox>) {
    inbox.0.push(Event::DoodadsSpawned(vec![Doodad {
        obj_id: FIG_OBJ,
        template_id: FIG_TEMPLATE,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: Pos::default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: FIG_PHASE,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: TIME_LEFT_MS,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    }]));
}

/// The phase change, delivered at [`SWAP_AT`].
#[derive(Resource, Default)]
struct Swapped {
    done: bool,
    at: Option<f32>,
}

fn drive(time: Res<Time>, mut swapped: ResMut<Swapped>, mut inbox: ResMut<Inbox>) {
    if !swapped.done && time.elapsed_secs() >= SWAP_AT {
        swapped.done = true;
        swapped.at = Some(time.elapsed_secs());
        inbox.0.push(Event::DoodadPhaseChanged {
            obj_id: FIG_OBJ,
            func_group_id: FIG_NEXT_PHASE,
            time_left: TIME_LEFT_MS,
            puzzle_group: -1,
            item_template_id: 0,
        });
        info!("farm-growth: SCDoodadPhaseChanged {FIG_NEXT_PHASE} with {TIME_LEFT_MS} ms left");
    }
}

/// Where the camera and `sync_views` think the player is: the growth itself needs no character model, but
/// `units::sync_views` only draws doodads around a `LocalPlayer` with a `Mover`.
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
        Transform::from_xyz(11.0, 7.0, 12.0).looking_at(Vec3::new(0.0, 3.0, 0.0), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.24, 0.35, 0.17))),
    ));
}

/// What the frame-by-frame sampler saw.
#[derive(Resource, Default)]
struct Observed {
    announced_at: Option<f32>,
    first_drawn: Option<(f32, f32, String)>,
    /// A frame after the first drawing without a model (the `SCDoodadPhaseChanged` swap must not leave one).
    blank_at: Option<f32>,
    seen_drawn: bool,
    /// The last scale sampled before the phase change, with its time.
    before_swap: Option<(f32, f32)>,
    /// The last scale and time sampled after the phase change.
    after_swap: Option<(f32, f32)>,
    path_after_swap: Option<String>,
    rendered_after_swap: Option<u32>,
}

/// Samples every frame: the model is there, the scale follows the announced time, and the phase's model swapped.
#[allow(clippy::too_many_arguments)]
fn observe(
    time: Res<Time>,
    roots: Query<(&RemoteDoodad, &DoodadVisual, &Transform)>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    entities: Query<()>,
    mut observed: ResMut<Observed>,
    swapped: Res<Swapped>,
) {
    let now = time.elapsed_secs();
    if observed.announced_at.is_none() {
        observed.announced_at = Some(now);
    }
    let Some((_, visual, transform)) = roots.iter().find(|(remote, _, _)| remote.obj_id == FIG_OBJ)
    else {
        return;
    };
    let drawn = visual.model.is_some_and(|model| {
        entities.contains(model)
            && children
                .iter_descendants(model)
                .any(|entity| meshes.contains(entity))
    });
    if drawn && observed.first_drawn.is_none() {
        observed.first_drawn = Some((now, transform.scale.x, visual.path.clone()));
        observed.seen_drawn = true;
        info!(
            "farm-growth: first drawn at {now:.2} s: scale {:.3}, model {}, phase {}",
            transform.scale.x, visual.path, visual.func_group_id
        );
    }
    if observed.seen_drawn && !drawn && observed.blank_at.is_none() {
        observed.blank_at = Some(now);
        error!("farm-growth: the doodad has no model at {now:.2} s (blank frame)");
    }
    match swapped.at {
        Some(at) if now >= at => {
            observed.after_swap = Some((now, transform.scale.x));
            observed.path_after_swap = Some(visual.path.clone());
            observed.rendered_after_swap = Some(visual.rendered_func_group_id);
        }
        _ => observed.before_swap = Some((now, transform.scale.x)),
    }
}

/// At [`CHECK_AT`]: compare the start scale, the halfway scale and the phase's model with the database's growth.
#[allow(clippy::too_many_arguments)]
fn report(
    time: Res<Time>,
    observed: Res<Observed>,
    swapped: Res<Swapped>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let now = time.elapsed_secs();
    if *done || now < CHECK_AT {
        return;
    }
    *done = true;
    let mut failures = Vec::new();
    let Some((first_at, first_scale, first_path)) = observed.first_drawn.clone() else {
        error!("farm-growth: the sapling was never drawn");
        exit.write(AppExit::error());
        return;
    };
    let announced = observed.announced_at.unwrap_or(0.0);
    let expected_first = 0.8 + 0.2 * ((first_at - announced) / (TIME_LEFT_MS as f32 / 1000.0));
    info!(
        "farm-growth: at {now:.2} s: first {first_scale:.3} at {first_at:.2} s ({first_path}), before swap \
         {:?}, after swap {:?} phase {:?}",
        observed.before_swap, observed.after_swap, observed.rendered_after_swap
    );
    if !first_path.ends_with("temp_tree_fig02no_grow.cgf") {
        failures.push(format!("the first model is {first_path}, not phase 1779's"));
    }
    if (first_scale - expected_first).abs() > 0.02 {
        failures.push(format!(
            "the first scale is {first_scale:.3}, expected {expected_first:.3} (start 0.8 at the announcement)"
        ));
    }
    if let (Some(_), Some((at, scale))) = (swapped.at, observed.before_swap) {
        let expected = 0.8 + 0.2 * ((at - announced) / (TIME_LEFT_MS as f32 / 1000.0));
        if (scale - expected).abs() > 0.03 {
            failures.push(format!(
                "the scale before the phase change is {scale:.3} at {at:.2} s, expected {expected:.3}"
            ));
        }
    } else {
        failures.push("the phase change was not delivered".into());
    }
    if let Some(blank) = observed.blank_at {
        failures.push(format!("a frame at {blank:.2} s had no model"));
    }
    let Some(swap_at) = swapped.at else {
        failures.push("the phase change was not delivered".into());
        return finish(failures, &mut exit);
    };
    let Some(path) = observed.path_after_swap.as_deref() else {
        failures.push("the phase's model was not sampled".into());
        return finish(failures, &mut exit);
    };
    if !path.ends_with("temp_tree_fig02no.cgf") {
        failures.push(format!(
            "the model after the phase change is {path}, not phase 1780's"
        ));
    }
    if observed.rendered_after_swap != Some(FIG_NEXT_PHASE) {
        failures.push(format!(
            "the rendered phase is {:?}, not {FIG_NEXT_PHASE}",
            observed.rendered_after_swap
        ));
    }
    if let Some((at, scale)) = observed.after_swap {
        let expected = 0.65 + 0.15 * ((at - swap_at) / (TIME_LEFT_MS as f32 / 1000.0));
        if (scale - expected).abs() > 0.03 {
            failures.push(format!(
                "the scale after the phase change is {scale:.3} at {at:.2} s, expected {expected:.3}"
            ));
        }
    } else {
        failures.push("no frame was sampled after the phase change".into());
    }
    finish(failures, &mut exit)
}

fn finish(failures: Vec<String>, exit: &mut MessageWriter<AppExit>) {
    if failures.is_empty() {
        info!("farm-growth: PASSED");
        if !shot_mode() {
            exit.write(AppExit::Success);
        }
    } else {
        error!("farm-growth: FAILED: {}", failures.join("; "));
        exit.write(AppExit::error());
    }
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
        .and_then(|t| t.parse().ok())
        .unwrap_or(5.0)
}
