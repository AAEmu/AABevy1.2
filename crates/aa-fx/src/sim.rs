//! CPU simulation of an effect's emitters and the billboard vertices they draw.
//!
//! The particles of an emitter live in one `Vec<Particle>` (compacted in place every step). They are simulated in world space
//! (Bevy axes, Y up, gravity -Y), or in the emitter's frame for `BindToEmitter` emitters. After the step the quads are written
//! into the emitter's vertex arrays in the **local space of the effect entity** (the mesh entity is the effect's child with an
//! identity transform), so small numbers stay small however far from the origin the effect is.
//!
//! CryEngine vectors (offsets, accelerations, angles) are Z up, Y forward; [`c2b`] maps them into Bevy.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use bevy::math::{Affine3A, Mat3, Quat, Vec2, Vec3};

use crate::geom::{Geom, GeomKey};
use crate::params::{Attach, AttachForm, Blend, EmitterDef, Facing, Spawn};

/// CryEngine (Z up) to Bevy (Y up).
#[inline]
pub fn c2b(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// The engine's particle vertex shader raises the authored colour to the power 2.2 (`particles.cfi`: `OUT.Color.xyz = pow(OUT.Color.xyz, 2.2)`)
/// before it multiplies the texture: colours are authored in gamma space and the blend happens in linear space, so a tint of
/// (1, 0.5, 0.2) is really (1, 0.22, 0.03).
#[inline]
pub fn engine_gamma(c: Vec3, g: f32) -> Vec3 {
    if g == 1.0 {
        return c;
    }
    Vec3::new(c.x.max(0.0).powf(g), c.y.max(0.0).powf(g), c.z.max(0.0).powf(g))
}

/// Irradiance of the sun (on a surface facing it) and of the sky on a lit mesh particle (`Illum` material), in the units of
/// the sprites' colours: the real client's sun is about 7.6 and its sky 1.8 in HDR units, and the picture is exposed by about
/// 1.6 (calibrated on the rocks of `fight.ground_launch_standalone` against the real client).
const LIT_SUN: f32 = 12.0;
const LIT_SKY: f32 = 2.9;
const GRAVITY: f32 = 9.81;
/// Particles one emitter may hold.
pub const MAX_PER_EMITTER: usize = 2048;
/// Segments of a tail the emitter keeps (`TailSteps` is cut to this).
const MAX_TAIL_STEPS: usize = 48;
/// Vertices one mesh emitter may write.
const MAX_MESH_VERTS: usize = 16_384;

/// A tiny xorshift generator: 0..1 floats, no allocation, one per emitter.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 40) as f32 / (1u32 << 24) as f32
    }

    #[inline]
    pub fn signed(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

/// The camera, in world space.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

/// What one frame's step needs to know about the world and the settings.
pub struct StepCtx<'a> {
    pub dt: f32,
    pub view: View,
    /// The effect entity's world transform (carries its scale).
    pub global: Affine3A,
    /// Rotation of the effect, scale removed.
    pub rot: Mat3,
    /// Uniform scale of the effect (entity scale times `Effect::scale`); for a stretched anchor the middle axis scale.
    pub scale: f32,
    /// How much longer than `scale` the entity is along each of its own axes (1 for a uniform scale). A beam anchor
    /// (`proj_physic_id` 2) is stretched on its Z by the length of the link: offsets and velocities of the particles and the
    /// mesh particles follow, the sprites' sizes do not.
    pub axis: Vec3,
    pub speed_scale: f32,
    pub count_scale: f32,
    /// 1 = full emission, less thins far effects.
    pub lod: f32,
    pub size_scale: f32,
    /// Half the width of a sprite per unit of `Size`.
    pub sprite_half: f32,
    /// See `FxSettings::colour_gamma`.
    pub colour_gamma: f32,
    /// A continuous emitter with a finite life spreads `Count` over that life (see `EmitterState::emit`).
    pub count_over_life: bool,
    /// Colour of the light on the effect.
    pub tint: Vec3,
    /// Direction to the sun in world space (shades opaque mesh particles).
    pub sun: Vec3,
    /// `FxSettings::sun_light`, and the cosine between the direction to the sun and the one to the camera: a camera facing sprite is
    /// lit by the sun like a surface that faces the camera.
    pub sun_light: f32,
    pub sun_facing: f32,
    /// How the effect entity moves (m/s, world space), for `InheritVelocity`.
    pub emitter_vel: Vec3,
    /// Height of the ground below a world position, for the emitters that collide (`CollideTerrain`).
    pub ground: Option<&'a (dyn Fn(Vec3) -> Option<f32> + Send + Sync)>,
    pub wind: Vec3,
    /// The unit the effect is on (world transform, feet at its origin): `AttachType` emitters emit from a capsule standing on it.
    /// `None`: the capsule stands at the effect's origin.
    pub body: Option<bevy::math::Affine3A>,
    /// The effect is on no unit at all (`Particle.SpawnEffect` at a point): `AttachType` emitters emit from the origin.
    pub no_body: bool,
    pub budget: &'a AtomicI32,
    /// Alpha factor of the effect (distance fade).
    pub fade: f32,
}

/// The frame particles are stored in.
#[derive(Clone, Copy)]
struct Frame {
    origin: Vec3,
    rot: Mat3,
    bound: bool,
}

impl StepCtx<'_> {
    /// A vector of the effect's local frame, stretched along the entity's axes (`axis`).
    #[inline]
    fn stretch(&self, v: Vec3) -> Vec3 {
        if self.axis == Vec3::ONE { v } else { self.rot * (self.axis * (self.rot.transpose() * v)) }
    }
}

impl Frame {
    #[inline]
    fn to_world_point(self, p: Vec3) -> Vec3 {
        if self.bound { self.origin + self.rot * p } else { p }
    }

    #[inline]
    fn to_world_dir(self, v: Vec3) -> Vec3 {
        if self.bound { self.rot * v } else { v }
    }

    #[inline]
    fn point_from_world(&self, p: Vec3) -> Vec3 {
        if self.bound { self.rot.transpose() * (p - self.origin) } else { p }
    }

