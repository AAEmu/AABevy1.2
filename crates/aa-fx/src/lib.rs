//! CryEngine 3 particle effects of the ArcheAge 1.2 client in Bevy.
//!
//! ```ignore
//! app.add_plugins(aa_fx::FxPlugin);
//! commands.spawn((aa_fx::Effect::new("X2_EFFECTS.fire_smoke.torchlight_b"), Transform::from_xyz(0.0, 0.0, -3.0)));
//! ```
//!
//! An [`Effect`] names a `library.group.name` of `game/libs/particles/*.xml` (see [`library`]). The library loads on a task
//! when the first effect of it appears. An effect far from the camera costs one distance test; within its activation radius it
//! gets one mesh entity (a child) per drawing emitter, a CPU simulation ([`sim`]) and is primed so that it looks burning at
//! once; beyond the radius it is dropped again. Every emitter is one dynamic mesh of camera facing quads drawn with
//! [`FxMaterial`] (unlit, additive / alpha, no depth write, soft against the depth prepass). Emitters with a `Geometry` emit
//! meshes instead ([`geom`]): the model is instanced on the CPU into the same kind of dynamic mesh. A global particle budget
//! ([`FxSettings::max_particles`]) caps the work.
//!
//! The effect's local frame is CryEngine's, mapped into Bevy by [`c2b`]: place effects built from CryEngine data with the same
//! rotation conversion as other cell objects. Effects that emit along "up" (`FocusGravityDir`, most of them) do not care.

mod assets;
pub mod geom;
pub mod library;
mod material;
pub mod params;
pub mod sim;

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bevy::asset::embedded_asset;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;

pub use assets::{FxAssets, FxLibraries, GeomState, LibStatus, MatKey, MeshMat, RefrKey, Refraction, SlotState, Source};
pub use geom::{Geom, GeomKey};
pub use library::{EffectDef, Library, split_name};
pub use material::FxMaterial;
pub use params::{Blend, EmitterDef, Facing, Spawn};
pub use sim::c2b;

use sim::{EffectSim, EmitterState, StepCtx, View};

const QUAD_INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];

/// Global tuning. Change it at any time; the colour gain applies to every material at once.
#[derive(Resource, Clone, Debug)]
pub struct FxSettings {
    /// Particles alive at once, all effects together.
    pub max_particles: usize,
    /// Activation radius cap in metres for a small effect (bigger effects get up to three times as much).
    pub max_distance: f32,
    /// Multiplier on every particle's colour. 1 for Bevy's own tone mapping; a world with pre-exposed HDR colours sets
    /// `radiance * exposure` here.
    pub gain: f32,
    /// How much of a lit particle's `DiffuseLighting` shows (the scene's light; 0.8 is a daylight guess).
    pub light: f32,
    /// Multiplier on `EmissiveLighting`.
    pub emissive: f32,
    /// Multiplier on every particle size.
    pub size_scale: f32,
    /// Half the width of a sprite in units of its `Size`: CryEngine's `Size` is a radius (the quad runs from -Size to +Size), so 1.
    /// 0.5 would read it as the full width (what this crate did before round 4). Meshes are not affected (`Size` scales the model).
    /// `AA_FX_SPRITE=0.5` goes back.
    pub sprite_half: f32,
    /// Multiplier on every particle's opacity (1 = as authored).
    pub alpha_scale: f32,
    /// A continuous emitter with a finite `EmitterLifeTime` emits `Count` over that life (otherwise `Count` per particle life).
    pub count_over_life: bool,
    /// Quality level of the data: 0 low, 1 medium, 2 high, 3 very high (emitters outside their `ConfigMin`/`ConfigMax` are off).
    pub quality: u8,
    /// Soft particle distance in metres (0 = off); needs a `DepthPrepass` on the camera.
    pub soft_distance: f32,
    /// New effects started per frame.
    pub activations_per_frame: usize,
    /// Wind in world space (m/s), felt by emitters with `WindInfluence`.
    pub wind: Vec3,
    /// Direction to the sun in world space: shades the opaque meshes (debris) of effects.
    pub sun: Vec3,
    /// Draw refraction particles (`RefrBumpScale` materials) as a distortion of the scene behind them; needs
    /// `ScreenSpaceTransmission` on the camera, which the plugin adds. Off: they are not drawn.
    pub refraction: bool,
    /// Height of the ground below a world position (terrain, floor): the emitters with `CollideTerrain` bounce on it and the
    /// ones that start on a collision (`SpawnIndirection=ParentCollide`) start there. `None`: nothing collides unless the effect
    /// names a [`Effect::ground_y`].
    pub ground: Option<Ground>,
    pub enabled: bool,
}

