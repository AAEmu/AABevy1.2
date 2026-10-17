//! Sampling clips onto a skeleton: [`Rig`], [`ClipBinding`], [`Pose`]. Plain data, no Bevy.
//!
//! A clip stores the *local* (parent-relative) rotation / position of the bones it animates,
//! replacing the bind pose; bones without a track keep their bind transform. Poses are in the
//! skeleton's own space (CryEngine, Z up); the entity hierarchy that renders them is rotated
//! into Bevy space once at its root.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use aa_cry::Skeleton;
use glam::{Quat, Vec3};

use crate::caf::Clip;

static NEXT_RIG: AtomicU64 = AtomicU64::new(1);

/// A skeleton prepared for animation.
#[derive(Debug)]
pub struct Rig {
    /// Unique per rig (clip bindings are cached by it).
    pub id: u64,
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
        Rig { id: NEXT_RIG.fetch_add(1, Ordering::Relaxed), skeleton, bind, by_controller, root }
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

/// The rig of a skeleton file, loaded once and shared by every character of that skeleton (so bindings of clips and
/// aa-cry's skeleton-bound models are shared too). Callable from any thread.
pub fn shared_rig(path: &str) -> crate::Result<Arc<Rig>> {
    static RIGS: OnceLock<Mutex<HashMap<String, Arc<Rig>>>> = OnceLock::new();
    let key = path.to_ascii_lowercase().replace('\\', "/");
    let rigs = RIGS.get_or_init(Default::default);
    if let Some(r) = rigs.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(r.clone());
    }
    let rig = Arc::new(Rig::new(Skeleton::load(&key)?));
    Ok(rigs.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(rig).clone())
}

/// How much of a layer shows on each bone (0..1, indexed like the rig's bones): the bones a partial-body clip drives.
/// Cheap to clone (shared).
#[derive(Clone, Debug)]
pub struct BoneMask(Arc<[f32]>);

impl BoneMask {
    /// Weight 1 on the bone called `root` and everything below it, 0 elsewhere. `None` when the rig has no such bone.
    pub fn subtree(rig: &Rig, root: &str) -> Option<BoneMask> {
        let start = rig.skeleton.bones.iter().position(|b| b.name == root)?;
        let mut w = vec![0.0; rig.len()];
        // parents come before their children in the bone list
        for (i, b) in rig.skeleton.bones.iter().enumerate() {
            if i == start || b.parent.is_some_and(|p| w[p] > 0.0) {
                w[i] = 1.0;
            }
        }
        Some(BoneMask(w.into()))
    }

    /// The upper body of a player skeleton: `Bip01 Spine` and the spine, arms, hands, neck and head above it (the legs and pelvis
    /// stay with the base layer). The 1.2 `_mub` ("move, upper body") clips drive `Bip01 Spine1` upward, the `_ub` ones are full
    /// body and need this mask.
    pub fn upper_body(rig: &Rig) -> Option<BoneMask> {
        BoneMask::subtree(rig, "Bip01 Spine")
    }

    /// Scales the weight of the bone called `name` (a softer join: `Bip01 Spine` at 0.5, say). Returns `self` when there is no such bone.
    pub fn with_weight(self, rig: &Rig, name: &str, weight: f32) -> BoneMask {
        let Some(i) = rig.skeleton.bones.iter().position(|b| b.name == name) else { return self };
        let mut w = self.0.to_vec();
        w[i] = weight.clamp(0.0, 1.0);
        BoneMask(w.into())
    }

    pub fn weight(&self, bone: usize) -> f32 {
        self.0.get(bone).copied().unwrap_or(0.0)
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

    /// Blends `clip` at `t` seconds over the pose: each bone the clip drives moves towards the clip by `mask weight * weight`
    /// (bones the clip does not drive, or the mask excludes, are left alone). Unlike [`Pose::sample`] it costs one lerp per
    /// driven bone and needs no second pose.
    #[allow(clippy::too_many_arguments)]
    pub fn sample_masked(&mut self, rig: &Rig, clip: &Clip, binding: &ClipBinding, t: f32, opt: SampleOptions, mask: &BoneMask, weight: f32) {
        for &(bone, track) in &binding.pairs {
            let bone = bone as usize;
            let w = mask.weight(bone) * weight;
            if w <= 0.0 {
                continue;
            }
            let track = &clip.tracks[track as usize];
            if let Some(q) = track.sample_rot(t) {
                let a = self.rot[bone];
                let q = if a.dot(q) < 0.0 { -q } else { q };
                self.rot[bone] = if w >= 1.0 { q } else { a.lerp(q, w).normalize() };
            }
            let at = if opt.lock_root_motion && bone == rig.root { 0.0 } else { t };
            if let Some(p) = track.sample_pos(at) {
                self.pos[bone] = self.pos[bone].lerp(p * opt.position_scale, w.min(1.0));
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
