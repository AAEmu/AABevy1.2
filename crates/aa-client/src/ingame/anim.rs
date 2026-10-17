//! Locomotion for characters and creatures: simple clip switching on top of aa-anim's [`Animator`].
//!
//! aa-anim parses the `.lmg` locomotion groups but only plays the stage idle; the client picks
//! the clips by name here (`fist_ba_relaxed_idle`, `fist_mo_normal_run_f`, ... straight from the
//! model's `.cal`), cross-fades between them and scales the playback speed so the feet follow the
//! ground speed.
//!
//! [`super::combat`] plays attack, cast, hit and death clips on top of it: a unit that is [`Acting`] or [`Dead`] is left alone by
//! [`drive_locomotion`], and [`Locomotion::invalidate`] makes it pick its clip again when the action is over. A unit that acts
//! while it moves plays the clip on its upper body (an [`aa_anim::Animator`] layer, [`Acting::layered`]); locomotion keeps driving
//! the legs underneath.

use std::sync::Arc;

use aa_anim::{Animator, BoneMask, Cal, ClipLibrary, pose::{BoundClip, Rig, SampleOptions}};
use bevy::prelude::*;

use super::combat::{Acting, Dead};

/// Direction suffixes of the 8-way locomotion clips; index = [`direction_index`].
const DIRS: [&str; 8] = ["f", "fl", "fr", "l", "r", "b", "bl", "br"];

/// Speeds (m/s) the server runs and walks the player at when a clip has no root motion to measure.
pub const RUN_SPEED: f32 = 5.4;
pub const WALK_SPEED: f32 = 2.4;

/// Binds the clip called `name` in `cal` (a `.cal` entry, e.g. `fist_mo_normal_run_f`) to `rig`.
pub fn bind_clip(rig: &Rig, cal: &Cal, clips: &ClipLibrary, name: &str) -> Option<BoundClip> {
    bind_clip_with(rig, cal, clips, name, true)
}

/// An action clip (a swing, a kick, a lunge): it moves the unit's body (the root bone) the way the clip says, as the real client
/// does (a kick steps about a metre forward and comes back; the unit's own position, the camera and the server's idea of it stay
/// where they are), unless the clip travels far (a dash or a charge: the server moves the unit then, the body stays with it).
pub fn bind_action_clip(rig: &Rig, cal: &Cal, clips: &ClipLibrary, name: &str) -> Option<BoundClip> {
    let locked = bind_clip(rig, cal, clips, name)?;
    if root_reach(rig, &locked) <= ROOT_REACH_MAX {
        bind_clip_with(rig, cal, clips, name, false).or(Some(locked))
    } else {
        Some(locked)
    }
}

/// The farthest an action clip may take the body from where it starts (m) and still be played with its root motion.
const ROOT_REACH_MAX: f32 = 2.5;

/// The farthest the root bone of a clip gets from its first position (m).
fn root_reach(rig: &Rig, clip: &BoundClip) -> f32 {
    let root = rig.skeleton.bones.get(rig.root).map(|b| b.controller_id);
    let Some(track) = root.and_then(|c| clip.clip.track(c)) else { return 0.0 };
    let Some(first) = track.sample_pos(0.0) else { return 0.0 };
    let steps = 24;
    (1..=steps)
        .filter_map(|i| track.sample_pos(clip.clip.duration * i as f32 / steps as f32))
        .map(|p| Vec2::new(p.x - first.x, p.y - first.y).length())
        .fold(0.0, f32::max)
        * clip.options.position_scale
}

/// [`bind_clip`] that lets the clip move the unit's body (the root bone) when `lock_root` is off.
pub fn bind_clip_with(rig: &Rig, cal: &Cal, clips: &ClipLibrary, name: &str, lock_root: bool) -> Option<BoundClip> {
    let entry = cal.animation(name)?;
    let options = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: lock_root };
    let bound = clips.bound(rig, &entry.path, options).ok()?;
    remember_file(&bound, &entry.path);
    Some(bound)
}

/// The `.caf` file of every clip the client bound, by the address of its data (the clip library never drops one): aa-anim's clips
/// carry the name from their file header, not their path, and the sound events of a clip are listed by file name.
fn files() -> &'static std::sync::Mutex<std::collections::HashMap<usize, Arc<str>>> {
    static FILES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<usize, Arc<str>>>> = std::sync::OnceLock::new();
    FILES.get_or_init(Default::default)
}

