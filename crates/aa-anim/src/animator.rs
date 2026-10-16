//! Playing clips on a spawned skeleton: the [`Animator`] component and [`AnimPlugin`].
//!
//! The skeleton of a character is an entity hierarchy with one entity per bone (see
//! [`spawn_bones`]); the [`Animator`] on the character root owns the clip state and writes the
//! bones' local `Transform`s every frame. Skinned meshes reference the bone entities as joints,
//! so Bevy's own skinning and transform propagation do the rest.
//!
//! The sampler is deliberately simple (no `AnimationGraph`): one current clip, an optional
//! cross-fade from the previous pose, an optional clip to run once and then return from, and an
//! optional random-idle schedule, which is all the login stage and the in-game idle need.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::caf::Clip;
use crate::pose::{BoundClip, Pose, Rig, SampleOptions};

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
    /// Playback speed multiplier.
    pub speed: f32,
    pub paused: bool,
    fade: Option<Fade>,
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
            fade: None,
            base_idle: None,
            pose,
            scratch,
        }
    }

    /// Starts `clip`, cross-fading from the current pose over `fade` seconds.
    pub fn play(&mut self, clip: BoundClip, looping: bool, fade: f32) {
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

    /// Plays `clip` once, then `after` (looping).
    pub fn play_once_then(&mut self, clip: BoundClip, after: BoundClip, fade: f32) {
        self.play(clip, false, fade);
        self.then = Some(Playing::new(after, true));
    }

    /// The pose written to the bones last frame.
    pub fn pose(&self) -> &Pose {
        &self.pose
    }

    /// Advances time and refreshes [`Animator::pose`]. Separate from the bevy system so tests can
    /// drive it without an `App`.
    pub fn update(&mut self, dt: f32) {
        let dt = if self.paused { 0.0 } else { dt * self.speed };
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
            f.elapsed += dt.abs().max(1e-4);
            let w = (f.elapsed / f.duration).clamp(0.0, 1.0);
            // blend from the frozen previous pose into the live pose
            self.scratch.clone_from(&f.from);
            self.scratch.blend_from(&self.pose, w);
            std::mem::swap(&mut self.scratch, &mut self.pose);
            if w >= 1.0 {
                self.fade = None;
            }
        }
    }
}

/// Registers the animation system. Add once.
pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClipLibrary>().add_systems(PostUpdate, animate.before(TransformSystems::Propagate));
    }
}

fn animate(time: Res<Time>, mut animators: Query<&mut Animator>, mut transforms: Query<&mut Transform>) {
    let dt = time.delta_secs();
    for mut a in &mut animators {
        if a.current.is_none() {
            continue;
        }
        a.update(dt);
        let a = &*a;
        for (i, &e) in a.bones.iter().enumerate() {
            if let Ok(mut t) = transforms.get_mut(e) {
                t.rotation = a.pose.rot[i];
                t.translation = a.pose.pos[i];
            }
        }
    }
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
}

impl ClipLibrary {
    /// Loads (once) the `.caf` at `path`.
    pub fn get(&self, path: &str) -> crate::Result<Arc<Clip>> {
        let key = path.to_ascii_lowercase().replace('\\', "/");
        if let Some(c) = self.clips.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let clip = Arc::new(Clip::load(&key)?);
        self.clips.lock().unwrap().insert(key, clip.clone());
        Ok(clip)
    }

    /// Loads a clip and binds it to a rig.
    pub fn bound(&self, rig: &Rig, path: &str, options: SampleOptions) -> crate::Result<BoundClip> {
        Ok(BoundClip::new(rig, self.get(path)?, options))
    }
}
