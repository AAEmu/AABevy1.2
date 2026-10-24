//! The parameters of one particle emitter (`<Params .../>` of a CryEngine 3 particle library).
//!
//! Every attribute is a string in the engine's type serialisation:
//!
//! * a scalar is `"1.5"`; an empty value means the default (`Size=""`, or just not written);
//! * a *variable* parameter (`TVarEPParam`) is `"base,variation,emitter curve,age curve"` with every part optional:
//!   `"2.27,,,(,0.045;0.25,0.29;1)"` is base 2.27 and a curve over the particle's age. The variation is the fraction
//!   of the base that a random draw takes away (`3.5,0.3` = 2.45 to 3.5 seconds);
//! * a curve is `(key;key;...)`, a key is `time,value[,flags]`; an omitted time or value is 0, so
//!   `(;0.07,1;0.53,0.16;1)` is 0 at the start, 1 at 7 %, 0.16 at 53 % and 0 at the end (a key that is only one number is a time);
//! * a vector is `"x,y,z"` with empty parts for the default (`",,0.5"`), a colour `"(r,g,b)"` with the default 1.
//!
//! Only what draws the particles is read; physics, lights and sounds are ignored.

use std::sync::{Arc, OnceLock};

use bevy::math::Vec3;

/// Splits at the commas that are not inside parentheses.
pub(crate) fn split_top(s: &str) -> Vec<&str> {
    let mut out = Vec::with_capacity(4);
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

fn num(s: &str) -> Option<f32> {
    let t = s.trim();
    if t.is_empty() { None } else { t.parse().ok() }
}

fn strip_parens(s: &str) -> &str {
    let t = s.trim();
    t.strip_prefix('(').and_then(|t| t.strip_suffix(')')).unwrap_or(t)
}

/// `"x,y,z"` or `"(x,y,z)"`; empty parts take `default`.
pub(crate) fn parse_vec3(s: &str, default: f32) -> Vec3 {
    let parts = split_top(strip_parens(s));
    let c = |i: usize| parts.get(i).and_then(|p| num(p)).unwrap_or(default);
    Vec3::new(c(0), c(1), c(2))
}

/// A piecewise linear curve over 0..1 (flat outside the keys).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve<T> {
    pub keys: Vec<(f32, T)>,
}

pub trait Lerp: Copy {
    fn lerp_to(self, other: Self, t: f32) -> Self;
}
impl Lerp for f32 {
    fn lerp_to(self, other: f32, t: f32) -> f32 {
        self + (other - self) * t
    }
}
impl Lerp for Vec3 {
    fn lerp_to(self, other: Vec3, t: f32) -> Vec3 {
        self + (other - self) * t
    }
}

impl<T: Lerp> Curve<T> {
    pub fn eval(&self, t: f32) -> Option<T> {
        let first = self.keys.first()?;
        if t <= first.0 {
            return Some(first.1);
        }
        for w in self.keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t <= b.0 {
                let span = b.0 - a.0;
                let f = if span > 1e-6 { (t - a.0) / span } else { 1.0 };
                return Some(a.1.lerp_to(b.1, f));
            }
        }
        self.keys.last().map(|k| k.1)
    }
}

/// `(key;key)` -> `(time, value token)` pairs. A key with a single part is a time (its value is then empty).
fn curve_keys(s: &str) -> Option<Vec<(f32, String)>> {
    let t = s.trim();
    if !t.starts_with('(') {
        return None;
    }
    let inner = strip_parens(t);
    let mut keys = Vec::new();
    for key in inner.split(';') {
        let parts = split_top(key);
        let time = parts.first().and_then(|p| num(p)).unwrap_or(0.0);
        let value = parts.get(1).map(|p| p.trim().to_string()).unwrap_or_default();
        keys.push((time, value));
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(keys)
}

fn parse_curve(s: &str) -> Option<Curve<f32>> {
    let keys = curve_keys(s)?;
    Some(Curve { keys: keys.into_iter().map(|(t, v)| (t, num(&v).unwrap_or(0.0))).collect() })
}

fn parse_curve3(s: &str) -> Option<Curve<Vec3>> {
    let keys = curve_keys(s)?;
    Some(Curve { keys: keys.into_iter().map(|(t, v)| (t, if v.is_empty() { Vec3::ONE } else { parse_key3(&v) })).collect() })
}

/// The colour of a curve key: an empty component is 1 (the default), a missing trailing one is 0. 418 keys of the libraries have two
/// components and they read as fires and smoke only with a blue of 0 (`(0.282,0.09)` is a dark red, `(0.035,0.027)` black); with a
/// blue of 1 the end of `firewall_launch` is violet, which the real client does not draw.
fn parse_key3(s: &str) -> Vec3 {
    let parts = split_top(strip_parens(s));
    let c = |i: usize| match parts.get(i) {
        None => 0.0,
        Some(p) => num(p).unwrap_or(1.0),
    };
    Vec3::new(c(0), c(1), c(2))
}

/// A scalar parameter: `base`, random `var`iation, a curve over the emitter's strength and one over the particle's age.
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    pub base: f32,
    pub var: f32,
    pub strength: Option<Curve<f32>>,
    pub age: Option<Curve<f32>>,
}

