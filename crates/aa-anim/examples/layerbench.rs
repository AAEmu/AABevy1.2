//! Cost of the animation system for a crowd, with and without partial-body layers:
//! `cargo run -p aa-anim --example layerbench --release -- [--units 120] [--frames 300]`.
//!
//! Headless: `units` Nuian males (162 bone entities each) run the forward run clip; some also play the bow cast on the upper body.
//! Prints the time of the `animate` system (sampling in parallel plus the serial bone `Transform` writes) and of the whole
//! `app.update()` (which adds Bevy's transform propagation) per frame, for each scenario.

use std::time::{Duration, Instant};

use aa_anim::animator::take_stats;
use aa_anim::pose::{SampleOptions, shared_rig};
use aa_anim::{AnimPlugin, Animator, BoneMask, ClipLibrary, spawn_bones};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";

fn arg(name: &str, default: usize) -> usize {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// `layered_every`: every N-th unit has a layer (0 = none); `interval`: seconds between updates of every unit (far LOD).
fn scenario(label: &str, units: usize, frames: usize, layered_every: usize, interval: f32) {
    let rig = shared_rig(SKELETON).expect("skeleton");
    let clips = ClipLibrary::default();
    let opt = SampleOptions::default();
    let run = clips.bound(&rig, "animations/nuian/male/fist/fist_mo_normal_run_f.caf", opt).expect("run clip");
    let cast = clips.bound(&rig, "animations/nuian/male/bow/bow_co_sk_launch_mub.caf", opt).expect("cast clip");
    let mask = BoneMask::upper_body(&rig).expect("upper body");

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, AnimPlugin)).insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_micros(16_667)));
    for i in 0..units {
        let mut commands = app.world_mut().commands();
        let root = commands.spawn((Transform::from_xyz(i as f32, 0.0, 0.0), Visibility::default())).id();
        let bones = spawn_bones(&mut commands, root, &rig);
        let mut animator = Animator::new(rig.clone(), bones);
        animator.play(run.clone(), true, 0.0);
        animator.interval = interval;
        if let Some(cur) = animator.current.as_mut() {
            cur.time = i as f32 * 0.037;
        }
        if layered_every > 0 && i % layered_every == 0 {
            animator.play_layer(mask.clone(), cast.clone(), true, 0.0);
        }
        commands.entity(root).insert(animator);
    }
    app.world_mut().flush();
    for _ in 0..20 {
        app.update(); // warm up (first frames build the schedules, spawn the thread pool tasks)
    }
    take_stats();
    let started = Instant::now();
    for _ in 0..frames {
        app.update();
    }
    let wall = started.elapsed();
    let (updated, nanos) = take_stats();
    println!(
        "{label:<34} animate {:6.1} us/frame ({:5.2} us per animator update)   whole update {:5.2} ms/frame",
        nanos as f64 / frames as f64 / 1000.0,
        nanos as f64 / f64::from(updated.max(1)) / 1000.0,
        wall.as_secs_f64() * 1000.0 / frames as f64,
    );
}

fn main() {
    if !aa_cry::pak_available() {
        eprintln!("the game pak is not available");
        return;
    }
    let units = arg("--units", 120);
    let frames = arg("--frames", 300);
    println!("{units} units x 162 bones, {frames} frames");
    scenario("no layers", units, frames, 0, 0.0);
    scenario("a layer on every 10th unit", units, frames, 10, 0.0);
    scenario("a layer on every unit", units, frames, 1, 0.0);
    scenario("no layers, 30 Hz", units, frames, 0, 1.0 / 30.0);
    scenario("a layer on every unit, 30 Hz", units, frames, 1, 1.0 / 30.0);
    scenario("no layers, 15 Hz", units, frames, 0, 1.0 / 15.0);
    scenario("a layer on every unit, 15 Hz", units, frames, 1, 1.0 / 15.0);
}
