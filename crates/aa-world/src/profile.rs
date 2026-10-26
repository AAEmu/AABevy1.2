//! Release-build profiling without Bevy's `trace` feature: `AA_PROFILE=1` (or `--profile`) adds [`ProfilePlugin`] to `fly` and to
//! the client. Every few seconds it logs an `aa-profile:` block:
//!
//! * the main thread split into `update` (First..Update), `anim+propagate` (PostUpdate before the visibility systems),
//!   `frusta` (visibility propagation, frusta), `check-visibility` (CPU culling) and `post` (the rest up to `Last`); `gap` is the time
//!   between one frame's `Last` and the next `First` that is not main world work (waiting for the render thread + extraction),
//! * the render thread split by `RenderSystems` stage (`render+present` includes waiting for the GPU when it is the bottleneck),
//! * Bevy's render diagnostics (CPU and GPU time of each pass, `RenderDiagnosticsPlugin`),
//! * with `AA_PROFILE_THREADS=1` the CPU cycles per thread (where the process burns its cycles; costs a frame spike per report),
//! * named timers of systems that call [`Timers::time`] (`Option<Res<Timers>>`, present only with the plugin),
//! * entity, mesh entity, visible mesh and distinct mesh x material counts.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use bevy::camera::visibility::{ViewVisibility, VisibilitySystems};
use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::transform::TransformSystems;

use crate::post::PostUniform;
use crate::stream::WorldTimings;

const MAIN: usize = 7;
const RENDER: usize = 15;
const MAIN_NAMES: [&str; MAIN - 1] = ["update", "pre", "anim+propagate", "frusta", "check-visibility", "post"];
const RENDER_NAMES: [&str; RENDER - 1] = ["extract-cmds", "assets", "meshes", "create-views", "specialize", "acquire-swapchain", "prepare-views", "queue+sort", "prep-resources", "batch-phases", "write-buffers", "bind-groups", "render+present", "cleanup"];

/// True when `AA_PROFILE` is set or `--profile` was given.
pub fn requested() -> bool {
    std::env::var_os("AA_PROFILE").is_some() || std::env::args().any(|a| a == "--profile")
}

/// Named timers that any system can feed (`let _t = timers.as_deref().map(|t| t.time("name"));`); they share the registry of
/// [`aa_cry::prof`], so scopes inside aa-cry / aa-anim show up in the same line.
#[derive(Resource, Clone, Default)]
pub struct Timers;

impl Timers {
    /// Times until the guard is dropped.
    pub fn time(&self, name: &'static str) -> aa_cry::prof::Scope {
        aa_cry::prof::scope(name)
    }

    /// Adds a measurement made elsewhere (wall and CPU milliseconds).
    pub fn add(&self, name: &'static str, ms: f64) {
        aa_cry::prof::add(name, ms, ms);
    }
}

#[derive(Default)]
struct Sums {
    main: [f64; MAIN - 1],
    gap: f64,
    frame: f64,
    frames: u32,
    /// Process CPU cycles over all threads, in millions.
    cycles: f64,
    /// The longest frame of the window and its main thread slices.
    worst: (f64, [f64; MAIN - 1]),
    render: [f64; RENDER - 1],
    render_frames: u32,
}

#[derive(Resource)]
struct Stamps {
    /// Thread cycles at the last report, by thread id.
    thread_cycles: BTreeMap<u32, u64>,
    t: [Option<Instant>; MAIN],
    last_end: Option<Instant>,
    last_first: Option<Instant>,
    last_cycles: u64,
    sums: Arc<Mutex<Sums>>,
}

#[derive(Resource)]
struct RenderStamps {
    t: [Option<Instant>; RENDER],
    sums: Arc<Mutex<Sums>>,
}

pub struct ProfilePlugin;