/// A ground height query, see [`FxSettings::ground`].
#[derive(Clone)]
pub struct Ground(pub Arc<dyn Fn(Vec3) -> Option<f32> + Send + Sync>);

impl std::fmt::Debug for Ground {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ground(..)")
    }
}

impl Default for FxSettings {
    fn default() -> Self {
        FxSettings {
            max_particles: 20_000,
            max_distance: 160.0,
            gain: 1.0,
            light: 0.8,
            emissive: 1.0,
            size_scale: 1.0,
            sprite_half: std::env::var("AA_FX_SPRITE").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            alpha_scale: 1.0,
            count_over_life: std::env::var_os("AA_FX_COUNT_OLD").is_none(),
            quality: 2,
            soft_distance: 0.6,
            activations_per_frame: 6,
            wind: Vec3::new(0.5, 0.0, 0.2),
            sun: Vec3::new(0.4, 0.8, 0.3).normalize(),
            refraction: std::env::var_os("AA_FX_NO_REFRACTION").is_none(),
            ground: None,
            enabled: true,
        }
    }
}

/// Counters of the last frame (read them for logs and HUDs).
#[derive(Resource, Default, Clone, Debug)]
pub struct FxStats {
    /// `Effect` entities.
    pub effects: usize,
    /// Effects with a running simulation.
    pub active: usize,
    pub emitters: usize,
    pub particles: usize,
    /// Particle budget the particles use (a mesh counts its vertices in quads).
    pub load: usize,
    /// Quads and mesh copies drawn.
    pub quads: usize,
    /// Main thread time of the simulation pass (ms), of the mesh upload, of the management systems.
    pub sim_ms: f32,
    pub upload_ms: f32,
    pub manage_ms: f32,
}

/// A particle effect of a library, e.g. `Effect::new("X2_EFFECTS.fire_smoke.torchlight_b")`. The entity's transform places it
/// (its scale scales the effect).
#[derive(Component, Clone, Debug)]
#[require(Transform, Visibility, FxRun)]
pub struct Effect {
    /// `library.group.name` (case does not matter).
    pub name: String,
    /// Extra size / offset / speed scale on top of the transform's.
    pub scale: f32,
    pub speed_scale: f32,
    /// Multiplier on every emitter's particle count.
    pub count_scale: f32,
    /// Multiplier on the opacity of this effect's particles.
    pub alpha_scale: f32,
    /// Multiplier on the colour of this effect's particles (the light that falls on them).
    pub brightness: f32,
    /// Colour of that light, multiplied into every particle (white = `Vec3::ONE`).
    pub tint: Vec3,
    /// Run the effect ahead when it starts so that it already looks established (`bPrime`).
    pub prime: bool,
    /// World height of a flat ground for this effect (when [`FxSettings::ground`] does not know): collisions use it.
    pub ground_y: Option<f32>,
    /// Start again when every emitter has finished (a one shot effect plays once per activation otherwise).
    pub looping: bool,
    /// Despawn the entity when the effect has finished.
    pub despawn_when_finished: bool,
}

impl Effect {
    pub fn new(name: impl Into<String>) -> Effect {
        Effect { name: name.into(), scale: 1.0, speed_scale: 1.0, count_scale: 1.0, alpha_scale: 1.0, brightness: 1.0, tint: Vec3::ONE, ground_y: None, prime: true, looping: false, despawn_when_finished: false }
    }

    pub fn with_scale(mut self, scale: f32) -> Effect {
        self.scale = scale;
        self
    }
}

