//! Secondary motion of rope bones: robes, skirts, tassets, cloaks, sleeves, hair.
//!
//! CryEngine gives the `rope*` bone chains of a character skeleton to the physics as ropes (`CRopeEntity`, one per chain),
//! pulled towards the animated pose. A chain starts at a bone whose name begins with `rope` and whose parent does not
//! (`ropeRobe01_01` under `Bip01 Pelvis`) and runs down its single `rope` child (`_02`, `_03`, ...); the bone heads are the
//! rope's vertices. Only the first bone carries the rope parameters, in its `alive` [`aa_cry::skeleton::BonePhysics`] record
//! (the others have `flags == -1`). The exporter stores plain numbers in the angle fields as degrees -> radians, so they read
//! back with `to_degrees()`:
//!
//! | field | read as | robe | skirt | cloak | meaning (`pe_params_rope`, see `<re-notes>\cryphysics\notes\entities.md` 2.2 / 4.2) |
//! |---|---|---|---|---|---|
//! | `max.x` | degrees | 500 | 200 | 500 | `stiffnessAnim`: spring towards the animated pose |
//! | `max.y` | degrees | 0.8 | 0.95 | 0.8 | `stiffnessDecayAnim`: vertex `i` of `n` gets `k * (1 - i * decay / n)` |
//! | `max.z` | degrees | 0.4 | 1.5 | 0.4 | `dampingAnim` |
//! | `damping.x` | | 0.5 | 0.1 | 0.5 | `damping` (inferred) |
//! | `damping.y` | | 1.0 | 1.0 | 1.0 | air resistance (inferred) |
//! | `damping.z` | | 0.01 | 0.01 | 0.01 | collision thickness (inferred) |
//!
//! There are no joint limits for ropes in the data (`min`, `spring_angle` are zero), so each chain family gets a fixed
//! cone around its animated direction.
//!
//! The solver is a small Verlet rope per chain, in world space so the cloth keeps its inertia when the character runs and
//! turns. Each fixed step (1/60 s near the camera, 1/30 s further away, nothing beyond [`RopeSettings::far`]) moves every
//! vertex by its velocity and a spring towards "parent vertex + animated segment"; a damper (`dampingAnim` plus a fixed
//! damping ratio) pulls the velocity towards the animated motion. Then, parent to child: a cone around the animated
//! direction, the body capsules, and the exact segment length (follow the leader; the root vertex is pinned to its animated
//! bone). The result is written back as the chain bones' local rotations (the smallest rotation from the animated to the
//! simulated segment direction), after the animator sampled the clip and before the bones are written: [`crate::animator`]
//! calls [`RopeSim::apply`].
//!
//! * **The clip is the target.** The locomotion clips already animate the rope bones (the run clips drive the robe, skirt,
//!   cloak and some hair chains: the trailing is baked in; the idles leave them alone), so the simulation adds only inertia:
//!   lag on starts, stops and turns, sway, the outward swing in a turn, and the body pushing the cloth. Drag against still
//!   air would count the baked trailing twice and is left out.
//! * **Gravity relative to the authored pose.** The authored shapes already hang under gravity, so a chain feels only the
//!   gravity its parent bone's tilt since the bind pose adds (`g - tilt * g`): an upright body keeps every rope exactly in
//!   its authored shape at rest (no droop of hair or flaps), a bent one lets them hang towards the ground.
//! * **Collision.** Twelve capsules on the animated `Bip01` bones (legs, pelvis to chest, upper back, neck, skull, shoulders,
//!   upper arms), in groups per chain family (robes and skirts: legs and torso; cloaks: also back and arms; hair: back,
//!   neck, skull, shoulders) and only those a chain can reach. Each radius is clamped to the bind-pose clearance of the
//!   segment, so the rest pose is never pushed.
//! * **Only skinned chains.** [`mask_ropes`] activates a chain when one of its bones is a joint of a skinned mesh under the
//!   character; [`RopeSim::set_skinned`] sets the mask by hand.
//!
//! [`AnimPlugin`](crate::AnimPlugin) adds a [`RopeSim`] to every [`Animator`] whose skeleton has rope chains; insert
//! [`NoRope`] on the animator's entity to opt out.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use glam::{Affine3A, Quat, Vec3};

use crate::animator::Animator;
use crate::pose::{Pose, Rig};

/// Parameters of one rope chain, decoded from its first bone (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RopeParams {
    /// Spring towards the animated pose at the root (1/s²).
    pub stiffness: f32,
    /// Fraction of the stiffness lost towards the tip.
    pub stiffness_decay: f32,
    /// Extra damping of the motion relative to the animation (1/s).
    pub damping_anim: f32,
    /// Damping of the absolute velocity (1/s).
    pub damping: f32,
    /// Drag against still air (1/s). Read but not applied, see the module documentation.
    pub air_resistance: f32,
    /// Collision thickness (m).
    pub thickness: f32,
}