impl Plugin for ProfilePlugin {
    fn build(&self, app: &mut App) {
        let sums = Arc::new(Mutex::new(Sums::default()));
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        if !app.is_plugin_added::<EntityCountDiagnosticsPlugin>() {
            app.add_plugins(EntityCountDiagnosticsPlugin::default());
        }
        if !app.is_plugin_added::<RenderDiagnosticsPlugin>() {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        uncapped(app);
        aa_cry::prof::enable(true);
        // `AA_SAMPLE="Render thread,main"` (optionally `AA_SAMPLE_AFTER=<s>`, `AA_SAMPLE_SECONDS=<s>`): a sampling profile of those threads
        if let Ok(names) = std::env::var("AA_SAMPLE") {
            let value = |name: &str, default: f32| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
            aa_cry::prof::sample_threads(
                names.split(',').map(|n| n.trim().to_string()).collect(),
                std::time::Duration::from_secs_f32(value("AA_SAMPLE_AFTER", 18.0)),
                value("AA_SAMPLE_SECONDS", 8.0),
                std::time::Duration::from_micros(700),
                |line| info!("{line}"),
            );
        }
        if std::env::var_os("AA_OFFSCREEN").is_some() {
            app.add_systems(Update, offscreen);
        }
        app.init_resource::<Timers>()
            .insert_resource(Stamps { thread_cycles: BTreeMap::new(), t: [None; MAIN], last_end: None, last_first: None, last_cycles: aa_cry::prof::process_cycles(), sums: sums.clone() })
            .add_systems(First, stamp::<0>)
            .add_systems(SpawnScene, stamp::<1>)
            .add_systems(
                PostUpdate,
                (
                    stamp::<2>.before(TransformSystems::Propagate),
                    stamp::<3>.after(TransformSystems::Propagate).before(VisibilitySystems::VisibilityPropagate).before(VisibilitySystems::UpdateFrusta),
                    stamp::<4>
                        .after(VisibilitySystems::VisibilityPropagate)
                        .after(VisibilitySystems::UpdateFrusta)
                        .before(VisibilitySystems::CheckVisibility),
                    stamp::<5>.after(VisibilitySystems::CheckVisibility),
                ),
            )
            .add_systems(Last, (stamp::<6>, report).chain());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(RenderStamps { t: [None; RENDER], sums });
            use RenderSystems as S;
            use bevy::render::view::{prepare_view_attachments, prepare_windows};
            render_app.add_systems(
                Render,
                (
                    rstamp::<0>.before(S::ExtractCommands),
                    rstamp::<1>.after(S::ExtractCommands).before(S::PrepareAssets),
                    rstamp::<2>.after(S::PrepareAssets).before(S::PrepareMeshes),
                    rstamp::<3>.after(S::PrepareMeshes).before(S::CreateViews),
                    rstamp::<4>.after(S::CreateViews).before(S::Specialize),
                    rstamp::<5>.after(S::Specialize).before(prepare_windows),
                    rstamp::<6>.after(prepare_windows).before(prepare_view_attachments),
                    rstamp::<7>.after(S::PrepareViews).before(S::Queue),
                    rstamp::<8>.after(S::PhaseSort).before(S::PrepareResources),
                    rstamp::<9>.after(S::PrepareResources).before(S::PrepareResourcesBatchPhases),
                    rstamp::<10>.after(S::PrepareResourcesBatchPhases).before(S::PrepareResourcesWritePhaseBuffers),
                    rstamp::<11>.after(S::PrepareResourcesWritePhaseBuffers).before(S::PrepareBindGroups),
                    rstamp::<12>.after(S::PrepareBindGroups).before(S::Render),
                    rstamp::<13>.after(S::Render).before(S::Cleanup),
                    rstamp::<14>.after(S::PostCleanup),
                ),
            );
        }
    }
}

fn stamp<const N: usize>(mut stamps: ResMut<Stamps>) {
    stamps.t[N] = Some(Instant::now());
}