/// Runtime state of an [`Effect`] (inserted with it).
#[derive(Component, Default)]
pub struct FxRun {
    def: Option<Arc<EffectDef>>,
    radius: f32,
    sim: Option<EffectSim>,
    meshes: Vec<Option<(Entity, Handle<Mesh>)>>,
    missing: bool,
    finished: bool,
    acc_dt: f32,
    frame: u32,
    warmup: u8,
    uploaded_units: Vec<usize>,
    /// Where the entity was last frame and how fast it moves (`InheritVelocity`).
    last_pos: Vec3,
    has_last: bool,
    vel: Vec3,
}

impl FxRun {
    /// The effect exists in its library.
    pub fn resolved(&self) -> bool {
        self.def.is_some()
    }

    /// The library or the effect does not exist.
    pub fn is_missing(&self) -> bool {
        self.missing
    }

    /// A simulation is running (the effect is within range).
    pub fn is_active(&self) -> bool {
        self.sim.is_some()
    }

    /// Every emitter has finished.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Radius in metres within which the effect runs.
    pub fn radius(&self) -> f32 {
        self.radius
    }

    pub fn particles(&self) -> usize {
        self.sim.as_ref().map_or(0, |s| s.alive())
    }
}

/// Marks effects with a running simulation.
#[derive(Component)]
struct FxActive;

/// The mesh entity of an emitter.
#[derive(Component)]
pub struct FxMesh;

/// The camera.
#[derive(Resource, Default)]
struct FxView(Option<View>);

/// Effects waiting for their library.
#[derive(Resource, Default)]
struct FxPending(Vec<Entity>);

/// Requests of the classification pass: (distance, effect, activate).
#[derive(Resource, Default)]
struct FxQueue(Mutex<Vec<(f32, Entity, bool)>>);

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fx.wesl");
        app.add_plugins(MaterialPlugin::<FxMaterial>::default())
            .init_resource::<FxSettings>()
            .init_resource::<FxStats>()
            .init_resource::<FxView>()
            .init_resource::<FxPending>()
            .init_resource::<FxQueue>()
            .init_resource::<FxLibraries>()
            .init_resource::<FxAssets>()
            .add_systems(
                PostUpdate,
                (camera_view, register, resolve, classify, apply, bind_geometry, ensure_transmission, simulate, upload)
                    .chain()
                    .run_if(|s: Res<FxSettings>| s.enabled)
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(VisibilitySystems::CheckVisibility),
            );
        if std::env::var_os("AA_FX_LOG").is_some() {
            app.add_systems(Update, log_stats);
        }
    }
}

/// `AA_FX_LOG=1`: the counters every 2 seconds.
fn log_stats(time: Res<Time>, stats: Res<FxStats>, view: Res<FxView>, mut since: Local<f32>, active: Query<(&Effect, &GlobalTransform, &FxRun), With<FxActive>>) {
    *since += time.delta_secs();
    if *since < 2.0 {
        return;
    }
    *since = 0.0;
    info!(
        "aa-fx: effects={} active={} emitters={} particles={} quads={} sim_ms={:.3} upload_ms={:.3} manage_ms={:.3} camera={:?}",
        stats.effects, stats.active, stats.emitters, stats.particles, stats.quads, stats.sim_ms, stats.upload_ms, stats.manage_ms,
        view.0.map(|v| v.pos)
    );
    if std::env::var("AA_FX_LOG").is_ok_and(|v| v == "2") {
        for (effect, gt, run) in &active {
            let Some(sim) = run.sim.as_ref() else { continue };
            for e in &sim.emitters {
                info!(
                    "aa-fx:   {} at {:?}: {} particles, {} quads, bounds {:?}..{:?}, drawn {}",
                    effect.name, gt.translation(), e.alive(), e.units, e.min, e.max, e.draws
                );
            }
        }
    }
}

/// Refraction materials read the scene drawn so far: the camera has to keep a copy of it.
fn ensure_transmission(mut commands: Commands, assets: Res<FxAssets>, cameras: Query<Entity, (With<Camera3d>, Without<bevy::pbr::ScreenSpaceTransmission>)>) {
    if !assets.refractive {
        return;
    }
    for camera in &cameras {
        commands.entity(camera).insert(bevy::pbr::ScreenSpaceTransmission { steps: 1, quality: bevy::pbr::ScreenSpaceTransmissionQuality::Low });
    }
}