impl Var {
    pub fn constant(base: f32) -> Var {
        Var { base, var: 0.0, strength: None, age: None }
    }

    pub fn parse(s: &str, default: f32) -> Var {
        let parts = split_top(s);
        Var {
            base: parts.first().and_then(|p| num(p)).unwrap_or(default),
            var: parts.get(1).and_then(|p| num(p)).unwrap_or(0.0),
            strength: parts.get(2).and_then(|p| parse_curve(p)),
            age: parts.get(3).and_then(|p| parse_curve(p)),
        }
    }

    /// The value for a new particle: `r` in 0..1 is the random draw, `strength` the emitter's strength.
    pub fn spawn(&self, r: f32, strength: f32) -> f32 {
        let mut v = self.base * (1.0 - self.var * r);
        if let Some(c) = &self.strength {
            v *= c.eval(strength).unwrap_or(1.0);
        }
        v
    }

    /// The value without randomness (the larger end).
    pub fn max(&self) -> f32 {
        self.base
    }
}

/// A colour parameter; the age curve multiplies the base.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorVar {
    pub base: Vec3,
    pub strength: Option<Curve<Vec3>>,
    pub age: Option<Curve<Vec3>>,
}

impl ColorVar {
    pub fn parse(s: &str) -> ColorVar {
        let parts = split_top(s);
        ColorVar {
            base: parts.first().map_or(Vec3::ONE, |p| if p.trim().is_empty() { Vec3::ONE } else { parse_vec3(p, 1.0) }),
            strength: parts.get(2).and_then(|p| parse_curve3(p)),
            age: parts.get(3).and_then(|p| parse_curve3(p)),
        }
    }
}

