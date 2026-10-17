//! The world's render pipelines compiled while the player sits on the character list.
//!
//! Bevy compiles a render pipeline the first time a mesh / material / view combination is drawn (80 to 160 of them in the
//! first frames of the world, about 1.7 s on the async compute pool), and the loading screen used to wait for that. Here the
//! character list does it ahead of time: once it has settled, the real world is streamed in at a small footprint around the
//! selected character (terrain, objects, vegetation, water, sky, the particle effects) and drawn by a camera that renders into
//! a tiny image. The camera is the in-game camera in everything that decides a pipeline (the lens, `Post` and with it HDR, MSAA,
//! the prepasses, SSAO, bloom, tonemapping, the screen space transmission of the particle refraction), the one character stands where the player will, and the loaders are the real ones, so the
//! variants compiled are the ones the game asks for. It ends when the pipelines have settled ([`PipelineStatus`]), or at once
//! when the player leaves the list; whatever compiled so far stays in the pipeline cache.
//!
//! The world is the real one, not a mock scene, because the pipeline of a material depends on its flags, on the mesh's
//! vertex layout and on the passes it is drawn in (prepass, shadow cascades), and the data decides which combinations exist.
//!
//! `--no-prewarm` (or `AA_PREWARM=0`) turns it off.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use aa_anim::{CharacterAssets, ClipLibrary, spawn_character};
use aa_cry::PreparedModel;
use aa_cry::scene::CryAssets;
use aa_cry::workers::Workers;
use aa_net::state::Characters;
use aa_world::objects::ObjectState;
use aa_world::pipelines::PipelineStatus;
use aa_world::stream::Streamer;
use aa_world::{TerrainQuery, WorldConfig, cry_to_bevy};
use bevy::camera::RenderTarget;
use bevy::prelude::*;

use super::camera::{OrbitCamera, camera_transform, projection, transmission};
use super::data::Db;
use super::dev::DevOptions;
use super::player::{height_or, placement};
use super::prepare::prepare_listed;
use crate::Screen;
use crate::login::WorldAnchor;

/// Seconds the character list must have been up before the prewarm starts (its own build, the stage character).
const SETTLE: f32 = 1.0;
/// The prewarm ends when nothing has changed for this long, ...
const STEADY: f32 = 0.6;
/// ... at the latest after this many seconds.
const TIMEOUT: f32 = 20.0;
/// The way the camera looks (towards the pond at the Nuian spawn; anywhere else it is just a view).
const YAW_DEG: f32 = 158.74;
/// Size of the image the camera renders into.
const TARGET: (u32, u32) = (96, 54);

pub fn plugin(app: &mut App) {
    app.init_resource::<Prewarm>()
        .add_systems(OnEnter(Screen::CharacterSelect), |mut p: ResMut<Prewarm>| p.list_secs = 0.0)
        .add_systems(OnExit(Screen::CharacterSelect), abort)
        .add_systems(OnEnter(Screen::WorldSelect), park)
        .add_systems(OnEnter(Screen::CharacterCreate), park)
        .add_systems(Update, (start, watch).chain().run_if(super::list_is_up))
        .add_systems(Update, tail.run_if(in_state(Screen::CharacterSelect)));
}

#[derive(Component)]
struct PrewarmCamera;

#[derive(Component)]
struct PrewarmCharacter;

type Prepared = Option<(Box<CharacterAssets>, Vec<PreparedModel>)>;

#[derive(Default)]
enum Phase {
    /// Not started (the list has not settled, or the run was cut short and may start again on the next visit).
    #[default]
    Waiting,
    Running(Box<Run>),
    /// Settled once: the pipelines stay compiled for the rest of the process.
    Done,
}

struct Run {
    since: f32,
    camera: Entity,
    pos: Vec3,
    /// The character being prepared on a worker, then the entity.
    prepared: Option<Mutex<Receiver<Prepared>>>,
    character: Option<Entity>,
    /// The streaming settings to put back.
    saved: WorldConfig,
    /// What `settled` last looked at, and for how long it has not changed.
    last: (usize, usize, usize),
    steady: f32,
    /// Worst frame (ms) while it runs.
    worst: f32,
    /// Pipelines requested when it began.
    pipelines_at_start: usize,
}

#[derive(Resource, Default)]
pub struct Prewarm {
    phase: Phase,
    /// Seconds the character list has been up.
    list_secs: f32,
    /// Seconds left to watch the frames after the world was dropped, and the worst of them.
    tail: f32,
    tail_worst: f32,
}