fn camera_view(cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>, mut view: ResMut<FxView>, mut stats: ResMut<FxStats>) {
    stats.manage_ms = 0.0;
    view.0 = cameras
        .iter()
        .filter(|(c, _)| c.is_active)
        .max_by_key(|(c, _)| c.order)
        .map(|(_, gt)| View { pos: gt.translation(), right: gt.right().into(), up: gt.up().into(), forward: gt.forward().into() });
}

/// `AA_FX_ONLY=a,b` / `AA_FX_SKIP=a,b` (debugging): only effects whose name contains one of the words run / the ones that contain
/// one of the words do not.
fn debug_filter(name: &str) -> bool {
    static FILTER: std::sync::OnceLock<(Vec<String>, Vec<String>)> = std::sync::OnceLock::new();
    let (only, skip) = FILTER.get_or_init(|| {
        let words = |var: &str| std::env::var(var).map(|v| v.split(',').map(|w| w.trim().to_ascii_lowercase()).filter(|w| !w.is_empty()).collect()).unwrap_or_default();
        (words("AA_FX_ONLY"), words("AA_FX_SKIP"))
    });
    let name = name.to_ascii_lowercase();
    (only.is_empty() || only.iter().any(|w| name.contains(w))) && !skip.iter().any(|w| name.contains(w))
}

/// `AA_FX_EMITTER=a,b` (debugging): only the emitters whose name contains one of the words are drawn.
fn emitter_filter(name: &str) -> bool {
    static WORDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let words = WORDS.get_or_init(|| std::env::var("AA_FX_EMITTER").map(|v| v.split(',').map(|w| w.trim().to_ascii_lowercase()).filter(|w| !w.is_empty()).collect()).unwrap_or_default());
    words.is_empty() || {
        let name = name.to_ascii_lowercase();
        words.iter().any(|w| name.contains(w))
    }
}

fn register(mut libs: ResMut<FxLibraries>, mut pending: ResMut<FxPending>, new: Query<(Entity, &Effect), Added<Effect>>) {
    for (e, effect) in &new {
        if !debug_filter(&effect.name) {
            continue;
        }
        match split_name(&effect.name) {
            Some((lib, _)) => {
                libs.request(lib);
                pending.0.push(e);
            }
            None => warn!("aa-fx: effect name {:?} is not library.name", effect.name),
        }
    }
}

/// Radius in metres within which an effect of this definition is simulated.
fn activation_radius(def: &EffectDef, scale: f32, settings: &FxSettings) -> f32 {
    let mut extent = 0.0f32;
    let mut cam_max = 0.0f32;
    for e in def.emitters.iter().filter(|e| e.def.draws()) {
        extent = extent.max(e.def.extent());
        cam_max = cam_max.max(e.def.camera_max_distance);
    }
    let extent = extent * scale;
    let cap = settings.max_distance * (extent / 3.0).clamp(1.0, 3.0);
    (extent * 333.0).min(cam_max).min(cap).max(8.0)
}

#[allow(clippy::too_many_arguments)]
fn resolve(
    settings: Res<FxSettings>,
    mut stats: ResMut<FxStats>,
    mut libs: ResMut<FxLibraries>,
    mut assets: ResMut<FxAssets>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<FxMaterial>>,
    mut pending: ResMut<FxPending>,
    mut q: Query<(&Effect, &GlobalTransform, &mut FxRun)>,
) {
    let t0 = Instant::now();
    libs.poll();
    assets.poll(&mut images, &mut materials);
    if settings.is_changed() {
        assets.set_params(&mut materials, settings.gain, settings.soft_distance);
    }
    pending.0.retain(|&e| {
        let Ok((effect, gt, mut run)) = q.get_mut(e) else { return false };
        let Some((lib, rest)) = split_name(&effect.name) else { return false };
        match libs.status(lib) {
            LibStatus::Loading => true,
            LibStatus::Failed => {
                run.missing = true;
                false
            }
            LibStatus::Ready(l) => {
                match l.get(rest) {
                    Some(def) => {
                        let scale = effect.scale * gt.affine().matrix3.y_axis.length();
                        run.radius = activation_radius(def, scale, &settings);
                        run.def = Some(def.clone());
                    }
                    None => {
                        warn!("aa-fx: effect {} not found in library {}", effect.name, lib);
                        run.missing = true;
                    }
                }
                false
            }
        }
    });
    stats.manage_ms += t0.elapsed().as_secs_f32() * 1000.0;
}

