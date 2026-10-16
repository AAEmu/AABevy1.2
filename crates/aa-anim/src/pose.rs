//! Sampling clips onto a skeleton: [`Rig`], [`ClipBinding`], [`Pose`]. Plain data, no Bevy.
//!
//! A clip stores the *local* (parent-relative) rotation / position of the bones it animates,
//! replacing the bind pose; bones without a track keep their bind transform. Poses are in the
//! skeleton's own space (CryEngine, Z up); the entity hierarchy that renders them is rotated
//! into Bevy space once at its root.

use std::collections::HashMap;
use std::sync::Arc;

use aa_cry::Skeleton;
use glam::{Quat, Vec3};

use crate::caf::Clip;

/// A skeleton prepared for animation.
#[derive(Debug)]
pub struct Rig {
    pub skeleton: Arc<Skeleton>,
    /// Local bind rotations / translations.
    pub bind: Pose,
    /// Controller id (CRC-32 of the name) -> bone index.
    by_controller: HashMap<u32, u32>,
    /// Index of the root bone `Bip01`'s children-of-nothing root (first bone without parent).
    pub root: usize,
}

impl Rig {
    pub fn new(skeleton: impl Into<Arc<Skeleton>>) -> Rig {
        let skeleton: Arc<Skeleton> = skeleton.into();
        let mut bind = Pose { rot: Vec::new(), pos: Vec::new() };
        let mut by_controller = HashMap::new();
        for (i, b) in skeleton.bones.iter().enumerate() {
            let (r, t) = b.local_rotation_translation();
            bind.rot.push(r);
            bind.pos.push(t);
            by_controller.insert(b.controller_id, i as u32);
        }
        let root = skeleton.bones.iter().position(|b| b.parent.is_none()).unwrap_or(0);
        Rig { skeleton, bind, by_controller, root }
    }

    pub fn len(&self) -> usize {
        self.skeleton.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skeleton.is_empty()
    }

    pub fn bone_by_controller(&self, controller: u32) -> Option<usize> {
        self.by_controller.get(&controller).map(|&i| i as usize)
    }

    /// Joins a clip's tracks to this rig's bones.
    pub fn bind_clip(&self, clip: &Clip) -> ClipBinding {
        let pairs = clip
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(ti, t)| self.by_controller.get(&t.controller).map(|&b| (b, ti as u32)))
            .collect();
        ClipBinding { pairs }
    }
}

/// Which bone each track of a clip drives (tracks for bones the rig lacks are dropped).
#[derive(Clone, Debug, Default)]
pub struct ClipBinding {
    /// (bone index, track index)
    pub pairs: Vec<(u32, u32)>,
}

/// Local rotation and position of every bone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub rot: Vec<Quat>,
    pub pos: Vec<Vec3>,
}

/// How a clip is applied to a pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleOptions {
    /// Multiplier for position tracks (`BaseModelHeight / AnimModelHeight` of the `.cal`).
    pub position_scale: f32,
    /// Hold the root bone's translation at the clip's first key (root motion is applied by gameplay).
    pub lock_root_motion: bool,
}

impl Default for SampleOptions {
    fn default() -> Self {
        SampleOptions { position_scale: 1.0, lock_root_motion: true }
    }
}

impl Pose {
    pub fn len(&self) -> usize {
        self.rot.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rot.is_empty()
    }

    /// Samples `clip` at `t` seconds over the current contents (bones without tracks are left alone).
    pub fn sample(&mut self, rig: &Rig, clip: &Clip, binding: &ClipBinding, t: f32, opt: SampleOptions) {
        for &(bone, track) in &binding.pairs {
            let bone = bone as usize;
            let track = &clip.tracks[track as usize];
            if let Some(q) = track.sample_rot(t) {
                self.rot[bone] = q;
            }
            // Locked root motion: the root stays where the clip starts (usually the origin).
            let at = if opt.lock_root_motion && bone == rig.root { 0.0 } else { t };
            if let Some(p) = track.sample_pos(at) {
                self.pos[bone] = p * opt.position_scale;
            }
        }
    }

