//! Release-build profiling without Bevy's `trace` feature: `AA_PROFILE=1` (or `--profile`) adds [`ProfilePlugin`] to `fly` and to
//! the client. Every few seconds it logs an `aa-profile:` block:
//!
//! * the main thread split into `update` (First..Update), `anim+propagate` (PostUpdate before the visibility systems),
//!   `frusta` (visibility propagation, frusta), `check-visibility` (CPU culling) and `post` (the rest up to `Last`); `gap` is the time
//!   between one frame's `Last` and the next `First` that is not main world work (waiting for the render thread + extraction),
//! * the render thread split by `RenderSystems` stage (`render+present` includes waiting for the GPU when it is the bottleneck),
//! * Bevy's render diagnostics (CPU and GPU time of each pass, `RenderDiagnosticsPlugin`),
//! * named timers of systems that call [`Timers::time`] (`Option<Res<Timers>>`, present only with the plugin),
//! * entity, mesh entity, visible mesh and distinct mesh x material counts.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::camera::visibility::{ViewVisibility, VisibilitySystems};
use bevy::diagnostic::{DiagnosticsStore, EntityCountDiagnosticsPlugin, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::transform::TransformSystems;

use crate::stream::WorldTimings;

const MAIN: usize = 7;
const RENDER: usize = 12;
const MAIN_NAMES: [&str; MAIN - 1] = ["update", "pre", "anim+propagate", "frusta", "check-visibility", "post"];
const RENDER_NAMES: [&str; RENDER - 1] = ["extract-cmds", "assets", "meshes", "create-views", "specialize", "acquire-swapchain", "prepare-views", "queue+sort", "prepare", "render+present", "cleanup"];

/// True when `AA_PROFILE` is set or `--profile` was given.
pub fn requested() -> bool {
    std::env::var_os("AA_PROFILE").is_some() || std::env::args().any(|a| a == "--profile")
}

/// Named millisecond timers that any system can feed (`let _t = timers.as_deref().map(|t| t.time("name"));`).
#[derive(Resource, Clone, Default)]
pub struct Timers(Arc<Mutex<BTreeMap<&'static str, (f64, u32)>>>);

impl Timers {
    /// Times until the guard is dropped.
    pub fn time(&self, name: &'static str) -> TimerGuard<'_> {
        TimerGuard { timers: self, name, start: Instant::now() }
    }

    pub fn add(&self, name: &'static str, ms: f64) {
        let mut m = self.0.lock().unwrap();
        let e = m.entry(name).or_default();
        e.0 += ms;
        e.1 += 1;
    }
}

pub struct TimerGuard<'a> {
    timers: &'a Timers,
    name: &'static str,
    start: Instant,
}

impl Drop for TimerGuard<'_> {
    fn drop(&mut self) {
        self.timers.add(self.name, self.start.elapsed().as_secs_f64() * 1000.0);
    }
}

#[derive(Default)]
struct Sums {
    main: [f64; MAIN - 1],
    gap: f64,
    frame: f64,
    frames: u32,
    render: [f64; RENDER - 1],
    render_frames: u32,
}

#[derive(Resource)]
struct Stamps {
    t: [Option<Instant>; MAIN],
    last_end: Option<Instant>,
    last_first: Option<Instant>,
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
        if std::env::var_os("AA_OFFSCREEN").is_some() {
            app.add_systems(Update, offscreen);
        }
        app.init_resource::<Timers>()
            .insert_resource(Stamps { t: [None; MAIN], last_end: None, last_first: None, sums: sums.clone() })
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
                    rstamp::<8>.after(S::PhaseSort).before(S::Prepare),
                    rstamp::<9>.after(S::Prepare).before(S::Render),
                    rstamp::<10>.after(S::Render).before(S::Cleanup),
                    rstamp::<11>.after(S::PostCleanup),
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
    timers: Res<Timers>,
    world_timings: Option<Res<WorldTimings>>,
    meshes: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>, &ViewVisibility)>,
) {
    let first = stamps.t[0].unwrap_or_else(Instant::now);
    let end = stamps.t[MAIN - 1].unwrap_or(first);
    {
        let mut s = stamps.sums.lock().unwrap();
        if let Some(prev_end) = stamps.last_end {
            s.gap += first.saturating_duration_since(prev_end).as_secs_f64() * 1000.0;
        }
        if let Some(prev_first) = stamps.last_first {
            s.frame += first.saturating_duration_since(prev_first).as_secs_f64() * 1000.0;
        }
        for i in 0..MAIN - 1 {
            if let (Some(a), Some(b)) = (stamps.t[i], stamps.t[i + 1]) {
                s.main[i] += b.saturating_duration_since(a).as_secs_f64() * 1000.0;
            }
        }
        s.frames += 1;
    }
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
    line += &format!(" gap {:.2}", sums.gap / n);
    info!("{line}");
    let mut line = String::from("aa-profile: render thread:");
    for (i, name) in RENDER_NAMES.iter().enumerate() {
        line += &format!(" {name} {:.2}", sums.render[i] / rn);
    }
    info!("{line}");

    let timed = std::mem::take(&mut *timers.0.lock().unwrap());
    let mut line = String::from("aa-profile: timers (ms per frame, per call):");
    for (name, (sum, calls)) in &timed {
        line += &format!(" {name} {:.3} ({:.3} x{});", sum / n, sum / f64::from((*calls).max(1)), calls);
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
    info!("aa-profile: fps {fps:.0} | entities {entities:.0}, mesh entities {total}, visible {visible}, distinct mesh+material {}", pairs.len());
}

/// `AA_OFFSCREEN=1` (with the profile plugin): the first 3D camera renders to an image of the window's size instead of the
/// swap chain. The window system paces presentation (a background window ran at 60 / 120 Hz whatever the scene cost), so
/// frame times of an uncapped run come from here: they are CPU + GPU work only.
fn offscreen(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<Entity, With<Camera3d>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(camera), Some(window)) = (cameras.iter().next(), windows.iter().next()) else { return };
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
