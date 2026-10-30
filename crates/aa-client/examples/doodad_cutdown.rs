//! A birch cut down, from the captured packet order; no server connection.
//!
//! The original client's 2026-10-30 capture (`tools/evidence/cutdown/capture-results.txt`) cut the world old birch
//! (doodad template 407, phase 944, object 2543) with skill 15874. Its log batch is played into the client here in
//! the captured order: `SCDoodadPhaseChanged` to 6251 (`time_left` 180000), `SCErrorMsg` 181, then
//! `SCVegetationCutdowning` from the cutter (unit 25178), the local player standing beside the tree. The tree
//! arrives the same way (`Event::DoodadsSpawned`), so `units::sync_views` builds it from the database and the pak as
//! in the game.
//!
//! The events take the route `aa_net`'s `receive_events` gives every event of a live session: folded into the state
//! (`Doodads::apply`), then written as a `NetEvent` in `PreUpdate`. Everything after that is the client's own code
//! (`units::receive_moves`, `units::sync_views`, `doodad_visual`, `cutdown`). The wire decode before it is aa-net's;
//! its byte-exact test pins the 6-byte cutdown body. The server's `SCDoodadRemoved` (2367 ms after the cut in the
//! capture) is left out, so the landed tree is still there for the last picture.
//!
//! Three pictures: with the tree standing (0.0 s, taken before the packets), 1.2 s after them (falling) and 2.4 s
//! after them (landed). Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example doodad_cutdown -- --mute --offscreen-shot shots/aa-client/doodad_cutdown.png`
//! writes `doodad_cutdown-0.0s.png`, `doodad_cutdown-1.2s.png` and `doodad_cutdown-2.4s.png` (`--shot` does the same
//! from the window). `--shot-after S` is the least time before the scene is staged (default 5). With neither flag the
//! cut is played once and the window stays open.
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

use std::path::PathBuf;

use aa_net::state::Doodads;
use aa_net::{Doodad, Event, NetEvent, Pos};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use ingame::cutdown::{Fallen, Falling};
use ingame::doodad_visual::DoodadVisual;
use ingame::player::{LocalPlayer, Mover};
use ingame::units::{RemoteDoodad, Views};