impl Default for RopeParams {
    fn default() -> Self {
        RopeParams { stiffness: 300.0, stiffness_decay: 0.8, damping_anim: 1.0, damping: 0.5, air_resistance: 1.0, thickness: 0.01 }
    }
}

impl RopeParams {
    /// The parameters of a chain's first bone; `None` when the bone has none (`flags == -1`).
    pub fn from_physics(p: &aa_cry::skeleton::BonePhysics) -> Option<RopeParams> {
        if p.flags == u32::MAX || p.max.x <= 0.0 {
            return None;
        }
        Some(RopeParams {
            stiffness: p.max.x.to_degrees(),
            stiffness_decay: p.max.y.to_degrees().clamp(0.0, 1.0),
            damping_anim: p.max.z.to_degrees().max(0.0),
            damping: p.damping.x.max(0.0),
            air_resistance: p.damping.y.max(0.0),
            thickness: p.damping.z.clamp(0.0, 0.1),
        })
    }
}

/// What a chain hangs on to, from its name (`ropeRobe01_01` -> `Robe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Robe,
    Skirt,
    Cloak,
    Calf,
    Arm,
    Breast,
    Hair,
    Other,
}

impl Family {
    fn of(name: &str) -> Family {
        let n = name.to_ascii_lowercase();
        let n = n.trim_start_matches("rope");
        if n.starts_with("robe") {
            Family::Robe
        } else if n.starts_with("skirt") || n.starts_with("accpelvis") {
            Family::Skirt
        } else if n.starts_with("cloak") || n.starts_with("cape") {
            Family::Cloak
        } else if n.starts_with("calf") {
            Family::Calf
        } else if n.starts_with("arm") {
            Family::Arm
        } else if n.starts_with("breast") {
            Family::Breast
        } else if n.starts_with("hair") {
            Family::Hair
        } else {
            Family::Other
        }
    }

    /// The capsule groups the family collides with.
    fn collides(self) -> u8 {
        match self {
            Family::Robe | Family::Skirt | Family::Calf => LEGS | TORSO,
            Family::Cloak => LEGS | TORSO | BACK | ARMS,
            Family::Hair => BACK | HEAD,
            Family::Arm => LEGS | TORSO,
            Family::Breast => TORSO,
            Family::Other => LEGS | TORSO | BACK | HEAD | ARMS,
        }
    }

    /// Largest angle (radians) between a simulated segment and its animated direction.
    fn max_angle(self) -> f32 {
        match self {
            Family::Robe | Family::Cloak => 75f32,
            Family::Skirt | Family::Other => 60.0,
            // per segment: three or four of them add up, and a braid must not curl over the head in a sharp turn
            Family::Hair => 35.0,
            Family::Calf | Family::Arm => 45.0,
            Family::Breast => 15.0,
        }
        .to_radians()
    }
}

/// One rope chain of a rig.
#[derive(Clone, Debug)]
pub struct Chain {
    pub name: String,
    pub family: Family,
    /// Bone indices root to tip; vertex `j` is the head of `bones[j]`.
    pub bones: Vec<usize>,
    /// The (non-rope) bone the chain hangs from.
    pub parent: usize,
    pub params: RopeParams,
    /// First vertex in the flat particle arrays.
    offset: usize,
    /// The capsules this chain can reach.
    caps: Vec<usize>,
    /// Model-space bind rotation of `parent` (inverse), for the gravity relative to the authored pose.
    parent_bind_inv: Quat,
    /// Per vertex `1..` and entry of `caps`: collision radius (model metres, 0 = no test).
    radii: Vec<f32>,
}

/// A body capsule between two points fixed to bones (`offset` along the bone's own X axis, which runs along a Biped bone).
#[derive(Clone, Copy, Debug)]
struct Capsule {
    a: usize,
    a_off: f32,
    b: usize,
    b_off: f32,
}

impl Capsule {
    fn ends(&self, model: &[(Quat, Vec3)]) -> (Vec3, Vec3) {
        let ((ra, ta), (rb, tb)) = (model[self.a], model[self.b]);
        (ta + ra * Vec3::X * self.a_off, tb + rb * Vec3::X * self.b_off)
    }
}

/// Capsule groups (bits), and which a chain family collides with.
const LEGS: u8 = 1;
const TORSO: u8 = 2;
const BACK: u8 = 4;
const HEAD: u8 = 8;
const ARMS: u8 = 16;