    #[inline]
    fn dir_from_world(&self, v: Vec3) -> Vec3 {
        if self.bound { self.rot.transpose() * v } else { v }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Particle {
    pos: Vec3,
    vel: Vec3,
    /// The part of `vel` that came from the emitter's own motion (`InheritVelocity`; it decays with the air resistance like the rest).
    /// Stretch and OrientToVelocity of a sprite follow `vel - inh`: a trail of sparks that fly along with a projectile are not
    /// stretched over the projectile's speed (the 14 m streaks of Ollo's Hammer).
    inh: Vec3,
    /// Direction of travel in effect space: the emit direction until the particle moves faster than 1 mm/s, then its velocity.
    dir: Vec3,
    age: f32,
    life: f32,
    inv_life: f32,
    size: f32,
    sx: f32,
    sy: f32,
    sz: f32,
    stretch: f32,
    color: Vec3,
    alpha: f32,
    air: f32,
    grav: f32,
    ang: Vec3,
    ang_rate: Vec3,
    /// Orbit (`Rotate*`): angle in radians, radians per second, radius in metres.
    spin: f32,
    spin_rate: f32,
    rlen: f32,
    /// Tail: the particle's slot in the emitter's history, the next write position, the samples held, time since the last one.
    atest: f32,
    slot: u32,
    t_head: u16,
    t_len: u16,
    t_acc: f32,
    /// Hit the ground already (a child emitter starts once per particle).
    hit: bool,
    variant: u16,
    seed: f32,
    /// Counts up from 0 in the emitter that made the particle (a child emitter follows its parent particle by it).
    id: u32,
}

/// A particle that casts light (`LightSourceIntensity` / `LightSourceRadius`): the engine makes one dynamic light of each.
#[derive(Clone, Copy, Debug)]
pub struct LightSample {
    /// World position.
    pub pos: Vec3,
    /// Linear colour times the intensity and `pow(hdr multiplier, LightHDRDynamic)`; 1 is the light of an ordinary lit surface.
    pub colour: Vec3,
    /// Reach in metres.
    pub radius: f32,
}

/// A child emitter riding on one particle of its parent while its emitter life lasts: a column of fire from a particle that falls
/// at 15 m/s is a line, not a puff at the spot where the particle was born.
#[derive(Clone, Copy, Debug)]
struct Follower {
    parent_id: u32,
    /// Seconds since it started.
    t: f32,
    /// Particles owed.
    acc: f32,
}

/// One emitter of a running effect.
pub struct EmitterState {
    pub def: Arc<EmitterDef>,
    pub parent: Option<usize>,
    /// Index into the material table of the plugin.
    pub material: u32,
    /// Width / height of one texture tile (the engine shrinks the shorter side of the quad by it).
    pub aspect: f32,
    /// The mesh entity (or `Entity::PLACEHOLDER` for an emitter that only triggers others).
    pub mesh: bevy::prelude::Entity,
    pub draws: bool,
    /// Colour factor of the sprite's `Illum` material (`Diffuse + Emissive`), white for any other.
    pub mat_tint: Vec3,
    /// The mesh a `Geometry` emitter instances, once it is loaded.
    pub geom: Option<Arc<Geom>>,
    /// The mesh a `Geometry` emitter waits for.
    pub pending_geom: Option<GeomKey>,
    /// Particle budget one particle costs (a mesh counts its vertices in quads).
    cost: usize,
    /// Past world positions of the particles with a tail: `tail_steps` per slot, a ring.
    trail: Vec<Vec3>,
    /// The width axis of the ribbon at every sample of `trail` (a `Facing = Free` ribbon only): the particle's own axis in the
    /// emitter's orientation at that moment, so a blade swept through the air leaves a sheet, not a camera facing strip.
    trail_axis: Vec<Vec3>,
    trail_free: Vec<u32>,
    tail_steps: usize,
    tail_dt: f32,
    rng: Rng,
    particles: Vec<Particle>,
    age: f32,
    delay: f32,
    spawn_acc: f32,
    /// A continuous emitter has emitted its first particle (it does so at its first step, not one period later).
    begun: bool,
    burst_done: bool,
    done: bool,
    off: bool,
    want_births: bool,
    want_deaths: bool,
    want_collides: bool,
    births: Vec<(u32, Vec3)>,
    /// `SpawnIndirection = ParentStart` children with an emitter life: the emitters riding on a parent particle.
    followers: Vec<Follower>,
    next_id: u32,
    deaths: Vec<Vec3>,
    collides: Vec<Vec3>,
    lit: f32,
    decal_grid: crate::decal::Draped,
    // output, effect-local space
    pub pos: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// Second texture coordinates of a mesh with an opacity map: the mesh's own, not animated.
    pub uv1: Vec<[f32; 2]>,
    pub col: Vec<[f32; 4]>,
    /// Quads (sprite emitter) or mesh copies written.
    pub units: usize,
    /// Births of the current frame that are not aged again (`update`): the particles from this index on.
    fresh: usize,
    pub min: Vec3,
    pub max: Vec3,
    /// The vertex arrays changed since the last upload.
    pub dirty: bool,
}

impl EmitterState {
    pub fn new(def: Arc<EmitterDef>, parent: Option<usize>, material: u32, aspect: f32, seed: u64, mesh: bevy::prelude::Entity) -> EmitterState {
        let draws = def.draws();
        let spawn = def.spawn;
        EmitterState {
            parent,
            material,
            aspect,
            mesh,
            draws,
            mat_tint: Vec3::ONE,
            geom: None,
            pending_geom: None,
            cost: 1,
            trail: Vec::new(),
            trail_axis: Vec::new(),
            trail_free: Vec::new(),
            tail_steps: 0,
            tail_dt: 0.0,
            rng: Rng::new(seed),
            particles: Vec::new(),
            age: 0.0,
            delay: def.spawn_delay.base.max(0.0),
            spawn_acc: 0.0,
            begun: false,
            burst_done: false,
            done: spawn != Spawn::Direct,
            off: false,
            want_births: false,
            want_deaths: false,
            want_collides: false,
            births: Vec::new(),
            followers: Vec::new(),
            next_id: 0,
            deaths: Vec::new(),
            collides: Vec::new(),
            lit: 1.0,
            decal_grid: crate::decal::Draped::default(),
            pos: Vec::new(),
            uv: Vec::new(),
            uv1: Vec::new(),
            col: Vec::new(),
            units: 0,
            fresh: 0,
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            dirty: false,
            def,
        }
    }

    pub fn alive(&self) -> usize {
        self.particles.len()
    }

    /// Particle budget the live particles use.
    pub fn load(&self) -> usize {
        self.particles.len() * self.cost
    }

    /// Vertices of one unit (a quad or a mesh copy) and its triangle indices.
    pub fn unit(&self) -> (u32, &[u32]) {
        const QUAD: [u32; 6] = [0, 1, 2, 0, 2, 3];
        match &self.geom {
            Some(g) => (g.tmpl.pos.len() as u32, &g.tmpl.idx),
            None => (4, &QUAD),
        }
    }

    /// The mesh of a `Geometry` emitter arrived.
    pub fn set_geom(&mut self, g: Arc<Geom>) {
        self.cost = (g.tmpl.pos.len() / 4).max(1);
        self.pending_geom = None;
        self.geom = Some(g);
    }

    fn cap(&self) -> usize {
        match &self.geom {
            Some(g) => (MAX_MESH_VERTS / g.tmpl.pos.len().max(1)).clamp(4, MAX_PER_EMITTER),
            None => MAX_PER_EMITTER,
        }
    }

    /// Turns the emitter off for good (its quality level is not wanted).
    pub fn disable(&mut self) {
        self.off = true;
        self.done = true;
        self.draws = false;
    }

    /// No particles and nothing more to emit.
    pub fn finished(&self) -> bool {
        self.done && self.particles.is_empty()
    }

    fn reset(&mut self) {
        self.particles.clear();
        self.trail.clear();
        self.trail_axis.clear();
        self.trail_free.clear();
        self.age = 0.0;
        self.followers.clear();
        self.delay = self.def.spawn_delay.base.max(0.0);
        self.spawn_acc = 0.0;
        self.fresh = 0;
        self.begun = false;
        self.burst_done = false;
        self.done = self.def.spawn != Spawn::Direct;
        self.units = 0;
        self.pos.clear();
        self.uv.clear();
        self.uv1.clear();
        self.col.clear();
        self.dirty = true;
    }

    fn frame(&self, ctx: &StepCtx) -> Frame {
        Frame { origin: ctx.global.translation.into(), rot: ctx.rot, bound: self.def.bind_to_emitter }
    }

    /// Takes `n` particles from the global budget.
    fn grant(&self, ctx: &StepCtx, n: usize) -> usize {
        let room = self.cap().saturating_sub(self.particles.len());
        let n = n.min(room);
        if n == 0 {
            return 0;
        }
        let want = (n * self.cost) as i32;
        let before = ctx.budget.fetch_sub(want, Ordering::Relaxed);
        if before >= want {
            n
        } else {
            ctx.budget.fetch_add(want, Ordering::Relaxed);
            before.max(0) as usize / self.cost
        }
    }

    /// Emission of one step; returns nothing, fills `births`.
    fn emit(&mut self, ctx: &StepCtx, dt: f32) {
        self.fresh = self.particles.len();
        if self.off {
            return;
        }
        let def = self.def.clone();
        if self.delay > 0.0 {
            self.delay -= dt;
            if self.delay > 0.0 {
                return;
            }
        }
        if def.spawn != Spawn::Direct {
            // triggered by the parent, see `EffectSim::advance`
            self.age += dt;
            return;
        }
        self.age += dt;
        let life = def.emitter_life.base;
        let endless = life <= 0.0;
        let active = endless || self.age < life;
        let strength = if endless { 0.0 } else { (self.age / life).min(1.0) };
        let pulse = def.pulse_period.base;
        let count = (def.count.spawn(0.0, strength) * ctx.count_scale).max(0.0);
        let plife = def.particle_life.base;

        if active {
            if def.continuous {
                if plife > 0.0 {
                    // `Count` particles over the emitter's life, or alive at once when the emitter never ends; a pulse shorter than
                    // the life restarts the emitter while the last emission still runs (`life / pulse` of them at a time)
                    let (period, overlap) = if life > 0.0 && ctx.count_over_life {
                        (life, if pulse > 0.0 && life > pulse { (life / pulse).ceil() } else { 1.0 })
                    } else {
                        (plife, 1.0)
                    };
                    let before = self.spawn_acc;
                    if !self.begun {
                        // the first particle comes at time 0, the next ones one period apart
                        self.begun = true;
                        if count > 0.0 {
                            self.spawn_acc += 1.0;
                        }
                    }
                    let rate = count / period * overlap * ctx.lod;
                    self.spawn_acc += rate * dt;
                    let n = self.spawn_acc as usize;
                    self.spawn_acc -= n as f32;
                    // the j-th birth happens when the accumulator reaches j, so it is `dt - (j - before) / rate` old at the end of the step;
                    // one born more than a life ago is already gone (a frame longer than the life shows only the last births of it)
                    let ages: Vec<f32> = (1..=n)
                        .map(|j| if rate > 0.0 { dt - ((j as f32 - before) / rate).clamp(0.0, dt) } else { 0.0 })
                        .filter(|&age| age < plife)
                        .collect();
                    let n = self.grant(ctx, ages.len());
                    for &age0 in ages.iter().rev().take(n) {
                        self.spawn(ctx, None, strength, age0);
                    }
                } else {
                    // endless particles: keep `count` of them
                    let want = (count * ctx.lod).round() as usize;
                    let n = self.grant(ctx, want.saturating_sub(self.particles.len()));
                    for _ in 0..n {
                        self.spawn(ctx, None, strength, 0.0);
                    }
                }
            } else if !self.burst_done {
                self.burst_done = true;
                let n = self.grant(ctx, (count * ctx.lod).round() as usize);
                for _ in 0..n {
                    self.spawn(ctx, None, strength, 0.0);
                }
            }
        }
        if pulse > 0.0 {
            if self.age >= pulse {
                self.age -= pulse;
                self.burst_done = false;
            }
        } else if !active || (!def.continuous && self.burst_done && endless) {
            self.done = true;
        }
    }

    /// The children of `ParentStart` that last `EmitterLifeTime` seconds ride on their parent particle.
    fn follows_parent(&self) -> bool {
        self.def.spawn == Spawn::ParentStart && self.def.continuous && self.def.emitter_life.base > 0.0
    }

    /// Particles this emitter emits over its whole life when triggered once.
    fn burst_count(&self, ctx: &StepCtx) -> f32 {
        let def = &self.def;
        let life = def.emitter_life.base;
        let plife = def.particle_life.base.max(0.05);
        let count = (def.count.spawn(0.0, 0.0) * ctx.count_scale * ctx.lod).max(0.0);
        if def.continuous && life > 0.0 && !ctx.count_over_life { count * (life / plife).min(1.0) } else { count }
    }

    /// Moves the followers on with the parent's particles (`parents`: id and world position of the live ones) and emits at them.
    fn follow(&mut self, ctx: &StepCtx, dt: f32, parents: &[(u32, Vec3)]) {
        if self.followers.is_empty() {
            return;
        }
        let life = self.def.emitter_life.base;
        let total = self.burst_count(ctx);
        let mut followers = std::mem::take(&mut self.followers);
        followers.retain_mut(|f| {
            let Some(&(_, at)) = parents.iter().find(|(id, _)| *id == f.parent_id) else { return false };
            f.t += dt;
            f.acc += total / life * dt;
            let n = f.acc as usize;
            f.acc -= n as f32;
            let n = self.grant(ctx, n);
            for _ in 0..n {
                let age0 = self.rng.next() * dt;
                self.spawn(ctx, Some(at), (f.t / life).min(1.0), age0);
            }
            f.t < life
        });
        // the ones that started while this ran were pushed to `self.followers`
        followers.append(&mut self.followers);
        self.followers = followers;
    }

    /// Emits the sub-burst of a parent particle born (or dead) at `at` (world).
    fn emit_from_parent(&mut self, ctx: &StepCtx, at: Vec3) {
        let def = self.def.clone();
        let life = def.emitter_life.base;
        let plife = def.particle_life.base.max(0.05);
        let count = (def.count.spawn(0.0, 0.0) * ctx.count_scale * ctx.lod).max(0.0);
        let n = if def.continuous && life > 0.0 { count * (life / plife).min(1.0) } else { count };
        let n = self.grant(ctx, n.round() as usize);
        for _ in 0..n {
            let age0 = self.rng.next() * life.max(0.0).min(plife);
            self.spawn(ctx, Some(at), 0.0, age0);
        }
    }

    /// One new particle at the emitter (or at `at`, world).
    fn spawn(&mut self, ctx: &StepCtx, at: Option<Vec3>, strength: f32, age0: f32) {
        let def = self.def.clone();
        let frame = self.frame(ctx);
        let r = &mut self.rng;
        let scale = ctx.scale;

        // position
        let mut off = def.position_offset;
        if def.random_offset != Vec3::ZERO {
            off += Vec3::new(r.signed(), r.signed(), r.signed()) * def.random_offset;
        }
        let mut local = c2b(off) * scale;
        if def.pos_random_offset > 0.0 {
            let mut d = Vec3::new(r.signed(), r.signed(), r.signed());
            if d.length_squared() > 1.0 {
                d = d.normalize_or_zero();
            }
            local += d * def.pos_random_offset * scale;
        }
        // emission from the shape of the entity the effect is on
        let mut surface_normal = None;
        let mut on_body = None;
        if def.attach != Attach::None && at.is_none() && !ctx.no_body {
            let (p, n) = attach_point(&def, r);
            match ctx.body {
                // the capsule of the unit: its own up, scale and place, wherever on it the effect hangs (a hand bone)
                Some(body) => {
                    on_body = Some(Vec3::from(body.translation) + Vec3::from(body.matrix3 * bevy::math::Vec3A::from(c2b(p))));
                    surface_normal = Some(n);
                }
                None => {
                    local += c2b(p) * scale;
                    surface_normal = Some(n);
                }
            }
        }
        let world_pos = match on_body {
            Some(at_body) => at_body + ctx.stretch(ctx.rot * local),
            None => at.unwrap_or(frame.origin) + ctx.stretch(ctx.rot * local),
        };
        let pos = frame.point_from_world(world_pos);

        // direction: focus axis, tilted by the focus angle, then the emit cone
        let axis = if def.focus_gravity { Vec3::Y } else { ctx.rot * c2b(Vec3::Y) };
        let (t, b) = perpendicular(axis);
        let mut dir = axis;
        let fa = def.focus_angle.spawn(r.next(), strength).to_radians();
        if fa != 0.0 {
            // CryEngine tilts the focus axis (emitter +Y) about the emitter's X axis toward +Z (up for a standing emitter) and
            // then spins that tilt about the axis by `FocusAzimuth` (default 0: it does not scatter by itself)
            let az = def.focus_azimuth.base + def.focus_azimuth.var * r.signed() * 180.0;
            let az = az.to_radians();
            let side = if def.focus_gravity {
                t * az.cos() + b * az.sin()
            } else {
                ctx.rot * c2b(Vec3::Z * az.cos() + Vec3::X * az.sin())
            };
            dir = axis * fa.cos() + side * fa.sin();
        }
        let emit_max = def.emit_angle.base.to_radians();
        if emit_max > 0.0 {
            let emit_min = emit_max * (1.0 - def.emit_angle.var);
            let c = emit_min.cos() + r.next() * (emit_max.cos() - emit_min.cos());
            let s = (1.0 - c * c).max(0.0).sqrt();
            let az = r.next() * std::f32::consts::TAU;
            let (t, b) = perpendicular(dir);
            dir = dir * c + (t * az.cos() + b * az.sin()) * s;
        }
        if let Some(n) = surface_normal
            && def.speed.base != 0.0
        {
            // particles leave the surface (of the unit's capsule: in its own axes)
            let rot = ctx.body.map_or(ctx.rot, |b| Mat3::from_cols(Vec3::from(b.matrix3.x_axis).normalize_or_zero(), Vec3::from(b.matrix3.y_axis).normalize_or_zero(), Vec3::from(b.matrix3.z_axis).normalize_or_zero()));
            dir = rot * c2b(n);
        }
        let speed = def.speed.spawn(r.next(), strength) * ctx.speed_scale * scale;
        let mut vel = frame.dir_from_world(ctx.stretch(dir * speed));
        let mut inh = Vec3::ZERO;
        if def.inherit_velocity != 0.0 && !def.bind_to_emitter {
            inh = frame.dir_from_world(ctx.emitter_vel * def.inherit_velocity);
            vel += inh;
        }

        let life = def.particle_life.spawn(r.next(), strength);
        let (life, inv_life) = if def.particle_life.base > 0.0 && life > 1e-3 { (life, 1.0 / life) } else { (f32::INFINITY, 0.0) };

        let size = def.size.spawn(r.next(), strength) * scale * ctx.size_scale;
        let sx = def.size_x.spawn(r.next(), strength);
        let sy = def.size_y.spawn(r.next(), strength);
        let sz = def.size_z.spawn(r.next(), strength);
        let mut color = def.color.base;
        if let Some(c) = def.color.strength.as_ref().and_then(|c| c.eval(strength)) {
            color *= c;
        }
        let alpha = def.alpha.spawn(r.next(), strength);

        let d2r = std::f32::consts::PI / 180.0;
        let ang = (def.init_angles + def.random_angles * Vec3::new(r.signed(), r.signed(), r.signed())) * d2r;
        let ang_rate = (def.rotation_rate + def.random_rotation_rate * Vec3::new(r.signed(), r.signed(), r.signed())) * d2r;
        let (spin, spin_rate, rlen) = if def.orbits() {
            let init = def.rotate_init.base * (1.0 - def.rotate_init.var * 2.0 * r.next());
            (init.to_radians(), def.rotate_speed.spawn(r.next(), strength), def.rotate_length.spawn(r.next(), strength) * scale)
        } else {
            (0.0, 0.0, 0.0)
        };
        let variant = if def.tiling.variants > 1 { (r.next() * def.tiling.variants as f32) as u16 } else { 0 };

        let mut p = Particle {
            pos,
            vel,
            inh,
            dir: frame.dir_from_world(dir).normalize_or_zero(),
            age: age0,
            life,
            inv_life,
            size,
            sx,
            sy,
            sz,
            stretch: def.stretch.spawn(r.next(), strength),
            color,
            alpha,
            air: def.air_resistance.spawn(r.next(), strength),
            grav: def.gravity.spawn(r.next(), strength),
            ang,
            ang_rate,
            spin,
            spin_rate,
            rlen,
            atest: def.alpha_test.spawn(r.next(), strength),
            slot: u32::MAX,
            t_head: 0,
            t_len: 0,
            t_acc: 0.0,
            hit: false,
            variant,
            seed: r.next() * 100.0,
            id: self.next_id,
        };
        self.next_id = self.next_id.wrapping_add(1);
        p.pos += p.vel * age0;
        if def.has_tail() {
            self.start_tail(&mut p, ctx);
        }
        if self.want_births {
            let basis = self.orbit_basis(ctx);
            let at = frame.to_world_point(p.pos) + self.orbit_offset(&p, 0.0, &basis);
            self.births.push((p.id, at));
        }
        self.particles.push(p);
    }

    /// Moves the particles by `dt` and drops the dead ones.
    fn update(&mut self, ctx: &StepCtx, dt: f32) {
        let def = self.def.clone();
        let luts = def.luts();
        let frame = self.frame(ctx);
        let g = frame.dir_from_world(Vec3::NEG_Y * GRAVITY);
        let acc = frame.dir_from_world(c2b(def.acceleration));
        let wind = frame.dir_from_world(ctx.wind);
        let wind_k = def.wind.max(0.0);
        let turb = def.turbulence_3d * ctx.scale;
        let want_deaths = self.want_deaths;
        let want_collides = self.want_collides;
        let collide = def.collides();
        let basis = self.orbit_basis(ctx);
        let orbits = def.orbits();
        let mut w = 0;
        let fresh = self.fresh;
        for i in 0..self.particles.len() {
            let mut p = self.particles[i];
            // a particle born in this very step (`emit`) was already placed `age0` into the step: it is not aged again, so one whose life is
            // shorter than the frame (the beam mesh of `life_absorb_launch` lives 0.05 s) is still there to be drawn
            let dt = if i >= fresh { 0.0 } else { dt };
            p.age += dt;
            if p.age >= p.life {
                if want_deaths {
                    let at = frame.to_world_point(p.pos) + self.orbit_offset(&p, 1.0, &basis);
                    self.deaths.push(at);
                }
                if p.slot != u32::MAX {
                    self.trail_free.push(p.slot);
                }
                continue;
            }
            let a = p.age * p.inv_life;
            let air = p.air * luts.air.at(a);
            let grav = p.grav * luts.gravity.at(a);
            p.vel += (g * grav + acc) * dt;
            if wind_k > 0.0 {
                p.vel += (wind - p.vel) * (wind_k * dt).min(1.0);
            }
            if air > 0.0 {
                p.vel *= 1.0 / (1.0 + air * dt);
                p.inh *= 1.0 / (1.0 + air * dt);
            }
            p.pos += p.vel * dt;
            if p.vel.length_squared() > 1e-6 {
                p.dir = p.vel.normalize();
            }
            if collide && let Some(ground) = ctx.ground {
                let mut wp = frame.to_world_point(p.pos);
                if let Some(gy) = ground(wp)
                    && wp.y < gy
                {
                    wp.y = gy;
                    let mut wv = frame.to_world_dir(p.vel);
                    if def.bounciness < 0.0 {
                        // dies on impact
                        p.life = p.age;
                    } else if wv.y < 0.0 {
                        wv.y = -wv.y * def.bounciness;
                        if wv.y < 0.3 {
                            wv.y = 0.0;
                        }
                    }
                    let slow = 1.0 / (1.0 + def.friction * dt * 0.5);
                    wv.x *= slow;
                    wv.z *= slow;
                    p.pos = frame.point_from_world(wp);
                    p.vel = frame.dir_from_world(wv);
                    if !p.hit {
                        p.hit = true;
                        if want_collides {
                            self.collides.push(wp);
                        }
                    }
                }
            }
            if turb > 0.0 {
                let t = p.age * 1.7 + p.seed;
                p.pos += Vec3::new(t.sin(), (t * 1.31 + 1.0).sin(), (t * 0.77 + 2.0).sin()) * turb * dt;
            }
            p.ang += p.ang_rate * dt;
            if orbits {
                p.spin += p.spin_rate * luts.rot_speed.at(a) * dt;
            }
            if p.slot != u32::MAX {
                p.t_acc += dt;
                if p.t_acc >= self.tail_dt {
                    p.t_acc = 0.0;
                    let at = frame.to_world_point(p.pos) + self.orbit_offset(&p, a, &basis);
                    let axis = self.ribbon_axis(&p, ctx);
                    self.push_tail(&mut p, at, axis);
                }
            }
            self.particles[w] = p;
            w += 1;
        }
        self.particles.truncate(w);
    }

    /// Gives a new particle its history ring and writes its first sample.
    fn start_tail(&mut self, p: &mut Particle, ctx: &StepCtx) {
        let def = self.def.clone();
        if self.tail_steps == 0 {
            self.tail_steps = (def.tail_steps as usize).clamp(2, MAX_TAIL_STEPS);
            // seconds of past path: as long as the particle lives at most; a sample per frame at the finest
            // an endless particle (a weapon trail with no `ParticleLifeTime` asks for 100 or 200 s of tail) shows the last second at most:
            // spread over the 64 steps it would be a sample every few seconds, a beam across the map
            let span = if p.life.is_finite() { def.tail_length.base.min(p.life) } else { def.tail_length.base.min(1.0) };
            self.tail_dt = (span / self.tail_steps as f32).max(1.0 / 90.0);
        }
        p.slot = match self.trail_free.pop() {
            Some(s) => s,
            None => {
                let s = (self.trail.len() / self.tail_steps) as u32;
                self.trail.resize(self.trail.len() + self.tail_steps, Vec3::ZERO);
                if def.facing == Facing::Free {
                    self.trail_axis.resize(self.trail.len(), Vec3::ZERO);
                }
                s
            }
        };
        p.t_head = 0;
        p.t_len = 0;
        p.t_acc = 0.0;
        let frame = self.frame(ctx);
        let basis = self.orbit_basis(ctx);
        let at = frame.to_world_point(p.pos) + self.orbit_offset(p, 0.0, &basis);
        let axis = self.ribbon_axis(p, ctx);
        self.push_tail(p, at, axis);
    }

    /// The width axis of a `Facing = Free` ribbon now (world space): the particle's cry +Y, which for a weapon bone is the blade
    /// (the swords are modelled along +Y and the hand bone holds them as modelled), turned by the particle's own angles and the
    /// effect's orientation. Zero for the other facings.
    #[inline]
    fn ribbon_axis(&self, p: &Particle, ctx: &StepCtx) -> Vec3 {
        if self.def.facing != Facing::Free {
            return Vec3::ZERO;
        }
        let q = Quat::from_euler(bevy::math::EulerRot::ZYX, p.ang.z, p.ang.y, p.ang.x);
        (ctx.rot * c2b(q * Vec3::Y)).normalize_or_zero()
    }

    #[inline]
    fn push_tail(&mut self, p: &mut Particle, at: Vec3, axis: Vec3) {
        let n = self.tail_steps;
        if !self.trail_axis.is_empty() {
            self.trail_axis[p.slot as usize * n + p.t_head as usize] = axis;
        }
        self.trail[p.slot as usize * n + p.t_head as usize] = at;
        p.t_head = ((p.t_head as usize + 1) % n) as u16;
        p.t_len = (p.t_len + 1).min(n as u16);
    }

    /// The two axes of the orbit plane in world space (`Rotate*`).
    fn orbit_basis(&self, ctx: &StepCtx) -> (Vec3, Vec3) {
        let d = &self.def;
        if !d.orbits() {
            return (Vec3::X, Vec3::Z);
        }
        let axis = if d.rotate_axis == Vec3::ZERO { Vec3::Y } else { c2b(d.rotate_axis).normalize_or_zero() };
        let axis = if d.rotate_world { axis } else { ctx.rot * axis };
        perpendicular(axis)
    }

    /// Where a particle at `a` of its life is off its path because it orbits (world space).
    #[inline]
    fn orbit_offset(&self, p: &Particle, a: f32, basis: &(Vec3, Vec3)) -> Vec3 {
        if p.rlen == 0.0 {
            return Vec3::ZERO;
        }
        let r = p.rlen * self.def.luts().rot_len.at(a);
        let (s, c) = p.spin.sin_cos();
        (basis.0 * c + basis.1 * s) * r
    }

    /// The light factor of the emitter now: the constant part (`lit`) plus the sun's share. The engine's particle pixel shader lights a
    /// sprite with its camera facing normal (`particles.cfi`: `fIllum = saturate(N.L - N.L * b + b)`, `b` = `DiffuseBacklighting`) and the
    /// sun's colour, times `DiffuseLighting`: a lit smoke is bright with the sun behind the camera and dark against it.
    fn lit_now(&self, ctx: &StepCtx) -> f32 {
        if ctx.sun_light <= 0.0 {
            return self.lit;
        }
        let b = self.def.backlight.clamp(0.0, 1.0);
        let illum = (ctx.sun_facing * (1.0 - b) + b).clamp(0.0, 1.0);
        self.lit + self.def.diffuse * ctx.sun_light * illum
    }

    /// The lights of the live particles (`CParticleContainer::RenderLights`): colour, radius and HDR power come from the
    /// particle's own random draws and the three age curves; the light is the particle's colour, not lit by anything.
    pub fn lights(&self, ctx: &StepCtx, hdr_base: f32, out: &mut Vec<LightSample>) {
        let def = &self.def;
        if !def.has_light() {
            return;
        }
        let luts = def.luts();
        let frame = self.frame(ctx);
        let basis = self.orbit_basis(ctx);
        let hdr_base = hdr_base.max(1e-3);
        for p in &self.particles {
            let a = p.age * p.inv_life;
            // three draws per particle, the engine keeps them as bytes in the particle
            let h = p.seed.to_bits().wrapping_mul(0x9E37_79B1);
            let draw = |k: u32| ((h.rotate_left(k * 11) ^ h.rotate_right(k * 7 + 3)).wrapping_mul(0x85EB_CA6B) >> 8) as f32 / 16_777_216.0;
            let intensity = def.light_intensity.spawn(draw(1), 1.0) * luts.light_intensity.at(a);
            let radius = def.light_radius.spawn(draw(2), 1.0) * luts.light_radius.at(a);
            let hdr = def.light_hdr.spawn(draw(3), 1.0) * luts.light_hdr.at(a);
            let strength = intensity * hdr_base.powf(hdr) * ctx.fade;
            if radius * strength < 1e-3 {
                continue;
            }
            let colour = p.color * luts.color.at(a) * ctx.tint * strength;
            out.push(LightSample { pos: frame.to_world_point(p.pos) + self.orbit_offset(p, a, &basis), colour, radius });
        }
    }

    /// Writes the quads of the live particles.
    fn build(&mut self, ctx: &StepCtx) {
        let def = self.def.clone();
        let luts = def.luts();
        let frame = self.frame(ctx);
        let n = self.particles.len();
        self.pos.clear();
        self.uv.clear();
        self.uv1.clear();
        self.col.clear();
        self.dirty = true;
        if n == 0 {
            self.units = 0;
            return;
        }
        let with_test = def.has_alpha_test();
        self.pos.reserve(n * 4);
        self.uv.reserve(n * 4);
        self.col.reserve(n * 4);

        let inv = ctx.global.inverse();
        let view = ctx.view;
        let tiling = def.tiling;
        let n_tiles = (tiling.x * tiling.y).max(1);
        let du = 1.0 / tiling.x as f32;
        let dv = 1.0 / tiling.y as f32;
        // the engine keeps the longer side of the texture tile at the authored size and shrinks the other
        let (kx, ky) = if self.aspect >= 1.0 { (1.0, 1.0 / self.aspect) } else { (self.aspect, 1.0) };
        let lit = self.lit_now(ctx);
        let facing = def.facing;
        // `Decal` / `Terrain` cards follow the ground (`FxSettings::ground`) when it is known
        let drape = if def.decal && facing == Facing::Flat { ctx.ground } else { None };
        let decal_alpha = if def.decal { def.decal_alpha } else { 1.0 };
        let orient = def.orient_to_velocity;
        let orbit = def.turbulence_size * ctx.scale;
        let orbit_speed = def.turbulence_speed.to_radians();
        let basis = self.orbit_basis(ctx);
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);

        for p in &self.particles {
            let a = p.age * p.inv_life;
            let sz = p.size * luts.size.at(a);
            let alpha = p.alpha * luts.alpha.at(a) * ctx.fade * decal_alpha;
            if alpha < 0.003 || sz < 1e-5 {
                continue;
            }
            let hx = ctx.sprite_half * sz * p.sx * luts.size_x.at(a) * kx;
            let hy = ctx.sprite_half * sz * p.sy * luts.size_y.at(a) * ky;
            let mut center = frame.to_world_point(p.pos) + self.orbit_offset(p, a, &basis);
            if orbit > 0.0 {
                let t = orbit_speed * p.age + p.seed;
                center += (Vec3::X * t.cos() + Vec3::Z * t.sin()) * orbit;
            }
            let colour = engine_gamma(p.color * luts.color.at(a) * ctx.tint, ctx.colour_gamma) * lit * self.mat_tint;

            // frame in the texture atlas
            if p.slot != u32::MAX {
                let colour = [colour.x, colour.y, colour.z];
                let head_axis = self.ribbon_axis(p, ctx);
                let strip = Strip { trail: &self.trail, axes: &self.trail_axis, steps: self.tail_steps, pos: &mut self.pos, uv: &mut self.uv, col: &mut self.col, uv1: &mut self.uv1 };
                let test = if with_test { Some(p.atest * luts.alpha_test.at(a)) } else { None };
                ribbon(strip, p, center, head_axis, hx, alpha, colour, test, &inv, view, &mut min, &mut max);
                continue;
            }

            let mut f = tiling.first + p.variant as u32;
            // `AnimBlend`: the next frame and how far this one has faded into it
            let mut next = None;
            if tiling.frames > 1 {
                let t = if tiling.fps > 0.0 { p.age * tiling.fps } else { a * tiling.frames as f32 };
                let k = t.floor() as i32;
                let last = tiling.frames as i32 - 1;
                let kk = if tiling.cycle { k.rem_euclid(tiling.frames as i32) } else { k.clamp(0, last) };
                f = tiling.first + kk as u32;
                let fade = t - k as f32;
                if tiling.blend && fade > 0.01 && (tiling.cycle || k < last) {
                    let kn = if tiling.cycle { (kk + 1) % tiling.frames as i32 } else { kk + 1 };
                    next = Some(((tiling.first + kn as u32) % n_tiles, fade.min(1.0)));
                }
            }
            let f = f % n_tiles;
            let u0 = (f % tiling.x) as f32 * du;
            let v0 = (f / tiling.x) as f32 * dv;

            // the quad's axes in local space, scaled by the half extents
            let rx: Vec3;
            let uy: Vec3;
            let shift: Vec3;
            match facing {
                Facing::Flat => {
                    let (s, c) = p.ang.z.sin_cos();
                    let r = frame.to_world_dir(Vec3::new(c, 0.0, s));
                    let u = frame.to_world_dir(Vec3::new(-s, 0.0, c));
                    rx = inv.transform_vector3(r) * hx;
                    uy = inv.transform_vector3(u) * hy;
                    // a hair above the surface it lies on (the engine projects these as decals; here they would z-fight)
                    shift = inv.transform_vector3(Vec3::Y * 0.02);
                }
                Facing::Free => {
                    let q = Quat::from_euler(bevy::math::EulerRot::ZYX, p.ang.z, p.ang.y, p.ang.x);
                    let rot = ctx.rot;
                    let mut r = rot * c2b(q * Vec3::X);
                    let mut u = rot * c2b(q * Vec3::Z);
                    if orient && let Some(dir) = frame.to_world_dir(p.vel).try_normalize() {
                        // The engine (`particle::write_sprite`, cry3dengine+0x9cfd0)
                        // rolls the card by the particle's angles first, then spins it about its normal until its up axis (the
                        // SizeY axis) lies along the velocity projected into the card's plane: a streak is long along its flight.
                        // A velocity along the normal leaves the card alone. Small speeds count: several authored rings move at
                        // only 0.001 m/s.
                        let n = rot * c2b(q * Vec3::Y);
                        if let Some(flat) = Some(dir - n * dir.dot(n)).filter(|f| f.length_squared() > 1e-8).and_then(Vec3::try_normalize) {
                            let spin = Quat::from_axis_angle(n, n.dot(u.cross(flat)).atan2(u.dot(flat)));
                            r = spin * r;
                            u = spin * u;
                        }
                    }
                    rx = inv.transform_vector3(r) * hx;
                    uy = inv.transform_vector3(u) * hy;
                    shift = Vec3::ZERO;
                }
                Facing::Velocity => {
                    // the card's normal (cry +Y) is turned the shortest way onto the direction of travel
                    let q = Quat::from_euler(bevy::math::EulerRot::ZYX, p.ang.z, p.ang.y, p.ang.x);
                    let rot = ctx.rot;
                    let (r0, u0, n0) = (rot * c2b(q * Vec3::X), rot * c2b(q * Vec3::Z), rot * c2b(q * Vec3::Y));
                    let arc = Quat::from_rotation_arc(n0.normalize_or_zero(), frame.to_world_dir(p.dir).normalize_or_zero());
                    rx = inv.transform_vector3(arc * r0) * hx;
                    uy = inv.transform_vector3(arc * u0) * hy;
                    shift = Vec3::ZERO;
                }
                Facing::Camera => {
                    // The engine's camera-facing sprite is a 2x2 matrix over the camera's right and up vectors: the roll
                    // `ang.z`, then (OrientToVelocity) a turn that puts the up row on the screen direction of the velocity plus
                    // that roll, then (Stretch) every row grows along the screen velocity by `velocity * stretch` weighted by
                    // how much the row points that way, and the centre moves on by `StretchOffsetRatio * stretch * velocity`.
                    let (s, c) = p.ang.z.sin_cos();
                    let (mut r, mut u) = (Vec2::new(c, s), Vec2::new(-s, c));
                    let k = p.stretch * luts.stretch.at(a);
                    let mut sh = Vec3::ZERO;
                    let vel = frame.to_world_dir(p.vel - p.inh);
                    let vs = Vec2::new(vel.dot(view.right), vel.dot(view.up));
                    let vl = vs.length();
                    let (mut rr, mut uu) = (r * hx, u * hy);
                    if (orient || k != 0.0) && vl > 1e-4 {
                        let d = vs / vl;
                        if orient {
                            let turn = |v: Vec2| Vec2::new(d.y * v.x + d.x * v.y, d.y * v.y - d.x * v.x);
                            r = turn(r);
                            u = turn(u);
                            rr = r * hx;
                            uu = u * hy;
                        }
                        if k != 0.0 {
                            if hx > 1e-6 {
                                rr += vs * (k * rr.dot(d) / hx);
                            }
                            if hy > 1e-6 {
                                uu += vs * (k * uu.dot(d) / hy);
                            }
                            sh = vel * (def.stretch_offset * k);
                        }
                    }
                    rx = inv.transform_vector3(view.right * rr.x + view.up * rr.y);
                    uy = inv.transform_vector3(view.right * uu.x + view.up * uu.y);
                    shift = inv.transform_vector3(sh);
                }
            }
            let c = inv.transform_point3(center) + shift;
            let v = [c - rx - uy, c + rx - uy, c + rx + uy, c - rx + uy];
            // Project even one-cell cards, and share the sampled terrain grid
            // with the public reticle helper. Ordinary sprites use their quad.
            let ground = drape.filter(|g| g(center).is_some());
            if let Some(ground) = ground {
                crate::decal::drape_into(
                    &mut self.decal_grid, ctx.global.transform_point3(c),
                    ctx.global.transform_vector3(rx), ctx.global.transform_vector3(uy),
                    ground, crate::decal::LIFT, crate::decal::cells_for(hx, hy),
                );
                for p in &mut self.decal_grid.positions {
                    let local = inv.transform_point3(Vec3::from(*p));
                    *p = local.to_array();
                    min = min.min(local);
                    max = max.max(local);
                }
            } else {
                for p in &v {
                    min = min.min(*p);
                    max = max.max(*p);
                }
            }
            // a blended frame is two quads on top of each other, weighted (1 - fade) and fade: exact for additive blends,
            // and within a few percent of the engine's lerp of the two frames for the others
            let (w0, second) = match next {
                Some((fn_, fade)) => (1.0 - fade, Some((fn_, fade))),
                None => (1.0, None),
            };
            for (tile, w) in std::iter::once((f, w0)).chain(second) {
                let (u0, v0) = if tile == f { (u0, v0) } else { ((tile % tiling.x) as f32 * du, (tile / tiling.x) as f32 * dv) };
                let colour = [colour.x, colour.y, colour.z, (alpha * w).min(1.0)];
                if ground.is_none() {
                    self.pos.extend_from_slice(&[v[0].to_array(), v[1].to_array(), v[2].to_array(), v[3].to_array()]);
                    self.uv.extend_from_slice(&[[u0, v0 + dv], [u0 + du, v0 + dv], [u0 + du, v0], [u0, v0]]);
                    if with_test {
                        self.uv1.extend_from_slice(&[[p.atest * luts.alpha_test.at(a), 0.0]; 4]);
                    }
                    self.col.extend_from_slice(&[colour, colour, colour, colour]);
                    continue;
                }
                let grid = &self.decal_grid;
                let cells = grid.cells;
                let width = cells + 1;
                for j in 0..cells {
                    for i in 0..cells {
                        let a0 = j * width + i;
                        for k in [a0, a0 + 1, a0 + width + 1, a0 + width] {
                            self.pos.push(grid.positions[k]);
                            let [s, t] = grid.uvs[k];
                            self.uv.push([u0 + s * du, v0 + t * dv]);
                        }
                        if with_test {
                            self.uv1.extend_from_slice(&[[p.atest * luts.alpha_test.at(a), 0.0]; 4]);
                        }
                        self.col.extend_from_slice(&[colour, colour, colour, colour]);
                    }
                }
            }
        }
        self.units = self.pos.len() / 4;
        if self.units > 0 {
            self.min = min;
            self.max = max;
        }
    }

    /// Writes the mesh copies of the live particles: the template transformed by every particle's position, orientation and scale.
    fn build_mesh(&mut self, ctx: &StepCtx, time: f32) {
        let Some(geom) = self.geom.clone() else { return };
        let def = self.def.clone();
        let luts = def.luts();
        let frame = self.frame(ctx);
        let (t, m) = (&geom.tmpl, &geom.mtl);
        let nv = t.pos.len();
        self.pos.clear();
        self.uv.clear();
        self.uv1.clear();
        self.col.clear();
        self.dirty = true;
        if self.particles.is_empty() || nv == 0 {
            self.units = 0;
            return;
        }
        let n = self.particles.len();
        self.pos.reserve(n * nv);
        self.uv.reserve(n * nv);
        self.col.reserve(n * nv);
        let with_mask = !m.opacity_map.is_empty();
        if with_mask {
            self.uv1.reserve(n * nv);
        }

        let inv = ctx.global.inverse();
        let inv3 = Mat3::from(inv.matrix3);
        let to_bevy = aa_cry::CRY_TO_BEVY;
        let static_uv = m.uv.is_static_identity();
        let opaque = m.blend == Blend::Opaque;
        let pivot = if def.no_offset { Vec3::ZERO } else { t.center };
        let alpha_k = if def.no_material_alpha { 1.0 } else { m.opacity };
        // RenderGeometry has its own lighting; the camera-facing particle shader
        // (`particles.cfi`) supplies the wrapped sun term only for sprites.
        let lit = self.lit + m.emissive * 0.5;
        let basis = self.orbit_basis(ctx);
        let to_local = inv3.x_axis.length().max(inv3.y_axis.length()).max(inv3.z_axis.length()).max(1.0);
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut units = 0;

        for p in &self.particles {
            let a = p.age * p.inv_life;
            let sz = p.size * luts.size.at(a);
            let alpha = p.alpha * luts.alpha.at(a) * ctx.fade * alpha_k;
            if alpha < 0.003 || sz < 1e-5 {
                continue;
            }
            // the scales are along the mesh's CryEngine axes: right, forward, up
            let scale = Vec3::new(p.sx * luts.size_x.at(a), p.sz * luts.size_z.at(a), p.sy * luts.size_y.at(a)) * sz;
            let q = Quat::from_euler(bevy::math::EulerRot::ZYX, p.ang.z, p.ang.y, p.ang.x);
            let turn = Mat3::from_quat(to_bevy * q * to_bevy.inverse());
            let vel = frame.to_world_dir(p.vel);
            let speed = vel.length();
            let rot = if def.facing == Facing::Velocity {
                // the mesh's forward axis (CryEngine +Y) is turned the shortest way onto the direction of travel
                let base = ctx.rot * turn;
                let to = frame.to_world_dir(p.dir).normalize_or_zero();
                Mat3::from_quat(Quat::from_rotation_arc((base * Vec3::NEG_Z).normalize_or_zero(), to)) * base
            } else if def.orient_to_velocity && speed > 0.02 {
                // the mesh's forward axis (CryEngine +Y) along the velocity
                Mat3::from_quat(Quat::from_rotation_arc(Vec3::NEG_Z, vel / speed)) * turn
            } else {
                ctx.rot * Mat3::from_diagonal(ctx.axis) * turn
            };
            let center = inv.transform_point3(frame.to_world_point(p.pos) + self.orbit_offset(p, a, &basis));
            let lin = inv3 * rot * Mat3::from_diagonal(scale);
            let shift = center - lin * pivot;
            let c = if m.lit { engine_gamma(p.color * luts.color.at(a) * ctx.tint, ctx.colour_gamma) * m.tint } else { engine_gamma(p.color * luts.color.at(a) * ctx.tint * m.tint, ctx.colour_gamma) * lit };

            self.pos.extend(t.pos.iter().map(|v| (lin * *v + shift).to_array()));
            if static_uv {
                self.uv.extend_from_slice(&t.uv);
            } else {
                let f = m.uv.frame(if def.independent_uv_timer { p.age } else { time });
                self.uv.extend(t.uv.iter().map(|u| f.apply(*u)));
            }
            if with_mask {
                self.uv1.extend_from_slice(&t.uv);
            }
            if m.lit {
                // a lit surface: reflectance times sun and sky (the real stones and golem hands are as bright as their textures)
                self.col.extend(t.nrm.iter().map(|n| {
                    let e = LIT_SUN * (rot * *n).dot(ctx.sun).max(0.0) + LIT_SKY;
                    [c.x * e, c.y * e, c.z * e, alpha.min(1.0)]
                }));
            } else if opaque {
                self.col.extend(t.nrm.iter().map(|n| {
                    let s = 0.45 + 0.75 * (rot * *n).dot(ctx.sun).max(0.0);
                    [c.x * s, c.y * s, c.z * s, alpha.min(1.0)]
                }));
            } else {
                self.col.extend(std::iter::repeat_n([c.x, c.y, c.z, alpha.min(1.0)], nv));
            }
            let mid = center + lin * (t.center - pivot);
            let r = Vec3::splat(t.extent * scale.max_element() * 0.9 * to_local);
            min = min.min(mid - r);
            max = max.max(mid + r);
            units += 1;
        }
        self.units = units;
        if units > 0 {
            self.min = min;
            self.max = max;
        }
    }
}

/// The output arrays of an emitter that a tail is written to.
struct Strip<'a> {
    trail: &'a [Vec3],
    /// The width axis per sample (empty: the ribbon faces the camera).
    axes: &'a [Vec3],
    steps: usize,
    pos: &'a mut Vec<[f32; 3]>,
    uv: &'a mut Vec<[f32; 2]>,
    col: &'a mut Vec<[f32; 4]>,
    uv1: &'a mut Vec<[f32; 2]>,
}

