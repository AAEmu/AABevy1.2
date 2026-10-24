//! Bounded local Wind. External points use status23; particles use the distinct
//! native sphere cache. Vegetation cache/cadence and physical solvers are outside
//! this partial ForceGeneration implementation.

use crate::{
    Effect, FxRun, FxSettings,
    library::Node,
    params::{EmitterDef, Spawn, Var},
};
use bevy::prelude::*;
use std::sync::Arc;

const HARD_CAP: usize = 256;

/// Application limits, independent of the native area's modification counter.
#[derive(Resource, Clone, Debug)]
pub struct FxWindSettings {
    pub enabled: bool,
    /// Default 64, clamped to the application hard ceiling 256. Zero disables publication.
    pub max_fields: usize,
}

impl Default for FxWindSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_fields: 64,
        }
    }
}

/// Completed local fields for this frame. All points/vectors use Bevy world axes.
#[derive(Resource, Default, Debug)]
pub struct FxWindFields {
    fields: Vec<Field>,
}

impl FxWindFields {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Local velocity delta (m/s), excluding ambient. Uses the audited zero-half
    /// status23 sphere response, including axial center/edge flow. No bbox/solver
    /// promise. Summing delta independently can change ambient-first f32 rounding.
    /// Invalid/no-hit/empty queries return zero, without allocation or mutation.
    pub fn sample(&self, point_world: Vec3) -> Vec3 {
        if self.is_empty() || !point_world.is_finite() {
            return Vec3::ZERO;
        }
        let point = cry(point_world);
        let mut sum = Vec3::ZERO;
        for field in &self.fields {
            sum += field.external(point);
        }
        crate::c2b(sum)
    }

    pub(crate) fn receiver(&self, owner: Entity) -> Option<Receiver<'_>> {
        (!self.is_empty()).then_some(Receiver {
            fields: self,
            owner,
        })
    }

    fn admit(&mut self, field: Field, cap: usize) {
        let key = field.key();
        if self.fields.len() == cap {
            if self.fields.last().is_some_and(|f| f.key() <= key) {
                return;
            }
            self.fields.pop();
        }
        let index = self.fields.partition_point(|f| f.key() < key);
        self.fields.insert(index, field);
    }
}

/// PostUpdate publication, after transforms/lifecycle and before FX sampling.
/// External readers order after this set. Later mutations await next publication.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FxWindPublished;

#[derive(Clone, Copy)]
pub(crate) struct Receiver<'a> {
    fields: &'a FxWindFields,
    owner: Entity,
}

impl Receiver<'_> {
    pub(crate) fn sample(self, point_world: Vec3) -> Vec3 {
        if !point_world.is_finite() {
            return Vec3::ZERO;
        }
        let point = cry(point_world);
        let mut sum = Vec3::ZERO;
        for field in &self.fields.fields {
            // All native nodes in an EffectSim share this owner. Entity includes
            // its generation; actor/bone/ChildOf relationships do not merge roots.
            if field.owner != self.owner {
                sum += field.cached(point);
            }
        }
        crate::c2b(sum)
    }
}

#[derive(Debug)]
struct Field {
    owner: Entity,
    ordinal: usize,
    center: Vec3, // native world axes: keep original square/add order
    inv_radius: f32,
    strength: f32,
}

fn cry(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}
fn square(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y) + v.z * v.z
}

// Original rsqrtss plus one Newton refinement. SSE is guaranteed on x86-64.
// Other architectures preserve branches/constants but not x86 approximation bits.
fn reciprocal_length(d: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    let r = unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(d)))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let r = d.sqrt().recip();
    (1.5 - ((r * d) * r) * 0.5) * r
}

impl Field {
    fn key(&self) -> (u64, usize) {
        (self.owner.to_bits(), self.ordinal)
    }

