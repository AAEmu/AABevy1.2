//! Playing clips on a spawned skeleton: the [`Animator`] component and [`AnimPlugin`].
//!
//! The skeleton of a character is an entity hierarchy with one entity per bone (see
//! [`spawn_bones`]); the [`Animator`] on the character root owns the clip state and writes the
//! bones' local `Transform`s every frame. Skinned meshes reference the bone entities as joints,
//! so Bevy's own skinning and transform propagation do the rest.
//!
//! The sampler is deliberately simple (no `AnimationGraph`): one current clip, an optional
//! cross-fade from the previous pose, an optional clip to run once and then return from, and an
//! optional random-idle schedule, which is all the login stage and the in-game idle need. One
//! partial-body layer can play over it ([`Animator::play_layer`]): a cast on the upper body while the
//! legs keep running.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::ecs::batching::BatchingStrategy;
use bevy::prelude::*;

use crate::caf::Clip;
use crate::pose::{BoneMask, BoundClip, ClipBinding, Pose, Rig, SampleOptions};
use crate::rope::RopeSim;

/// One clip being played.
#[derive(Clone, Debug)]
pub struct Playing {
    pub clip: BoundClip,
    /// Seconds into the clip.
    pub time: f32,
    pub looping: bool,
    /// Loops completed (for [`RandomIdle`]).
    pub loops: u32,
}

impl Playing {
    pub fn new(clip: BoundClip, looping: bool) -> Playing {
        Playing { clip, time: 0.0, looping, loops: 0 }
    }

    fn advance(&mut self, dt: f32) {
        let d = self.clip.clip.duration;
        self.time += dt;
        if d <= 0.0 {
            return;
        }
        if self.looping {
            while self.time >= d {
                self.time -= d;
                self.loops += 1;
            }
        } else if self.time > d {
            self.time = d;
        }
    }

    pub fn finished(&self) -> bool {
        !self.looping && self.time >= self.clip.clip.duration
    }

    fn sample(&self, pose: &mut Pose, rig: &Rig) {
        pose.sample(rig, &self.clip.clip, &self.clip.binding, self.time, self.clip.options);
    }
}

/// Plays `variants` now and then while the idle loops: after `every_loops` loops of the idle
/// (jittered), one random variant runs once, then the idle resumes.
#[derive(Clone, Debug)]
pub struct RandomIdle {
    pub variants: Vec<BoundClip>,
    pub every_loops: u32,
    seed: u32,
    countdown: u32,
}

impl RandomIdle {
    pub fn new(variants: Vec<BoundClip>, every_loops: u32, seed: u32) -> RandomIdle {
        let mut r = RandomIdle { variants, every_loops: every_loops.max(1), seed: seed.wrapping_mul(2654435761).wrapping_add(1), countdown: 0 };
        r.countdown = r.next_countdown();
        r
    }

    fn rand(&mut self) -> u32 {
        // xorshift32
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        x
    }

    fn next_countdown(&mut self) -> u32 {
        self.every_loops + self.rand() % self.every_loops.max(1)
    }
}

/// A clip playing over part of the body, on top of the base clip.
#[derive(Clone, Debug)]
struct Layer {
    mask: BoneMask,
    playing: Playing,
    /// Seconds to fade in, and out at the end of a one-shot or after [`Animator::stop_layer`].
    fade: f32,
    /// Seconds since it started (a layer that replaces another starts at the old one's weight).
    age: f32,
    /// Fading out after a stop: the weight when the fade began, the seconds since, its length.
    stopping: Option<(f32, f32, f32)>,
    weight: f32,
}

impl Layer {
    /// Moves the clip and the weight on by `dt` real seconds. `false` once the layer is over.
    fn advance(&mut self, dt: f32) -> bool {
        self.playing.advance(dt);
        self.age += dt;
        let fade = self.fade;
        let ramp = |x: f32| if fade > 0.0 { (x / fade).clamp(0.0, 1.0) } else { 1.0 };
        let mut w = ramp(self.age);
        let mut alive = true;
        if !self.playing.looping {
            w = w.min(ramp(self.playing.clip.clip.duration - self.playing.time));
            alive = !self.playing.finished();
        }
        if let Some((from, t, len)) = &mut self.stopping {
            *t += dt;
            w = w.min(*from * if *len > 0.0 { 1.0 - (*t / *len).min(1.0) } else { 0.0 });
            alive &= *t < *len;
        }
        self.weight = w;
        alive
    }
}