/// A particle's tail as a strip of quads from its head (the current position) back along its history; it tapers and fades
/// towards the end. The texture runs along the ribbon, `u = 0` at the head (the sword line textures are bright there) and
/// `v = 0` on the `-side` edge. A `Facing = Free` ribbon (`head_axis` not zero) is as wide as the particle's axis says, which
/// is the blade of a weapon: its `+axis` edge is `v = 0`, the edge the sword line textures are lit on, so the bright line
/// follows the tip. The other ribbons face the camera.
#[allow(clippy::too_many_arguments)]
fn ribbon(out: Strip, p: &Particle, head: Vec3, head_axis: Vec3, half: f32, alpha: f32, colour: [f32; 3], test: Option<f32>, inv: &Affine3A, view: View, min: &mut Vec3, max: &mut Vec3) {
    // points: the head, then the samples that are not on top of it
    let mut pts: [Vec3; MAX_TAIL_STEPS + 1] = [Vec3::ZERO; MAX_TAIL_STEPS + 1];
    let mut axes: [Vec3; MAX_TAIL_STEPS + 1] = [Vec3::ZERO; MAX_TAIL_STEPS + 1];
    let mut n = 1;
    pts[0] = head;
    axes[0] = head_axis;
    let steps = out.steps;
    let free = !out.axes.is_empty();
    for i in 0..p.t_len as usize {
        let k = p.slot as usize * steps + (p.t_head as usize + steps - 1 - i) % steps;
        let s = out.trail[k];
        if n <= MAX_TAIL_STEPS && s.distance_squared(pts[n - 1]) > 1e-8 {
            pts[n] = s;
            if free {
                axes[n] = out.axes[k];
            }
            n += 1;
        }
    }
    if n < 2 {
        return;
    }
    let mut side = [Vec3::ZERO; MAX_TAIL_STEPS + 1];
    let mut last = view.right;
    for i in 0..n {
        if free && axes[i].length_squared() > 0.5 {
            side[i] = -axes[i];
            last = side[i];
            continue;
        }
        let dir = pts[i.saturating_sub(1)] - pts[(i + 1).min(n - 1)];
        let s = dir.cross(view.forward);
        last = if s.length_squared() > 1e-10 { s.normalize() } else { last };
        side[i] = last;
    }
    for i in 0..n - 1 {
        let (t0, t1) = (i as f32 / (n - 1) as f32, (i + 1) as f32 / (n - 1) as f32);
        // a camera facing ribbon tapers; a blade's sheet keeps the blade's length and only fades
        let taper = if free { 0.0 } else { 0.8 };
        let (w0, w1) = (half * (1.0 - taper * t0), half * (1.0 - taper * t1));
        let c = |t: f32| [colour[0], colour[1], colour[2], alpha * (1.0 - t).clamp(0.0, 1.0)];
        let v = [
            inv.transform_point3(pts[i] - side[i] * w0),
            inv.transform_point3(pts[i] + side[i] * w0),
            inv.transform_point3(pts[i + 1] + side[i + 1] * w1),
            inv.transform_point3(pts[i + 1] - side[i + 1] * w1),
        ];
        for q in &v {
            *min = min.min(*q);
            *max = max.max(*q);
        }
        out.pos.extend_from_slice(&[v[0].to_array(), v[1].to_array(), v[2].to_array(), v[3].to_array()]);
        out.uv.extend_from_slice(&[[t0, 0.0], [t0, 1.0], [t1, 1.0], [t1, 0.0]]);
        out.col.extend_from_slice(&[c(t0), c(t0), c(t1), c(t1)]);
        if let Some(t) = test {
            out.uv1.extend_from_slice(&[[t, 0.0]; 4]);
        }
    }
}