/// Body capsules: (group, bone, offset, bone, offset, radius), metres for a Nuian male (scaled by the skeleton's head height).
const CAPSULES: [(u8, &str, f32, &str, f32, f32); 12] = [
    (LEGS, "Bip01 L Thigh", 0.0, "Bip01 L Calf", 0.0, 0.085),
    (LEGS, "Bip01 R Thigh", 0.0, "Bip01 R Calf", 0.0, 0.085),
    (LEGS, "Bip01 L Calf", 0.0, "Bip01 L Foot", 0.0, 0.06),
    (LEGS, "Bip01 R Calf", 0.0, "Bip01 R Foot", 0.0, 0.06),
    (TORSO, "Bip01 Pelvis", 0.0, "Bip01 Spine3", 0.0, 0.13),
    // upper back, neck, skull (the Head bone sits at the top of the neck, its X axis points up)
    (BACK, "Bip01 Spine3", 0.0, "Bip01 Neck", 0.0, 0.12),
    (HEAD, "Bip01 Neck", 0.0, "Bip01 Head", 0.0, 0.055),
    (HEAD, "Bip01 Head", 0.06, "Bip01 Head", 0.13, 0.095),
    // shoulders and upper arms
    (BACK, "Bip01 L Clavicle", 0.06, "Bip01 L UpperArm", 0.0, 0.06),
    (BACK, "Bip01 R Clavicle", 0.06, "Bip01 R UpperArm", 0.0, 0.06),
    (ARMS, "Bip01 L UpperArm", 0.0, "Bip01 L Forearm", 0.0, 0.05),
    (ARMS, "Bip01 R UpperArm", 0.0, "Bip01 R Forearm", 0.0, 0.05),
];

/// Head height of the Nuian male skeleton the capsule sizes are for.
const REFERENCE_HEAD_HEIGHT: f32 = 1.733;

/// The rope chains and body capsules of a rig (shared by every character of the rig).
#[derive(Debug)]
pub struct RopeRig {
    pub rig_id: u64,
    pub chains: Vec<Chain>,
    capsules: Vec<Capsule>,
    particles: usize,
    /// Bones whose model space is computed, ascending.
    needed: Vec<usize>,
}

impl RopeRig {
    /// Finds the chains (`hair`: also `ropeHair*`) and the capsules of `rig`.
    pub fn new(rig: &Rig, hair: bool) -> RopeRig {
        let bones = &rig.skeleton.bones;
        let is_rope = |i: usize| bones[i].name.len() >= 4 && bones[i].name[..4].eq_ignore_ascii_case("rope");
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); bones.len()];
        for (i, b) in bones.iter().enumerate() {
            if let Some(p) = b.parent {
                children[p].push(i);
            }
        }
        let bind = rig.bind.model_space(rig);
        let size = rig
            .skeleton
            .find("Bip01 Head")
            .map_or(1.0, |h| (bind[h].1.z / REFERENCE_HEAD_HEIGHT).clamp(0.5, 1.5));
        let mut capsules = Vec::new();
        let mut capsule_radius = Vec::new();
        let mut capsule_group = Vec::new();
        for (group, a, a_off, b, b_off, r) in CAPSULES {
            if let (Some(a), Some(b)) = (rig.skeleton.find(a), rig.skeleton.find(b)) {
                capsules.push(Capsule { a, a_off: a_off * size, b, b_off: b_off * size });
                capsule_radius.push(r * size);
                capsule_group.push(group);
            }
        }
        let capsule_ends: Vec<(Vec3, Vec3)> = capsules.iter().map(|c| c.ends(&bind)).collect();
        let mut chains = Vec::new();
        let mut particles = 0;
        for (i, b) in bones.iter().enumerate() {
            let Some(parent) = b.parent else { continue };
            if !is_rope(i) || is_rope(parent) {
                continue;
            }
            let family = Family::of(&b.name);
            if family == Family::Hair && !hair {
                continue;
            }
            let mut chain = vec![i];
            while let Some(&next) = children[*chain.last().unwrap()].iter().find(|&&c| is_rope(c)) {
                chain.push(next);
            }
            if chain.len() < 2 {
                continue;
            }
            let params = RopeParams::from_physics(&b.physics[0]).unwrap_or_default();
            // the capsules at the bind pose: never push a vertex further out than it already sits, and leave out the ones
            // the chain cannot reach
            let length: f32 = chain.windows(2).map(|w| bind[w[0]].1.distance(bind[w[1]].1)).sum();
            let root = bind[chain[0]].1;
            // vertex `j` (1..) is tested as the segment from vertex `j - 1`
            let radius_of = |j: usize, c: usize| {
                let (a, b) = capsule_ends[c];
                let (p0, p1) = (bind[chain[j - 1]].1, bind[chain[j]].1);
                let (s, q) = closest_segment_points(p0, p1, a, b);
                let r = (capsule_radius[c] + params.thickness).min(p0.lerp(p1, s).distance(q) - 0.01);
                if r >= 0.02 { r } else { 0.0 }
            };
            let caps: Vec<usize> = (0..capsules.len())
                .filter(|&c| {
                    if capsule_group[c] & family.collides() == 0 {
                        return false;
                    }
                    let (a, b) = capsule_ends[c];
                    point_segment_distance(root, a, b) <= length + capsule_radius[c] + 0.05 && (1..chain.len()).any(|j| radius_of(j, c) > 0.0)
                })
                .collect();
            let radii = (1..chain.len()).flat_map(|j| caps.iter().map(move |&c| (j, c))).map(|(j, c)| radius_of(j, c)).collect();
            chains.push(Chain { name: b.name.clone(), family, bones: chain.clone(), parent, params, offset: particles, caps, parent_bind_inv: bind[parent].0.inverse(), radii });
            particles += chain.len();
        }
        // the bones whose model space the solver needs: chains, capsules and their ancestors (parents come first)
        let mut need = vec![false; bones.len()];
        let mut mark = |mut i: usize| loop {
            if need[i] {
                break;
            }
            need[i] = true;
            match bones[i].parent {
                Some(p) => i = p,
                None => break,
            }
        };
        for c in &chains {
            c.bones.iter().for_each(|&b| mark(b));
        }
        for c in &capsules {
            mark(c.a);
            mark(c.b);
        }
        let needed = (0..bones.len()).filter(|&i| need[i]).collect();
        RopeRig { rig_id: rig.id, chains, capsules, particles, needed }
    }

    /// The shared rope rig of `rig`.
    pub fn shared(rig: &Rig, hair: bool) -> Arc<RopeRig> {
        type Cache = Mutex<HashMap<(u64, bool), Arc<RopeRig>>>;
        static RIGS: OnceLock<Cache> = OnceLock::new();
        let rigs = RIGS.get_or_init(Default::default);
        let mut rigs = rigs.lock().unwrap_or_else(|e| e.into_inner());
        rigs.entry((rig.id, hair)).or_insert_with(|| Arc::new(RopeRig::new(rig, hair))).clone()
    }

    pub fn is_empty(&self) -> bool {
        self.chains.is_empty()
    }
}