/// Which effects should start or stop: a distance test per effect, in parallel.
fn classify(view: Res<FxView>, queue: Res<FxQueue>, mut stats: ResMut<FxStats>, q: Query<(Entity, &GlobalTransform, &FxRun)>) {
    let t0 = Instant::now();
    let Some(view) = view.0 else { return };
    q.par_iter().for_each(|(e, gt, run)| {
        if run.def.is_none() {
            return;
        }
        let d = gt.translation().distance(view.pos);
        let active = run.sim.is_some();
        if ((!active && !run.finished && d < run.radius) || (active || run.finished) && d > run.radius * 1.15)
            && let Ok(mut v) = queue.0.lock()
        {
            v.push((d, e, !active && !run.finished));
        }
    });
    stats.manage_ms += t0.elapsed().as_secs_f32() * 1000.0;
}

/// The dynamic mesh of an emitter: one quad until the first upload, or (`geometry`) no triangles yet.
fn new_mesh(geometry: bool, with_uv1: bool) -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4]);
    if with_uv1 {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32; 2]; 4]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4]);
    mesh.insert_indices(Indices::U32(if geometry { Vec::new() } else { QUAD_INDICES.to_vec() }));
    mesh
}

/// The entity that draws an emitter's mesh (a child of the effect).
fn spawn_mesh(commands: &mut Commands, effect: Entity, layers: Option<&RenderLayers>, mesh: Handle<Mesh>, material: Handle<FxMaterial>) -> Entity {
    let mut spawned = commands.spawn((
        FxMesh,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
        Aabb::from_min_max(Vec3::splat(-0.01), Vec3::splat(0.01)),
        NotShadowCaster,
        NotShadowReceiver,
        ChildOf(effect),
    ));
    // render layers are not inherited: the mesh is drawn for the cameras that see the effect
    if let Some(layers) = layers {
        spawned.insert(layers.clone());
    }
    spawned.id()
}

#[allow(clippy::too_many_arguments)]
fn apply(
    mut commands: Commands,
    settings: Res<FxSettings>,
    queue: Res<FxQueue>,
    mut stats: ResMut<FxStats>,
    mut assets: ResMut<FxAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FxMaterial>>,
    mut q: Query<(&Effect, &mut FxRun, Option<&RenderLayers>)>,
) {
    let t0 = Instant::now();
    let mut list = std::mem::take(&mut *queue.0.lock().unwrap_or_else(|e| e.into_inner()));
    if list.is_empty() {
        return;
    }
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut started = 0;
    for (_, entity, activate) in list {
        let Ok((effect, mut run, layers)) = q.get_mut(entity) else { continue };
        if !activate {
            for (m, _) in run.meshes.drain(..).flatten() {
                commands.entity(m).despawn();
            }
            run.sim = None;
            run.finished = false;
            run.uploaded_units.clear();
            commands.entity(entity).remove::<FxActive>();
            continue;
        }
        if started >= settings.activations_per_frame {
            continue;
        }
        started += 1;
        let Some(def) = run.def.clone() else { continue };
        let mut states = Vec::with_capacity(def.emitters.len());
        let mut handles = Vec::with_capacity(def.emitters.len());
        for (i, fe) in def.emitters.iter().enumerate() {
            let d = &fe.def;
            let enabled = settings.quality >= d.config_min && settings.quality <= d.config_max && emitter_filter(&d.name);
            let mut material = 0;
            let mut mesh = Entity::PLACEHOLDER;
            let mut handle = None;
            let mut pending_geom = None;
            let mut drawn = true;
            if d.draws() && enabled {
                if d.has_geometry {
                    // the mesh and its material come first, see `bind_geometry`
                    let key = GeomKey {
                        path: d.geometry.clone(),
                        material: d.material.clone(),
                        texture: d.texture.clone(),
                        blend: d.blend_explicit.then_some(d.blend),
                    };
                    assets.geom(&key);
                    pending_geom = Some(key);
                } else {
                    let source = if d.texture.is_empty() { Source::Material(d.material.clone()) } else { Source::Texture(d.texture.clone()) };
                    // a refraction material distorts the scene behind it: without that it shows nothing
                    let refraction = if d.texture.is_empty() { assets.refraction(&d.material) } else { None };
                    let is_refraction = refraction.is_some();
                    let refract = match refraction {
                        Some(r) if r.normal.is_empty() || !settings.refraction => None,
                        Some(r) => Some(RefrKey { normal: r.normal, bump: (r.bump * 1000.0).round().clamp(0.0, 60_000.0) as u16 }),
                        None => None,
                    };
                    if !is_refraction || refract.is_some() {
                        material = assets.slot(
                            MatKey { source, blend: d.blend, soft: d.soft, draw_last: d.draw_last, mesh: None, refract, atest: d.has_alpha_test() },
                            &mut materials,
                            settings.gain,
                            settings.soft_distance,
                        );
                        let mesh_handle = meshes.add(new_mesh(false, d.has_alpha_test()));
                        mesh = spawn_mesh(&mut commands, entity, layers, mesh_handle.clone(), assets.slots[material as usize].handle.clone());
                        handle = Some((mesh, mesh_handle));
                    } else {
                        drawn = false;
                    }
                }
            }
            let mut st = EmitterState::new(fe.def.clone(), fe.parent, material, 1.0, hash_seed(entity, i), mesh);
            st.pending_geom = pending_geom;
            if !drawn {
                st.draws = false;
            }
            if !enabled {
                st.draws = false;
                st.disable();
            }
            states.push(st);
            handles.push(handle);
        }
        run.sim = Some(EffectSim::new(states, settings.light * effect.brightness, settings.emissive * effect.brightness));
        run.meshes = handles;
        run.warmup = 2;
        run.acc_dt = 0.0;
        run.has_last = false;
        run.vel = Vec3::ZERO;
        run.uploaded_units = vec![1; def.emitters.len()];
        commands.entity(entity).insert(FxActive);
    }
    stats.manage_ms += t0.elapsed().as_secs_f32() * 1000.0;
}