/// A point on (or in) the shape an attached emitter emits from, in CryEngine axes (Z up, the entity's feet at the origin), and
/// the outward normal there. The shapes are generic: a human sized capsule for meshes and physics proxies, a box for bounds.
fn attach_point(def: &EmitterDef, r: &mut Rng) -> (Vec3, Vec3) {
    let volume = def.attach_form == AttachForm::Volume;
    match def.attach {
        Attach::BoundingBox => {
            let half = Vec3::new(0.4, 0.3, 0.9);
            let centre = Vec3::new(0.0, 0.0, 0.9);
            if volume {
                return (centre + Vec3::new(r.signed(), r.signed(), r.signed()) * half, Vec3::Z);
            }
            // a random face, weighted by its area
            let area = [half.y * half.z, half.x * half.z, half.x * half.y];
            let total = area[0] + area[1] + area[2];
            let pick = r.next() * total;
            let axis = if pick < area[0] { 0 } else if pick < area[0] + area[1] { 1 } else { 2 };
            let sign = if r.next() < 0.5 { -1.0 } else { 1.0 };
            let mut p = Vec3::new(r.signed(), r.signed(), r.signed()) * half;
            let mut n = Vec3::ZERO;
            p[axis] = sign * half[axis];
            n[axis] = sign;
            (centre + p, n)
        }
        _ => {
            // a capsule: radius 0.3, from the ankles to above the head
            let radius = 0.3;
            let z = 0.15 + r.next() * 1.55;
            let a = r.next() * std::f32::consts::TAU;
            let dir = Vec3::new(a.cos(), a.sin(), 0.0);
            if volume {
                (dir * (radius * r.next().sqrt()) + Vec3::Z * z, dir)
            } else {
                (dir * radius + Vec3::Z * z, dir)
            }
        }
    }
}