impl Default for ColorVar {
    fn default() -> Self {
        ColorVar { base: Vec3::ONE, strength: None, age: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Blend {
    #[default]
    Alpha,
    Additive,
    /// Premultiplied "screen": `dst * (1 - src) + src`.
    ColorBased,
    Multiplicative,
    Opaque,
    /// Not drawn.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Facing {
    /// A billboard facing the camera.
    #[default]
    Camera,
    /// A card in 3D space turned by the particle's angles.
    Free,
    /// `Velocity`: a card in 3D space whose normal follows the direction of travel (the rings of a thrown spear).
    Velocity,
    /// Flat on the ground (`Bottom`, `Water`, `Decal`, `Horizontal`, `Terrain`). `Bottom` is the ring, ripple, glow and fire of
    /// the ground: the click marker of the interface (`ui.click_0000`) stacks a `Decal` ring and a `Bottom` ring of the same
    /// texture, the ground fires and the bases of the area skills (`Size 10`) are `Bottom` too (it was a standing billboard
    /// before round 5, which stood every magic circle up in the air).
    Flat,
}

/// What an emitter emits from (`AttachType`): the effect's origin by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Attach {
    #[default]
    None,
    /// The bounding box of the entity the effect is on.
    BoundingBox,
    /// Its render mesh or physics proxy: a human sized capsule is used.
    Body,
}

/// Where on that shape the particles start (`AttachForm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AttachForm {
    #[default]
    Surface,
    Edges,
    Vertices,
    Volume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Spawn {
    /// Starts with its parent emitter.
    #[default]
    Direct,
    /// Emits when a particle of the parent is born.
    ParentStart,
    /// Emits when a particle of the parent dies.
    ParentDeath,
    /// Emits when a particle of the parent hits the ground (see [`crate::FxSettings::ground`]).
    ParentCollide,
}

/// Tiled / animated texture: `TextureTiling="tilesX,tilesY,firstTile,variants,frames,fps,cycle,blend"`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiling {
    pub x: u32,
    pub y: u32,
    pub first: u32,
    pub variants: u32,
    pub frames: u32,
    /// Frames per second, 0 = spread over the particle's life.
    pub fps: f32,
    pub cycle: bool,
    /// `AnimBlend`: the frame fades into the next one (the engine's pixel shader lerps the two frames).
    pub blend: bool,
}

impl Default for Tiling {
    fn default() -> Self {
        Tiling { x: 1, y: 1, first: 0, variants: 1, frames: 1, fps: 0.0, cycle: false, blend: false }
    }
}

/// One emitter of an effect: the parameters that decide how its particles look and move. Vectors are in
/// CryEngine space (Z up, Y forward) and are mapped into Bevy by the simulation.
#[derive(Clone, Debug)]
pub struct EmitterDef {
    pub name: String,
    pub enabled: bool,
    pub continuous: bool,
    pub count: Var,
    pub emitter_life: Var,
    pub spawn_delay: Var,
    pub pulse_period: Var,
    pub particle_life: Var,
    /// Pak path of the diffuse texture, or empty.
    pub texture: String,
    /// Pak path of a `.mtl` whose diffuse texture is used instead, or empty.
    pub material: String,
    pub tiling: Tiling,
    pub blend: Blend,
    pub facing: Facing,
    pub orient_to_velocity: bool,
    pub size: Var,
    pub size_x: Var,
    pub size_y: Var,
    /// Scale of a mesh along its up axis (`Geometry` emitters only).
    pub size_z: Var,
    pub stretch: Var,
    pub stretch_offset: f32,
    pub alpha: Var,
    /// `AlphaTest`: the texture's alpha has to be above this to show (a curve over the age makes the particle dissolve).
    pub alpha_test: Var,
    pub color: ColorVar,
    pub diffuse: f32,
    pub backlight: f32,
    pub emissive: f32,
    /// `EmissiveHDRDynamic`: the emissive light is scaled by `FxSettings::hdr_base` to this power (the engine: `colour *= diffuse
    /// + emissive * pow(hdr_multiplier, EmissiveHDRDynamic)` when the renderer draws in HDR).
    pub emissive_hdr: f32,
    /// `Facing` is `Decal` or `Terrain`: the quad lies on the ground and follows its height ([`crate::FxSettings::ground`]).
    pub decal: bool,
    /// `DecalAlphaMultiply` (1 when absent or 0).
    pub decal_alpha: f32,
    /// `LightSourceIntensity`, `LightSourceRadius` (metres) and `LightHDRDynamic`: every particle is a point light (default: none).
    pub light_intensity: Var,
    pub light_radius: Var,
    pub light_hdr: Var,
    pub speed: Var,
    pub air_resistance: Var,
    pub gravity: Var,
    pub acceleration: Vec3,
    pub wind: f32,
    pub emit_angle: Var,
    pub focus_angle: Var,
    pub focus_azimuth: Var,
    pub focus_gravity: bool,
    pub position_offset: Vec3,
    pub random_offset: Vec3,
    pub pos_random_offset: f32,
    pub init_angles: Vec3,
    pub random_angles: Vec3,
    pub rotation_rate: Vec3,
    pub random_rotation_rate: Vec3,
    pub turbulence_3d: f32,
    pub turbulence_size: f32,
    pub turbulence_speed: f32,
    /// `TailLength`: seconds of its past path a particle draws as a ribbon (0 = a sprite).
    pub tail_length: Var,
    /// `TailSteps`: segments of the ribbon.
    pub tail_steps: u32,
    /// `RotateSpeed`: radians per second of a particle about the axis (the particle orbits the emitter; the editor range is 0..10).
    pub rotate_speed: Var,
    /// `RotateLength`: radius of that orbit in metres.
    pub rotate_length: Var,
    /// `RotatePosition`: the axis of the orbit (CryEngine axes, zero = up).
    pub rotate_axis: Vec3,
    /// `RotateInitAngle`: where on the orbit a particle starts, in degrees.
    pub rotate_init: Var,
    /// `RotateWorldSpace`: the axis is a world axis, not the emitter's.
    pub rotate_world: bool,
    pub bind_to_emitter: bool,
    /// `CollideTerrain`: the particles bounce on the ground (only with a `PhysicsType`, see `collides`).
    pub collide: bool,
    /// `PhysicsType` is not `None` (SimpleCollision, SimplePhysics, RigidBody).
    pub physics: bool,
    /// `Bounciness`: 0 stops dead, 1 bounces fully, below 0 the particle dies on impact.
    pub bounciness: f32,
    /// `DynamicFriction`: how fast a particle on the ground slows down.
    pub friction: f32,
    /// `InheritVelocity`: how much of the emitter's own velocity a new particle gets.
    pub inherit_velocity: f32,
    pub attach: Attach,
    pub attach_form: AttachForm,
    pub spawn: Spawn,
    pub soft: bool,
    pub draw_last: i32,
    pub sort_oldest_first: bool,
    pub camera_max_distance: f32,
    pub config_min: u8,
    pub config_max: u8,
    pub has_geometry: bool,
    /// The `Geometry` model of an emitter that emits meshes (see [`crate::geom`]).
    pub geometry: String,
    pub(crate) geometry_in_pieces: bool,
    /// `BlendType` was written (a mesh takes its blend mode from its `.mtl` otherwise).
    pub blend_explicit: bool,
    /// `NoOffset`: a mesh keeps its own pivot instead of being centred on its bounding box.
    pub no_offset: bool,
    /// `NoUseMaterialAlpha`: the opacity of a mesh's material is ignored.
    pub no_material_alpha: bool,
    /// `IndependentUVTimer`: the texture animation of a mesh's material starts with every particle.
    pub independent_uv_timer: bool,
    // ForceGeneration is a force/target producer, never a request for extra particles.
    pub(crate) force_target: bool,
    pub(crate) wind_source: bool,
    pub(crate) ignore_attractor: bool,
    // Nearest first; retained even when an ancestor has no drawable or particles.
    pub(crate) target_sources: Vec<Arc<EmitterDef>>,
    luts: OnceLock<Luts>,
}

impl Default for EmitterDef {
    fn default() -> Self {
        EmitterDef {
            name: String::new(),
            enabled: true,
            continuous: false,
            count: Var::constant(1.0),
            emitter_life: Var::constant(0.0),
            spawn_delay: Var::constant(0.0),
            pulse_period: Var::constant(0.0),
            particle_life: Var::constant(1.0),
            texture: String::new(),
            material: String::new(),
            tiling: Tiling::default(),
            blend: Blend::Alpha,
            facing: Facing::Camera,
            orient_to_velocity: false,
            size: Var::constant(1.0),
            size_x: Var::constant(1.0),
            size_y: Var::constant(1.0),
            size_z: Var::constant(1.0),
            stretch: Var::constant(0.0),
            stretch_offset: 0.0,
            alpha: Var::constant(1.0),
            alpha_test: Var::constant(0.0),
            color: ColorVar::default(),
            diffuse: 1.0,
            backlight: 0.0,
            emissive: 0.0,
            emissive_hdr: 0.0,
            decal: false,
            decal_alpha: 1.0,
            light_intensity: Var::constant(0.0),
            light_radius: Var::constant(0.0),
            light_hdr: Var::constant(0.0),
            speed: Var::constant(1.0),
            air_resistance: Var::constant(0.0),
            gravity: Var::constant(0.0),
            acceleration: Vec3::ZERO,
            wind: 0.0,
            emit_angle: Var::constant(0.0),
            focus_angle: Var::constant(0.0),
            focus_azimuth: Var::constant(0.0),
            focus_gravity: false,
            position_offset: Vec3::ZERO,
            random_offset: Vec3::ZERO,
            pos_random_offset: 0.0,
            init_angles: Vec3::ZERO,
            random_angles: Vec3::ZERO,
            rotation_rate: Vec3::ZERO,
            random_rotation_rate: Vec3::ZERO,
            turbulence_3d: 0.0,
            turbulence_size: 0.0,
            turbulence_speed: 0.0,
            tail_length: Var::constant(0.0),
            tail_steps: 8,
            rotate_speed: Var::constant(0.0),
            rotate_length: Var::constant(0.0),
            rotate_axis: Vec3::ZERO,
            rotate_init: Var::constant(0.0),
            rotate_world: false,
            bind_to_emitter: false,
            collide: false,
            physics: false,
            bounciness: 0.0,
            friction: 0.0,
            inherit_velocity: 0.0,
            attach: Attach::None,
            attach_form: AttachForm::Surface,
            spawn: Spawn::Direct,
            soft: false,
            draw_last: 0,
            sort_oldest_first: false,
            camera_max_distance: 1.0e6,
            config_min: 0,
            config_max: 3,
            has_geometry: false,
            geometry: String::new(),
            geometry_in_pieces: false,
            blend_explicit: false,
            no_offset: false,
            no_material_alpha: false,
            independent_uv_timer: false,
            force_target: false,
            wind_source: false,
            ignore_attractor: false,
            target_sources: Vec::new(),
            luts: OnceLock::new(),
        }
    }
}

fn truth(v: &str) -> bool {
    v.eq_ignore_ascii_case("true") || v == "1"
}

fn quality(v: &str) -> u8 {
    match v.to_ascii_lowercase().as_str() {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        _ => 3,
    }
}

/// Pak path of a `Materials/...` reference (`.mtl`, lowercase, no `game/`).
fn material_path(v: &str) -> String {
    let mut p = v.trim().replace('\\', "/").to_ascii_lowercase();
    if let Some(rest) = p.strip_prefix("game/") {
        p = rest.to_string();
    }
    if !p.ends_with(".mtl") {
        p.push_str(".mtl");
    }
    p
}

impl EmitterDef {
    /// An emitter that does nothing (a `<Particles>` without `<Params>`).
    pub fn disabled(name: &str) -> EmitterDef {
        EmitterDef { name: name.to_string(), enabled: false, ..Default::default() }
    }