/// Animation state of one character; lives on the character root.
#[derive(Component)]
pub struct Animator {
    pub rig: Arc<Rig>,
    /// Bone entities, indexed like `rig.skeleton.bones`.
    pub bones: Vec<Entity>,
    pub current: Option<Playing>,
    /// Clip to start when `current` finishes (non-looping), usually the idle.
    pub then: Option<Playing>,
    pub random_idle: Option<RandomIdle>,
    /// Base clip playback speed multiplier. Cross-fades and the partial-body layer use real seconds.
    pub speed: f32,
    /// Stops clip and transition clocks. Explicitly selected times can still be sampled.
    pub paused: bool,
    /// Gameplay framehold blocks starts independently of a manually paused preview.
    frameheld: bool,
    /// Seconds between updates (0 = every frame): far characters animate at a lower rate, which saves sampling, bone
    /// transform propagation and skin extraction. The clip time still advances in real time.
    pub interval: f32,
    /// Time since the last update.
    pub accum: f32,
    /// The pose changed and its bones still have to be written.
    fresh: bool,
    fade: Option<Fade>,
    layer: Option<Layer>,
    /// The idle that `random_idle` interrupts.
    base_idle: Option<Playing>,
    pose: Pose,
    scratch: Pose,
}

#[derive(Clone, Debug)]
struct Fade {
    from: Pose,
    elapsed: f32,
    duration: f32,
}

impl Animator {
    #[cfg(test)]
    pub(crate) fn playback_snapshot(&self) -> String {
        format!("{:?}", (self.rig.id, &self.bones, &self.current, &self.then, &self.random_idle,
            (self.speed, self.paused, self.frameheld, self.interval, self.accum, self.fresh), &self.fade, &self.layer,
            &self.base_idle, &self.pose, &self.scratch))
    }

    pub fn new(rig: Arc<Rig>, bones: Vec<Entity>) -> Animator {
        let pose = rig.bind.clone();
        let scratch = rig.bind.clone();
        Animator {
            rig,
            bones,
            current: None,
            then: None,
            random_idle: None,
            speed: 1.0,
            paused: false,
            frameheld: false,
            interval: 0.0,
            accum: 0.0,
            fresh: false,
            fade: None,
            layer: None,
            base_idle: None,
            pose,
            scratch,
        }
    }

    /// Sets the gameplay framehold: stops clip/transition clocks and rejects new starts.
    /// Manual [`Animator::paused`] and [`Animator::speed`] are retained on release.
    pub fn set_framehold(&mut self, held: bool) {
        self.frameheld = held;
    }

    /// Whether a new clip can start. Producers must check before changing tools,
    /// stopping layers, or recording a clip selection as successful.
    /// Manual pause and zero playback speed still allow starts.
    pub fn starts_enabled(&self) -> bool {
        !self.frameheld
    }

    /// Starts `clip`, cross-fading from the current pose over `fade` seconds.
    /// Does nothing during gameplay framehold.
    pub fn play(&mut self, clip: BoundClip, looping: bool, fade: f32) {
        if !self.starts_enabled() {
            return;
        }
        if fade > 0.0 && self.current.is_some() {
            self.fade = Some(Fade { from: self.pose.clone(), elapsed: 0.0, duration: fade });
        } else {
            self.fade = None;
        }
        let p = Playing::new(clip, looping);
        if looping {
            self.base_idle = Some(p.clone());
        }
        self.current = Some(p);
        self.then = None;
    }

    /// Plays `clip` once, then `after` (looping). Does nothing during gameplay framehold,
    /// including leaving an existing queued return clip intact.
    pub fn play_once_then(&mut self, clip: BoundClip, after: BoundClip, fade: f32) {
        if !self.starts_enabled() {
            return;
        }
        self.play(clip, false, fade);
        self.then = Some(Playing::new(after, true));
    }

    /// Plays `clip` over the base clip on the bones `mask` selects (the base clip keeps playing everywhere else, and underneath),
    /// fading in over `fade` seconds. A one-shot layer (`looping == false`) fades out over `fade` seconds at its end and removes
    /// itself; a looping one plays until [`Animator::stop_layer`]. A layer already playing is replaced, the new one continuing
    /// from its weight. The layer runs in real time: [`Animator::speed`] (which matches the base clip to the ground speed) does
    /// not apply to it, manual pause and gameplay framehold do. Does nothing during framehold.
    pub fn play_layer(&mut self, mask: BoneMask, clip: BoundClip, looping: bool, fade: f32) {
        if !self.starts_enabled() {
            return;
        }
        let weight = self.layer.as_ref().map_or(0.0, |l| l.weight);
        self.layer = Some(Layer { mask, playing: Playing::new(clip, looping), fade, age: weight * fade, stopping: None, weight });
    }