fn perpendicular(v: Vec3) -> (Vec3, Vec3) {
    let a = if v.x.abs() < 0.9 { Vec3::X } else { Vec3::Z };
    let t = v.cross(a).normalize_or_zero();
    (t, v.cross(t))
}

/// A running effect: its emitters, parents first.
pub struct EffectSim {
    pub emitters: Vec<EmitterState>,
    pub time: f32,
    pub primed: bool,
}

impl EffectSim {
    pub fn new(mut emitters: Vec<EmitterState>, light: f32, emissive: f32, hdr_base: f32) -> EffectSim {
        let triggers: Vec<(Option<usize>, Spawn)> = emitters.iter().map(|e| (e.parent, e.def.spawn)).collect();
        for (parent, spawn) in triggers {
            if let Some(p) = parent {
                match spawn {
                    Spawn::ParentStart => emitters[p].want_births = true,
                    Spawn::ParentDeath => emitters[p].want_deaths = true,
                    Spawn::ParentCollide => emitters[p].want_collides = true,
                    _ => {}
                }
            }
        }
        let mut sim = EffectSim { emitters, time: 0.0, primed: false };
        sim.set_lighting(light, emissive, hdr_base);
        sim
    }

    /// Refresh scene lighting without changing particle ages, random draws or emission.
    pub(crate) fn set_lighting(&mut self, light: f32, emissive: f32, hdr_base: f32) {
        for e in &mut self.emitters {
            let d = &e.def;
            // RenderGeometry: colour *= diffuse + emissive * pow(HDR multiplier, EmissiveHDRDynamic).
            e.lit = d.diffuse * light + d.backlight * light * 0.5 + d.emissive * emissive * hdr_base.max(1e-3).powf(d.emissive_hdr);
        }
    }

    pub fn alive(&self) -> usize {
        self.emitters.iter().map(|e| e.alive()).sum()
    }

    /// Particle budget the effect uses.
    pub fn load(&self) -> usize {
        self.emitters.iter().map(|e| e.load()).sum()
    }

    /// Every emitter is done and no particle is left.
    pub fn finished(&self) -> bool {
        self.emitters.iter().all(|e| e.finished())
    }

    /// Forgets every particle (the effect went out of range).
    pub fn reset(&mut self) {
        for e in &mut self.emitters {
            e.reset();
        }
        self.time = 0.0;
        self.primed = false;
    }

    fn advance(&mut self, ctx: &StepCtx, dt: f32) {
        for e in &mut self.emitters {
            e.births.clear();
            e.deaths.clear();
            e.collides.clear();
            e.emit(ctx, dt);
            e.update(ctx, dt);
        }
        // sub emitters start where their parent's particles were born or died
        for i in 0..self.emitters.len() {
            let Some(p) = self.emitters[i].parent else { continue };
            let spawn = self.emitters[i].def.spawn;
            if self.emitters[i].follows_parent() {
                // the ones riding on a parent particle move on with it; new ones start at its birth
                let parents: Vec<(u32, Vec3)> = {
                    let pe = &self.emitters[p];
                    let frame = pe.frame(ctx);
                    let basis = pe.orbit_basis(ctx);
                    pe.particles.iter().map(|q| (q.id, frame.to_world_point(q.pos) + pe.orbit_offset(q, q.age * q.inv_life, &basis))).collect()
                };
                let births = self.emitters[p].births.clone();
                let child = &mut self.emitters[i];
                child.follow(ctx, dt, &parents);
                for (id, at) in births {
                    child.followers.push(Follower { parent_id: id, t: 0.0, acc: 1.0 });
                    // the first particle at once, from where the parent was born
                    let n = child.grant(ctx, 1);
                    for _ in 0..n {
                        child.spawn(ctx, Some(at), 0.0, 0.0);
                    }
                    if let Some(f) = child.followers.last_mut() {
                        f.acc = 0.0;
                    }
                }
                continue;
            }
            let events: Vec<Vec3> = match spawn {
                Spawn::ParentStart => self.emitters[p].births.iter().map(|b| b.1).collect(),
                Spawn::ParentDeath => self.emitters[p].deaths.clone(),
                Spawn::ParentCollide => self.emitters[p].collides.clone(),
                _ => continue,
            };
            for at in events {
                self.emitters[i].emit_from_parent(ctx, at);
            }
        }
    }