    /// Reads the attributes of a `<Params>` element.
    pub fn from_attributes<'a, 'i: 'a>(name: &str, attrs: impl Iterator<Item = roxmltree::Attribute<'a, 'i>>) -> EmitterDef {
        let mut d = EmitterDef { name: name.to_string(), ..Default::default() };
        for a in attrs {
            let v = a.value();
            match a.name() {
                "Enabled" => d.enabled = truth(v),
                "Continuous" => d.continuous = truth(v),
                "Count" => d.count = Var::parse(v, 1.0),
                "EmitterLifeTime" => d.emitter_life = Var::parse(v, 0.0),
                "SpawnDelay" => d.spawn_delay = Var::parse(v, 0.0),
                "PulsePeriod" => d.pulse_period = Var::parse(v, 0.0),
                "ForceGeneration" => { d.force_target = v == "Target"; d.wind_source = v == "Wind"; }
                "IgnoreAttractor" => d.ignore_attractor = truth(v),
                "ParticleLifeTime" => d.particle_life = Var::parse(v, 1.0),
                "Texture" => d.texture = aa_cry::material::texture_pak_path(v),
                "Material" => d.material = material_path(v),
                "TextureTiling" => {
                    let p = split_top(v);
                    let n = |i: usize, def: u32| p.get(i).and_then(|s| num(s)).map_or(def, |f| f.max(0.0) as u32);
                    d.tiling = Tiling {
                        x: n(0, 1).max(1),
                        y: n(1, 1).max(1),
                        first: n(2, 0),
                        variants: n(3, 1).max(1),
                        frames: n(4, 1).max(1),
                        fps: p.get(5).and_then(|s| num(s)).unwrap_or(0.0),
                        // field 6 is `AnimCycle`, 7 is `AnimBlend` (true on 3877 emitters: frames fade into each other, see `Tiling::blend`; 36 loop)
                        cycle: p.get(6).is_some_and(|s| truth(s.trim())),
                        blend: p.get(7).is_some_and(|s| truth(s.trim())),
                    };
                }
                "BlendType" => {
                    d.blend_explicit = true;
                    d.blend = match v {
                        "Additive" => Blend::Additive,
                        "ColorBased" => Blend::ColorBased,
                        "Multiplicative" => Blend::Multiplicative,
                        "Opaque" => Blend::Opaque,
                        "None" => Blend::None,
                        _ => Blend::Alpha,
                    }
                }
                "Facing" => {
                    d.decal = matches!(v, "Decal" | "Terrain");
                    d.facing = match v {
                        "Free" => Facing::Free,
                        "Velocity" => Facing::Velocity,
                        "Bottom" | "Water" | "Decal" | "Horizontal" | "Terrain" => Facing::Flat,
                        _ => Facing::Camera,
                    };
                }
                "OrientToVelocity" => d.orient_to_velocity |= truth(v),
                "Size" => d.size = Var::parse(v, 1.0),
                "SizeX" => d.size_x = Var::parse(v, 1.0),
                "SizeY" => d.size_y = Var::parse(v, 1.0),
                "SizeZ" => d.size_z = Var::parse(v, 1.0),
                "NoOffset" => d.no_offset = truth(v),
                "NoUseMaterialAlpha" => d.no_material_alpha = truth(v),
                "IndependentUVTimer" => d.independent_uv_timer = truth(v),
                "Stretch" => d.stretch = Var::parse(v, 0.0),
                "StretchOffsetRatio" => d.stretch_offset = num(v).unwrap_or(0.0),
                "Alpha" => d.alpha = Var::parse(v, 1.0),
                "AlphaTest" => d.alpha_test = Var::parse(v, 0.0),
                "Color" => d.color = ColorVar::parse(v),
                "DiffuseLighting" => d.diffuse = Var::parse(v, 1.0).base,
                "DiffuseBacklighting" => d.backlight = Var::parse(v, 0.0).base,
                "EmissiveLighting" => d.emissive = Var::parse(v, 0.0).base,
                "EmissiveHDRDynamic" => d.emissive_hdr = Var::parse(v, 0.0).base,
                "DecalAlphaMultiply" => d.decal_alpha = num(v).filter(|m| *m > 0.0).unwrap_or(1.0),
                "LightSourceIntensity" => d.light_intensity = Var::parse(v, 0.0),
                "LightSourceRadius" => d.light_radius = Var::parse(v, 0.0),
                "LightHDRDynamic" => d.light_hdr = Var::parse(v, 0.0),
                "Speed" => d.speed = Var::parse(v, 1.0),
                "AirResistance" => d.air_resistance = Var::parse(v, 0.0),
                "GravityScale" => d.gravity = Var::parse(v, 0.0),
                "Acceleration" => d.acceleration = parse_vec3(v, 0.0),
                "WindInfluence" => d.wind = num(v).unwrap_or(0.0),
                "EmitAngle" => d.emit_angle = Var::parse(v, 0.0),
                "FocusAngle" => d.focus_angle = Var::parse(v, 0.0),
                "FocusAzimuth" => d.focus_azimuth = Var::parse(v, 0.0),
                "FocusGravityDir" => d.focus_gravity = truth(v),
                "PositionOffset" => d.position_offset = parse_vec3(v, 0.0),
                "RandomOffset" => d.random_offset = parse_vec3(v, 0.0),
                "PosRandomOffset" => d.pos_random_offset = Var::parse(v, 0.0).base,
                "InitAngles" => d.init_angles = parse_vec3(v, 0.0),
                "RandomAngles" => d.random_angles = parse_vec3(v, 0.0),
                "RotationRate" => d.rotation_rate = parse_vec3(v, 0.0),
                "RandomRotationRate" => d.random_rotation_rate = parse_vec3(v, 0.0),
                "Turbulence3DSpeed" => d.turbulence_3d = Var::parse(v, 0.0).base,
                "TurbulenceSize" => d.turbulence_size = Var::parse(v, 0.0).base,
                "TurbulenceSpeed" => d.turbulence_speed = Var::parse(v, 0.0).base,
                "TailLength" => d.tail_length = Var::parse(v, 0.0),
                "TailSteps" => d.tail_steps = num(v).map_or(8, |f| f.max(0.0) as u32),
                "RotateSpeed" => d.rotate_speed = Var::parse(v, 0.0),
                "RotateLength" => d.rotate_length = Var::parse(v, 0.0),
                "RotatePosition" => d.rotate_axis = parse_vec3(v, 0.0),
                "RotateInitAngle" => d.rotate_init = Var::parse(v, 0.0),
                "RotateWorldSpace" => d.rotate_world = truth(v),
                "BindToEmitter" | "MoveRelEmitter" => d.bind_to_emitter |= truth(v),
                "CollideTerrain" => d.collide = truth(v),
                "PhysicsType" => d.physics = !matches!(v, "" | "None"),
                // some libraries say 20: a bounce never gains energy
                "Bounciness" => d.bounciness = Var::parse(v, 0.0).base.min(1.0),
                "DynamicFriction" => d.friction = Var::parse(v, 0.0).base,
                "InheritVelocity" => d.inherit_velocity = Var::parse(v, 0.0).base,
                "AttachType" => {
                    d.attach = match v {
                        "BoundingBox" => Attach::BoundingBox,
                        "Physics" | "Render" => Attach::Body,
                        _ => Attach::None,
                    }
                }
                "AttachForm" => {
                    d.attach_form = match v {
                        "Edges" => AttachForm::Edges,
                        "Vertices" => AttachForm::Vertices,
                        "Volume" => AttachForm::Volume,
                        _ => AttachForm::Surface,
                    }
                }
                "SpawnIndirection" => {
                    d.spawn = match v {
                        "ParentStart" => Spawn::ParentStart,
                        "ParentDeath" => Spawn::ParentDeath,
                        "ParentCollide" => Spawn::ParentCollide,
                        _ => Spawn::Direct,
                    }
                }
                "SoftParticle" => d.soft = truth(v),
                "DrawLast" => d.draw_last = num(v).unwrap_or(0.0) as i32,
                "SortOldestFirst" => d.sort_oldest_first = truth(v),
                "CameraMaxDistance" => d.camera_max_distance = num(v).unwrap_or(1.0e6),
                "ConfigMin" => d.config_min = quality(v),
                "ConfigMax" => d.config_max = quality(v),
                "Geometry" => {
                    d.has_geometry = !v.trim().is_empty();
                    d.geometry = v.trim().to_string();
                }
                "GeometryInPieces" => d.geometry_in_pieces = truth(v),
                _ => {}
            }
        }
        d.wind_source &= crate::wind::eligible(&d);
        d
    }

