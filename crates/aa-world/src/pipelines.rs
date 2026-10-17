//! How far the GPU pipelines are. Bevy compiles render pipelines on background tasks the first time a mesh / material /
//! view combination is drawn, and leaves the mesh out until its pipeline is there; the first frames of a world pop in and
//! stall. The render world counts the pipelines it has asked for and the ones still compiling, so a loading screen can
//! draw the world behind itself and wait until nothing is waiting and the total stops growing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};

/// Shared between the main and the render world (clones count the same atomics).
#[derive(Resource, Clone, Default)]
pub struct PipelineStatus {
    waiting: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    frames: Arc<AtomicU64>,
}

impl PipelineStatus {
    /// Pipelines queued or still being compiled.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::Relaxed)
    }

    /// Pipelines requested so far (grows while new mesh / material / view combinations show up).
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Relaxed)
    }

    /// Frames the render world has run.
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

pub fn build(app: &mut App) {
    let status = PipelineStatus::default();
    app.insert_resource(status.clone());
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.insert_resource(status).add_systems(Render, update.in_set(RenderSystems::Cleanup));
    }
}

fn update(cache: Res<PipelineCache>, status: Res<PipelineStatus>, mut dumped: Local<usize>) {
    status.waiting.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
    status.total.store(cache.pipelines().count(), Ordering::Relaxed);
    status.frames.fetch_add(1, Ordering::Relaxed);
    // `AA_PIPELINE_DUMP=<file>`: one line per requested pipeline (what is different between two runs?)
    if let Some(path) = std::env::var_os("AA_PIPELINE_DUMP") {
        let total = cache.pipelines().count();
        if total > *dumped {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                for (i, p) in cache.pipelines().enumerate().skip(*dumped) {
                    let _ = writeln!(f, "{i}	{:?}", p.descriptor);
                }
            }
            *dumped = total;
        }
    }
}