    /// Fades the layer out over `fade` seconds and removes it (at once with 0).
    /// Stopping is permitted during framehold; a timed fade waits for clocks to resume.
    pub fn stop_layer(&mut self, fade: f32) {
        if fade <= 0.0 {
            self.layer = None;
        } else if let Some(l) = &mut self.layer {
            // a layer that is already going out is left to it
            l.stopping.get_or_insert((l.weight, 0.0, fade));
        }
    }

    /// Whether a layer is playing (fading out ones included).
    pub fn has_layer(&self) -> bool {
        self.layer.is_some()
    }

    /// The pose written to the bones last frame.
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// Advances time and refreshes [`Animator::pose`]. Separate from the bevy system so tests can
    /// drive it without an `App`.
    pub fn update(&mut self, dt: f32) {
        let real_dt = if self.paused || self.frameheld { 0.0 } else { dt };
        let dt = real_dt * self.speed;
        if let Some(cur) = &mut self.current {
            cur.advance(dt);
            let mut start_variant = None;
            if cur.finished() {
                if let Some(next) = self.then.take() {
                    self.fade = Some(Fade { from: self.pose.clone(), elapsed: 0.0, duration: 0.2 });
                    *cur = next;
                }
            } else if cur.looping {
                if let Some(ri) = &mut self.random_idle {
                    if cur.loops >= ri.countdown && !ri.variants.is_empty() {
                        let i = ri.rand() as usize % ri.variants.len();
                        start_variant = Some(ri.variants[i].clone());
                        ri.countdown = ri.next_countdown();
                    }
                }
            }
            if let (Some(v), Some(idle)) = (start_variant, self.base_idle.clone()) {
                self.fade = Some(Fade { from: self.pose.clone(), elapsed: 0.0, duration: 0.25 });
                *cur = Playing::new(v, false);
                self.then = Some(Playing::new(idle.clip, true));
            }
        }
        self.pose.clone_from(&self.rig.bind);
        if let Some(cur) = &self.current {
            cur.sample(&mut self.pose, &self.rig);
        }
        if let Some(f) = &mut self.fade {
            // Native FIFO transitions use the layer clock, independently of the clip's speed.
            f.elapsed += real_dt.abs();
            let w = (f.elapsed / f.duration).clamp(0.0, 1.0);
            // blend from the frozen previous pose into the live pose
            self.scratch.clone_from(&f.from);
            self.scratch.blend_from(&self.pose, w);
            std::mem::swap(&mut self.scratch, &mut self.pose);
            if w >= 1.0 {
                self.fade = None;
            }
        }
        if let Some(layer) = &mut self.layer {
            if layer.advance(real_dt) {
                let (c, w) = (&layer.playing, layer.weight);
                self.pose.sample_masked(&self.rig, &c.clip.clip, &c.clip.binding, c.time, c.clip.options, &layer.mask, w);
            } else {
                self.layer = None;
            }
        }
    }
}

/// Registers the animation system. Add once.
pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        use crate::rope::{RopeSettings, attach_ropes, mask_ropes, track_ropes};
        app.init_resource::<ClipLibrary>()
            .init_resource::<RopeSettings>()
            .add_systems(PostUpdate, (attach_ropes, mask_ropes, track_ropes, animate).chain().before(TransformSystems::Propagate));
    }
}

static UPDATED: AtomicU32 = AtomicU32::new(0);
static NANOS: AtomicU64 = AtomicU64::new(0);

/// Animators updated and nanoseconds spent in the animation system since the last call (for profiling).
pub fn take_stats() -> (u32, u64) {
    (UPDATED.swap(0, Ordering::Relaxed), NANOS.swap(0, Ordering::Relaxed))
}

