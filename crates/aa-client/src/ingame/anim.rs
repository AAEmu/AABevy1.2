//! Locomotion for characters and creatures: simple clip switching on top of aa-anim's [`Animator`].
//!
//! aa-anim parses the `.lmg` locomotion groups but only plays the stage idle; the client picks
//! the clips by name here (`fist_ba_relaxed_idle`, `fist_mo_normal_run_f`, ... straight from the
//! model's `.cal`), cross-fades between them and scales the playback speed so the feet follow the
//! ground speed.

use std::sync::Arc;

use aa_anim::{Animator, Cal, ClipLibrary, pose::{BoundClip, Rig, SampleOptions}};
use bevy::prelude::*;

/// Direction suffixes of the 8-way locomotion clips; index = [`direction_index`].
const DIRS: [&str; 8] = ["f", "fl", "fr", "l", "r", "b", "bl", "br"];

/// Speeds (m/s) the server runs and walks the player at when a clip has no root motion to measure.
pub const RUN_SPEED: f32 = 5.4;
pub const WALK_SPEED: f32 = 2.4;

/// The clips a model can play while moving.
pub struct ClipSet {
    pub idle: BoundClip,
    run: [Option<BoundClip>; 8],
    walk: [Option<BoundClip>; 8],
    jump: Option<BoundClip>,
    /// Ground speed (m/s) the forward run / walk clips cover per second of playback.
    run_clip_speed: f32,
    walk_clip_speed: f32,
}

impl ClipSet {
    /// Binds the clips of `cal` to `rig`. `None` when the model has no idle at all.
    pub fn load(rig: &Arc<Rig>, cal: &Cal, clips: &ClipLibrary) -> Option<ClipSet> {
        let bind = |name: &str| -> Option<BoundClip> {
            let entry = cal.animation(name)?;
            let options = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
            clips.bound(rig, &entry.path, options).ok()
        };
        let idle = bind("fist_ba_relaxed_idle").or_else(|| bind("fist_ba_idle")).or_else(|| bind("fist_ac_stage_idle"))?;
        let dirs = |prefix: &str| -> [Option<BoundClip>; 8] { std::array::from_fn(|i| bind(&format!("{prefix}_{}", DIRS[i]))) };
        let run = dirs("fist_mo_normal_run");
        let walk = dirs("fist_mo_normal_walk");
        let jump = bind("fist_mo_relaxed_jump_s_loop");
        let run_clip_speed = run[0].as_ref().map_or(0.0, |c| ground_speed(rig, c));
        let walk_clip_speed = walk[0].as_ref().map_or(0.0, |c| ground_speed(rig, c));
        Some(ClipSet { idle, run, walk, jump, run_clip_speed, walk_clip_speed })
    }

    pub fn can_move(&self) -> bool {
        self.run[0].is_some() || self.walk[0].is_some()
    }
}

/// Metres per second the root bone travels in a (forward) locomotion clip.
fn ground_speed(rig: &Rig, clip: &BoundClip) -> f32 {
    let root = rig.skeleton.bones.get(rig.root).map(|b| b.controller_id);
    let Some(track) = root.and_then(|c| clip.clip.track(c)) else { return 0.0 };
    let (Some(a), Some(b)) = (track.sample_pos(0.0), track.sample_pos(clip.clip.duration)) else { return 0.0 };
    let travelled = Vec2::new(b.x - a.x, b.y - a.y).length() * clip.options.position_scale;
    if clip.clip.duration > 0.0 { travelled / clip.clip.duration } else { 0.0 }
}

/// What a unit is doing, as far as the animation cares. Written by whatever moves the unit.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Motion {
    /// Ground speed (m/s).
    pub speed: f32,
    /// Direction of travel relative to where the unit faces: x right, y forward (unit length, or
    /// zero when standing).
    pub local_dir: Vec2,
    pub airborne: bool,
}

/// The clips of a model and what it is currently playing.
#[derive(Component)]
pub struct Locomotion {
    pub clips: ClipSet,
    state: State,
}

impl Locomotion {
    pub fn new(clips: ClipSet) -> Locomotion {
        Locomotion { clips, state: State::Idle }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    /// Direction index and whether it is the walk set.
    Moving(usize, bool),
    Air,
}

/// Index into [`DIRS`] of the 8-way direction closest to `local_dir` (x right, y forward).
pub fn direction_index(local_dir: Vec2) -> usize {
    // angle 0 = forward, positive to the right
    let a = local_dir.x.atan2(local_dir.y).to_degrees();
    match a {
        a if (-22.5..22.5).contains(&a) => 0,
        a if (-67.5..-22.5).contains(&a) => 1,
        a if (22.5..67.5).contains(&a) => 2,
        a if (-112.5..-67.5).contains(&a) => 3,
        a if (67.5..112.5).contains(&a) => 4,
        a if (-157.5..-112.5).contains(&a) => 6,
        a if (112.5..157.5).contains(&a) => 7,
        _ => 5,
    }
}

/// Picks the clip for the unit's [`Motion`] and keeps the feet matched to its speed.
pub fn drive_locomotion(mut units: Query<(&mut Animator, &mut Locomotion, &Motion)>) {
    for (mut animator, mut loco, motion) in &mut units {
        let moving = motion.speed > 0.3 && loco.clips.can_move();
        let walk = moving && motion.speed < (WALK_SPEED + RUN_SPEED) * 0.5;
        let wanted = if motion.airborne && loco.clips.jump.is_some() {
            State::Air
        } else if moving {
            State::Moving(direction_index(motion.local_dir), walk)
        } else {
            State::Idle
        };
        if wanted != loco.state {
            // fall back to the other speed set / forward clip when a direction has no clip
            let clip = match wanted {
                State::Idle => Some(loco.clips.idle.clone()),
                State::Air => loco.clips.jump.clone(),
                State::Moving(d, walk) => {
                    let (primary, secondary) = if walk { (&loco.clips.walk, &loco.clips.run) } else { (&loco.clips.run, &loco.clips.walk) };
                    primary[d].clone().or_else(|| secondary[d].clone()).or_else(|| primary[0].clone()).or_else(|| secondary[0].clone())
                }
            };
            if let Some(clip) = clip {
                animator.play(clip, true, 0.15);
                animator.random_idle = None;
            }
            loco.state = wanted;
        }
        animator.speed = match loco.state {
            State::Moving(_, walk) => {
                let clip_speed = if walk { loco.clips.walk_clip_speed } else { loco.clips.run_clip_speed };
                if clip_speed > 0.1 { (motion.speed / clip_speed).clamp(0.4, 2.0) } else { 1.0 }
            }
            _ => 1.0,
        };
    }
}