    /// One frame: emit, move, write the vertices.
    pub fn step(&mut self, ctx: &StepCtx) {
        self.time += ctx.dt;
        self.advance(ctx, ctx.dt);
        let time = self.time;
        for e in &mut self.emitters {
            if e.draws {
                if e.geom.is_some() {
                    e.build_mesh(ctx, time);
                } else if e.pending_geom.is_none() {
                    e.build(ctx);
                }
            }
        }
    }

    /// Runs the effect ahead so that it appears already burning (`bPrime`): up to eight seconds in coarse steps.
    pub fn prime(&mut self, ctx: &StepCtx) {
        let span = self
            .emitters
            .iter()
            .map(|e| {
                let life = if e.def.particle_life.base > 0.0 { e.def.particle_life.base } else { 2.0 };
                life + e.def.spawn_delay.base + if e.def.spawn != Spawn::Direct { 1.0 } else { 0.0 }
            })
            .fold(0.0f32, f32::max)
            .min(8.0);
        let steps = if span < 0.3 { 3 } else { 10 };
        let dt = span / steps as f32;
        for _ in 0..steps {
            self.advance(ctx, dt);
        }
        self.time = span;
        self.primed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;

    fn ctx(budget: &AtomicI32) -> StepCtx<'_> {
        StepCtx {
            dt: 1.0 / 60.0,
            view: View { pos: Vec3::new(0.0, 1.0, 5.0), right: Vec3::X, up: Vec3::Y, forward: Vec3::NEG_Z },
            global: Affine3A::IDENTITY,
            rot: Mat3::IDENTITY,
            scale: 1.0,
            axis: Vec3::ONE,
            speed_scale: 1.0,
            count_scale: 1.0,
            lod: 1.0,
            size_scale: 1.0,
            sprite_half: 1.0,
            colour_gamma: 2.2,
            count_over_life: true,
            tint: Vec3::ONE,
            sun: Vec3::Y,
            sun_light: 0.0,
            sun_facing: 0.0,
            emitter_vel: Vec3::ZERO,
            ground: None,
            wind: Vec3::ZERO,
            body: None,
            no_body: false,
            budget,
            fade: 1.0,
        }
    }

    fn sim(xml_params: &str) -> EffectSim {
        let xml = format!(r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs><Particles Name="e"><Params {xml_params}/></Particles></Childs></Particles></ParticleLibrary>"#);
        let lib = Library::parse(&xml).unwrap();
        let eff = lib.get("a").unwrap();
        let em = eff
            .emitters
            .iter()
            .enumerate()
            .map(|(i, e)| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7 + i as u64, bevy::prelude::Entity::PLACEHOLDER))
            .collect();
        EffectSim::new(em, 0.8, 1.0, 1.0)
    }