    fn external(&self, point: Vec3) -> Vec3 {
        let d = point - self.center;
        let q = square(d * self.inv_radius);
        if q > 1.0 {
            return Vec3::ZERO;
        }
        if q == 1.0 {
            return Vec3::Z * self.strength;
        }
        let magnitude = self.strength * (1.0 - q.sqrt());
        let distance2 = square(d);
        let direction = if distance2 <= f32::from_bits(0x1e3ce508) {
            Vec3::Z
        } else {
            d * reciprocal_length(distance2)
        };
        direction * magnitude
    }

    fn cached(&self, point: Vec3) -> Vec3 {
        // Native cached affine applies multiply then translation, rather than
        // subtracting center first. Preserve that rounding distinction.
        let q = square(point * self.inv_radius - self.center * self.inv_radius);
        if q >= 1.0 {
            return Vec3::ZERO;
        }
        let d = point - self.center;
        let direction = d * reciprocal_length(square(d) + f32::from_bits(0x00800000));
        direction * (self.strength * (1.0 - q.sqrt()))
    }
}

/// One root clock; snapshots are rebuilt, so retired runs have no registrations
/// or IDs to leak. Count/draw completion, assets, admission and throttling never
/// own this clock. Only actual loop/reset starts another cycle.
#[derive(Default)]
pub(crate) struct Run {
    born: Option<f64>,
    definition: Option<Arc<crate::EffectDef>>,
    sources: Vec<Arc<EmitterDef>>,
}

impl Run {
    pub(crate) fn restart(&mut self, now: f64) {
        self.born = Some(now);
    }

    pub(crate) fn holding(&self, now: f64, quality: u8) -> bool {
        self.born.is_some_and(|born| {
            self.sources.iter().any(|s| {
                let age = (now - born) as f32 - s.spawn_delay.base;
                age <= s.particle_life.base
                    && quality >= s.config_min
                    && quality <= s.config_max
                    && crate::emitter_filter(&s.name)
            })
        })
    }

    fn resolve(&mut self, def: &Arc<crate::EffectDef>) {
        if self
            .definition
            .as_ref()
            .is_some_and(|d| Arc::ptr_eq(d, def))
        {
            return;
        }
        self.sources.clear();
        fn visit(node: &Node, out: &mut Vec<Arc<EmitterDef>>) {
            if out.len() >= HARD_CAP {
                return;
            }
            if node.def.wind_source {
                out.push(node.def.clone());
            }
            // Direct children remain independent even below disabled parents.
            for child in &node.children {
                visit(child, out);
            }
        }
        visit(&def.root, &mut self.sources);
        self.definition = Some(def.clone());
    }
}

pub(crate) fn observe(time: Res<Time>, mut runs: Query<(Ref<Effect>, &mut FxRun)>) {
    let now = time.elapsed_secs_f64();
    for (effect, mut run) in &mut runs {
        if effect.is_added() {
            run.wind = Run::default();
        }
        run.wind.born.get_or_insert(now);
    }
}

pub(crate) fn publish(
    time: Res<Time>,
    settings: Res<FxWindSettings>,
    fx: Res<FxSettings>,
    mut fields: ResMut<FxWindFields>,
    mut runs: Query<(Entity, &Effect, &GlobalTransform, &mut FxRun)>,
) {
    fields.fields.clear();
    let cap = settings.max_fields.min(HARD_CAP);
    // Withdrawal is unconditional, including normal FX's disabled run condition.
    if !settings.enabled || !fx.enabled {
        return;
    }
    if fields.fields.capacity() < cap {
        fields.fields.reserve_exact(cap);
    }
    let now = time.elapsed_secs_f64();
    for (owner, effect, transform, mut run) in &mut runs {
        let Some(def) = run.def.clone() else { continue };
        run.wind.resolve(&def);
        // Zero admission must not turn Count0 draw completion into source death.
        // Resolve the native lifetime even when no field can be published.
        if cap == 0 || run.sim.is_none() || effect.prime || effect.speed_scale != 1.0 {
            continue;
        }
        let Some(born) = run.wind.born else { continue };
        let age = (now - born) as f32;
        for (ordinal, source) in run.wind.sources.iter().enumerate() {
            if fx.quality < source.config_min
                || fx.quality > source.config_max
                || !crate::emitter_filter(&source.name)
            {
                continue;
            }
            if let Some(field) = source_field(source, owner, ordinal, effect, transform, age) {
                fields.admit(field, cap);
            }
        }
    }
}