/// The old birch of the capture: its template and phase, the doodad and the unit that cut it.
const BIRCH_TEMPLATE: u32 = 407;
const STANDING_PHASE: u32 = 944;
const BIRCH_OBJ: u32 = 2543;
const CUTTER_OBJ: u32 = 25178;
/// The phase the cut gave it (no model of its own: the template's, the same tree).
const CUT_PHASE: u32 = 6251;
/// `BAG_INVALID_ITEM`, the `SCErrorMsg` between the phase change and the cut in the captured batch.
const ERROR_MSG: i16 = 181;
/// Seconds after the packets at which a picture is taken. The first is the one before them (the tree still stands).
const PICTURES: [f32; 3] = [0.0, 1.2, 2.4];
/// How far west of the tree the cutter stands (m). The tree falls away from the cutter: east, to the right of the
/// picture.
const CUTTER_DISTANCE: f32 = 2.5;
/// Seconds the tree and the pipelines must have been in order before the first picture.
const SETTLE: f32 = 1.5;
/// Seconds the scene may take to be staged, and the pictures to be written, before the run gives up.
const STAGING_TIMEOUT: f32 = 180.0;
const SAVING_TIMEOUT: f32 = 30.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!(
            "doodad_cutdown: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "doodad_cutdown: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = ingame::data::Db::open() else {
        eprintln!("doodad_cutdown: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    let args = aa_dev::DevArgs::from_env();
    let sequence = Sequence::new(&args);
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("Doodad cutdown"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        // `Sequence` takes the pictures and ends the run: DevPlugin's one `--shot` would end it at the first picture
        .insert_resource(aa_dev::DevArgs {
            shot: None,
            offscreen_shot: None,
            ..args
        })
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .insert_resource(sequence)
        .init_resource::<Inbox>()
        .init_resource::<Views>()
        .init_resource::<aa_net::state::Units>()
        .init_resource::<Doodads>()
        .add_message::<NetEvent>()
        .add_plugins(ingame::doodad_visual::plugin)
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .add_systems(Startup, (stage, cutter, birch))
        // where `NetPlugin` runs `receive_events`: before everything that reads the events
        .add_systems(PreUpdate, (drive, deliver).chain())
        .add_systems(
            Update,
            (ingame::units::receive_moves, ingame::units::sync_views).chain(),
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

/// The captured batch, in its order.
fn cut_batch() -> Vec<Event> {
    vec![
        Event::DoodadPhaseChanged {
            obj_id: BIRCH_OBJ,
            func_group_id: CUT_PHASE,
            time_left: 180_000,
            puzzle_group: -1,
            item_template_id: 0,
        },
        Event::ServerError { code: ERROR_MSG },
        Event::VegetationCutdown {
            unit_obj_id: CUTTER_OBJ,
            doodad_obj_id: BIRCH_OBJ,
        },
    ]
}

/// The standing birch at the origin.
fn birch(mut inbox: ResMut<Inbox>) {
    inbox.0.push(Event::DoodadsSpawned(vec![Doodad {
        obj_id: BIRCH_OBJ,
        template_id: BIRCH_TEMPLATE,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: Pos::default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: STANDING_PHASE,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    }]));
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The old birch is about 40 m tall (the bounds of its meshes) and falls about as far: seen from the south, level
    // with its middle and far enough back to hold it standing and lying, it falls to the right.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(18.0, 19.0, 66.0).looking_at(Vec3::new(18.0, 19.0, 0.0), Vec3::Y),
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
        Mesh3d(meshes.add(Plane3d::default().mesh().size(2000.0, 2000.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.24, 0.35, 0.17))),
    ));
}

/// The cutter: the local player (who is not in `Units`), facing the tree. `cutdown::start` takes the fall direction
/// from its `Mover`.
fn cutter(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
) {
    use aa_anim::CharacterDesc;
    use aa_anim::db::{GameDb, Gender, Race};
    let db = GameDb::open().expect("game database");
    let assets = CharacterDesc::from_preset(&db, Race::Nuian, Gender::Male, 0)
        .and_then(|desc| desc.resolve(&db))
        .expect("the cutter's character");
    let cal = aa_anim::Cal::shared(&assets.cal).expect("character animation list");
    let rig = aa_anim::pose::shared_rig(&assets.skeleton).expect("character skeleton");
    let entry = cal.animation("fist_ba_relaxed_idle").expect("idle clip");
    let idle = clips
        .bound(
            &rig,
            &entry.path,
            aa_anim::pose::SampleOptions {
                position_scale: entry.position_scale(cal.base_height),
                lock_root_motion: true,
            },
        )
        .expect("bound idle clip");
    let pos = Vec3::new(-CUTTER_DISTANCE, 0.0, 0.0);
    // game yaw: 0 is north, counter-clockwise; east is a quarter turn clockwise
    let facing = -std::f32::consts::FRAC_PI_2;
    let actor = ingame::player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        assets,
        ingame::player::placement(pos, facing, 1.0),
        Vec::new(),
    )
    .expect("the cutter's character");
    commands.entity(actor).insert((
        LocalPlayer { obj_id: CUTTER_OBJ },
        Mover {
            pos,
            facing,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
    ));
    commands.queue(move |world: &mut World| {
        let mut animator = world.get_mut::<aa_anim::Animator>(actor).unwrap();
        animator.play(idle, true, 0.0);
        animator.random_idle = None;
    });
}

/// Where the run is.
#[derive(Clone, Copy)]
enum Stage {
    /// The tree is not drawn or the pipelines are compiling: since when everything has been in order.
    Staging { ready_since: Option<f32> },
    /// The picture before the cut is asked for; the packets follow in the next frame.
    Before,
    /// The packets went in during the frame that began at `cut`, where the fall's own clock starts too; the pictures
    /// from `next` on are still to take.
    Cut { cut: f32, next: usize },
    /// Every picture is asked for, at `since`; the run ends when they are all written.
    Saving { since: f32 },
    /// A run without pictures has played the cut and stays open.
    Over,
}

#[derive(Resource)]
struct Sequence {
    /// `--offscreen-shot` / `--shot`: the pictures are named after it. None: no pictures.
    base: Option<PathBuf>,
    /// `--shot-after`: the clock before which the scene is not staged.
    after: f32,
    stage: Stage,
    /// Pictures written so far.
    saved: usize,
}

impl Sequence {
    fn new(args: &aa_dev::DevArgs) -> Sequence {
        Sequence {
            base: args
                .offscreen_shot
                .as_ref()
                .or(args.shot.as_ref())
                .map(PathBuf::from),
            after: args.shot_after,
            stage: Stage::Staging { ready_since: None },
            saved: 0,
        }
    }

    /// `shots/x/name.png` -> `shots/x/name-1.2s.png`.
    fn picture(&self, index: usize) -> Option<PathBuf> {
        let base = self.base.as_deref()?;
        let stem = base.file_stem()?.to_string_lossy();
        let extension = base
            .extension()
            .map_or("png".into(), |e| e.to_string_lossy());
        Some(base.with_file_name(format!("{stem}-{:.1}s.{extension}", PICTURES[index])))
    }
}

/// A picture of what the window (or the `--offscreen-shot` image) shows now, saved when the GPU hands it back.
fn shoot(commands: &mut Commands, target: Option<&aa_dev::OffscreenTarget>, path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let shot = target.map_or_else(Screenshot::primary_window, |t| {
        Screenshot::image(t.0.clone())
    });
    let mut save = save_to_disk(path);
    commands.spawn(shot).observe(
        move |captured: On<ScreenshotCaptured>, mut sequence: ResMut<Sequence>| {
            save(captured);
            sequence.saved += 1;
        },
    );
}

/// What the birch looks like now.
struct Look {
    /// It has meshes under it.
    drawn: bool,
    falling: bool,
    fallen: bool,
    /// How far it has turned from upright, in degrees.
    tilt: f32,
    phase: u32,
    /// The phase whose model is drawn: the cut phase is drawn only after the tree has landed.
    rendered: u32,
}

impl Look {
    fn describe(&self) -> String {
        format!(
            "{} (tilt {:.1} deg, phase {}, model of phase {})",
            match (self.falling, self.fallen) {
                (true, _) => "falling",
                (_, true) => "lying",
                _ => "standing",
            },
            self.tilt,
            self.phase,
            self.rendered
        )
    }
}

/// Runs the sequence: stages the scene, takes the pictures and delivers the captured batch.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    time: Res<Time>,
    status: Res<aa_world::pipelines::PipelineStatus>,
    target: Option<Res<aa_dev::OffscreenTarget>>,
    mut sequence: ResMut<Sequence>,
    mut inbox: ResMut<Inbox>,
    trees: Query<(
        &RemoteDoodad,
        &Transform,
        &DoodadVisual,
        Has<Falling>,
        Has<Fallen>,
    )>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let target = target.as_deref();
    let look = trees.iter().find(|tree| tree.0.obj_id == BIRCH_OBJ).map(
        |(_, transform, visual, falling, fallen)| Look {
            drawn: visual.model.is_some_and(|model| {
                children
                    .iter_descendants(model)
                    .any(|entity| meshes.contains(entity))
            }),
            falling,
            fallen,
            tilt: (transform.rotation * Vec3::Y)
                .angle_between(Vec3::Y)
                .to_degrees(),
            phase: visual.func_group_id,
            rendered: visual.rendered_func_group_id,
        },
    );
    let state = || look.as_ref().map_or("not drawn".into(), Look::describe);
    match sequence.stage {
        Stage::Staging { ready_since } => {
            let ready = look.as_ref().is_some_and(|look| look.drawn)
                && status.waiting() == 0
                && now >= sequence.after;
            let since = ready.then(|| ready_since.unwrap_or(now));
            if since.is_some_and(|since| now - since >= SETTLE) {
                info!("doodad_cutdown: staged at {now:.1} s: {}", state());
                if let Some(path) = sequence.picture(0) {
                    shoot(&mut commands, target, path);
                }
                sequence.stage = Stage::Before;
            } else if now > STAGING_TIMEOUT {
                error!(
                    "doodad_cutdown: the tree was not staged within {STAGING_TIMEOUT} s: {}",
                    state()
                );
                exit.write(AppExit::error());
                sequence.stage = Stage::Over;
            } else {
                sequence.stage = Stage::Staging { ready_since: since };
            }
        }
        Stage::Before => {
            inbox.0.extend(cut_batch());
            info!(
                "doodad_cutdown: delivered SCDoodadPhaseChanged {CUT_PHASE}, SCErrorMsg {ERROR_MSG} and \
                 SCVegetationCutdowning"
            );
            sequence.stage = if sequence.base.is_some() {
                // `cutdown::fall` adds this frame's delta to the fall's clock too
                Stage::Cut {
                    cut: now - time.delta_secs(),
                    next: 1,
                }
            } else {
                Stage::Over
            };
        }
        Stage::Cut { cut, next } => {
            if next == PICTURES.len() {
                sequence.stage = Stage::Saving { since: now };
            } else if now - cut >= PICTURES[next] {
                let path = sequence.picture(next).expect("a base name");
                info!(
                    "doodad_cutdown: {:.1} s after the packets: {} -> {}",
                    PICTURES[next],
                    state(),
                    path.display()
                );
                shoot(&mut commands, target, path);
                sequence.stage = Stage::Cut {
                    cut,
                    next: next + 1,
                };
            }
        }
        Stage::Saving { since } => {
            if sequence.saved == PICTURES.len() {
                info!(
                    "doodad_cutdown: {} pictures written, the tree is {}",
                    PICTURES.len(),
                    state()
                );
                // the cut phase's model is drawn after the landing and the tree stays down
                if look.as_ref().is_some_and(|look| {
                    look.fallen && look.tilt > 80.0 && look.rendered == CUT_PHASE
                }) {
                    exit.write(AppExit::Success);
                } else {
                    error!("doodad_cutdown: the tree did not end up lying down");
                    exit.write(AppExit::error());
                }
                sequence.stage = Stage::Over;
            } else if now - since > SAVING_TIMEOUT {
                error!(
                    "doodad_cutdown: {} of {} pictures written",
                    sequence.saved,
                    PICTURES.len()
                );
                exit.write(AppExit::error());
                sequence.stage = Stage::Over;
            }
        }
        Stage::Over => {}
    }
}