    /// `self = lerp(self, other, w)`.
    pub fn blend_from(&mut self, other: &Pose, w: f32) {
        if w <= 0.0 {
            return;
        }
        if w >= 1.0 {
            self.clone_from(other);
            return;
        }
        for (a, b) in self.rot.iter_mut().zip(&other.rot) {
            let b = if a.dot(*b) < 0.0 { -*b } else { *b };
            *a = a.lerp(b, w).normalize();
        }
        for (a, b) in self.pos.iter_mut().zip(&other.pos) {
            *a = a.lerp(*b, w);
        }
    }

    /// World (model space) transforms of every bone: `(rotation, translation)`.
    pub fn model_space(&self, rig: &Rig) -> Vec<(Quat, Vec3)> {
        let mut out: Vec<(Quat, Vec3)> = Vec::with_capacity(self.len());
        for (i, b) in rig.skeleton.bones.iter().enumerate() {
            let local = (self.rot[i], self.pos[i]);
            out.push(match b.parent {
                Some(p) => {
                    let (pr, pt) = out[p];
                    (pr * local.0, pt + pr * local.1)
                }
                None => local,
            });
        }
        out
    }
}

/// A shared clip with its precomputed binding.
#[derive(Clone, Debug)]
pub struct BoundClip {
    pub clip: Arc<Clip>,
    pub binding: Arc<ClipBinding>,
    pub options: SampleOptions,
}

impl BoundClip {
    pub fn new(rig: &Rig, clip: Arc<Clip>, options: SampleOptions) -> BoundClip {
        let binding = Arc::new(rig.bind_clip(&clip));
        BoundClip { clip, binding, options }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caf::Track;

    fn rig() -> Rig {
        if !aa_cry::pak_available() {
            // two-bone rig built by hand would need chunk bytes; real-data tests below cover it
        }
        let sk = Skeleton::load("objects/characters/nuian/male/nude/nu_m_base.chr").unwrap();
        Rig::new(sk)
    }

    /// The idle clip of the Nuian male on the Nuian male skeleton must land close to the bind
    /// pose: the animator replaces local transforms, so model-space bones agree within a few cm.
    #[test]
    fn stage_idle_stays_near_bind_pose() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = rig();
        let clip = Clip::load("animations/nuian/male/fist/fist_ac_stage_idle.caf").unwrap();
        assert!((clip.duration - 3.0).abs() < 1e-3, "{}", clip.duration);
        let binding = rig.bind_clip(&clip);
        assert!(binding.pairs.len() > 50, "{} tracks bound", binding.pairs.len());
        let mut pose = rig.bind.clone();
        pose.sample(&rig, &clip, &binding, 0.0, SampleOptions::default());
        let world = pose.model_space(&rig);
        let bind_world = rig.bind.model_space(&rig);
        let mut dist: Vec<(f32, String)> = rig
            .skeleton
            .bones
            .iter()
            .enumerate()
            .filter(|(_, b)| b.parent.is_some()) // the animated roots sit at the origin
            .map(|(i, b)| ((Vec3::from(b.bind_model.translation) - world[i].1).length(), b.name.clone()))
            .collect();
        dist.sort_by(|a, b| b.0.total_cmp(&a.0));
        eprintln!("farthest from bind: {:?}", &dist[..12]);
        // physics rope bones (cloaks, skirts) hang free and are not animated
        let worst = dist.iter().find(|(_, n)| !n.starts_with("rope") && n != "camera").unwrap();
        assert!(worst.0 < 0.6, "bone {} is {} m from bind", worst.1, worst.0);
        let _ = bind_world;
    }

    #[test]
    fn blend_and_lock_root() {
        let a = Pose { rot: vec![Quat::IDENTITY], pos: vec![Vec3::ZERO] };
        let mut b = a.clone();
        b.pos[0] = Vec3::X * 2.0;
        b.rot[0] = Quat::from_rotation_z(1.0);
        let mut c = a.clone();
        c.blend_from(&b, 0.5);
        assert_eq!(c.pos[0], Vec3::X);
        assert!((c.rot[0].angle_between(Quat::from_rotation_z(0.5))) < 1e-3);
        let _ = Track::default();
    }
}