/// Gives the mesh emitters of active effects their mesh, material and entity once the model is loaded.
fn bind_geometry(
    mut commands: Commands,
    settings: Res<FxSettings>,
    mut assets: ResMut<FxAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FxMaterial>>,
    mut q: Query<(Entity, &mut FxRun, Option<&RenderLayers>), With<FxActive>>,
) {
    for (entity, mut run, layers) in &mut q {
        let run = &mut *run;
        let Some(sim) = run.sim.as_mut() else { continue };
        for (i, e) in sim.emitters.iter_mut().enumerate() {
            let Some(key) = e.pending_geom.clone() else { continue };
            match assets.geom(&key) {
                GeomState::Loading => {}
                GeomState::Failed => {
                    e.pending_geom = None;
                    e.draws = false;
                }
                GeomState::Ready(g) => {
                    let m = &g.mtl;
                    let source = if m.texture.is_empty() { Source::White } else { Source::Repeat(m.texture.clone()) };
                    let mesh_key = MeshMat {
                        mask: m.opacity_map.clone(),
                        mask_chan: m.opacity_mask.map(|c| (c * 255.0).round() as u8),
                        cutoff: (m.cutoff * 255.0).round() as u8,
                    };
                    let slot = assets.slot(
                        MatKey { source, blend: m.blend, soft: e.def.soft, draw_last: e.def.draw_last, mesh: Some(mesh_key), refract: None, atest: false },
                        &mut materials,
                        settings.gain,
                        settings.soft_distance,
                    );
                    let mesh_handle = meshes.add(new_mesh(true, !m.opacity_map.is_empty()));
                    let mesh = spawn_mesh(&mut commands, entity, layers, mesh_handle.clone(), assets.slots[slot as usize].handle.clone());
                    e.material = slot;
                    e.mesh = mesh;
                    e.set_geom(g);
                    if let Some(h) = run.meshes.get_mut(i) {
                        *h = Some((mesh, mesh_handle));
                    }
                    if let Some(u) = run.uploaded_units.get_mut(i) {
                        *u = 0;
                    }
                }
            }
        }
    }
}