    #[test]
    fn a_continuous_emitter_emits_its_first_particle_at_once() {
        let budget = AtomicI32::new(10_000);
        // two particles per 4 s: one on the first frame, the next one 2 s later
        let mut s = sim(r#"Continuous="true" Count="2" ParticleLifeTime="4" Texture="textures/x.dds" Speed="0""#);
        let c = ctx(&budget);
        s.step(&c);
        assert_eq!(s.alive(), 1);
        for _ in 0..59 {
            s.step(&c);
        }
        assert_eq!(s.alive(), 1, "nothing new after 1 s");
        for _ in 0..120 {
            s.step(&c);
        }
        assert_eq!(s.alive(), 2, "the second one after 2 s");
    }

    #[test]
    fn a_particle_shorter_than_the_frame_is_still_drawn_every_frame() {
        // the beam mesh of `life_absorb_launch`: one particle of 0.05 s, at 10, 30 and 60 frames per second
        for fps in [10.0f32, 30.0, 60.0] {
            let budget = AtomicI32::new(10_000);
            let mut s = sim(r#"Continuous="true" ParticleLifeTime="0.05" Texture="textures/x.dds" Speed="0""#);
            let mut c = ctx(&budget);
            c.dt = 1.0 / fps;
            let mut seen = Vec::new();
            for _ in 0..200 {
                s.step(&c);
                seen.push(s.alive());
            }
            assert!(seen.iter().all(|&n| n >= 1), "{fps} fps: a frame without the particle: {seen:?}");
            assert!(seen.iter().all(|&n| n <= 2), "{fps} fps: too many at once: {seen:?}");
            let mean = seen.iter().sum::<usize>() as f32 / seen.len() as f32;
            assert!((0.9..=1.6).contains(&mean), "{fps} fps: {mean} alive on average");
        }
    }

    /// Dash's `speed_full_pc_meshe_high` (Count 2, ParticleLifeTime 0.4, no EmitterLifeTime) restarts its shell for as long as the buff
    /// holds the effect; as a first cycle it emits two shells 0.2 s apart and is done after 0.4 s.
    #[test]
    fn a_first_cycle_emitter_stops_after_one_particle_life() {
        let xml = r#"Continuous="true" Count="2" ParticleLifeTime="0.4" Texture="textures/x.dds" Speed="-0.2" SpawnDelay="0.1""#;
        let budget = AtomicI32::new(10_000);
        let c = ctx(&budget);
        let mut endless = sim(xml);
        let mut once = {
            let xml = format!(r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs><Particles Name="e"><Params {xml}/></Particles></Childs></Particles></ParticleLibrary>"#);
            let lib = Library::parse(&xml).unwrap();
            let def = lib.get("a").unwrap().emitters[0].def.first_cycle().expect("a continuous emitter with no life of its own");
            assert_eq!((def.emitter_life.base, def.particle_life.base, def.count.base), (0.4, 0.4, 2.0));
            EffectSim::new(vec![EmitterState::new(std::sync::Arc::new(def), None, 0, 1.0, 7, bevy::prelude::Entity::PLACEHOLDER)], 0.8, 1.0, 1.0)
        };
        let (mut most, mut born) = (0, 0);
        for _ in 0..(5.0 / c.dt) as usize {
            endless.step(&c);
            let before = once.alive();
            once.step(&c);
            most = most.max(once.alive());
            born += once.alive().saturating_sub(before);
        }
        assert_eq!(once.alive(), 0, "nothing is left after the first cycle");
        assert!(once.finished());
        assert!(most <= 2 && born >= 2, "two shells in all: at most {most} at once, {born} born");
        assert!(endless.alive() >= 1 && !endless.finished(), "the continuous emitter is still going");
        // an emitter that ends already is left as it is
        let ends = r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs><Particles Name="e"><Params Continuous="true" Count="2" EmitterLifeTime="0.1" ParticleLifeTime="0.4" Texture="textures/x.dds"/></Particles><Particles Name="b"><Params Count="2" Texture="textures/x.dds"/></Particles></Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(ends).unwrap();
        assert!(lib.get("a").unwrap().emitters.iter().all(|e| e.def.first_cycle().is_none()));
    }

    #[test]
    fn continuous_emitter_reaches_its_count() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Continuous="true" Count="20" ParticleLifeTime="2" Texture="textures/x.dds" Speed="1""#);
        let c = ctx(&budget);
        for _ in 0..300 {
            s.step(&c);
        }
        let n = s.alive();
        assert!((16..=24).contains(&n), "alive {n}");
        assert_eq!(s.emitters[0].units, n);
        assert_eq!(s.emitters[0].pos.len(), n * 4);
    }

    #[test]
    fn count_spreads_over_the_emitter_life_and_pulses_overlap() {
        // flow_fog_01: Count 10 over EmitterLifeTime 1, restarted every 0.5 s, particles live 3.5 s: 2 emissions of 10 / s
        let xml = r#"Continuous="true" Count="10" EmitterLifeTime="1" PulsePeriod="0.5" ParticleLifeTime="3.5" Texture="textures/x.dds" Speed="1""#;
        let budget = AtomicI32::new(10_000);
        let mut s = sim(xml);
        let c = ctx(&budget);
        for _ in 0..600 {
            s.step(&c);
        }
        let n = s.alive();
        assert!((60..=80).contains(&n), "alive {n}");
        // the old reading: Count alive at once
        let mut old = ctx(&budget);
        old.count_over_life = false;
        let mut s = sim(xml);
        for _ in 0..600 {
            s.step(&old);
        }
        let n = s.alive();
        assert!((8..=12).contains(&n), "alive {n}");
    }

    fn triangle() -> std::sync::Arc<Geom> {
        use crate::geom::{GeomMtl, GeomTemplate};
        let tmpl = GeomTemplate {
            pos: vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            nrm: vec![Vec3::Z; 3],
            uv: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            idx: vec![0, 1, 2],
            center: Vec3::new(0.5, 0.5, 0.0),
            extent: 1.0,
        };
        std::sync::Arc::new(Geom { tmpl, mtl: GeomMtl { blend: Blend::Additive, ..Default::default() } })
    }

    #[test]
    fn mesh_particles_instance_the_template() {
        let budget = AtomicI32::new(10_000);
        // two copies, twice the size, no centring: the first vertex stays at the particle
        let mut s = sim(r#"Count="2" ParticleLifeTime="2" Geometry="x.cgf" NoOffset="true" Size="2" Speed="0""#);
        s.emitters[0].set_geom(triangle());
        let c = ctx(&budget);
        s.step(&c);
        let e = &s.emitters[0];
        assert_eq!(e.units, 2);
        assert_eq!(e.pos.len(), 6);
        assert_eq!(e.col.len(), 6);
        let (a, b) = (Vec3::from(e.pos[0]), Vec3::from(e.pos[1]));
        assert!(a.length() < 0.01, "{a:?}");
        // the template's +X is the mesh's right: along Bevy +X, scaled by Size
        assert!(((b - a) - Vec3::new(2.0, 0.0, 0.0)).length() < 0.01, "{:?}", b - a);
        // the template's +Y is Bevy up here: SizeZ would scale it; Size scales it by 2 as well
        let u = Vec3::from(e.pos[2]) - a;
        assert!((u - Vec3::new(0.0, 2.0, 0.0)).length() < 0.01, "{u:?}");
        // the cost of a mesh particle in the budget is its vertices in quads
        assert_eq!(e.load(), 2);
        // centred: the box centre sits on the particle
        let mut s = sim(r#"Count="1" ParticleLifeTime="2" Geometry="x.cgf" Size="2" Speed="0""#);
        s.emitters[0].set_geom(triangle());
        s.step(&c);
        let mid = (Vec3::from(s.emitters[0].pos[1]) + Vec3::from(s.emitters[0].pos[2])) * 0.5;
        assert!(mid.length() < 0.01, "{mid:?}");
    }

    #[test]
    fn particles_orbit_the_emitter() {
        let budget = AtomicI32::new(10_000);
        // radius 2 about the vertical axis, a quarter turn per second, starting on +X
        let mut s = sim(r#"Count="1" ParticleLifeTime="4" Texture="textures/x.dds" Speed="0" Size="0.1" RotateLength="2" RotateSpeed="1.5707964" RotatePosition=",,1" RotateInitAngle="0""#);
        let c = ctx(&budget);
        let centre = |s: &EffectSim| {
            let e = &s.emitters[0];
            let q: Vec<Vec3> = e.pos[0..4].iter().map(|p| Vec3::from(*p)).collect();
            (q[0] + q[2]) * 0.5
        };
        s.step(&c);
        let p0 = centre(&s);
        assert!((p0.length() - 2.0).abs() < 0.05 && p0.y.abs() < 0.01, "{p0:?}");
        for _ in 0..60 {
            s.step(&c);
        }
        // after one second a quarter turn: 90 degrees away, still at radius 2 in the horizontal plane
        let p1 = centre(&s);
        assert!((p1.length() - 2.0).abs() < 0.05 && p1.y.abs() < 0.01, "{p1:?}");
        assert!(p0.dot(p1).abs() < 0.2, "{p0:?} {p1:?}");
    }

    #[test]
    fn particles_bounce_on_the_ground_and_start_collision_children() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs>
            <Particles Name="rock"><Params Count="1" ParticleLifeTime="4" Texture="textures/x.dds" Speed="0" GravityScale="1" PositionOffset="0,0,1" PhysicsType="SimpleCollision" CollideTerrain="true" Bounciness="0.5" Size="0.1"/>
            <Childs><Particles Name="dust"><Params Count="3" ParticleLifeTime="1" Texture="textures/x.dds" Speed="0" SpawnIndirection="ParentCollide"/></Particles></Childs></Particles>
            </Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        let eff = lib.get("a").unwrap();
        let em = eff.emitters.iter().enumerate().map(|(i, e)| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7 + i as u64, bevy::prelude::Entity::PLACEHOLDER)).collect();
        let mut s = EffectSim::new(em, 0.8, 1.0, 1.0);
        assert_eq!(s.emitters.len(), 2);
        let budget = AtomicI32::new(10_000);
        let ground = |_: Vec3| Some(0.0f32);
        let mut c = ctx(&budget);
        c.ground = Some(&ground);
        let (mut lowest, mut highest_after_hit, mut hit_at) = (f32::MAX, 0.0f32, None);
        for i in 0..240 {
            s.step(&c);
            if let Some(p) = s.emitters[0].particles.first() {
                lowest = lowest.min(p.pos.y);
                if hit_at.is_some() {
                    highest_after_hit = highest_after_hit.max(p.pos.y);
                } else if s.emitters[1].alive() > 0 {
                    hit_at = Some(i);
                }
            }
        }
        // it fell 1 m (about 0.45 s), never went below the ground, bounced up again and the child started once
        assert!(lowest > -0.01, "went through the ground: {lowest}");
        assert!(hit_at.is_some_and(|i| (20..40).contains(&i)), "first hit at {hit_at:?}");
        assert!(highest_after_hit > 0.05, "no bounce: {highest_after_hit}");
        assert!(s.emitters[1].alive() <= 3, "child started more than once: {}", s.emitters[1].alive());
        // without a ground nothing happens
        let mut s2 = {
            let em = eff.emitters.iter().enumerate().map(|(i, e)| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7 + i as u64, bevy::prelude::Entity::PLACEHOLDER)).collect();
            EffectSim::new(em, 0.8, 1.0, 1.0)
        };
        let c = ctx(&budget);
        for _ in 0..90 {
            s2.step(&c);
        }
        assert_eq!(s2.emitters[1].alive(), 0);
    }

    #[test]
    fn a_tail_draws_the_past_path_as_a_ribbon() {
        let budget = AtomicI32::new(10_000);
        // one particle flying along +X at 5 m/s (focus axis is up, so tilt it 90 degrees), tail of 1 s in 10 steps
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Size="0.2" Speed="5" FocusAngle="90" FocusAzimuth="0" TailLength="1" TailSteps="10""#);
        let c = ctx(&budget);
        for _ in 0..30 {
            s.step(&c);
        }
        let e = &s.emitters[0];
        // a ribbon of several quads, not one sprite
        assert!(e.units >= 5, "{} quads", e.units);
        let span = (e.max - e.min).max_element();
        // 0.5 s at 5 m/s of flight, all of it within the 1 s tail
        assert!((span - 2.5).abs() < 0.6, "span {span}");
        // the ribbon fades towards its end
        let heads: Vec<f32> = e.col.iter().map(|c| c[3]).collect();
        assert!(heads[0] > heads[heads.len() - 1], "{heads:?}");
    }

    /// A `Facing = Free` ribbon is as wide as the particle's +Y axis (cry; Bevy -Z) says, whatever the camera sees: the sheet a
    /// blade sweeps. The other ribbons face the camera.
    #[test]
    fn a_free_ribbon_takes_its_width_from_the_emitter_axis() {
        let budget = AtomicI32::new(10_000);
        let attrs = r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Size="0.2" Speed="5" FocusAngle="90" FocusAzimuth="0" TailLength="1" TailSteps="10""#;
        let run = |extra: &str| {
            let mut s = sim(&format!("{attrs} {extra}"));
            let c = ctx(&budget);
            for _ in 0..30 {
                s.step(&c);
            }
            let e = &s.emitters[0];
            e.max - e.min
        };
        // the particle flies in the x / y plane (the focus axis is cry +Y = Bevy -Z, tilted by 90 degrees): the free ribbon is
        // 0.4 m wide along z, the cry Y axis
        let free = run(r#"Facing="Free""#);
        assert!((free.z - 0.4).abs() < 0.05, "free ribbon extent {free:?}");
        // the camera looks along -z, so a camera facing ribbon is wide in the plane of x / y, not along z
        let cam = run("");
        assert!(cam.z < 0.05, "camera ribbon extent {cam:?}");
    }

    #[test]
    fn a_free_sprite_puts_its_up_axis_on_even_a_slow_velocity() {
        let budget = AtomicI32::new(10_000);
        for bound in [false, true] {
            for speed in [0.001_f32, -0.001, 3.0] {
                let mut s = sim(&format!(
                    r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Facing="Free" OrientToVelocity="true" Speed="{speed}" FocusGravityDir="true" BindToEmitter="{bound}""#
                ));
                let mut c = ctx(&budget);
                let rotation = Quat::from_rotation_z(0.7) * Quat::from_rotation_y(0.4);
                c.rot = Mat3::from_quat(rotation);
                c.global = Affine3A::from_scale_rotation_translation(Vec3::splat(2.0), rotation, Vec3::new(4.0, 2.0, -3.0));
                s.step(&c);
                let e = &s.emitters[0];
                assert_eq!(e.units, 1);
                let up = c.global.transform_vector3(Vec3::from(e.pos[3]) - Vec3::from(e.pos[0])).normalize();
                // FocusGravityDir emits vertically in world space, even on a rotated, bound emitter; the card's up (its SizeY axis)
                // lies along that velocity projected into the card's plane (its normal is cry +Y = Bevy -Z of the emitter)
                let normal = rotation * Vec3::NEG_Z;
                let velocity = Vec3::Y * speed.signum();
                let expected = (velocity - normal * velocity.dot(normal)).normalize();
                assert!(up.dot(expected) > 0.999, "bound={bound}, speed={speed}: {up:?} {expected:?}");
                let edge = c.global.transform_vector3(Vec3::from(e.pos[1]) - Vec3::from(e.pos[0])).normalize();
                assert!(edge.dot(up).abs() < 0.001);
            }
        }
    }

    #[test]
    fn a_free_sprite_flying_along_its_normal_is_left_alone() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Facing="Free" OrientToVelocity="true" Speed="3" FocusGravityDir="true" InitAngles="90""#);
        let c = ctx(&budget);
        s.step(&c);
        let e = &s.emitters[0];
        let right = (Vec3::from(e.pos[1]) - Vec3::from(e.pos[0])).normalize();
        let up = (Vec3::from(e.pos[3]) - Vec3::from(e.pos[0])).normalize();
        assert!(right.dot(Vec3::X) > 0.999, "{right:?}");
        assert!(up.dot(Vec3::Z) > 0.999, "the 90 degree roll turns the card's up into +Z, and a velocity along the normal leaves it: {up:?}");
    }

    /// Dash's `speed_full_pc2`: `SizeY` 10, `SizeX` 0.5, `InitAngles` 90, flying back at 20 m/s. The engine's roll lays the card's up along
    /// the emission axis and the spin about the normal puts it on the velocity: a streak long along its flight, not a bar across it.
    #[test]
    fn dash_streaks_are_long_along_their_flight() {
        let budget = AtomicI32::new(10_000);
        for angle in ["", "0,180", "0,77"] {
            let mut s = sim(&format!(
                r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Facing="Free" OrientToVelocity="true" Speed="-20" InitAngles="90" RandomAngles="{angle}" SizeX="0.5" SizeY="10""#
            ));
            let c = ctx(&budget);
            s.step(&c);
            let e = &s.emitters[0];
            assert_eq!(e.units, 1);
            let width = Vec3::from(e.pos[1]) - Vec3::from(e.pos[0]);
            let length = Vec3::from(e.pos[3]) - Vec3::from(e.pos[0]);
            // the emission axis (cry +Y) is Bevy -Z, the speed is negative: the particle flies along +Z
            assert!(length.length() > 10.0 * width.length(), "{length:?} {width:?}");
            assert!(length.normalize().dot(Vec3::Z).abs() > 0.999, "RandomAngles {angle}: the card's length lies along the flight: {length:?}");
        }
    }

    #[test]
    fn a_stationary_free_sprite_keeps_its_authored_orientation() {
        let budget = AtomicI32::new(10_000);
        let attrs = r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Facing="Free" Speed="0" InitAngles="30,70,20""#;
        let mut plain = sim(attrs);
        let mut oriented = sim(&format!(r#"{attrs} OrientToVelocity="true""#));
        let c = ctx(&budget);
        plain.step(&c);
        oriented.step(&c);
        assert_eq!(plain.emitters[0].pos, oriented.emitters[0].pos);
    }

    #[test]
    fn burst_emits_once_and_ends() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="10" ParticleLifeTime="0.5" Texture="textures/x.dds""#);
        let c = ctx(&budget);
        s.step(&c);
        assert_eq!(s.alive(), 10);
        for _ in 0..120 {
            s.step(&c);
        }
        assert_eq!(s.alive(), 0);
        assert!(s.finished());
    }

    #[test]
    fn budget_limits_spawning() {
        let budget = AtomicI32::new(5);
        let mut s = sim(r#"Count="50" ParticleLifeTime="2" Texture="textures/x.dds""#);
        s.step(&ctx(&budget));
        assert_eq!(s.alive(), 5);
    }

    #[test]
    fn priming_fills_the_effect() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Continuous="true" Count="40" ParticleLifeTime="4" Texture="textures/x.dds""#);
        let c = ctx(&budget);
        s.prime(&c);
        let n = s.alive();
        assert!((30..=48).contains(&n), "alive {n}");
    }

    #[test]
    fn gravity_pulls_down_and_focus_gravity_goes_up() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Speed="0" GravityScale="1""#);
        let c = ctx(&budget);
        for _ in 0..60 {
            s.step(&c);
        }
        assert!(s.emitters[0].particles[0].pos.y < -3.0);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Speed="2" FocusGravityDir="true""#);
        for _ in 0..30 {
            s.step(&c);
        }
        let p = s.emitters[0].particles[0];
        assert!(p.pos.y > 0.9 && p.pos.x.abs() < 1e-3, "{:?}", p.pos);
    }

    /// `FocusAngle` tilts the emitter's +Y axis toward +Z (up for a standing emitter) about X, and the azimuth does not scatter
    /// by itself: the 4400 authored `FocusAngle="90"` emitters (fire, smoke, sparks) all rise.
    #[test]
    fn focus_angle_ninety_points_up_with_a_fixed_azimuth() {
        let budget = AtomicI32::new(10_000);
        for _ in 0..3 {
            let mut s = sim(r#"Count="20" ParticleLifeTime="3" Texture="textures/x.dds" Speed="2" FocusAngle="90""#);
            s.step(&ctx(&budget));
            for p in &s.emitters[0].particles {
                assert!(p.vel.y > 1.99 && p.vel.x.abs() < 1e-3 && p.vel.z.abs() < 1e-3, "{:?}", p.vel);
            }
        }
        // a half turn of azimuth sends them down
        let mut s = sim(r#"Count="4" ParticleLifeTime="3" Texture="textures/x.dds" Speed="2" FocusAngle="90" FocusAzimuth="180""#);
        s.step(&ctx(&budget));
        assert!(s.emitters[0].particles.iter().all(|p| p.vel.y < -1.99));
    }

    /// A sprite that flies along with a moving emitter (`InheritVelocity`) moves with it but is not stretched over that speed: the
    /// hammer of Ollo's Hammer flew at 14 m/s with `Stretch 1` and drew 14 m streaks.
    #[test]
    fn inherited_velocity_moves_the_particle_but_not_its_stretch() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Speed="0" InheritVelocity="1" Stretch="1""#);
        let mut c = ctx(&budget);
        c.emitter_vel = Vec3::new(14.0, 0.0, 0.0);
        s.step(&c);
        let p = s.emitters[0].particles[0];
        assert!((p.vel.x - 14.0).abs() < 1e-3, "{:?}", p.vel);
        assert!((p.vel - p.inh).length() < 1e-3, "{:?} {:?}", p.vel, p.inh);
    }

    /// A beam anchor (`proj_physic_id` 2) is stretched on its Z by the length of the link: the particles' offsets and velocities
    /// follow along the entity's axes, the sizes do not (the engine's beam projectiles, `combat::update_beams`).
    #[test]
    fn a_stretched_anchor_stretches_offsets_and_velocities_but_not_sizes() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Size="0.5" Speed="2" PositionOffset="0,1,0""#);
        let mut c = ctx(&budget);
        c.axis = Vec3::new(1.0, 1.0, 4.0);
        s.step(&c);
        let p = s.emitters[0].particles[0];
        // cry +Y is Bevy -Z: one metre of offset and 2 m/s become four and eight
        assert!((p.pos.z + 4.0).abs() < 0.2 && p.pos.x.abs() < 1e-3, "{:?}", p.pos);
        assert!((p.vel.z + 8.0).abs() < 1e-3 && p.vel.x.abs() < 1e-3, "{:?}", p.vel);
        assert!((p.size - 0.5).abs() < 1e-6, "{}", p.size);
        // a rotated anchor stretches along its own Z
        let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        c.rot = Mat3::from_quat(rotation);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Speed="2""#);
        s.step(&c);
        let v = s.emitters[0].particles[0].vel;
        assert!((v - rotation * Vec3::new(0.0, 0.0, -8.0)).length() < 1e-3, "{v:?}");
    }

    /// `Facing = Velocity` turns the card's normal (cry +Y) onto the direction of travel: a ring thrown along +X seen from
    /// the side is edge on.
    #[test]
    fn velocity_facing_puts_the_cards_normal_on_the_direction_of_travel() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Facing="Velocity" Size="1" Speed="3" FocusGravityDir="true""#);
        let c = ctx(&budget);
        s.step(&c);
        s.step(&c);
        let e = &s.emitters[0];
        // the particle flies up (focus gravity): the card lies flat, so it has no extent along y
        let extent = e.max - e.min;
        assert!(extent.y < 0.01 && extent.x > 0.5 && extent.z > 0.5, "{extent:?}");
    }

    /// Stretch: every row grows along the screen velocity by `velocity * stretch` (the half length, so the quad is twice that
    /// longer), the engine's `CParticleContainer` sprite writer.
    #[test]
    fn stretch_adds_velocity_times_stretch_to_the_half_length() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Size="0.1" Speed="4" FocusGravityDir="true" OrientToVelocity="true" Stretch="0.5""#);
        let c = ctx(&budget);
        s.step(&c);
        let e = &s.emitters[0];
        let ext = e.max - e.min;
        assert!((ext.y - 4.2).abs() < 0.15 && (ext.x - 0.2).abs() < 0.02, "{ext:?}");
        // without OrientToVelocity the rows keep their roll: a square card grows to a parallelogram along the velocity
        let mut s = sim(r#"Count="1" ParticleLifeTime="3" Texture="textures/x.dds" Size="0.1" Speed="4" FocusGravityDir="true" Stretch="0.5""#);
        s.step(&c);
        let e = &s.emitters[0];
        let ext = e.max - e.min;
        assert!(ext.y > 4.0 && ext.x < 0.3, "{ext:?}");
    }
    #[test]
    fn a_decal_follows_the_ground_under_it() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="4" Facing="Decal" Texture="textures/x.dds" Size="3" Speed="0" PositionOffset="0,0,2""#);
        // a slope: 0.5 m up per metre along x
        let ground = |p: Vec3| Some(0.5 * p.x);
        let mut c = ctx(&budget);
        c.ground = Some(&ground);
        s.step(&c);
        let e = &s.emitters[0];
        // 6 m wide: cut into cells, every corner on the slope (and a hair above it)
        assert!(e.units > 4, "one flat card: {}", e.units);
        for p in &e.pos {
            assert!((p[1] - (0.5 * p[0] + crate::decal::LIFT)).abs() < 1e-3, "off the ground: {p:?}");
        }
        // without a ground height it is one flat card
        let mut s = sim(r#"Count="1" ParticleLifeTime="4" Facing="Decal" Texture="textures/x.dds" Size="3" Speed="0""#);
        s.step(&ctx(&budget));
        assert_eq!(s.emitters[0].units, 1);
    }

    #[test]
    fn a_single_cell_decal_projects_in_a_rotated_effect_frame() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"ParticleLifeTime="4" Facing="Decal" Texture="textures/x.dds" Size="0.2" Speed="0""#);
        let ground = |p: Vec3| Some(0.5 * p.x - 0.2 * p.z);
        let mut c = ctx(&budget);
        let rotation = Quat::from_rotation_y(0.7);
        c.global = Affine3A::from_rotation_translation(rotation, Vec3::new(4.0, 8.0, -2.0));
        c.rot = Mat3::from_quat(rotation);
        c.ground = Some(&ground);
        s.step(&c);
        let e = &s.emitters[0];
        assert_eq!(e.units, 1);
        for p in &e.pos {
            let local = Vec3::from(*p);
            let world = c.global.transform_point3(local);
            assert!((world.y - ground(world).unwrap() - crate::decal::LIFT).abs() < 1e-5, "{world:?}");
            assert!(local.cmpge(e.min).all() && local.cmple(e.max).all());
        }
    }