/// How ropes are simulated (a resource; read when a [`RopeSim`] is attached, and every frame for the distances).
#[derive(Resource, Clone, Debug)]
pub struct RopeSettings {
    pub enabled: bool,
    /// Simulate `ropeHair*` chains too (long hair, ponytails, helmet plumes).
    pub hair: bool,
    /// Simulate only the chains some skinned mesh under the character uses ([`mask_ropes`]).
    pub skinned_only: bool,
    /// Camera distance (m) up to which ropes step at 60 Hz; beyond it at 30 Hz.
    pub near: f32,
    /// Camera distance (m) beyond which ropes are not simulated (the animated pose shows).
    pub far: f32,
}

impl Default for RopeSettings {
    fn default() -> Self {
        RopeSettings { enabled: true, hair: true, skinned_only: true, near: 20.0, far: 60.0 }
    }
}

/// Opts an [`Animator`]'s entity out of rope simulation.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct NoRope;

/// Rope state of one character; lives next to its [`Animator`].
#[derive(Component, Debug)]
pub struct RopeSim {
    pub rig: Arc<RopeRig>,
    /// Skeleton (Cry model) space -> world, refreshed every frame by [`track_ropes`].
    pub world: Affine3A,
    /// Seconds per fixed step; 0 = not simulated (too far).
    pub step: f32,
    accum: f32,
    ready: bool,
    /// Vertex positions now and one step ago (world).
    x: Vec<Vec3>,
    prev: Vec<Vec3>,
    /// Animated vertex positions at the last update (world).
    target: Vec<Vec3>,
    /// Scratch: model space of the pose; this update's targets.
    model: Vec<(Quat, Vec3)>,
    target_now: Vec<Vec3>,
    capsules: Vec<(Vec3, Vec3)>,
    /// Per chain of [`RopeRig::chains`]: simulated (something is skinned to it).
    active: Vec<bool>,
    /// The mask came from the skinned meshes ([`mask_ropes`]) rather than from [`RopeSim::set_skinned`].
    auto_mask: bool,
}

impl RopeSim {
    pub fn new(rig: Arc<RopeRig>) -> RopeSim {
        let n = rig.particles;
        RopeSim {
            world: Affine3A::IDENTITY,
            step: 1.0 / 60.0,
            accum: 0.0,
            ready: false,
            x: vec![Vec3::ZERO; n],
            prev: vec![Vec3::ZERO; n],
            target: vec![Vec3::ZERO; n],
            model: Vec::new(),
            target_now: vec![Vec3::ZERO; n],
            capsules: Vec::new(),
            active: vec![true; rig.chains.len()],
            auto_mask: true,
            rig,
        }
    }

    /// Simulates only the chains with a bone for which `skinned(bone index)` is true (the others keep the animated pose, at no
    /// cost), and stops the automatic mask from the skinned meshes ([`mask_ropes`]) for this character. For a character
    /// assembly that knows which bones its parts are weighted to.
    pub fn set_skinned(&mut self, skinned: impl Fn(usize) -> bool) {
        self.auto_mask = false;
        self.set_active(skinned);
    }

    fn set_active(&mut self, skinned: impl Fn(usize) -> bool) {
        let active: Vec<bool> = self.rig.chains.iter().map(|c| c.bones.iter().any(|&b| skinned(b))).collect();
        if active != self.active {
            self.active = active;
            self.ready = false;
        }
    }