    /// The emitter as a one-time emission: a continuous emitter with no `EmitterLifeTime` runs for as long as its effect is held, this
    /// one gives it its particle's life as the emitter's, so it emits `Count` particles over that life (the first at once) and stops.
    /// `None` for an emitter that already ends (not continuous, an `EmitterLifeTime`, or particles that never die).
    pub fn first_cycle(&self) -> Option<EmitterDef> {
        (self.continuous && self.emitter_life.base <= 0.0 && self.particle_life.base > 0.0)
            .then(|| EmitterDef { emitter_life: Var::constant(self.particle_life.base), ..self.clone() })
    }

    /// Whether the emitter draws: sprites need a texture (or a material), meshes a model; the blend mode must show.
    pub fn draws(&self) -> bool {
        self.enabled && self.blend != Blend::None && (self.has_geometry || !self.texture.is_empty() || !self.material.is_empty())
    }

    /// Whether the particles hit the ground: `CollideTerrain` counts for emitters with a `PhysicsType` only.
    pub fn collides(&self) -> bool {
        self.collide && self.physics
    }

    /// Sprites that dissolve: they carry a per-vertex alpha test threshold.
    pub fn has_alpha_test(&self) -> bool {
        self.alpha_test.base > 0.0 && !self.has_geometry
    }