fn disabled(dev: &DevOptions) -> bool {
    dev.no_prewarm || std::env::var("AA_PREWARM").is_ok_and(|v| v == "0")
}

/// A character of the list to stand in for the player: the first one with a position.
fn pick(characters: &Characters) -> Option<(&aa_net::model::Character, Vec3)> {
    let c = characters.list.iter().find(|c| c.pos.x != 0.0 || c.pos.y != 0.0)?;
    Some((c, Vec3::new(c.pos.x, c.pos.y, c.pos.z)))
}

#[allow(clippy::too_many_arguments)]
fn start(
    mut commands: Commands,
    time: Res<Time<Real>>,
    dev: Res<DevOptions>,
    db: Option<Res<Db>>,
    mut prewarm: ResMut<Prewarm>,
    characters: Res<Characters>,
    mut cfg: ResMut<WorldConfig>,
    terrain: Res<TerrainQuery>,
    status: Res<PipelineStatus>,
    ambient: Option<Res<aa_world::AmbientCubes>>,
    mut anchor: Query<&mut Transform, With<WorldAnchor>>,
    mut cry: CryAssets,
) {
    prewarm.list_secs += time.delta_secs();
    if !matches!(prewarm.phase, Phase::Waiting) || prewarm.list_secs < SETTLE || disabled(&dev) {
        return;
    }
    let (Some(db), Some((character, pos))) = (db, pick(&characters)) else { return };
    let Ok(mut anchor) = anchor.single_mut() else { return };
    let ground = height_or(&terrain, pos.x, pos.y, pos.z);
    let pos = Vec3::new(pos.x, pos.y, ground);
    let facing = YAW_DEG.to_radians();
    info!("prewarm: building the world around {} at ({:.0}, {:.0}) behind the character list", character.name, pos.x, pos.y);

    // the real streaming, at a small footprint and a light touch on the main thread
    let saved = cfg.clone();
    cfg.load_radius = 450.0;
    cfg.unload_radius = 700.0;
    cfg.brush_distance = 300.0;
    cfg.vegetation_distance = 150.0;
    cfg.tree_distance = 300.0;
    cfg.water_distance = 500.0;
    cfg.object_budget_ms = 2.0;
    cfg.finish_budget_ms = 1.0;
    terrain.prefetch(pos.x, pos.y, 500.0);
    anchor.translation = cry_to_bevy(pos);

    let orbit = OrbitCamera { yaw: facing, ..default() };
    let image = Image::new_target_texture(TARGET.0, TARGET.1, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    // aa-world puts the ambient environment light on the camera that has the world focus, which is the anchor here; the
    // pipelines of the game's camera have the `environment_map` view layout, so this one needs the component too (the
    // colours do not matter, only that it is there; aa-world's own maps are already on the GPU)
    let environment = match ambient {
        Some(maps) => maps.light(),
        None => {
            let grey = Color::linear_rgb(0.05, 0.05, 0.05);
            bevy::light::EnvironmentMapLight::hemispherical_gradient(&mut cry.images, grey, grey, grey)
        }
    };
    let camera = commands
        .spawn((
            Name::new("prewarm camera"),
            PrewarmCamera,
            Camera3d::default(),
            projection(),
            camera_transform(&orbit, pos),
            RenderTarget::Image(cry.images.add(image).into()),
            // the post pass is what makes the in-game camera HDR, MSAA-free, prepassed and SSAO'd (aa-world's `setup_post_cameras`)
            aa_world::post::Post::default(),
            environment,
            // the view layout of the game's camera has the transmission texture binding (see `camera::transmission`)
            transmission(),
        ))
        .id();

    // the character, prepared on a worker like the units of the game
    let (tx, rx) = channel();
    let (db, loader, c) = (db.clone(), cry.loader(), character.clone());
    Workers::global().spawn(-1.0, move || {
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_listed(&db, &loader, &c))).unwrap_or(None);
        let _ = tx.send(built);
    });
    prewarm.phase = Phase::Running(Box::new(Run {
        since: 0.0,
        camera,
        pos,
        prepared: Some(Mutex::new(rx)),
        character: None,
        saved,
        last: (0, 0, 0),
        steady: 0.0,
        worst: 0.0,
        pipelines_at_start: status.total(),
    }));
}