fn remember_file(clip: &BoundClip, path: &str) {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase();
    files().lock().unwrap_or_else(|e| e.into_inner()).entry(Arc::as_ptr(&clip.clip) as usize).or_insert_with(|| Arc::from(name.as_str()));
}

/// The file name (`fist_mo_normal_run_f.caf`, lower case) of a clip the client bound.
pub fn clip_file(clip: &BoundClip) -> Option<Arc<str>> {
    files().lock().unwrap_or_else(|e| e.into_inner()).get(&(Arc::as_ptr(&clip.clip) as usize)).cloned()
}

/// The clips a model can play while moving.
#[derive(Clone)]
pub struct ClipSet {
    /// The model's animation name table: every clip of the model by name (combat plays attack and hit clips from it).
    pub cal: Arc<Cal>,
    pub idle: BoundClip,
    /// The bones an action plays on while the unit moves: the upper body (`None` for a rig without a `Bip01 Spine`).
    pub upper: Option<BoneMask>,
    run: [Option<BoundClip>; 8],
    walk: [Option<BoundClip>; 8],
    jump: Option<BoundClip>,
    /// Ground speed (m/s) the forward run / walk clips cover per second of playback.
    run_clip_speed: f32,
    walk_clip_speed: f32,
}

impl ClipSet {
    /// Binds the clips of `cal` to `rig`. `None` when the model has no idle at all.
    pub fn load(rig: &Arc<Rig>, cal: Arc<Cal>, clips: &ClipLibrary) -> Option<ClipSet> {
        let bind = |name: &str| bind_clip(rig, &cal, clips, name);
        let idle = bind("fist_ba_relaxed_idle").or_else(|| bind("fist_ba_idle")).or_else(|| bind("fist_ac_stage_idle"))?;
        let dirs = |prefix: &str| -> [Option<BoundClip>; 8] { std::array::from_fn(|i| bind(&format!("{prefix}_{}", DIRS[i]))) };
        let run = dirs("fist_mo_normal_run");
        let walk = dirs("fist_mo_normal_walk");
        let jump = bind("fist_mo_relaxed_jump_s_loop");
        let run_clip_speed = run[0].as_ref().map_or(0.0, |c| ground_speed(rig, c));
        let walk_clip_speed = walk[0].as_ref().map_or(0.0, |c| ground_speed(rig, c));
        Some(ClipSet { cal, idle, upper: BoneMask::upper_body(rig), run, walk, jump, run_clip_speed, walk_clip_speed })
    }

    pub fn can_move(&self) -> bool {
        self.run[0].is_some() || self.walk[0].is_some()
    }
}

/// The clips of the model whose skeleton and `.cal` are given, bound once and shared (any thread); `None` when the model has
/// no idle clip. Characters of one skeleton share a [`Rig`], so the bindings are the same for all of them.
pub fn shared_clip_set(skeleton: &str, cal: &str, clips: &ClipLibrary) -> Option<Arc<ClipSet>> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Sets = Mutex<HashMap<(String, String), Option<Arc<ClipSet>>>>;
    static SETS: OnceLock<Sets> = OnceLock::new();
    let key = (skeleton.to_string(), cal.to_string());
    let sets = SETS.get_or_init(Default::default);
    if let Some(set) = sets.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return set.clone();
    }
    let _t = aa_cry::prof::scope("anim.clip_set");
    let set = aa_anim::pose::shared_rig(skeleton)
        .ok()
        .zip(Cal::shared(cal).ok())
        .and_then(|(rig, cal)| ClipSet::load(&rig, cal, clips))
        .map(Arc::new);
    sets.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(set).clone()
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

    /// Forgets which clip is playing: the next [`drive_locomotion`] starts the right one (after something else, an attack or a
    /// hit reaction, played on the animator).
    pub fn invalidate(&mut self) {
        self.state = State::Unknown;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// Something else played last: the clip has to be started whatever the unit does.
    Unknown,
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
pub fn drive_locomotion(mut units: Query<(&mut Animator, &mut Locomotion, &Motion, Option<&Acting>), Without<Dead>>) {
    for (mut animator, mut loco, motion, acting) in &mut units {
        // an action on the whole body owns the clip; one on the upper body leaves the legs to locomotion
        if acting.is_some_and(|a| !a.layered) {
            continue;
        }
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
                State::Unknown | State::Idle => Some(loco.clips.idle.clone()),
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