    /// Whether the particles draw a ribbon of their past path (`TailLength`).
    pub fn has_tail(&self) -> bool {
        self.tail_length.base > 0.0 && self.tail_steps >= 2
    }

    /// Whether the particles orbit the emitter (`RotateLength`). A radius alone is not an orbit: it needs a `RotateSpeed` or an
    /// authored `RotatePosition` (the axis of a ring). `ghost_summon_launch4` has `RotateLength` 50 and `el_fire0001_proj_trail_smoke`
    /// 180 with neither (22 emitters of the libraries): the real client shows those clouds where the effect is, not 50 m off.
    pub fn orbits(&self) -> bool {
        self.rotate_length.base != 0.0 && (self.rotate_speed.base != 0.0 || self.rotate_speed.var != 0.0 || self.rotate_axis != Vec3::ZERO)
    }

    /// Emitter life in seconds, 0 = endless.
    pub fn is_endless(&self) -> bool {
        self.emitter_life.base <= 0.0
    }

    /// Rough size of what the emitter can show, in metres at scale 1 (for distance culling).
    pub fn extent(&self) -> f32 {
        let size = self.size.base * self.size_x.base.max(self.size_y.base).max(1.0) * 2.0;
        let travel = if self.particle_life.base > 0.0 { self.speed.base.abs() * self.particle_life.base * 0.5 } else { 0.0 };
        let stretch = self.speed.base.abs() * self.stretch.base.abs();
        let target = if self.ignore_attractor { 0.0 } else { self.target_sources.iter().map(|s| s.position_offset.length()).fold(0.0, f32::max) };
        size + travel + stretch + self.random_offset.max_element().abs() * 1.0 + target
    }