fn hash_seed(e: Entity, i: usize) -> u64 {
    (e.to_bits() ^ 0x5851_F42D_4C95_7F2D).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(i as u64 * 7919)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    time: Res<Time>,
    settings: Res<FxSettings>,
    view: Res<FxView>,
    assets: Res<FxAssets>,
    mut stats: ResMut<FxStats>,
    mut q: Query<(Entity, &Effect, &GlobalTransform, &mut FxRun), With<FxActive>>,
    all: Query<(), With<Effect>>,
    mut commands: Commands,
) {
    let t0 = Instant::now();
    let Some(view) = view.0 else { return };
    let dt_frame = time.delta_secs().clamp(0.0, 0.1);
    let budget = AtomicI32::new(settings.max_particles.saturating_sub(stats.load) as i32);
    let alive = AtomicUsize::new(0);
    let load = AtomicUsize::new(0);
    let quads = AtomicUsize::new(0);
    let emitters = AtomicUsize::new(0);
    let despawn: Mutex<Vec<Entity>> = Mutex::new(Vec::new());
    let settings = &*settings;
    let assets = &*assets;

    let settings_ground = settings.ground.as_ref().map(|g| g.0.clone());
    q.par_iter_mut().for_each(|(entity, effect, gt, mut run)| {
        let run = &mut *run;
        let flat = effect.ground_y.map(|y| move |_: Vec3| Some(y));
        let global = settings_ground.as_ref().map(|g| move |p: Vec3| g(p));
        let ground_fn: Option<&(dyn Fn(Vec3) -> Option<f32> + Send + Sync)> = match (&global, &flat) {
            (Some(g), _) => Some(g),
            (None, Some(f)) => Some(f),
            _ => None,
        };
        let Some(mut sim) = run.sim.take() else { return };
        let m = gt.affine().matrix3;
        let (lx, ly, lz) = (m.x_axis.length().max(1e-6), m.y_axis.length().max(1e-6), m.z_axis.length().max(1e-6));
        let rot = Mat3::from_cols(Vec3::from(m.x_axis) / lx, Vec3::from(m.y_axis) / ly, Vec3::from(m.z_axis) / lz);
        let dist = gt.translation().distance(view.pos);
        let pos = gt.translation();
        if run.has_last && dt_frame > 1e-4 {
            // a teleport is not a speed
            let v = ((pos - run.last_pos) / dt_frame).clamp_length_max(40.0);
            run.vel = run.vel.lerp(v, 0.5);
        }
        run.last_pos = pos;
        run.has_last = true;
        let near = (dist / run.radius.max(1.0)).clamp(0.0, 1.0);
        let far = ((near - 0.45) / 0.55).clamp(0.0, 1.0);
        let mut ctx = StepCtx {
            dt: 0.0,
            view,
            global: gt.affine(),
            rot,
            scale: effect.scale * (lx + ly + lz) / 3.0,
            speed_scale: effect.speed_scale,
            count_scale: effect.count_scale,
            lod: 1.0 - 0.6 * far,
            size_scale: settings.size_scale,
            sprite_half: settings.sprite_half,
            count_over_life: settings.count_over_life,
            tint: effect.tint,
            sun: settings.sun,
            emitter_vel: run.vel,
            ground: ground_fn,
            wind: settings.wind,
            budget: &budget,
            fade: (1.0 - smoothstep(0.8, 1.0, near)) * settings.alpha_scale * effect.alpha_scale,
        };

        if !sim.primed {
            // wait until every texture is there, then start
            let mut ready = true;
            for e in sim.emitters.iter_mut().filter(|e| e.draws) {
                if e.pending_geom.is_some() {
                    ready = false;
                    continue;
                }
                let slot = &assets.slots[e.material as usize];
                match slot.state {
                    SlotState::Loading => ready = false,
                    SlotState::Failed => e.draws = false,
                    SlotState::Ready if e.geom.is_none() => {
                        let t = e.def.tiling;
                        e.aspect = (slot.size.y as f32 * t.x as f32 / (slot.size.x as f32 * t.y as f32)).clamp(0.25, 4.0);
                    }
                    SlotState::Ready => {}
                }
            }
            if ready {
                if effect.prime {
                    sim.prime(&ctx);
                } else {
                    sim.primed = true;
                }
            }
            run.sim = Some(sim);
            return;
        }

        run.acc_dt += dt_frame;
        run.frame = run.frame.wrapping_add(1);
        // far effects update every other frame
        let interval = if far > 0.5 { 2 } else { 1 };
        if interval == 1 || (run.frame + entity.index_u32()) % interval == 0 {
            ctx.dt = run.acc_dt.min(0.1);
            run.acc_dt = 0.0;
            sim.step(&ctx);
            if sim.finished() {
                if effect.looping {
                    sim.reset();
                    if effect.prime {
                        sim.prime(&ctx);
                    } else {
                        sim.primed = true;
                    }
                } else {
                    run.finished = true;
                    if effect.despawn_when_finished && let Ok(mut d) = despawn.lock() {
                        d.push(entity);
                    }
                }
            }
        }
        alive.fetch_add(sim.alive(), Ordering::Relaxed);
        load.fetch_add(sim.load(), Ordering::Relaxed);
        quads.fetch_add(sim.emitters.iter().map(|e| e.units).sum::<usize>(), Ordering::Relaxed);
        emitters.fetch_add(sim.emitters.len(), Ordering::Relaxed);
        run.sim = Some(sim);
    });

    for e in despawn.into_inner().unwrap_or_default() {
        commands.entity(e).despawn();
    }
    stats.effects = all.count();
    stats.active = q.iter().count();
    stats.particles = alive.into_inner();
    stats.load = load.into_inner();
    stats.quads = quads.into_inner();
    stats.emitters = emitters.into_inner();
    stats.sim_ms = t0.elapsed().as_secs_f32() * 1000.0;
}