fn rstamp<const N: usize>(mut stamps: ResMut<RenderStamps>) {
    stamps.t[N] = Some(Instant::now());
    if N == RENDER - 1 {
        let mut s = stamps.sums.lock().unwrap();
        for i in 0..RENDER - 1 {
            if let (Some(a), Some(b)) = (stamps.t[i], stamps.t[i + 1]) {
                s.render[i] += b.saturating_duration_since(a).as_secs_f64() * 1000.0;
            }
        }
        s.render_frames += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn report(
    mut stamps: ResMut<Stamps>,
    time: Res<Time<Real>>,
    mut since: Local<f32>,
    diagnostics: Res<DiagnosticsStore>,
    world_timings: Option<Res<WorldTimings>>,
    meshes: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>, &ViewVisibility)>,
    cameras: Query<(&Camera, Option<&Name>, &bevy::camera::RenderTarget, Has<Camera3d>)>,
) {
    let first = stamps.t[0].unwrap_or_else(Instant::now);
    let end = stamps.t[MAIN - 1].unwrap_or(first);
    let cycles = aa_cry::prof::process_cycles();
    {
        let mut s = stamps.sums.lock().unwrap();
        if let Some(prev_end) = stamps.last_end {
            s.gap += first.saturating_duration_since(prev_end).as_secs_f64() * 1000.0;
        }
        let mut slices = [0.0; MAIN - 1];
        for i in 0..MAIN - 1 {
            if let (Some(a), Some(b)) = (stamps.t[i], stamps.t[i + 1]) {
                slices[i] = b.saturating_duration_since(a).as_secs_f64() * 1000.0;
                s.main[i] += slices[i];
            }
        }
        if let Some(prev_first) = stamps.last_first {
            let frame = first.saturating_duration_since(prev_first).as_secs_f64() * 1000.0;
            s.frame += frame;
            if frame > s.worst.0 {
                s.worst = (frame, slices);
            }
        }
        s.frames += 1;
        s.cycles += cycles.saturating_sub(stamps.last_cycles) as f64 / 1e6;
    }
    stamps.last_cycles = cycles;
    stamps.last_end = Some(end);
    stamps.last_first = Some(first);

    *since += time.delta_secs();
    if *since < 3.0 {
        return;
    }
    *since = 0.0;
    let sums = std::mem::take(&mut *stamps.sums.lock().unwrap());
    let n = sums.frames.max(1) as f64;
    let rn = sums.render_frames.max(1) as f64;
    let mut line = format!("aa-profile: frame {:.2} ms | main thread:", sums.frame / n);
    for (i, name) in MAIN_NAMES.iter().enumerate() {
        line += &format!(" {name} {:.2}", sums.main[i] / n);
    }
    line += &format!(" gap {:.2} | cpu {:.1} Mcycles/frame (all threads)", sums.gap / n, sums.cycles / n);
    info!("{line}");
    let mut line = format!("aa-profile: longest frame {:.1} ms, its main thread slices:", sums.worst.0);
    for (i, name) in MAIN_NAMES.iter().enumerate() {
        line += &format!(" {name} {:.1}", sums.worst.1[i]);
    }
    info!("{line}");
    let mut line = String::from("aa-profile: render thread:");
    for (i, name) in RENDER_NAMES.iter().enumerate() {
        line += &format!(" {name} {:.2}", sums.render[i] / rn);
    }
    info!("{line}");

    // CPU per thread (threads of one kind are summed): where the cycles of the process go
    if std::env::var_os("AA_PROFILE_THREADS").is_some() {
        // slow (a system wide thread snapshot, tens of ms): a spike in the frame time of the report itself
        let now = aa_cry::prof::all_thread_cycles();
        let mut by_name: BTreeMap<String, (f64, u32)> = BTreeMap::new();
        for (id, name, cycles) in &now {
            let used = cycles.saturating_sub(stamps.thread_cycles.get(id).copied().unwrap_or(*cycles));
            let name = if name.is_empty() { "(unnamed)".to_string() } else { name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == '(' || c == ')').to_string() };
            let e = by_name.entry(name).or_default();
            e.0 += used as f64 / 1e6 / n;
            e.1 += 1;
        }
        stamps.thread_cycles = now.into_iter().map(|(id, _, c)| (id, c)).collect();
        let mut rows: Vec<_> = by_name.into_iter().filter(|(_, v)| v.0 >= 0.05).collect();
        rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        let mut line = String::from("aa-profile: threads (Mcycles per frame, count):");
        for (name, (m, count)) in rows {
            line += &format!(" {name} {m:.1} (x{count});");
        }
        info!("{line}");
    }
    let timed = aa_cry::prof::take();
    let mut line = format!("aa-profile: timers (ms per frame: thread cpu / wall; calls, longest call; clock {:.0} MHz):", aa_cry::prof::mhz());
    for (name, stat) in &timed {
        line += &format!(" {name} {:.3}/{:.3} (x{}, max {:.2});", stat.cpu / n, stat.wall / n, stat.calls, stat.max);
    }
    if let Some(t) = world_timings {
        line += &format!(" world: cells {:.2} receive {:.2} lod {:.2} objects {:.2}", t.cells_ms, t.receive_ms, t.lod_ms, t.objects_ms);
    }
    info!("{line}");

    // Bevy render diagnostics: CPU / GPU time per pass
    let mut passes: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for d in diagnostics.iter() {
        let path = d.path().as_str();
        let Some(rest) = path.strip_prefix("render/") else { continue };
        let Some(avg) = d.average() else { continue };
        if let Some(name) = rest.strip_suffix("/elapsed_gpu") {
            passes.entry(name.to_string()).or_default().1 = avg;
        } else if let Some(name) = rest.strip_suffix("/elapsed_cpu") {
            passes.entry(name.to_string()).or_default().0 = avg;
        }
    }
    let gpu_sum: f64 = passes.iter().filter(|(k, _)| !k.contains('/') || k.matches('/').count() == 0).map(|(_, v)| v.1).sum();
    let mut line = String::from("aa-profile: passes cpu/gpu ms:");
    for (name, (cpu, gpu)) in &passes {
        if cpu.max(*gpu) >= 0.01 {
            line += &format!(" {name} {cpu:.2}/{gpu:.2};");
        }
    }
    info!("{line} (gpu sum {gpu_sum:.2})");

    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.average()).unwrap_or(0.0);
    let entities = diagnostics.get(&EntityCountDiagnosticsPlugin::ENTITY_COUNT).and_then(|d| d.value()).unwrap_or(0.0);
    let (mut total, mut visible) = (0usize, 0usize);
    let mut pairs = HashSet::new();
    for (m, mat, vis) in &meshes {
        total += 1;
        if vis.get() {
            visible += 1;
            pairs.insert((m.0.id(), mat.0.id()));
        }
    }
    let mut line = String::from("aa-profile: cameras:");
    for (c, name, target, is3d) in &cameras {
        let target = match target {
            bevy::camera::RenderTarget::Window(_) => "window",
            bevy::camera::RenderTarget::Image(_) => "image",
            _ => "other",
        };
        line += &format!(" [{} order {} {} {} {}]", name.map_or("-", |n| n.as_str()), c.order, if is3d { "3d" } else { "2d" }, target, if c.is_active { "active" } else { "inactive" });
    }
    info!("{line}");
    info!("aa-profile: fps {fps:.0} | entities {entities:.0}, mesh entities {total}, visible {visible}, distinct mesh+material {}", pairs.len());
}