    /// Every particle casts a point light (`LightSourceIntensity` and `LightSourceRadius` are set).
    pub fn has_light(&self) -> bool {
        self.light_intensity.base > 0.0 && self.light_radius.base > 0.0
    }

    pub fn luts(&self) -> &Luts {
        self.luts.get_or_init(|| Luts::bake(self))
    }
}

/// Number of intervals of a baked curve.
pub const LUT: usize = 32;

/// A curve over the particle's age baked into 33 samples (identity 1 when the parameter has no curve).
#[derive(Clone, Debug)]
pub struct Lut(pub [f32; LUT + 1]);

impl Lut {
    fn bake(c: Option<&Curve<f32>>) -> Lut {
        let mut v = [1.0; LUT + 1];
        if let Some(c) = c {
            for (i, s) in v.iter_mut().enumerate() {
                *s = c.eval(i as f32 / LUT as f32).unwrap_or(1.0);
            }
        }
        Lut(v)
    }

    #[inline]
    pub fn at(&self, t: f32) -> f32 {
        let x = t.clamp(0.0, 1.0) * LUT as f32;
        let i = (x as usize).min(LUT - 1);
        let f = x - i as f32;
        self.0[i] + (self.0[i + 1] - self.0[i]) * f
    }
}

#[derive(Clone, Debug)]
pub struct Lut3(pub [Vec3; LUT + 1]);

impl Lut3 {
    #[inline]
    pub fn at(&self, t: f32) -> Vec3 {
        let x = t.clamp(0.0, 1.0) * LUT as f32;
        let i = (x as usize).min(LUT - 1);
        let f = x - i as f32;
        self.0[i] + (self.0[i + 1] - self.0[i]) * f
    }
}

/// The age curves of an emitter, baked once.
#[derive(Clone, Debug)]
pub struct Luts {
    pub size: Lut,
    pub size_x: Lut,
    pub size_y: Lut,
    pub size_z: Lut,
    pub rot_speed: Lut,
    pub rot_len: Lut,
    pub alpha: Lut,
    pub alpha_test: Lut,
    pub color: Lut3,
    pub air: Lut,
    pub gravity: Lut,
    pub stretch: Lut,
    pub light_intensity: Lut,
    pub light_radius: Lut,
    pub light_hdr: Lut,
}

