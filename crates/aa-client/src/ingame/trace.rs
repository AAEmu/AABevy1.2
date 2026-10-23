//! `AA_ENTER_TRACE=1` (or `--enter-trace`): what the world does on the way in, one `enter-trace:` line per second from the loading
//! screen to 20 s into the game, to see what still loads (pops in) after the loading screen let go.
//!
//! Each line has the frames and the worst frame of the second, the terrain cells (loaded, loading) and the terrain LOD swaps,
//! aa-world's object stats (sectors, models being prepared), the units (announced, outstanding), the pipelines, the assets
//! added and the GPU's share of them (images and meshes prepared by the render world), and the meshes that became visible to a
//! camera for the first time in that second, by what they are (`brush`, `veg`, `tree`, `entity`, `terrain`, `unit`, `doodad`,
//! `player`, `other`) with the nearest of them in metres. Visible means `ViewVisibility` (in a camera's frustum, in its
//! visibility range), so on the loading screen it counts what the hidden world camera sees; a mesh that first shows after
//! the switch is a mesh that popped in. The game counts from the moment the loading curtain is lifted (`login::curtain`);
//! the frames of the game screen under the curtain are `CURT`.
//!
//! `--enter-shots DIR` saves screenshots of frames 1, 2, 5, 20 and 50 under the curtain (`DIR/curtainNN.png`) and of the
//! window 0, 0.25, 0.5, 1, 2, 4 and 8 s after the curtain was lifted (`DIR/t0.00.png` ...).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aa_world::objects::{Kind, ObjectSector, ObjectState};
use aa_world::pipelines::PipelineStatus;
use aa_world::stream::{Streamer, TerrainPatch};
use bevy::prelude::*;
use bevy::render::mesh::RenderMesh;
use bevy::render::render_asset::RenderAssets;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};

use super::player::LocalPlayer;
use super::units::{RemoteDoodad, RemoteUnit, Views};
use super::warm::Warm;
use crate::Screen;

/// Seconds into the game the trace runs for.
const SPAN: f32 = 20.0;
/// Seconds after the switch at which `--enter-shots` takes its screenshots.
const SHOTS: [f32; 7] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
const KINDS: [&str; 9] = ["brush", "veg", "tree", "entity", "terrain", "unit", "doodad", "player", "other"];

pub fn requested() -> bool {
    std::env::var("AA_ENTER_TRACE").is_ok_and(|v| v != "0") || std::env::args().any(|a| a == "--enter-trace")
}

/// GPU-side counts, written by the render world.
#[derive(Resource, Clone, Default)]
struct Gpu {
    images: Arc<AtomicUsize>,
    meshes: Arc<AtomicUsize>,
}

#[derive(Resource, Default)]
struct Trace {
    /// Seconds on the loading screen / in game.
    loading: f32,
    in_game: Option<f32>,
    /// What the current second has seen.
    frames: u32,
    worst: f32,
    lod_swaps: usize,
    images: usize,
    meshes: usize,
    /// Meshes seen visible so far.
    seen: HashMap<Entity, ()>,
    /// First seen this second, per kind: count and nearest distance.
    new: [(usize, f32); 9],
    shots_taken: usize,
    /// UI rebuilds at the last line, and the slowest UI frame (Lua tick + rebuild) since, in ms.
    ui_rebuilds: u64,
    ui_worst: f32,
    /// Frames of the game screen under the curtain.
    curtain_frames: u32,
    shots: Option<String>,
    frame: u64,
    done: bool,
}

pub fn plugin(app: &mut App) {
    if !requested() {
        return;
    }
    let gpu = Gpu::default();
    app.insert_resource(gpu.clone())
        .insert_resource(Trace {
            shots: std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == "--enter-shots").map(|w| w[1].clone()),
            new: [(0, f32::MAX); 9],
            ..default()
        })
        .add_systems(OnEnter(Screen::Loading), |mut t: ResMut<Trace>| {
            let shots = t.shots.take();
            *t = Trace { shots, new: [(0, f32::MAX); 9], ..default() };
        })
        .add_systems(Last, trace.run_if(super::world_screens));
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.insert_resource(gpu).add_systems(Render, count_gpu.in_set(RenderSystems::Cleanup));
    }
}

fn count_gpu(gpu: Res<Gpu>, images: Res<RenderAssets<GpuImage>>, meshes: Res<RenderAssets<RenderMesh>>) {
    gpu.images.store(images.iter().count(), Ordering::Relaxed);
    gpu.meshes.store(meshes.iter().count(), Ordering::Relaxed);
}