/// `AA_OFFSCREEN=1` (with the profile plugin): the first 3D camera renders to an image of the window's size instead of the
/// swap chain. The window system paces presentation (a background window ran at 60 / 120 Hz whatever the scene cost), so
/// frame times of an uncapped run come from here: they are CPU + GPU work only.
fn offscreen(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(Entity, &bevy::camera::RenderTarget), (With<Camera3d>, With<PostUniform>)>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    // the camera with the world's post pass (the login stage and UI cameras are other cameras)
    let (Some((camera, bevy::camera::RenderTarget::Window(_))), Some(window)) = (cameras.iter().next(), windows.iter().next()) else { return };
    let image = Image::new_target_texture(window.physical_width(), window.physical_height(), bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
    commands.entity(camera).insert(bevy::camera::RenderTarget::Image(images.add(image).into()));
    *done = true;
    info!("aa-profile: rendering offscreen ({} x {})", window.physical_width(), window.physical_height());
}

/// Bevy updates an unfocused window at 60 Hz only (`WinitSettings::game`), which is where an unattended benchmark window
/// lives: every frame time came out as 16.7 ms. Benchmarks update as fast as they can instead.
pub fn uncapped(app: &mut App) {
    app.insert_resource(bevy::winit::WinitSettings::continuous());
}