#[allow(clippy::too_many_arguments)]
fn watch(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut prewarm: ResMut<Prewarm>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    mut cfg: ResMut<WorldConfig>,
    status: Res<PipelineStatus>,
    objects: Option<Res<ObjectState>>,
    streamer: Option<Res<Streamer>>,
    mut anchor: Query<&mut Transform, With<WorldAnchor>>,
) {
    let prewarm = &mut *prewarm;
    let Phase::Running(run) = &mut prewarm.phase else { return };
    let dt = time.delta_secs();
    run.since += dt;
    run.worst = run.worst.max(dt * 1000.0);

    // the character arrives from its worker: models into the assets, then the entity
    let arrived = run.prepared.as_ref().and_then(|rx| rx.lock().unwrap().try_recv().ok());
    if let Some(built) = arrived {
        run.prepared = None;
        if let Some((assets, models)) = built {
            for m in models {
                cry.finish_model(m);
            }
            let facing = YAW_DEG.to_radians();
            match spawn_character(&mut commands, &mut cry, &clips, *assets, placement(run.pos, facing, 1.0)) {
                Ok(e) => {
                    commands.entity(e).insert(PrewarmCharacter);
                    run.character = Some(e);
                }
                Err(e) => warn!("prewarm: cannot spawn the character: {e}"),
            }
        } else {
            debug!("prewarm: no character to draw");
        }
    }

    // settled: the world has stopped changing and nothing is compiling
    let (entities, models) = objects.as_ref().map_or((0, 0), |o| (o.stats.entities, o.stats.models_pending));
    let cells = streamer.as_ref().map_or(0, |s| s.loaded_count());
    let look = (entities, status.total(), cells);
    let busy = look != run.last || status.waiting() > 0 || models > 0 || run.prepared.is_some() || cells == 0;
    run.last = look;
    run.steady = if busy { 0.0 } else { run.steady + dt };
    let settled = run.steady >= STEADY && run.since >= 1.0;
    if !(settled || run.since > TIMEOUT) {
        return;
    }
    let Phase::Running(run) = std::mem::replace(&mut prewarm.phase, Phase::Done) else { return };
    info!(
        "prewarm: {} after {:.1} s: {} pipelines ({} at the start, {} waiting), {} entities, {} cells; worst frame {:.0} ms",
        if settled { "pipelines settled" } else { "gave up" },
        run.since,
        status.total(),
        run.pipelines_at_start,
        status.waiting(),
        entities,
        cells,
        run.worst
    );
    teardown(&mut commands, &run, &mut cfg);
    // the world goes with the anchor: far away, the cells unload (the hitch of that is watched, see `tail`)
    if let Ok(mut t) = anchor.single_mut() {
        t.translation = FAR;
    }
    prewarm.tail = 1.0;
    prewarm.tail_worst = 0.0;
}

/// Where the anchor sits when no world is wanted (`login::spawn_world_anchor`).
const FAR: Vec3 = Vec3::new(-1.0e6, 0.0, 1.0e6);

fn teardown(commands: &mut Commands, run: &Run, cfg: &mut WorldConfig) {
    commands.entity(run.camera).despawn();
    if let Some(c) = run.character {
        commands.entity(c).despawn();
    }
    *cfg = run.saved.clone();
}

/// The frames right after the world was dropped (the unload is where a hitch would show).
fn tail(time: Res<Time<Real>>, mut prewarm: ResMut<Prewarm>) {
    if prewarm.tail <= 0.0 {
        return;
    }
    let dt = time.delta_secs();
    prewarm.tail -= dt;
    prewarm.tail_worst = prewarm.tail_worst.max(dt * 1000.0);
    if prewarm.tail <= 0.0 {
        info!("prewarm: worst frame in the second after the world was dropped: {:.0} ms", prewarm.tail_worst);
    }
}

/// The player left the list (Start, or back to the server list) before it settled: drop the camera and the character and put the
/// streaming back; the world stays where it is (Start points the anchor at the spawn, which may be this very place).
fn abort(
    mut commands: Commands,
    mut prewarm: ResMut<Prewarm>,
    mut cfg: ResMut<WorldConfig>,
    status: Res<PipelineStatus>,
) {
    let Phase::Running(run) = std::mem::replace(&mut prewarm.phase, Phase::Waiting) else { return };
    info!(
        "prewarm: cut short after {:.1} s ({} pipelines, {} at the start, {} waiting); worst frame {:.0} ms",
        run.since,
        status.total(),
        run.pipelines_at_start,
        status.waiting(),
        run.worst
    );
    teardown(&mut commands, &run, &mut cfg);
}

/// To the server list or the create screen: no world wanted there.
fn park(mut anchor: Query<&mut Transform, With<WorldAnchor>>, mut prewarm: ResMut<Prewarm>) {
    for mut t in &mut anchor {
        t.translation = FAR;
    }
    prewarm.tail = 0.0;
}