#[allow(clippy::type_complexity)]
fn kind_of(
    e: Entity,
    parents: &Query<&ChildOf>,
    tags: &Query<(Has<TerrainPatch>, Option<&ObjectSector>, Has<RemoteUnit>, Has<RemoteDoodad>, Has<LocalPlayer>)>,
) -> usize {
    let mut at = e;
    for _ in 0..12 {
        if let Ok((terrain, sector, unit, doodad, player)) = tags.get(at) {
            if terrain {
                return 4;
            }
            if let Some(s) = sector {
                return match s.kind {
                    Kind::Brush => 0,
                    Kind::Vegetation => 1,
                    Kind::Trees => 2,
                    _ => 3,
                };
            }
            if unit {
                return 5;
            }
            if doodad {
                return 6;
            }
            if player {
                return 7;
            }
        }
        match parents.get(at) {
            Ok(p) => at = p.parent(),
            Err(_) => break,
        }
    }
    8
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn trace(
    mut commands: Commands,
    time: Res<Time<Real>>,
    screen: Res<State<Screen>>,
    mut t: ResMut<Trace>,
    gpu: Res<Gpu>,
    streamer: Option<Res<Streamer>>,
    objects: Option<Res<ObjectState>>,
    pipelines: Res<PipelineStatus>,
    warm: Res<Warm>,
    views: Res<Views>,
    (images, meshes, curtain, ui): (Res<Assets<Image>>, Res<Assets<Mesh>>, Option<Res<crate::login::curtain::Curtain>>, Res<aa_ui::UiStats>),
    (mut image_events, mut mesh_events): (MessageReader<AssetEvent<Image>>, MessageReader<AssetEvent<Mesh>>),
    swaps: Query<(), (With<TerrainPatch>, Changed<Mesh3d>)>,
    visible: Query<(Entity, &ViewVisibility, &GlobalTransform), With<Mesh3d>>,
    (camera, parents, tags): (
        Query<(&GlobalTransform, &Camera, &bevy::camera::RenderTarget, Has<bevy::camera::Hdr>, Option<&Msaa>), With<super::camera::OrbitCamera>>,
        Query<&ChildOf>,
        Query<(Has<TerrainPatch>, Option<&ObjectSector>, Has<RemoteUnit>, Has<RemoteDoodad>, Has<LocalPlayer>)>,
    ),
) {
    if t.done {
        return;
    }
    let dt = time.delta_secs();
    // the game is seen once the loading curtain is lifted
    // Direct entry (`--script` / `--enter-world`) does not install the login
    // curtain. Its absence means the world is already exposed on the window.
    let curtain_up = curtain.as_ref().is_some_and(|c| c.is_up());
    let in_game = *screen.get() == Screen::InGame && !curtain_up;
    let label = if in_game { "GAME" } else if *screen.get() == Screen::InGame { "CURT" } else { "LOAD" };
    let before = if in_game { t.in_game.unwrap_or(0.0) } else { t.loading };
    if in_game {
        *t.in_game.get_or_insert(-dt) += dt;
    } else {
        t.loading += dt;
    }
    let now = if in_game { t.in_game.unwrap_or(0.0) } else { t.loading };
    t.frames += 1;
    t.frame += 1;
    t.worst = t.worst.max(dt * 1000.0);
    t.ui_worst = t.ui_worst.max(ui.frame_us / 1000.0);
    t.lod_swaps += swaps.iter().count();
    t.images += image_events.read().filter(|e| matches!(e, AssetEvent::Added { .. })).count();
    t.meshes += mesh_events.read().filter(|e| matches!(e, AssetEvent::Added { .. })).count();

    // meshes a camera sees for the first time (every other frame: a mesh is at most a frame late)
    if t.frame.is_multiple_of(2) {
        let eye = camera.single().map(|(c, ..)| c.translation()).ok();
        let t = &mut *t;
        for (e, vis, gt) in &visible {
            if !vis.get() || t.seen.contains_key(&e) {
                continue;
            }
            t.seen.insert(e, ());
            let k = kind_of(e, &parents, &tags);
            let d = eye.map_or(f32::MAX, |c| c.distance(gt.translation()));
            t.new[k].0 += 1;
            t.new[k].1 = t.new[k].1.min(d);
        }
    }

    // under the curtain: what the window shows must be the loading screen
    if label == "CURT" {
        t.curtain_frames += 1;
        if let Some(dir) = t.shots.clone()
            && [1, 2, 5, 20, 50].contains(&t.curtain_frames)
        {
            let path = format!("{dir}/curtain{:02}.png", t.curtain_frames);
            let _ = std::fs::create_dir_all(&dir);
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
        }
    }
    if in_game
        && let Some(dir) = t.shots.clone()
        && t.shots_taken < SHOTS.len()
        && now >= SHOTS[t.shots_taken]
    {
        let path = format!("{dir}/t{:04.2}.png", SHOTS[t.shots_taken]);
        let _ = std::fs::create_dir_all(&dir);
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
        info!("enter-trace: screenshot at +{now:.2} s -> {path}");
        t.shots_taken += 1;
    }

    // a line per whole second (and one at the switch)
    let switched = in_game && before <= 0.0 && t.in_game.is_some_and(|g| g < dt * 1.5);
    if now.floor() == before.floor() && !switched {
        return;
    }
    let (cells, loading) = streamer.as_ref().map_or((0, false), |s| (s.loaded_count(), s.is_loading()));
    let stats = objects.as_ref().map(|o| o.stats.clone()).unwrap_or_default();
    let new: Vec<String> = t
        .new
        .iter()
        .enumerate()
        .filter(|(_, n)| n.0 > 0)
        .map(|(i, n)| if n.1 < f32::MAX { format!("{} {} @{:.0}m", KINDS[i], n.0, n.1) } else { format!("{} {}", KINDS[i], n.0) })
        .collect();
    if let Ok((transform, view, target, hdr, msaa)) = camera.single() {
        let eye = aa_world::bevy_to_cry(transform.translation());
        info!(
            "enter-trace: view eye ({:.3}, {:.3}, {:.3}) hdr {hdr} msaa {:?} active {} target {target:?} | ui quads {} draws {}",
            eye.x, eye.y, eye.z, msaa.copied().unwrap_or_default(), view.is_active, ui.quads, ui.draw_calls,
        );
    }
    info!(
        "enter-trace: {} {:>5.1}s | {} fr worst {:.0} ms | cells {cells}{} lod swaps {} | sectors {} models pending {} cached {} | units {} outstanding {} spawned {}u/{}d | pipelines {} waiting {} | +img {} +mesh {} | gpu img {}/{} mesh {}/{} | workers {} | ui rebuilds {} worst {:.0} ms | new visible: {}",
        label,
        now,
        t.frames,
        t.worst,
        if loading { " (loading)" } else { "" },
        t.lod_swaps,
        stats.sectors,
        stats.models_pending,
        stats.models_cached,
        warm.announced,
        warm.outstanding,
        views.spawned_units,
        views.spawned_doodads,
        pipelines.total(),
        pipelines.waiting(),
        t.images,
        t.meshes,
        gpu.images.load(Ordering::Relaxed),
        images.len(),
        gpu.meshes.load(Ordering::Relaxed),
        meshes.len(),
        aa_cry::workers::Workers::global().pending(),
        ui.rebuilds - t.ui_rebuilds.min(ui.rebuilds),
        t.ui_worst,
        if new.is_empty() { "-".to_string() } else { new.join(", ") },
    );
    t.frames = 0;
    t.worst = 0.0;
    t.ui_rebuilds = ui.rebuilds;
    t.ui_worst = 0.0;
    t.lod_swaps = 0;
    t.images = 0;
    t.meshes = 0;
    t.new = [(0, f32::MAX); 9];
    if in_game && now > SPAN {
        t.done = true;
        info!("enter-trace: done");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut world = World::new();
        world.insert_resource(State::new(Screen::InGame));
        world.init_resource::<Time<Real>>();
        world.resource_mut::<Time<Real>>().advance_by(std::time::Duration::from_secs_f32(0.02));
        world.init_resource::<Trace>();
        world.init_resource::<Gpu>();
        world.init_resource::<PipelineStatus>();
        world.init_resource::<Warm>();
        world.init_resource::<Views>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<aa_ui::UiStats>();
        world.init_resource::<Messages<AssetEvent<Image>>>();
        world.init_resource::<Messages<AssetEvent<Mesh>>>();
        world
    }

    #[test]
    fn direct_entry_traces_without_a_login_curtain() {
        let mut world = world();
        let mut schedule = Schedule::default();
        schedule.add_systems(trace);
        schedule.run(&mut world);
        let trace = world.resource::<Trace>();
        assert_eq!(trace.frame, 1);
        assert!(trace.in_game.is_some());
        assert_eq!(trace.loading, 0.0);
    }

    #[test]
    fn normal_entry_still_counts_frames_under_the_curtain() {
        let mut world = world();
        let mut curtain = crate::login::curtain::Curtain::default();
        curtain.phase = crate::login::curtain::Phase::Up;
        world.insert_resource(curtain);
        let mut schedule = Schedule::default();
        schedule.add_systems(trace);
        schedule.run(&mut world);
        let trace = world.resource::<Trace>();
        assert_eq!(trace.curtain_frames, 1);
        assert_eq!(trace.in_game, None);
        assert!(trace.loading > 0.0);
    }
}