pub(crate) fn animate(time: Res<Time>, mut animators: Query<(&mut Animator, Option<&mut RopeSim>)>, mut transforms: Query<&mut Transform>) {
    let started = Instant::now();
    let dt = time.delta_secs();
    // sample the poses of the characters that are due (in parallel: a crowd is hundreds of bones each)
    animators.par_iter_mut().batching_strategy(BatchingStrategy::fixed(8)).for_each(|(mut a, rope)| {
        if a.current.is_none() {
            return;
        }
        a.accum += dt;
        if a.accum < a.interval {
            return;
        }
        let step = std::mem::take(&mut a.accum);
        a.update(step);
        // rope bones (robes, cloaks, skirts) follow the sampled pose with their own motion
        if let Some(mut rope) = rope {
            let a = &mut *a;
            rope.apply(&a.rig, &mut a.pose, step);
        }
        a.fresh = true;
    });
    let mut updated = 0;
    for (mut a, _) in &mut animators {
        if !a.fresh {
            continue;
        }
        a.fresh = false;
        updated += 1;
        let a = &*a;
        for (i, &e) in a.bones.iter().enumerate() {
            if let Ok(mut t) = transforms.get_mut(e) {
                t.rotation = a.pose.rot[i];
                t.translation = a.pose.pos[i];
            }
        }
    }
    UPDATED.fetch_add(updated, Ordering::Relaxed);
    NANOS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

/// Spawns one entity per bone under `parent` (bind pose, `Name`d) and returns them in bone order.
pub fn spawn_bones(commands: &mut Commands, parent: Entity, rig: &Rig) -> Vec<Entity> {
    let mut entities: Vec<Entity> = Vec::with_capacity(rig.len());
    for (i, bone) in rig.skeleton.bones.iter().enumerate() {
        let transform = Transform {
            translation: rig.bind.pos[i],
            rotation: rig.bind.rot[i],
            scale: Vec3::ONE,
        };
        let parent_entity = bone.parent.map(|p| entities[p]).unwrap_or(parent);
        let e = commands
            .spawn((Name::new(bone.name.clone()), transform, Visibility::default(), ChildOf(parent_entity)))
            .id();
        entities.push(e);
    }
    entities
}

/// Decoded clips shared between characters, by pak path.
#[derive(Resource, Default, Clone)]
pub struct ClipLibrary {
    clips: Arc<Mutex<HashMap<String, Arc<Clip>>>>,
    /// Bindings by rig and clip: characters sharing a rig bind each clip once.
    bindings: Arc<Mutex<HashMap<(u64, String), Arc<ClipBinding>>>>,
}

impl ClipLibrary {
    /// Loads (once) the `.caf` at `path`.
    pub fn get(&self, path: &str) -> crate::Result<Arc<Clip>> {
        let key = path.to_ascii_lowercase().replace('\\', "/");
        if let Some(c) = self.clips.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let _t = aa_cry::prof::scope("anim.clip_load");
        let clip = Arc::new(Clip::load(&key)?);
        self.clips.lock().unwrap().insert(key, clip.clone());
        Ok(clip)
    }

    /// Loads a clip and binds it to a rig.
    pub fn bound(&self, rig: &Rig, path: &str, options: SampleOptions) -> crate::Result<BoundClip> {
        let clip = self.get(path)?;
        let key = (rig.id, path.to_ascii_lowercase().replace('\\', "/"));
        let cached = self.bindings.lock().unwrap().get(&key).cloned();
        let binding = match cached {
            Some(b) => b,
            None => {
                let b = Arc::new(rig.bind_clip(&clip));
                self.bindings.lock().unwrap().insert(key.clone(), b.clone());
                b
            }
        };
        Ok(BoundClip { source: Some(key.1.into()), clip, binding, options })
    }
}

#[cfg(test)]
#[path = "animator/clock_tests.rs"]
mod clock_tests;

#[cfg(test)]
#[path = "animator/framehold_tests.rs"]
mod framehold_tests;

#[cfg(test)]
mod tests {
    use super::*;

    const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";

    fn bound(rig: &Rig, clips: &ClipLibrary, name: &str) -> BoundClip {
        clips.bound(rig, &format!("animations/nuian/male/{name}.caf"), SampleOptions::default()).unwrap()
    }

    fn animator(rig: &Arc<Rig>) -> Animator {
        Animator::new(rig.clone(), Vec::new())
    }

    fn bone(rig: &Rig, name: &str) -> usize {
        rig.skeleton.bones.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"))
    }

    #[test]
    fn upper_body_mask_keeps_the_legs_out() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = Rig::new(aa_cry::Skeleton::load(SKELETON).unwrap());
        let mask = BoneMask::upper_body(&rig).unwrap();
        for name in ["Bip01 Spine", "Bip01 Spine3", "Bip01 Neck", "Bip01 Head", "Bip01 L Clavicle", "Bip01 R Hand", "Bip01 L Finger21", "item_hand_r"] {
            assert_eq!(mask.weight(bone(&rig, name)), 1.0, "{name}");
        }
        for name in ["Bip01", "Bip01 Pelvis", "Bip01 L Thigh", "Bip01 R Foot", "Bip01 R Toe0"] {
            assert_eq!(mask.weight(bone(&rig, name)), 0.0, "{name}");
        }
        assert!(BoneMask::subtree(&rig, "no such bone").is_none());
    }

    /// A full-body cast clip on the upper-body mask changes the arms and leaves the legs the base clip's.
    #[test]
    fn layer_overrides_the_upper_body_only() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = shared_rig_for_test();
        let clips = ClipLibrary::default();
        let run = bound(&rig, &clips, "fist/fist_mo_normal_run_f");
        let cast = bound(&rig, &clips, "bow/bow_co_sk_cast_ub");
        let mask = BoneMask::upper_body(&rig).unwrap();

        let mut base = animator(&rig);
        base.play(run.clone(), true, 0.0);
        base.update(0.2);

        let mut layered = animator(&rig);
        layered.play(run, true, 0.0);
        layered.play_layer(mask, cast.clone(), true, 0.0);
        layered.update(0.2);

        // what the layer clip alone gives at its own time
        let mut alone = rig.bind.clone();
        alone.sample(&rig, &cast.clip, &cast.binding, 0.2, cast.options);

        let (hand, thigh, foot, pelvis) = (bone(&rig, "Bip01 R Hand"), bone(&rig, "Bip01 L Thigh"), bone(&rig, "Bip01 R Foot"), bone(&rig, "Bip01 Pelvis"));
        for b in [thigh, foot, pelvis] {
            assert_eq!(layered.pose().rot[b], base.pose().rot[b], "{}", rig.skeleton.bones[b].name);
            assert_eq!(layered.pose().pos[b], base.pose().pos[b], "{}", rig.skeleton.bones[b].name);
        }
        assert_eq!(layered.pose().rot[hand], alone.rot[hand]);
        assert!(layered.pose().rot[hand].angle_between(base.pose().rot[hand]) > 0.05, "the hand should differ from the run's");
    }

    /// Fade in, a one-shot ends by itself, a looping layer stops when told to; the base clip is not touched by any of it.
    #[test]
    fn layer_fades_and_clears() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = shared_rig_for_test();
        let clips = ClipLibrary::default();
        let idle = bound(&rig, &clips, "fist/fist_ba_relaxed_idle");
        let shot = bound(&rig, &clips, "bow/bow_co_sk_launch_mub");
        let length = shot.clip.duration;
        let mask = BoneMask::upper_body(&rig).unwrap();
        let hand = bone(&rig, "Bip01 R Hand");

        let mut a = animator(&rig);
        a.play(idle.clone(), true, 0.0);
        // the same base clip without a layer, advanced by the same times
        let mut plain = animator(&rig);
        plain.play(idle, true, 0.0);
        let step = |a: &mut Animator, plain: &mut Animator, dt: f32| {
            a.update(dt);
            plain.update(dt);
        };
        step(&mut a, &mut plain, 0.0);

        // fades in: after half the fade the hand is part way
        a.play_layer(mask.clone(), shot.clone(), false, 0.2);
        step(&mut a, &mut plain, 0.1);
        assert!(a.has_layer());
        let half = a.layer.as_ref().unwrap().weight;
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        // full weight in the middle, then it fades out and removes itself after the clip
        step(&mut a, &mut plain, length * 0.5);
        assert_eq!(a.layer.as_ref().unwrap().weight, 1.0);
        assert!(a.pose().rot[hand].angle_between(plain.pose().rot[hand]) > 0.05);
        step(&mut a, &mut plain, length);
        assert!(!a.has_layer());
        step(&mut a, &mut plain, 0.0);
        assert_eq!(a.pose().rot[hand], plain.pose().rot[hand], "back to the base clip");

        // a looping layer stays until stopped, then fades out
        a.play_layer(mask, shot, true, 0.1);
        a.update(length * 3.0);
        assert!(a.has_layer());
        a.stop_layer(0.2);
        a.update(0.1);
        assert!(a.has_layer());
        assert!((a.layer.as_ref().unwrap().weight - 0.5).abs() < 1e-3);
        a.update(0.2);
        assert!(!a.has_layer());
    }

    /// The layer runs in real time while `speed` retimes the base clip (the run cycle follows the ground speed).
    #[test]
    fn layer_ignores_the_base_speed() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = shared_rig_for_test();
        let clips = ClipLibrary::default();
        let mut a = animator(&rig);
        a.play(bound(&rig, &clips, "fist/fist_mo_normal_run_f"), true, 0.0);
        a.play_layer(BoneMask::upper_body(&rig).unwrap(), bound(&rig, &clips, "bow/bow_co_sk_cast_ub"), true, 0.0);
        a.speed = 2.0;
        a.update(0.25);
        assert!((a.current.as_ref().unwrap().time - 0.5).abs() < 1e-4);
        assert!((a.layer.as_ref().unwrap().playing.time - 0.25).abs() < 1e-4);
    }

    fn shared_rig_for_test() -> Arc<Rig> {
        crate::pose::shared_rig(SKELETON).unwrap()
    }
}
