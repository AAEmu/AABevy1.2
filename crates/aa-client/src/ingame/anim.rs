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

use std::sync::{Arc, OnceLock};

use aa_anim::{Animator, BoneMask, Cal, ClipLibrary, pose::{BoundClip, Rig, SampleOptions}};
use bevy::prelude::*;

use super::combat::{Acting, Dead};
use super::weapons::{Drawn, Out, Wield};

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
    /// The combat stances, bound when a unit first fights in one.
    stances: Arc<[OnceLock<Arc<StanceClips>>; 5]>,
}

/// The stance a unit fights in, from the weapon it holds: its idle and run clips are `<prefix>_ba_combat_idle` and
/// `<prefix>_mo_combat_run_<dir>` of the model's `.cal`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    /// One handed (and dual wield) weapons.
    Onehand,
    Twohand,
    /// The two handed staff: its `staff_` clips are the one handed ones.
    Staff,
    /// Instruments.
    Music,
    /// The bow and everything without a stance of its own (the `fist_` clips are the one handed ones).
    Fist,
}

impl Stance {
    const ALL: [Stance; 5] = [Stance::Onehand, Stance::Twohand, Stance::Staff, Stance::Music, Stance::Fist];

    fn prefix(self) -> &'static str {
        match self {
            Stance::Onehand => "onehand",
            Stance::Twohand => "twohand",
            Stance::Staff => "staff",
            Stance::Music => "music",
            Stance::Fist => "fist",
        }
    }

    /// The stance of a unit with `out` in its hand.
    pub fn of(out: Out, wield: Wield) -> Stance {
        match out {
            Out::Blade { .. } if wield.staff => Stance::Staff,
            Out::Blade { .. } if wield.two_handed => Stance::Twohand,
            Out::Blade { .. } => Stance::Onehand,
            Out::Instrument => Stance::Music,
            Out::Bow => Stance::Fist,
        }
    }
}

/// The clips of one stance: standing and running with the weapon out (a clip a stance lacks is the one handed one, else the
/// peaceful one).
pub struct StanceClips {
    idle: Option<BoundClip>,
    run: [Option<BoundClip>; 8],
    run_clip_speed: f32,
}

impl StanceClips {
    fn load(rig: &Rig, cal: &Cal, clips: &ClipLibrary, stance: Stance) -> StanceClips {
        let prefix = stance.prefix();
        let bind = |name: String, fallback: String| bind_clip(rig, cal, clips, &name).or_else(|| bind_clip(rig, cal, clips, &fallback));
        let base = if stance == Stance::Staff { "onehand" } else { "fist" };
        let idle = bind(format!("{prefix}_ba_combat_idle"), format!("{base}_ba_combat_idle"));
        let run: [Option<BoundClip>; 8] = std::array::from_fn(|i| bind(format!("{prefix}_mo_combat_run_{}", DIRS[i]), format!("{base}_mo_combat_run_{}", DIRS[i])));
        let run_clip_speed = run[0].as_ref().map_or(0.0, |c| ground_speed(rig, c));
        StanceClips { idle, run, run_clip_speed }
    }
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
        Some(ClipSet { cal, idle, upper: BoneMask::upper_body(rig), run, walk, jump, run_clip_speed, walk_clip_speed, stances: Default::default() })
    }

    /// The clips of a stance, bound the first time a unit of this model fights in it (the characters of a skeleton share them).
    pub(super) fn stance(&self, rig: &Rig, clips: &ClipLibrary, stance: Stance) -> Arc<StanceClips> {
        let i = Stance::ALL.iter().position(|s| *s == stance).unwrap_or(0);
        self.stances[i].get_or_init(|| Arc::new(StanceClips::load(rig, &self.cal, clips, stance))).clone()
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
    /// The combat stance the playing clip belongs to (`None`: the peaceful clips).
    stance: Option<Stance>,
}

impl Locomotion {
    pub fn new(clips: ClipSet) -> Locomotion {
        Locomotion { clips, state: State::Idle, stance: None }
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

/// Picks the clip for the unit's [`Motion`] and keeps the feet matched to its speed. A unit with its weapon out ([`Drawn`]) stands
/// and runs in the stance of the weapon until it puts it away; walking stays the peaceful walk.
pub fn drive_locomotion(
    time: Res<Time>,
    library: Res<ClipLibrary>,
    mut units: Query<(&mut Animator, &mut Locomotion, &Motion, Option<&Acting>, Option<&Drawn>, Option<&Wield>), Without<Dead>>,
) {
    let now = time.elapsed_secs_f64();
    for (mut animator, mut loco, motion, acting, drawn, wield) in &mut units {
        // an action on the whole body owns the clip; one on the upper body leaves the legs to locomotion
        if acting.is_some_and(|a| !a.layered) {
            continue;
        }
        let stance = drawn.and_then(|d| d.stance(now)).map(|out| Stance::of(out, wield.copied().unwrap_or_default()));
        if stance != loco.stance {
            loco.stance = stance;
            loco.state = State::Unknown;
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
        let stance_clips = stance.map(|s| loco.clips.stance(&animator.rig, &library, s));
        let stance_clips = stance_clips.as_deref();
        if wanted != loco.state {
            // fall back to the other speed set / forward clip when a direction has no clip
            let clip = match wanted {
                State::Unknown | State::Idle => stance_clips.and_then(|c| c.idle.clone()).or_else(|| Some(loco.clips.idle.clone())),
                State::Air => loco.clips.jump.clone(),
                State::Moving(d, walk) => {
                    let (primary, secondary) = if walk { (&loco.clips.walk, &loco.clips.run) } else { (&loco.clips.run, &loco.clips.walk) };
                    let combat_run = stance_clips.filter(|_| !walk).and_then(|c| c.run[d].clone());
                    combat_run.or_else(|| primary[d].clone()).or_else(|| secondary[d].clone()).or_else(|| primary[0].clone()).or_else(|| secondary[0].clone())
                }
            };
            if let Some(clip) = clip {
                animator.play(clip, true, 0.15);
                animator.random_idle = None;
            }
            loco.state = wanted;
        }
        animator.speed = match loco.state {
            State::Moving(d, walk) => {
                let combat_run = stance_clips.filter(|c| !walk && c.run[d].is_some());
                let clip_speed = match combat_run {
                    Some(c) if c.run_clip_speed > 0.1 => c.run_clip_speed,
                    _ if walk => loco.clips.walk_clip_speed,
                    _ => loco.clips.run_clip_speed,
                };
                if clip_speed > 0.1 { (motion.speed / clip_speed).clamp(0.4, 2.0) } else { 1.0 }
            }
            _ => 1.0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_weapon_in_hand_picks_the_stance() {
        let none = Wield::default();
        assert_eq!(Stance::of(Out::Blade { dual: false }, none), Stance::Onehand);
        assert_eq!(Stance::of(Out::Blade { dual: true }, none), Stance::Onehand);
        assert_eq!(Stance::of(Out::Blade { dual: false }, Wield { two_handed: true, ..none }), Stance::Twohand);
        assert_eq!(Stance::of(Out::Instrument, none), Stance::Music);
        assert_eq!(Stance::of(Out::Bow, none), Stance::Fist);
        // every stance has a slot
        assert_eq!(Stance::ALL.len(), 5);
        assert_eq!(Stance::of(Out::Blade { dual: false }, Wield { two_handed: true, staff: true, ..none }), Stance::Staff);
    }
}