    /// The chains being simulated.
    pub fn active_chains(&self) -> impl Iterator<Item = &Chain> {
        self.rig.chains.iter().zip(&self.active).filter(|(_, a)| **a).map(|(c, _)| c)
    }

    /// Forgets the motion (the next update starts from the animated pose).
    pub fn reset(&mut self) {
        self.ready = false;
    }

    /// Advances the ropes by `dt` and bends the chain bones of `pose` (sampled from the clip) to them.
    pub fn apply(&mut self, rig: &Rig, pose: &mut Pose, dt: f32) {
        if self.step <= 0.0 || self.rig.rig_id != rig.id || pose.len() != rig.len() || !self.active.contains(&true) {
            self.ready = false;
            return;
        }
        let started = web_time::Instant::now();
        model_space(rig, pose, &self.rig.needed, &mut self.model);
        let world = self.world;
        let scale = world.matrix3.x_axis.length().max(1e-4);
        let chains = || self.rig.chains.iter().zip(&self.active).filter(|(_, a)| **a).map(|(c, _)| c);
        for c in chains() {
            for (j, &b) in c.bones.iter().enumerate() {
                self.target_now[c.offset + j] = world.transform_point3(self.model[b].1);
            }
        }
        // first update, or a teleport: start at rest in the animated pose
        let jumped = chains().any(|c| self.target_now[c.offset].distance_squared(self.target[c.offset]) > 1.5 * 1.5);
        if !self.ready || jumped {
            self.x.copy_from_slice(&self.target_now);
            self.prev.copy_from_slice(&self.target_now);
            self.target.copy_from_slice(&self.target_now);
            self.accum = 0.0;
            self.ready = true;
        }
        // fixed steps, the targets interpolated between the last update and this one
        let h = self.step;
        self.accum = (self.accum + dt.max(0.0)).min(h * 4.0);
        let steps = (self.accum / h) as u32;
        self.accum -= steps as f32 * h;
        self.capsules.clear();
        for c in &self.rig.capsules {
            let (a, b) = c.ends(&self.model);
            self.capsules.push((world.transform_point3(a), world.transform_point3(b)));
        }
        for s in 0..steps {
            let f0 = s as f32 / steps as f32;
            let f1 = (s + 1) as f32 / steps as f32;
            for c in self.rig.chains.iter().zip(&self.active).filter(|(_, a)| **a).map(|(c, _)| c) {
                // gravity the authored pose does not already hang under: the change of the parent bone's tilt since the
                // bind pose (an upright body keeps its rope shapes; a bent one lets them hang towards the ground)
                let down = Vec3::NEG_Z * 9.81;
                let tilt = self.model[c.parent].0 * c.parent_bind_inv;
                let gravity = world.transform_vector3(down - tilt * down);
                step_chain(c, &mut self.x, &mut self.prev, &self.target, &self.target_now, f0, f1, h, gravity, scale, &self.capsules);
            }
        }
        self.target.copy_from_slice(&self.target_now);
        // write back: rotate each chain bone so its child sits on the simulated vertex
        let to_model = world.inverse();
        for c in self.rig.chains.iter().zip(&self.active).filter(|(_, a)| **a).map(|(c, _)| c) {
            let mut parent_rot = self.model[c.parent].0;
            for j in 0..c.bones.len() - 1 {
                let (b, child) = (c.bones[j], c.bones[j + 1]);
                let rot = parent_rot * pose.rot[b];
                let animated = rot * pose.pos[child];
                let simulated = to_model.transform_vector3(self.x[c.offset + j + 1] - self.x[c.offset + j]);
                let new_rot = match (animated.try_normalize(), simulated.try_normalize()) {
                    (Some(a), Some(s)) => (Quat::from_rotation_arc(a, s) * rot).normalize(),
                    _ => rot,
                };
                pose.rot[b] = parent_rot.inverse() * new_rot;
                parent_rot = new_rot;
            }
        }
        STEPPED.fetch_add(1, Ordering::Relaxed);
        NANOS.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

/// Damping ratio of the spring towards the animated pose (on top of the data's `dampingAnim`).
const SPRING_DAMPING_RATIO: f32 = 0.35;

#[allow(clippy::too_many_arguments)]
fn step_chain(
    c: &Chain,
    x: &mut [Vec3],
    prev: &mut [Vec3],
    t0: &[Vec3],
    t1: &[Vec3],
    f0: f32,
    f1: f32,
    h: f32,
    gravity: Vec3,
    scale: f32,
    capsules: &[(Vec3, Vec3)],
) {
    let p = &c.params;
    let n = c.bones.len() - 1;
    let o = c.offset;
    let target = |i: usize, f: f32| t0[i].lerp(t1[i], f);
    // absolute damping only: AA's locomotion clips already animate the rope bones trailing (the run clips drive the robe,
    // skirt and cloak chains, the idles do not), so drag against still air (`air_resistance`) would count it twice
    let drag = (h * p.damping).min(1.0);
    // integrate (Verlet, positions per step)
    for j in 1..=n {
        let i = o + j;
        let k = (p.stiffness * (1.0 - j as f32 * p.stiffness_decay / n as f32)).clamp(2.0, 0.9 / (h * h));
        let follow = (h * (p.damping_anim + 2.0 * SPRING_DAMPING_RATIO * k.sqrt())).min(1.0);
        let mut v = x[i] - prev[i];
        // the animated motion of this vertex during the step
        let vt = target(i, f1) - target(i, f0);
        v -= v * drag;
        v += (vt - v) * follow;
        let desired = x[i - 1] + (target(i, f1) - target(i - 1, f1));
        let acc = gravity + (desired - x[i]) * k;
        prev[i] = x[i];
        x[i] += v + acc * h * h;
    }
    // constraints, root to tip: the root follows its bone
    x[o] = target(o, f1);
    prev[o] = target(o, f0);
    let cos_max = c.family.max_angle().cos();
    for j in 1..=n {
        let i = o + j;
        let len = (t1[i] - t1[i - 1]).length();
        let animated = (target(i, f1) - target(i - 1, f1)).normalize_or_zero();
        // the segment direction, kept inside the cone around the animated direction
        let limited = |d: Vec3| {
            let d = d.normalize_or(animated);
            let cos = d.dot(animated);
            if cos >= cos_max || animated == Vec3::ZERO {
                return d;
            }
            let side = (d - animated * cos).normalize_or_zero();
            animated * cos_max + side * (1.0 - cos_max * cos_max).sqrt()
        };
        for _ in 0..2 {
            x[i] = x[i - 1] + limited(x[i] - x[i - 1]) * len;
            // body capsules: push the segment out of each (moving its end; the lever is capped so a contact near the
            // segment's root cannot fling it)
            for (k, &ci) in c.caps.iter().enumerate() {
                let (a, b) = capsules[ci];
                let r = c.radii[(j - 1) * c.caps.len() + k] * scale;
                // cheap reject: the whole segment is further than its length from the capsule
                if r <= 0.0 || point_segment_distance(x[i], a, b) > r + len {
                    continue;
                }
                let (s, q) = closest_segment_points(x[i - 1], x[i], a, b);
                let off = x[i - 1].lerp(x[i], s) - q;
                let dist = off.length();
                if dist < r {
                    x[i] += off.normalize_or(Vec3::X) * (r - dist) / s.max(0.3);
                }
            }
        }
        x[i] = x[i - 1] + limited(x[i] - x[i - 1]) * len;
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn point_segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 1e-12 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// Closest points of the segments `p0`-`p1` and `q0`-`q1`: (parameter on the first, point on the second).
fn closest_segment_points(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> (f32, Vec3) {
    let d1 = p1 - p0;
    let d2 = q1 - q0;
    let r = p0 - q0;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-12 {
        s = 0.0;
        t = if e > 1e-12 { (f / e).clamp(0.0, 1.0) } else { 0.0 };
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (s, q0 + d2 * t)
}

/// Model-space transforms of the bones `needed` (ascending, closed under parents) of `pose`, into `out` (indexed by bone; no
/// allocation once sized).
fn model_space(rig: &Rig, pose: &Pose, needed: &[usize], out: &mut Vec<(Quat, Vec3)>) {
    out.resize(pose.len(), (Quat::IDENTITY, Vec3::ZERO));
    for &i in needed {
        out[i] = match rig.skeleton.bones[i].parent {
            Some(p) => {
                let (pr, pt) = out[p];
                (pr * pose.rot[i], pt + pr * pose.pos[i])
            }
            None => (pose.rot[i], pose.pos[i]),
        };
    }
}

static STEPPED: AtomicU32 = AtomicU32::new(0);
static MASKED: AtomicU32 = AtomicU32::new(0);
static NANOS: AtomicU64 = AtomicU64::new(0);

/// Rope updates and nanoseconds spent in them since the last call (for profiling).
pub fn take_stats() -> (u32, u64) {
    (STEPPED.swap(0, Ordering::Relaxed), NANOS.swap(0, Ordering::Relaxed))
}

/// Chain masks recomputed from the skinned meshes since the last call.
pub fn take_mask_updates() -> u32 {
    MASKED.swap(0, Ordering::Relaxed)
}

/// Animators that were just added and did not opt out.
type NewAnimators = (Added<Animator>, Without<NoRope>);

/// Gives every new [`Animator`] whose skeleton has rope chains a [`RopeSim`].
pub(crate) fn attach_ropes(
    mut commands: Commands,
    settings: Res<RopeSettings>,
    animators: Query<(Entity, &Animator), NewAnimators>,
) {
    if !settings.enabled {
        return;
    }
    for (e, a) in &animators {
        if a.bones.is_empty() {
            continue;
        }
        // a new (or re-inserted) animator: start over with the ropes of its rig
        let rope = RopeRig::shared(&a.rig, settings.hair);
        if rope.is_empty() {
            commands.entity(e).remove::<RopeSim>();
        } else {
            commands.entity(e).insert(RopeSim::new(rope));
        }
    }
}

type RopesMut<'w, 's> = Query<'w, 's, (&'static Animator, &'static mut RopeSim)>;
type RopesRef<'w, 's> = Query<'w, 's, (Entity, Ref<'static, RopeSim>)>;

/// Keeps [`RopeSim`]'s chain mask in step with the skinned meshes under the character: a chain is simulated when one of its
/// bones is a joint of some `SkinnedMesh` below the animator's entity (a part's joints are the bones of its own `.chr`, which
/// lists only the bones it is weighted to). Recomputed for a character when one of its skins is added or removed, or
/// when its rope is attached; characters with [`RopeSim::set_skinned`] keep their own mask.
#[allow(clippy::too_many_arguments)]
pub(crate) fn mask_ropes(
    settings: Res<RopeSettings>,
    mut ropes: ParamSet<(RopesMut, RopesRef)>,
    new_skins: Query<Entity, Added<SkinnedMesh>>,
    mut removed_skins: RemovedComponents<SkinnedMesh>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    skins: Query<&SkinnedMesh>,
    mut owner: Local<HashMap<Entity, Entity>>,
) {
    if !settings.skinned_only {
        return;
    }
    let mut dirty: Vec<Entity> = ropes.p1().iter().filter(|(_, r)| r.is_added()).map(|(e, _)| e).collect();
    for skin in &new_skins {
        // up the hierarchy to the character (skin -> part root -> model root -> character)
        let mut e = skin;
        for _ in 0..16 {
            if ropes.p1().contains(e) {
                dirty.push(e);
                break;
            }
            let Ok(p) = parents.get(e) else { break };
            e = p.parent();
        }
    }
    for skin in removed_skins.read() {
        if let Some(rope) = owner.remove(&skin) {
            dirty.push(rope);
        }
    }
    dirty.sort_unstable();
    dirty.dedup();
    let mut ropes = ropes.p0();
    for e in dirty {
        let Ok((a, mut sim)) = ropes.get_mut(e) else { continue };
        if !sim.auto_mask {
            continue;
        }
        let index: HashMap<Entity, usize> = a.bones.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let mut skinned = vec![false; a.bones.len()];
        for d in children.iter_descendants(e) {
            let Ok(skin) = skins.get(d) else { continue };
            owner.insert(d, e);
            for j in &skin.joints {
                if let Some(&i) = index.get(j) {
                    skinned[i] = true;
                }
            }
        }
        sim.set_active(|b| skinned[b]);
        MASKED.fetch_add(1, Ordering::Relaxed);
    }
}

/// Refreshes each rope's world transform (last frame's, of the skeleton's root entity) and its level of detail.
pub(crate) fn track_ropes(
    settings: Res<RopeSettings>,
    mut ropes: Query<(&Animator, &mut RopeSim, Has<NoRope>)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
) {
    let eye = cameras.iter().find(|(c, _)| c.is_active).map(|(_, g)| g.translation());
    for (a, mut sim, opted_out) in &mut ropes {
        let Some(&root) = a.bones.get(a.rig.root) else { continue };
        let Some(world) = parents.get(root).ok().and_then(|p| globals.get(p.parent()).ok()) else { continue };
        let world = world.affine();
        let distance = eye.map_or(0.0, |e| e.distance(Vec3::from(world.translation)));
        let step = if !settings.enabled || opted_out || distance > settings.far {
            0.0
        } else if distance > settings.near {
            1.0 / 30.0
        } else {
            1.0 / 60.0
        };
        // write only on change (keeps change detection quiet)
        if sim.step != step {
            sim.step = step;
        }
        if sim.world != world {
            sim.world = world;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKELETON: &str = "objects/characters/nuian/male/nude/nu_m_base.chr";

    #[test]
    fn closest_points() {
        let (s, q) = closest_segment_points(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::ZERO, Vec3::Z);
        assert!((s - 0.5).abs() < 1e-5 && q.length() < 1e-5, "{s} {q}");
        assert!((point_segment_distance(Vec3::new(0.0, 2.0, 0.5), Vec3::ZERO, Vec3::Z) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn chains_of_the_nuian_male() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = crate::pose::shared_rig(SKELETON).unwrap();
        let rope = RopeRig::new(&rig, false);
        let names: Vec<&str> = rope.chains.iter().map(|c| c.name.as_str()).collect();
        for n in ["ropeRobe01_01", "ropeRobe03_01", "ropeSkirt01_01", "ropeCloak02_01", "ropeCalf01_01", "ropeArm01_01"] {
            assert!(names.contains(&n), "{n} missing from {names:?}");
        }
        assert!(!names.iter().any(|n| n.starts_with("ropeHair")));
        let robe = rope.chains.iter().find(|c| c.name == "ropeRobe01_01").unwrap();
        assert_eq!(robe.bones.len(), 6);
        assert_eq!(rig.skeleton.bones[robe.parent].name, "Bip01 Pelvis");
        assert!((robe.params.stiffness - 500.0).abs() < 0.1 && (robe.params.stiffness_decay - 0.8).abs() < 1e-3, "{:?}", robe.params);
        assert_eq!(rope.capsules.len(), 12);
        assert!(RopeRig::new(&rig, true).chains.iter().any(|c| c.family == Family::Hair));
    }

    /// Every chain of the player skeletons (hair included): the tip at rest stays near the authored pose, and running in a
    /// tight circle with sudden stops keeps everything finite and inside the cone.
    #[test]
    fn every_chain_rests_and_stays_stable() {
        if !aa_cry::pak_available() {
            return;
        }
        for path in [SKELETON, "objects/characters/nuian/female/nude/nu_f_base.chr", "objects/characters/elf/female/nude/el_f_base.chr"] {
            let Ok(rig) = crate::pose::shared_rig(path) else { continue };
            let rope = Arc::new(RopeRig::new(&rig, true));
            let mut sim = RopeSim::new(rope.clone());
            let base = Affine3A::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            let bind = rig.bind.model_space(&rig);
            let mut pose = rig.bind.clone();
            let mut step = |sim: &mut RopeSim, world: Affine3A| {
                sim.world = world;
                pose.clone_from(&rig.bind);
                sim.apply(&rig, &mut pose, 1.0 / 60.0);
                pose.model_space(&rig)
            };
            let mut rest = Vec::new();
            for _ in 0..180 {
                rest = step(&mut sim, base);
            }
            let mut worst = vec![0.0f32; rope.chains.len()];
            let mut t = 0.0f32;
            for f in 0..600 {
                // 5 m/s on a 2 m circle, stopping dead every 1.5 s
                if (f / 90) % 2 == 0 {
                    t += 2.5 / 60.0;
                }
                let world = Affine3A::from_translation(Vec3::new(t.cos(), 0.0, -t.sin()) * 2.0) * Affine3A::from_rotation_y(t) * base;
                let m = step(&mut sim, world);
                for (k, c) in rope.chains.iter().enumerate() {
                    let tip = *c.bones.last().unwrap();
                    assert!(m[tip].1.is_finite(), "{path} {}", c.name);
                    worst[k] = worst[k].max(m[tip].1.distance(bind[tip].1));
                }
            }
            for (k, c) in rope.chains.iter().enumerate() {
                let tip = *c.bones.last().unwrap();
                let droop = rest[tip].1.distance(bind[tip].1);
                let len: f32 = c.bones.windows(2).map(|w| bind[w[0]].1.distance(bind[w[1]].1)).sum();
                eprintln!("{:40} {:18} len {len:.2} rest droop {droop:.3} max swing {:.3} caps {}", path.rsplit('/').next().unwrap(), c.name, worst[k], c.caps.len());
                assert!(worst[k] <= len * 2.0 + 1e-3);
            }
        }
    }

    /// At rest in the bind pose the ropes stay where they are (gravity sag only), and a sideways jerk of the body swings them
    /// behind, then they settle back; nothing explodes.
    #[test]
    fn rest_and_settle() {
        if !aa_cry::pak_available() {
            return;
        }
        let rig = crate::pose::shared_rig(SKELETON).unwrap();
        let mut sim = RopeSim::new(RopeRig::shared(&rig, false));
        // Cry Z up -> Bevy Y up, as the character model root does
        let base = Affine3A::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        sim.world = base;
        let robe = rig.skeleton.find("ropeRobe01_05").unwrap();
        let tip_bind = Vec3::from(rig.skeleton.bones[robe].bind_model.translation);
        let tip = |pose: &Pose| pose.model_space(&rig)[robe].1;
        let mut pose = rig.bind.clone();
        for _ in 0..120 {
            pose.clone_from(&rig.bind);
            sim.apply(&rig, &mut pose, 1.0 / 60.0);
        }
        let rest = tip(&pose);
        assert!(rest.distance(tip_bind) < 0.15, "rest {rest} vs bind {tip_bind}");
        // run sideways at 6 m/s for half a second
        let mut moved = 0.0f32;
        for f in 0..30 {
            sim.world = Affine3A::from_translation(Vec3::X * 0.1 * f as f32) * base;
            pose.clone_from(&rig.bind);
            sim.apply(&rig, &mut pose, 1.0 / 60.0);
            moved = moved.max(tip(&pose).distance(rest));
        }
        assert!(moved > 0.02, "the robe should trail ({moved})");
        for _ in 0..240 {
            pose.clone_from(&rig.bind);
            sim.apply(&rig, &mut pose, 1.0 / 60.0);
        }
        let back = tip(&pose);
        eprintln!("robe tip: bind {tip_bind}, rest {rest}, trailed {moved:.3} m, back {back}");
        assert!(back.is_finite() && back.distance(rest) < 0.02, "settles back: {back} vs {rest}");
    }
}