fn source_field(
    d: &EmitterDef,
    owner: Entity,
    ordinal: usize,
    effect: &Effect,
    transform: &GlobalTransform,
    age: f32,
) -> Option<Field> {
    let age = age - d.spawn_delay.base;
    if age < 0.0 || age > d.particle_life.base {
        return None;
    }
    let affine = transform.affine();
    let m = affine.matrix3;
    let scale = m.x_axis.length();
    // Full spheres only: reject stretched/sheared/reflected/singular sources.
    if !scale.is_finite()
        || scale <= 0.0
        || m.determinant() <= 0.0
        || (m.y_axis.length() - scale).abs() > scale * 1e-5
        || (m.z_axis.length() - scale).abs() > scale * 1e-5
        || m.x_axis.dot(m.y_axis).abs() > scale * scale * 1e-5
        || m.x_axis.dot(m.z_axis).abs() > scale * scale * 1e-5
        || m.y_axis.dot(m.z_axis).abs() > scale * scale * 1e-5
    {
        return None;
    }
    let scale = scale * effect.scale;
    let radius = d.speed.base.abs() * age * scale;
    let strength = d.speed.base * d.alpha.base.max(0.0) * scale;
    let center = transform.translation() + (m * crate::c2b(d.position_offset)) * effect.scale;
    // Explicit application safety bounds; reject rather than clip native shape.
    if !center.is_finite()
        || !radius.is_finite()
        || !(radius > 0.0 && radius <= 4096.0)
        || !strength.is_finite()
        || strength == 0.0
        || strength.abs() > 4096.0
    {
        return None;
    }
    let inv_radius = radius.recip();
    if !inv_radius.is_finite() {
        return None;
    }
    Some(Field {
        owner,
        ordinal,
        center: cry(center),
        inv_radius,
        strength,
    })
}

pub(crate) fn eligible(d: &EmitterDef) -> bool {
    fn constant(v: &Var) -> bool {
        v.base.is_finite() && v.var == 0.0 && v.strength.is_none() && v.age.is_none()
    }
    d.enabled
        && d.spawn == Spawn::Direct
        && !d.continuous
        && [
            &d.spawn_delay,
            &d.emitter_life,
            &d.pulse_period,
            &d.particle_life,
            &d.speed,
            &d.alpha,
            &d.size,
            &d.air_resistance,
            &d.gravity,
            &d.focus_angle,
            &d.focus_azimuth,
        ]
        .into_iter()
        .all(constant)
        && (0.0..=8.0).contains(&d.spawn_delay.base)
        && d.emitter_life.base == 0.0
        && d.pulse_period.base == 0.0
        && d.particle_life.base > 0.0
        && d.particle_life.base <= 8.0
        && d.size.base == 0.0
        && d.speed.base != 0.0
        && d.alpha.base > 0.0
        && d.air_resistance.base == 0.0
        && d.gravity.base == 0.0
        && d.acceleration == Vec3::ZERO
        && d.random_offset == Vec3::ZERO
        && d.pos_random_offset == 0.0
        && d.position_offset.is_finite()
        && d.focus_angle.base == 0.0
        && d.focus_azimuth.base == 0.0
        && d.emit_angle.base == 180.0
        && d.emit_angle.var == 1.0
        && d.emit_angle.strength.is_none()
        && d.emit_angle.age.is_none()
        && d.attach == crate::params::Attach::None
}

#[cfg(test)]
mod tests;