/// Copies the new vertices into the mesh assets (a swap, no copy) and shows / hides the meshes.
fn upload(
    mut stats: ResMut<FxStats>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut q: Query<&mut FxRun, With<FxActive>>,
    mut vis: Query<(&mut Aabb, &mut Visibility), With<FxMesh>>,
) {
    let t0 = Instant::now();
    for mut run in &mut q {
        let run = &mut *run;
        let Some(sim) = run.sim.as_mut() else { continue };
        if !sim.primed {
            continue;
        }
        if run.warmup > 0 {
            // the meshes are children spawned this frame: their transforms propagate next frame
            run.warmup -= 1;
            continue;
        }
        for (i, e) in sim.emitters.iter_mut().enumerate() {
            if !e.dirty {
                continue;
            }
            let Some(Some((entity, handle))) = run.meshes.get(i) else { continue };
            let Ok((mut aabb, mut visibility)) = vis.get_mut(*entity) else { continue };
            e.dirty = false;
            if e.units == 0 {
                if *visibility != Visibility::Hidden {
                    *visibility = Visibility::Hidden;
                }
                continue;
            }
            let Some(mut mesh) = meshes.get_mut(handle) else { continue };
            if let Some(VertexAttributeValues::Float32x3(v)) = mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
                std::mem::swap(v, &mut e.pos);
            }
            if let Some(VertexAttributeValues::Float32x2(v)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
                std::mem::swap(v, &mut e.uv);
            }
            if let Some(VertexAttributeValues::Float32x2(v)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_1) {
                std::mem::swap(v, &mut e.uv1);
            }
            if let Some(VertexAttributeValues::Float32x4(v)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
                std::mem::swap(v, &mut e.col);
            }
            if run.uploaded_units[i] != e.units
                && let Some(Indices::U32(idx)) = mesh.indices_mut()
            {
                // the triangles of one quad or one mesh copy, shifted by its vertices
                let (verts, unit): (u32, &[u32]) = match &e.geom {
                    Some(g) => (g.tmpl.pos.len() as u32, &g.tmpl.idx),
                    None => (4, &QUAD_INDICES),
                };
                let have = idx.len() / unit.len();
                if e.units > have {
                    idx.reserve((e.units - have) * unit.len());
                    for u in have..e.units {
                        let b = u as u32 * verts;
                        idx.extend(unit.iter().map(|i| i + b));
                    }
                } else {
                    idx.truncate(e.units * unit.len());
                }
                run.uploaded_units[i] = e.units;
            }
            *aabb = Aabb::from_min_max(e.min, e.max);
            if *visibility != Visibility::Inherited {
                *visibility = Visibility::Inherited;
            }
        }
    }
    stats.upload_ms = t0.elapsed().as_secs_f32() * 1000.0;
}
