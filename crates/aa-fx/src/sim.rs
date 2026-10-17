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

use bevy::math::{Affine3A, Mat3, Quat, Vec3};

use crate::geom::{Geom, GeomKey};
use crate::params::{Blend, EmitterDef, Facing, Spawn};

/// CryEngine (Z up) to Bevy (Y up).
#[inline]
pub fn c2b(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

const GRAVITY: f32 = 9.81;
/// Particles one emitter may hold.
pub const MAX_PER_EMITTER: usize = 2048;
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
    /// Uniform scale of the effect (entity scale times `Effect::scale`).
    pub scale: f32,
    pub speed_scale: f32,
    pub count_scale: f32,
    /// 1 = full emission, less thins far effects.
    pub lod: f32,
    pub size_scale: f32,
    /// A continuous emitter with a finite life spreads `Count` over that life (see `EmitterState::emit`).
    pub count_over_life: bool,
    /// Colour of the light on the effect.
    pub tint: Vec3,
    /// Direction to the sun in world space (shades opaque mesh particles).
    pub sun: Vec3,
    pub wind: Vec3,
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
    variant: u16,
    seed: f32,
}

/// One emitter of a running effect.
pub struct EmitterState {
    pub def: Arc<EmitterDef>,
    pub parent: Option<usize>,
    /// Index into the material table of the plugin.
    pub material: u32,
    /// Height / width of one texture tile.
    pub aspect: f32,
    /// The mesh entity (or `Entity::PLACEHOLDER` for an emitter that only triggers others).
    pub mesh: bevy::prelude::Entity,
    pub draws: bool,
    /// The mesh a `Geometry` emitter instances, once it is loaded.
    pub geom: Option<Arc<Geom>>,
    /// The mesh a `Geometry` emitter waits for.
    pub pending_geom: Option<GeomKey>,
    /// Particle budget one particle costs (a mesh counts its vertices in quads).
    cost: usize,
    rng: Rng,
    particles: Vec<Particle>,
    age: f32,
    delay: f32,
    spawn_acc: f32,
    burst_done: bool,
    done: bool,
    off: bool,
    want_births: bool,
    want_deaths: bool,
    births: Vec<Vec3>,
    deaths: Vec<Vec3>,
    lit: f32,
    // output, effect-local space
    pub pos: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// Second texture coordinates of a mesh with an opacity map: the mesh's own, not animated.
    pub uv1: Vec<[f32; 2]>,
    pub col: Vec<[f32; 4]>,
    /// Quads (sprite emitter) or mesh copies written.
    pub units: usize,
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
            geom: None,
            pending_geom: None,
            cost: 1,
            rng: Rng::new(seed),
            particles: Vec::new(),
            age: 0.0,
            delay: def.spawn_delay.base.max(0.0),
            spawn_acc: 0.0,
            burst_done: false,
            done: spawn != Spawn::Direct,
            off: false,
            want_births: false,
            want_deaths: false,
            births: Vec::new(),
            deaths: Vec::new(),
            lit: 1.0,
            pos: Vec::new(),
            uv: Vec::new(),
            uv1: Vec::new(),
            col: Vec::new(),
            units: 0,
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
        self.age = 0.0;
        self.delay = self.def.spawn_delay.base.max(0.0);
        self.spawn_acc = 0.0;
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
                    self.spawn_acc += count / period * overlap * dt * ctx.lod;
                    let n = self.spawn_acc as usize;
                    self.spawn_acc -= n as f32;
                    let n = self.grant(ctx, n);
                    for _ in 0..n {
                        let age0 = self.rng.next() * dt;
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
        let world_pos = at.unwrap_or(frame.origin) + ctx.rot * local;
        let pos = frame.point_from_world(world_pos);

        // direction: focus axis, tilted by the focus angle, then the emit cone
        let axis = if def.focus_gravity { Vec3::Y } else { ctx.rot * c2b(Vec3::Y) };
        let (t, b) = perpendicular(axis);
        let mut dir = axis;
        let fa = def.focus_angle.spawn(r.next(), strength).to_radians();
        if fa != 0.0 {
            let az = if def.focus_azimuth.base != 0.0 || def.focus_azimuth.var > 0.0 {
                def.focus_azimuth.spawn(r.next(), strength).to_radians()
            } else {
                r.next() * std::f32::consts::TAU
            };
            let side = t * az.cos() + b * az.sin();
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
        let speed = def.speed.spawn(r.next(), strength) * ctx.speed_scale * scale;
        let vel = frame.dir_from_world(dir * speed);

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
            variant,
            seed: r.next() * 100.0,
        };
        p.pos += p.vel * age0;
        if self.want_births {
            let basis = self.orbit_basis(ctx);
            let at = frame.to_world_point(p.pos) + self.orbit_offset(&p, 0.0, &basis);
            self.births.push(at);
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
        let basis = self.orbit_basis(ctx);
        let orbits = def.orbits();
        let mut w = 0;
        for i in 0..self.particles.len() {
            let mut p = self.particles[i];
            p.age += dt;
            if p.age >= p.life {
                if want_deaths {
                    let at = frame.to_world_point(p.pos) + self.orbit_offset(&p, 1.0, &basis);
                    self.deaths.push(at);
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
            }
            p.pos += p.vel * dt;
            if turb > 0.0 {
                let t = p.age * 1.7 + p.seed;
                p.pos += Vec3::new(t.sin(), (t * 1.31 + 1.0).sin(), (t * 0.77 + 2.0).sin()) * turb * dt;
            }
            p.ang += p.ang_rate * dt;
            if orbits {
                p.spin += p.spin_rate * luts.rot_speed.at(a) * dt;
            }
            self.particles[w] = p;
            w += 1;
        }
        self.particles.truncate(w);
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

    /// Writes the quads of the live particles.
    fn build(&mut self, ctx: &StepCtx) {
        let def = self.def.clone();
        let luts = def.luts();
        let frame = self.frame(ctx);
        let n = self.particles.len();
        self.pos.clear();
        self.uv.clear();
        self.col.clear();
        self.dirty = true;
        if n == 0 {
            self.units = 0;
            return;
        }
        self.pos.reserve(n * 4);
        self.uv.reserve(n * 4);
        self.col.reserve(n * 4);

        let inv = ctx.global.inverse();
        let view = ctx.view;
        let tiling = def.tiling;
        let n_tiles = (tiling.x * tiling.y).max(1);
        let du = 1.0 / tiling.x as f32;
        let dv = 1.0 / tiling.y as f32;
        let aspect = self.aspect;
        let lit = self.lit;
        let facing = def.facing;
        let orient = def.orient_to_velocity;
        let has_stretch = def.stretch.base != 0.0;
        let orbit = def.turbulence_size * ctx.scale;
        let orbit_speed = def.turbulence_speed.to_radians();
        let basis = self.orbit_basis(ctx);
        // camera axes in effect-local space (unit scale of world vectors)
        let ir = inv.transform_vector3(view.right);
        let iu = inv.transform_vector3(view.up);
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);

        for p in &self.particles {
            let a = p.age * p.inv_life;
            let sz = p.size * luts.size.at(a);
            let alpha = p.alpha * luts.alpha.at(a) * ctx.fade;
            if alpha < 0.003 || sz < 1e-5 {
                continue;
            }
            let hx = 0.5 * sz * p.sx * luts.size_x.at(a);
            let mut hy = 0.5 * sz * p.sy * luts.size_y.at(a) * aspect;
            let mut center = frame.to_world_point(p.pos) + self.orbit_offset(p, a, &basis);
            if orbit > 0.0 {
                let t = orbit_speed * p.age + p.seed;
                center += (Vec3::X * t.cos() + Vec3::Z * t.sin()) * orbit;
            }
            let colour = p.color * luts.color.at(a) * lit * ctx.tint;

            // frame in the texture atlas
            let mut f = tiling.first + p.variant as u32;
            if tiling.frames > 1 {
                let t = if tiling.fps > 0.0 { p.age * tiling.fps } else { a * tiling.frames as f32 };
                let k = t.floor() as i32;
                let k = if tiling.cycle { k.rem_euclid(tiling.frames as i32) } else { k.clamp(0, tiling.frames as i32 - 1) };
                f = tiling.first + k as u32;
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
                    shift = Vec3::ZERO;
                }
                Facing::Free => {
                    let q = Quat::from_euler(bevy::math::EulerRot::ZYX, p.ang.z, p.ang.y, p.ang.x);
                    let r = ctx.rot * c2b(q * Vec3::X);
                    let u = ctx.rot * c2b(q * Vec3::Z);
                    rx = inv.transform_vector3(r) * hx;
                    uy = inv.transform_vector3(u) * hy;
                    shift = Vec3::ZERO;
                }
                Facing::Bottom => {
                    let r = view.forward.cross(Vec3::Y);
                    let r = if r.length_squared() > 1e-4 { r.normalize() } else { view.right };
                    rx = inv.transform_vector3(r) * hx;
                    uy = inv.transform_vector3(Vec3::Y) * hy;
                    shift = inv.transform_vector3(Vec3::Y) * hy;
                }
                Facing::Camera => {
                    let vel = frame.to_world_dir(p.vel);
                    let speed = vel.length();
                    if (orient || has_stretch) && speed > 0.02 {
                        let dir = vel / speed;
                        let flat = dir - view.forward * dir.dot(view.forward);
                        let fl = flat.length();
                        if fl > 0.02 {
                            let up = flat / fl;
                            let right = view.forward.cross(up);
                            let len = speed * p.stretch * luts.stretch.at(a);
                            hy += len * 0.5;
                            let sh = -up * (len * def.stretch_offset);
                            rx = inv.transform_vector3(right) * hx;
                            uy = inv.transform_vector3(up) * hy;
                            shift = inv.transform_vector3(sh);
                        } else {
                            // moving along the view axis: a round sprite
                            rx = ir * hx;
                            uy = iu * hy;
                            shift = Vec3::ZERO;
                        }
                    } else {
                        let (s, c) = p.ang.z.sin_cos();
                        rx = (ir * c + iu * s) * hx;
                        uy = (iu * c - ir * s) * hy;
                        shift = Vec3::ZERO;
                    }
                }
            }
            let c = inv.transform_point3(center) + shift;
            let v = [c - rx - uy, c + rx - uy, c + rx + uy, c - rx + uy];
            for q in &v {
                min = min.min(*q);
                max = max.max(*q);
            }
            self.pos.extend_from_slice(&[v[0].to_array(), v[1].to_array(), v[2].to_array(), v[3].to_array()]);
            self.uv.extend_from_slice(&[[u0, v0 + dv], [u0 + du, v0 + dv], [u0 + du, v0], [u0, v0]]);
            let colour = [colour.x, colour.y, colour.z, alpha.min(1.0)];
            self.col.extend_from_slice(&[colour, colour, colour, colour]);
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
            let rot = if def.orient_to_velocity && speed > 0.02 {
                // the mesh's forward axis (CryEngine +Y) along the velocity
                Mat3::from_quat(Quat::from_rotation_arc(Vec3::NEG_Z, vel / speed)) * turn
            } else {
                ctx.rot * turn
            };
            let center = inv.transform_point3(frame.to_world_point(p.pos) + self.orbit_offset(p, a, &basis));
            let lin = inv3 * rot * Mat3::from_diagonal(scale);
            let shift = center - lin * pivot;
            let c = p.color * luts.color.at(a) * lit * ctx.tint * m.tint;

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
            if opaque {
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
    pub fn new(mut emitters: Vec<EmitterState>, light: f32, emissive: f32) -> EffectSim {
        for e in emitters.iter_mut() {
            let d = &e.def;
            e.lit = d.diffuse * light + d.backlight * light * 0.5 + d.emissive * emissive;
        }
        let triggers: Vec<(Option<usize>, Spawn)> = emitters.iter().map(|e| (e.parent, e.def.spawn)).collect();
        for (parent, spawn) in triggers {
            if let Some(p) = parent {
                match spawn {
                    Spawn::ParentStart => emitters[p].want_births = true,
                    Spawn::ParentDeath => emitters[p].want_deaths = true,
                    _ => {}
                }
            }
        }
        EffectSim { emitters, time: 0.0, primed: false }
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
            e.emit(ctx, dt);
            e.update(ctx, dt);
        }
        // sub emitters start where their parent's particles were born or died
        for i in 0..self.emitters.len() {
            let Some(p) = self.emitters[i].parent else { continue };
            let spawn = self.emitters[i].def.spawn;
            let events: Vec<Vec3> = match spawn {
                Spawn::ParentStart => self.emitters[p].births.clone(),
                Spawn::ParentDeath => self.emitters[p].deaths.clone(),
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
            speed_scale: 1.0,
            count_scale: 1.0,
            lod: 1.0,
            size_scale: 1.0,
            count_over_life: true,
            tint: Vec3::ONE,
            sun: Vec3::Y,
            wind: Vec3::ZERO,
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
        EffectSim::new(em, 0.8, 1.0)
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
}