    #[test]
    fn a_light_particle_gives_its_colour_times_the_intensity_and_the_hdr_power() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"Count="1" ParticleLifeTime="4" Texture="textures/x.dds" Speed="0" Color="(1,0.5,0)" LightSourceIntensity="2" LightSourceRadius="3" LightHDRDynamic="1""#);
        let c = ctx(&budget);
        s.step(&c);
        let mut out = Vec::new();
        s.emitters[0].lights(&c, 2.0, &mut out);
        assert_eq!(out.len(), 1);
        // 2 (intensity) x 2^1 (HDR power) = 4
        assert!((out[0].colour - Vec3::new(4.0, 2.0, 0.0)).abs().max_element() < 1e-4, "{:?}", out[0].colour);
        assert_eq!(out[0].radius, 3.0);
        // no radius, no light
        let mut s = sim(r#"Count="1" ParticleLifeTime="4" Texture="textures/x.dds" LightSourceIntensity="2""#);
        s.step(&c);
        out.clear();
        s.emitters[0].lights(&c, 2.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn the_sun_lights_a_sprite_by_its_facing_and_the_backlighting() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs><Particles Name="e"><Params Texture="textures/x.dds" DiffuseLighting="0.5" DiffuseBacklighting="0.2"/></Particles></Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        let eff = lib.get("a").unwrap();
        let em = eff.emitters.iter().map(|e| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7, bevy::prelude::Entity::PLACEHOLDER)).collect();
        let sim = EffectSim::new(em, 1.0, 1.0, 2.0);
        let e = &sim.emitters[0];
        let budget = AtomicI32::new(10_000);
        let mut c = ctx(&budget);
        // no sun: the constant part (diffuse 0.5 x light 1 + backlighting 0.2 x light x 0.5)
        assert!((e.lit_now(&c) - e.lit).abs() < 1e-6);
        c.sun_light = 10.0;
        // the sun behind the camera (the sprite faces it): the whole sun times the diffuse factor
        c.sun_facing = 1.0;
        assert!((e.lit_now(&c) - (e.lit + 5.0)).abs() < 1e-5);
        // the sun at right angles: only the backlighting coefficient is left; the sun in front of the camera: nothing (the wrap is 0.2)
        c.sun_facing = 0.0;
        assert!((e.lit_now(&c) - (e.lit + 0.5 * 10.0 * 0.2)).abs() < 1e-5);
        c.sun_facing = -1.0;
        assert!((e.lit_now(&c) - e.lit).abs() < 1e-5);
    }

    #[test]
    fn sunlight_changes_live_sprite_colours_without_restarting_them() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"ParticleLifeTime="4" Texture="textures/x.dds" DiffuseLighting="0.5" DiffuseBacklighting="0.2" Speed="0""#);
        let mut c = ctx(&budget);
        s.step(&c);
        let age = s.emitters[0].particles[0].age;
        let ambient = s.emitters[0].col[0][0];
        c.sun_light = 10.0;
        c.sun_facing = 1.0;
        s.step(&c);
        assert!((s.emitters[0].col[0][0] - ambient - 5.0).abs() < 1e-5);
        assert!(s.emitters[0].particles[0].age > age);
        c.sun_facing = -1.0;
        s.step(&c);
        assert!((s.emitters[0].col[0][0] - ambient).abs() < 1e-5);
        // An emissive particle has no diffuse response to the sun.
        let mut glow = sim(r#"Texture="textures/x.dds" DiffuseLighting="0" EmissiveLighting="2" Speed="0""#);
        glow.step(&c);
        let before = glow.emitters[0].col[0];
        c.sun_facing = 1.0;
        glow.step(&c);
        assert_eq!(glow.emitters[0].col[0], before);
    }

    #[test]
    fn lighting_and_hdr_changes_reach_existing_particles() {
        let budget = AtomicI32::new(10_000);
        let mut s = sim(r#"ParticleLifeTime="4" Texture="textures/x.dds" DiffuseLighting="0.5" EmissiveLighting="0.5" EmissiveHDRDynamic="2" Speed="0""#);
        let c = ctx(&budget);
        s.step(&c);
        let id = s.emitters[0].particles[0].id;
        let age = s.emitters[0].particles[0].age;
        assert!((s.emitters[0].col[0][0] - 0.9).abs() < 1e-5);
        s.set_lighting(2.0, 1.0, 2.0);
        s.step(&c);
        assert!((s.emitters[0].col[0][0] - 3.0).abs() < 1e-5);
        assert_eq!(s.emitters[0].particles[0].id, id);
        assert!(s.emitters[0].particles[0].age > age);
        assert_eq!(s.alive(), 1);
        // The separate geometry path gets the same refreshed HDR term, without the sprite-facing sun term.
        let mut mesh = sim(r#"Geometry="x.cgf" ParticleLifeTime="4" DiffuseLighting="0.5" EmissiveLighting="0.5" EmissiveHDRDynamic="2" Speed="0""#);
        mesh.emitters[0].set_geom(triangle());
        mesh.step(&c);
        assert!((mesh.emitters[0].col[0][0] - 0.9).abs() < 1e-5);
        mesh.set_lighting(0.8, 1.0, 2.0);
        let mut sun = ctx(&budget);
        sun.sun_light = 10.0;
        sun.sun_facing = 1.0;
        mesh.step(&sun);
        assert!((mesh.emitters[0].col[0][0] - 2.4).abs() < 1e-5);
    }

    #[test]
    fn the_emissive_hdr_power_scales_the_emissive_light() {
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs><Particles Name="e"><Params Texture="textures/x.dds" DiffuseLighting="0" EmissiveLighting="0.5" EmissiveHDRDynamic="2"/></Particles></Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        let eff = lib.get("a").unwrap();
        let mk = |base| {
            let em = eff.emitters.iter().map(|e| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7, bevy::prelude::Entity::PLACEHOLDER)).collect();
            EffectSim::new(em, 0.8, 1.0, base).emitters[0].lit
        };
        assert!((mk(2.0) - 2.0).abs() < 1e-5);
        assert!((mk(1.0) - 0.5).abs() < 1e-5);
    }
    #[test]
    fn a_child_with_an_emitter_life_rides_on_its_parent_particle() {
        // the parent particle falls at 10 m/s from 3 m; the child emits 6 puffs over 0.2 s from wherever it is
        let xml = r#"<ParticleLibrary Name="T"><Particles Name="a"><Params/><Childs>
            <Particles Name="fall"><Params PositionOffset="0,0,3" Speed="-10" FocusAngle="90"/>
            <Childs><Particles Name="fire"><Params Continuous="true" Count="6" EmitterLifeTime="0.2" ParticleLifeTime="1" Texture="textures/x.dds" SpawnIndirection="ParentStart" Speed="0"/></Particles></Childs></Particles>
            </Childs></Particles></ParticleLibrary>"#;
        let lib = Library::parse(xml).unwrap();
        let eff = lib.get("a").unwrap();
        let em = eff.emitters.iter().enumerate().map(|(i, e)| EmitterState::new(e.def.clone(), e.parent, 0, 1.0, 7 + i as u64, bevy::prelude::Entity::PLACEHOLDER)).collect();
        let mut s = EffectSim::new(em, 0.8, 1.0, 1.0);
        let budget = AtomicI32::new(10_000);
        let mut c = ctx(&budget);
        c.dt = 1.0 / 60.0;
        for _ in 0..20 {
            s.step(&c);
        }
        let ys: Vec<f32> = s.emitters[1].particles.iter().map(|p| p.pos.y).collect();
        let (lo, hi) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!(ys.len() >= 4, "puffs: {}", ys.len());
        // 0.2 s at 10 m/s: the puffs start along 2 m of the fall
        assert!(hi - lo > 1.2, "a clump, not a column: {lo}..{hi}");
        // and it stopped emitting after the emitter life
        let n = s.emitters[1].particles.len();
        for _ in 0..10 {
            s.step(&c);
        }
        assert!(s.emitters[1].particles.len() <= n, "kept emitting");
    }
}