impl Luts {
    fn bake(d: &EmitterDef) -> Luts {
        let mut color = [Vec3::ONE; LUT + 1];
        if let Some(c) = &d.color.age {
            for (i, s) in color.iter_mut().enumerate() {
                *s = c.eval(i as f32 / LUT as f32).unwrap_or(Vec3::ONE);
            }
        }
        Luts {
            size: Lut::bake(d.size.age.as_ref()),
            size_x: Lut::bake(d.size_x.age.as_ref()),
            size_y: Lut::bake(d.size_y.age.as_ref()),
            size_z: Lut::bake(d.size_z.age.as_ref()),
            rot_speed: Lut::bake(d.rotate_speed.age.as_ref()),
            rot_len: Lut::bake(d.rotate_length.age.as_ref()),
            alpha: Lut::bake(d.alpha.age.as_ref()),
            alpha_test: Lut::bake(d.alpha_test.age.as_ref()),
            color: Lut3(color),
            air: Lut::bake(d.air_resistance.age.as_ref()),
            gravity: Lut::bake(d.gravity.age.as_ref()),
            stretch: Lut::bake(d.stretch.age.as_ref()),
            light_intensity: Lut::bake(d.light_intensity.age.as_ref()),
            light_radius: Lut::bake(d.light_radius.age.as_ref()),
            light_hdr: Lut::bake(d.light_hdr.age.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_at_top_level_commas() {
        assert_eq!(split_top("2.3,0.3,(,0.73;1,1),(,0.454,32;1,1)"), vec!["2.3", "0.3", "(,0.73;1,1)", "(,0.454,32;1,1)"]);
        assert_eq!(split_top(",,0.5"), vec!["", "", "0.5"]);
    }

    #[test]
    fn variable_parameters() {
        let v = Var::parse("2.27,,,(,0.04545455;0.255814,0.2954545;0.505814,0.8181818;1)", 1.0);
        assert_eq!(v.base, 2.27);
        assert_eq!(v.var, 0.0);
        let age = v.age.unwrap();
        assert_eq!(age.keys.len(), 4);
        assert_eq!(age.keys[3], (1.0, 0.0));
        assert!((age.eval(0.255814).unwrap() - 0.2954545).abs() < 1e-5);
        let v = Var::parse(",0.2", 1.0);
        assert_eq!((v.base, v.var), (1.0, 0.2));
        assert_eq!(Var::parse("", 7.0).base, 7.0);
    }

    #[test]
    fn alpha_curves_fade_in_and_out() {
        let v = Var::parse("0.4,0.35,,(;0.4017857,0.613;1)", 1.0);
        let c = v.age.unwrap();
        assert_eq!(c.eval(0.0), Some(0.0));
        assert!((c.eval(0.4017857).unwrap() - 0.613).abs() < 1e-6);
        assert_eq!(c.eval(1.0), Some(0.0));
        // keys before the first one are flat
        let c = Var::parse(",,,(0.5,1;1)", 1.0).age.unwrap();
        assert_eq!(c.eval(0.1), Some(1.0));
        assert_eq!(c.eval(1.0), Some(0.0));
    }

    #[test]
    fn colours_default_to_one() {
        let c = ColorVar::parse("(,0.82,0.675)");
        assert_eq!(c.base, Vec3::new(1.0, 0.82, 0.675));
        let c = ColorVar::parse(",,,(0.00877193,(1,0.9137256,0.6156863);0.1491228,(1,1,1))");
        assert_eq!(c.age.unwrap().keys.len(), 2);
    }

    /// `RotateLength` alone is not an orbit (`ghost_summon_launch4`: 50 m, no speed, no axis); a speed or an axis makes it one.
    #[test]
    fn a_radius_alone_does_not_orbit() {
        let def = |attrs: &str| {
            let xml = format!(r#"<Params {attrs}/>"#);
            let doc = roxmltree::Document::parse(&xml).unwrap();
            EmitterDef::from_attributes("x", doc.root_element().attributes())
        };
        assert!(!def(r#"RotateLength="50" RotateInitAngle=",1""#).orbits());
        assert!(def(r#"RotateLength="2" RotateSpeed="3""#).orbits());
        assert!(def(r#"RotateLength="2" RotatePosition=",,1""#).orbits());
    }

    #[test]
    fn emitter_from_xml() {
        let xml = r#"<Params Continuous="true" Count="2" ParticleLifeTime=",0.2" BlendType="Additive" Texture="game/textures/p_effects/fire_flame10_v.dds" TextureTiling="6,6,,,36,36" Alpha=",,,(;0.08,1;0.92,1;1)" Size="0.3,0.1" Speed="0.001" Acceleration=",,0.2" FocusGravityDir="true"/>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let d = EmitterDef::from_attributes("x", doc.root_element().attributes());
        assert!(d.continuous && d.focus_gravity && d.draws());
        assert_eq!(d.blend, Blend::Additive);
        assert_eq!(d.texture, "textures/p_effects/fire_flame10_v.dds");
        assert_eq!((d.tiling.x, d.tiling.y, d.tiling.frames, d.tiling.fps), (6, 6, 36, 36.0));
        assert_eq!(d.particle_life.base, 1.0);
        assert_eq!(d.acceleration, Vec3::new(0.0, 0.0, 0.2));
        assert!((d.luts().alpha.at(0.5) - 1.0).abs() < 1e-5);
    }
    #[test]
    fn a_colour_key_with_two_components_has_no_blue() {
        let c = ColorVar::parse(",,,(0.1,(1,0.525,0.267);0.9,(0.282,0.09))");
        let age = c.age.unwrap();
        assert_eq!(age.keys[1].1, Vec3::new(0.282, 0.09, 0.0));
        // the base colour keeps the default for what is missing
        assert_eq!(ColorVar::parse("(0.14,0.4)").base, Vec3::new(0.14, 0.4, 1.0));
    }
}
