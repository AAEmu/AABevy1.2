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

/// Log every new pipeline's label, shader defs and vertex / target formats (the browser's `?pipeline-log`; it has no
/// files for `AA_PIPELINE_DUMP`).
pub static LOG_PIPELINES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn summary(d: &bevy::render::render_resource::PipelineDescriptor) -> String {
    use bevy::render::render_resource::PipelineDescriptor as P;
    let defs = |v: &[bevy::shader::ShaderDefVal]| {
        let mut v: Vec<String> = v.iter().map(|d| format!("{d:?}").replace("Bool(\"", "").replace("\", true)", "")).collect();
        v.sort();
        v.join(",")
    };
    match d {
        P::RenderPipelineDescriptor(r) => {
            let attrs: Vec<String> = r.vertex.buffers.iter().map(|b| b.attributes.iter().map(|a| format!("{}:{:?}", a.shader_location, a.format)).collect::<Vec<_>>().join(" ")).collect();
            let targets: Vec<String> = r.fragment.iter().flat_map(|f| f.targets.iter().map(|t| t.as_ref().map_or("-".into(), |t| format!("{:?}{}", t.format, if t.blend.is_some() { "+blend" } else { "" })))).collect();
            format!(
                "R {} | vdefs {} | fdefs {} | attrs {} | targets {} | depth {:?} | msaa {} | cull {:?}",
                r.label.as_deref().unwrap_or(""),
                defs(&r.vertex.shader_defs),
                r.fragment.as_ref().map_or("(none)".into(), |f| defs(&f.shader_defs)),
                attrs.join(" / "),
                targets.join(" "),
                r.depth_stencil.as_ref().map(|d| d.format),
                r.multisample.count,
                r.primitive.cull_mode,
            )
        }
        P::ComputePipelineDescriptor(c) => format!("C {} | defs {}", c.label.as_deref().unwrap_or(""), defs(&c.shader_defs)),
        P::MeshPipelineDescriptor(m) => format!("M {}", m.label.as_deref().unwrap_or("")),
    }
}

fn update(cache: Res<PipelineCache>, status: Res<PipelineStatus>, mut dumped: Local<usize>) {
    status.waiting.store(cache.waiting_pipelines().count(), Ordering::Relaxed);
    status.total.store(cache.pipelines().count(), Ordering::Relaxed);
    status.frames.fetch_add(1, Ordering::Relaxed);
    // `AA_PIPELINE_DUMP=<file>`: one line per requested pipeline (what is different between two runs?)
    if LOG_PIPELINES.load(Ordering::Relaxed) {
        let total = cache.pipelines().count();
        for (i, p) in cache.pipelines().enumerate().skip(*dumped) {
            info!("pipeline {i}: {}", summary(&p.descriptor));
        }
        *dumped = total;
    } else if let Some(path) = std::env::var_os("AA_PIPELINE_DUMP") {
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
