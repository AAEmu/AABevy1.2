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

use crate::params::{EmitterDef, Facing, Spawn};

/// CryEngine (Z up) to Bevy (Y up).
#[inline]
pub fn c2b(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

const GRAVITY: f32 = 9.81;
/// Particles one emitter may hold.
pub const MAX_PER_EMITTER: usize = 2048;

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
    stretch: f32,
    color: Vec3,
    alpha: f32,
    air: f32,
    grav: f32,
    ang: Vec3,
    ang_rate: Vec3,
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
    pub col: Vec<[f32; 4]>,
    pub quads: usize,
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
            col: Vec::new(),
            quads: 0,
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            dirty: false,
            def,
        }
    }

    pub fn alive(&self) -> usize {
        self.particles.len()
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
        self.quads = 0;
        self.pos.clear();
        self.uv.clear();
        self.col.clear();
        self.dirty = true;
    }

    fn frame(&self, ctx: &StepCtx) -> Frame {
        Frame { origin: ctx.global.translation.into(), rot: ctx.rot, bound: self.def.bind_to_emitter }
    }

    /// Takes `n` particles from the global budget.
    fn grant(&self, ctx: &StepCtx, n: usize) -> usize {
        let room = MAX_PER_EMITTER.saturating_sub(self.particles.len());
        let n = n.min(room);
        if n == 0 {
            return 0;
        }
        let before = ctx.budget.fetch_sub(n as i32, Ordering::Relaxed);
        if before >= n as i32 {
            n
        } else {
            ctx.budget.fetch_add(n as i32, Ordering::Relaxed);
            before.max(0) as usize
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
                    self.spawn_acc += count / plife * dt * ctx.lod;
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
        let mut color = def.color.base;
        if let Some(c) = def.color.strength.as_ref().and_then(|c| c.eval(strength)) {
            color *= c;
        }
        let alpha = def.alpha.spawn(r.next(), strength);

        let d2r = std::f32::consts::PI / 180.0;
        let ang = (def.init_angles + def.random_angles * Vec3::new(r.signed(), r.signed(), r.signed())) * d2r;
        let ang_rate = (def.rotation_rate + def.random_rotation_rate * Vec3::new(r.signed(), r.signed(), r.signed())) * d2r;
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
            stretch: def.stretch.spawn(r.next(), strength),
            color,
            alpha,
            air: def.air_resistance.spawn(r.next(), strength),
            grav: def.gravity.spawn(r.next(), strength),
            ang,
            ang_rate,
            variant,
            seed: r.next() * 100.0,
        };
        p.pos += p.vel * age0;
        if self.want_births {
            self.births.push(frame.to_world_point(p.pos));
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
        let mut w = 0;
        for i in 0..self.particles.len() {
            let mut p = self.particles[i];
            p.age += dt;
            if p.age >= p.life {
                if want_deaths {
                    self.deaths.push(frame.to_world_point(p.pos));
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
            self.particles[w] = p;
            w += 1;
        }
        self.particles.truncate(w);
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
            self.quads = 0;
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
            let mut center = frame.to_world_point(p.pos);
            if orbit > 0.0 {
                let t = orbit_speed * p.age + p.seed;
                center += (Vec3::X * t.cos() + Vec3::Z * t.sin()) * orbit;
            }
            let colour = p.color * luts.color.at(a) * lit;

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
        self.quads = self.pos.len() / 4;
        if self.quads > 0 {
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
        for e in &mut self.emitters {
            if e.draws {
                e.build(ctx);
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
        assert_eq!(s.emitters[0].quads, n);
        assert_eq!(s.emitters[0].pos.len(), n * 4);
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
