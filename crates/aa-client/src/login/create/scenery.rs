//! The training field behind step 3 (skillset and name): terrain, grass, rocks and the straw dummies
//! of the real client's login level, drawn through aa-cry / aa-world's public pieces on the stage
//! render layer.
//!
//! The field is not in the `game/create_<race>.lyr` layer files (those only hold lights and
//! cameras): it is a corner of the **login2** world, cell `000_001`. The capture cameras
//! (`05_skill_<race>`, `reference/character_create/step3_*/camera.json`) are in that cell's local
//! coordinates and so is the spot every figure stands on (`skill_<race>`: the terrain heights of the
//! cell match the solved feet z of all eight models). Per race the real client shows:
//!
//! * the cell's terrain: `heightmap.dat` patches with the baked `cover.ctc` colour and the
//!   surface type detail layers (aa-world's `terrain_layers` shader and [`LayerLibrary`], built for
//!   login2 here: the in-game world's own library knows only the main world);
//! * its vegetation (`vegetation.dat` + `vegetation.xml`: grass, flowers) and brushes (rocks);
//! * the `target_<race>_N` AnimObjects of `entities.xml`: three straw training dummies
//!   (`learning_ob01_c.cgf`) per race, a quaternion `Rotate="w,x,y,z"` each;
//! * a time of day per race (`e_time_of_day` of the capture: Nuian and Elf at dawn, Hariharan and
//!   Firran at dusk) which is only approximated here by a fog colour and a terrain tint.
//!
//! The ground mist and leaves (`flow_fog_01`, `leaf_01`, ... of `entities.xml`, aa-fx) are spawned with the props; `elf_fog_01` is
//! named by the level but missing from every particle library of the pak (nothing to draw).
//! Not drawn: birds, wind.
//!
//! Everything is built from public API of aa-world and aa-cry inside this module; the file loading
//! runs on a task when the screen opens, the models of the race are loaded ahead while steps 1 and 2
//! are shown, the entities are spawned over a few frames when step 3 is entered and removed when it
//! is left.
//!
//! ## The look
//!
//! The field is lit by the stage lights of the figure, which are about 8 times too bright for the
//! captured ground, so the terrain tint and the grass / rock materials are dimmed per race ([`mood`]),
//! and a linear [`DistanceFog`] in the colour of the lit far ground brings the ridge up to the
//! captured brightness. The sky is the painted `step3_<race>` background, drawn with +0.92 EV and no
//! tone curve ([`SKY_GRADE`]); the captures show it that much brighter than the raw texture.
//!
//! The grass (`Kind::Grass`: the plants and bushes of the vegetation, not the flowers) also gets a tone curve
//! ([`super::grass`]): the captured near grass is far more contrasty than the stage's lighting makes it (its dark
//! quarter is black, its sunlit blades golden), about a power of 2 - 5 of the radiance the stage gives, the engine's
//! HDR sun and film curve. The curve (exponent, reference radiance, linear part, saturation) and a colour factor per
//! race are fitted on the luminance quantiles, bright and dark tails of the near and mid grass of the captures.
//!
//! The values were fitted on the eight captures (the fog, terrain and rocks on mean colours of regions: sky, haze,
//! ridge; the grass as above). `AA_FIELD=ground,grass,fr,fg,fb,start,end[,gamma,reference,floor,saturation[,gr,gg,gb]]` and
//! `AA_BG_GRADE=tonemapping,ev,saturation` override them for another fit, `AA_CREATE_NO_FIELD=1` leaves the field
//! out, `AA_DUMMY_YAW=deg` turns the dummies.
//!
//! The dummy model (`learning_ob01_c.cgf`) is a static bind pose with two arm logs and a painted
//! shield (material `skill_df`) that the game poses away or shows only for a skill demonstration; the
//! shield and the logs are left out, and the model is turned a quarter so the shoulders face the
//! camera like in the captures.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use web_time::Instant;

use aa_anim::Race;
use aa_cry::scene::{CryAssets, LoadedModel, MaterialHook, ModelOptions, VertexHook, render_transform};
use aa_fx::Effect;
use aa_world::coords::CRY_TO_BEVY;
use aa_world::cover::{Cover, CoverAtlas};
use aa_world::heightmap::CellHeights;
use aa_world::placement::{CellObjects, PlacedEffect, load_cell_objects};
use aa_world::terrain::{LOD_COUNT, PATCHES, PATCH_UNITS, build_patch_mesh, cover_image, lod_for_distance};
use aa_world::terrain_layers::{CellSurfaces, DETAIL_DISTANCE, LayerLibrary, TerrainExt, TerrainMaterial};
use aa_world::vegetation::VegGroups;
use bevy::camera::visibility::RenderLayers;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::grass::{GrassExt, GrassMaterial};
use super::{CreateScreen, lock, poses};
use crate::login::stage3d::{BackgroundGrade, Grade, STAGE_LAYER, STAGE_ORIGIN};

/// The world and cell of the field.
const WORLD: &str = "login2";
const CELL: (i32, i32) = (0, 1);
/// Vegetation and brushes are placed within this distance of the camera (m).
const OBJECT_RADIUS: f32 = 170.0;
/// The arm logs of the dummy model end beyond this distance from its axis (m): triangles reaching further are dropped.
const ARM_LOG_X: f32 = 0.32;
/// Particle effects (ground mist, leaves) are placed within this distance of the camera (m).
const EFFECT_RADIUS: f32 = 120.0;
/// How the flowing mist (`flow_fog_*`) and the haze (`fog_*`) are lit in a field: a colour gain on the library's colours (linear
/// RGB, the stage's light) and, for the flowing mist only, an opacity factor and a particle count factor. The library's plain
/// values (grey sprites at 0.2 alpha) vanish over the field; the captures' mist is a pale, warm veil. Fitted per race on the
/// captures (the mist's peak colour and the band's mean excess over the grass, `docs/aa-fx.md`); the table is
/// `(alpha, count, r, g, b)`, `AA_MIST=alpha,count,r,g,b` overrides it.
#[derive(Clone, Copy)]
struct Mist {
    alpha: f32,
    count: f32,
    light: Vec3,
}

fn mist(race: Race) -> Mist {
    let m = |alpha, count, r, g, b| Mist { alpha, count, light: Vec3::new(r, g, b) };
    if let Some(v) = std::env::var("AA_MIST").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()) {
        if v.len() == 5 {
            return m(v[0], v[1], v[2], v[3], v[4]);
        }
    }
    match race {
        Race::Nuian => m(1.0, 1.0, 1.85, 1.8, 1.3),
        Race::Elf => m(1.0, 0.5, 2.75, 2.75, 1.95),
        Race::Hariharan => m(3.0, 3.0, 2.0, 1.98, 1.77),
        Race::Ferre => m(1.0, 0.6, 5.8, 3.4, 2.6),
    }
}
/// Main thread time spent on spawning per frame (ms).
const SPAWN_BUDGET_MS: f32 = 6.0;

/// The painted sky behind the field.
const SKY_GRADE: Grade = Grade { tonemapping: bevy::core_pipeline::tonemapping::Tonemapping::Linear, exposure: 0.92, saturation: 1.0 };

/// What a race's time of day does to the stage: the fog, factors on the terrain, grass and rock colours, and the tone
/// curve of the grass.
struct Mood {
    fog: Color,
    fog_start: f32,
    fog_end: f32,
    /// Linear factors on the colour of the terrain and of the grass (the field is lit like the figure,
    /// which is far brighter than the captured ground).
    ground: f32,
    grass: Vec3,
    /// Factor on the colour of the rocks and bushes (`Kind::Brush`).
    brush: f32,
    /// The tone curve of the grass (exponent, reference radiance, linear part, saturation), see `grass.rs`.
    curve: Vec4,
}

/// `AA_FIELD=ground,grass,fr,fg,fb,start,end[,gamma,reference,floor,saturation[,gr,gg,gb]]` overrides the mood of every race
/// (fitting against captures); the last three are colour factors on `grass`.
fn mood(race: Race) -> Mood {
    if let Some(v) = std::env::var("AA_FIELD").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()) {
        if v.len() >= 7 {
            let get = |i: usize, default: f32| v.get(i).copied().unwrap_or(default);
            return Mood {
                fog: Color::srgb(v[2], v[3], v[4]),
                fog_start: v[5],
                fog_end: v[6],
                ground: v[0],
                grass: Vec3::new(get(11, 1.0), get(12, 1.0), get(13, 1.0)) * v[1],
                brush: v[1] * 2.5,
                curve: Vec4::new(get(7, 1.0), get(8, 0.03), get(9, 0.0), get(10, 1.0)),
            };
        }
    }
    // fitted on the captures: the fog, the terrain and rocks on the mean colours of regions (sky, haze, ridge), the grass
    // on the luminance quantiles and bright and dark tails of the near and mid grass (`docs/aa-client-create.md`)
    let m = |fog: [f32; 3], ground: f32, grass: [f32; 3], brush: f32, curve: [f32; 4]| Mood {
        fog: Color::srgb(fog[0], fog[1], fog[2]),
        fog_start: 10.0,
        fog_end: 70.0,
        ground,
        grass: Vec3::from(grass),
        brush,
        curve: Vec4::from(curve),
    };
    match race {
        Race::Nuian => m([0.65, 0.62, 0.12], 0.25, [0.088, 0.104, 0.093], 0.30, [4.47, 0.0234, 0.16, 1.0]),
        Race::Elf => m([0.55, 0.60, 0.66], 0.35, [0.231, 0.231, 0.321], 0.45, [2.15, 0.0304, 0.06, 0.3]),
        Race::Hariharan => m([0.65, 0.55, 0.10], 0.35, [0.131, 0.161, 0.167], 0.30, [2.17, 0.0200, 0.32, 1.0]),
        Race::Ferre => m([0.88, 0.86, 0.20], 0.25, [0.084, 0.098, 0.088], 0.25, [4.65, 0.0237, 0.16, 1.0]),
    }
}

fn race_key(race: Race) -> &'static str {
    match race {
        Race::Nuian => "nuian",
        Race::Elf => "elf",
        Race::Hariharan => "hariharan",
        Race::Ferre => "ferre",
    }
}

/// A `target_<race>_N` AnimObject of the cell's `entities.xml`.
struct Dummy {
    name: String,
    model: String,
    pos: Vec3,
    rot: Quat,
}

/// Everything read from the pak on the task.
struct FieldData {
    heights: Arc<CellHeights>,
    cover: Option<CoverAtlas>,
    surfaces: CellSurfaces,
    arrays: [Image; aa_world::terrain_layers::LAYER_ARRAYS],
    objects: CellObjects,
    groups: VegGroups,
    dummies: Vec<Dummy>,
}

fn floats<const N: usize>(text: &str) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    let mut it = text.split(',');
    for v in &mut out {
        *v = it.next()?.trim().parse().ok()?;
    }
    Some(out)
}

fn parse_dummies(xml: &str) -> Vec<Dummy> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    let mut out = Vec::new();
    for e in doc.descendants().filter(|n| n.has_tag_name("Entity")) {
        let name = e.attribute("Name").unwrap_or("");
        if !name.starts_with("target_") {
            continue;
        }
        let Some(pos) = e.attribute("Pos").and_then(floats::<3>) else { continue };
        // CryEngine writes its quaternions as w, x, y, z
        let Some([w, x, y, z]) = e.attribute("Rotate").and_then(floats::<4>) else { continue };
        let Some(model) = e.descendants().find_map(|n| n.attribute("object_Model")) else { continue };
        out.push(Dummy { name: name.to_string(), model: model.to_string(), pos: Vec3::from(pos), rot: Quat::from_xyzw(x, y, z, w).normalize() });
    }
    out
}

fn load_field() -> Option<FieldData> {
    let t0 = Instant::now();
    let pak = aa_pak::Pak::global();
    let dir = aa_world::world::cell_dir(WORLD, CELL.0, CELL.1);
    let heights = CellHeights::parse(&pak.read(&format!("{dir}/client/terrain/heightmap.dat")).ok()?).ok()?;
    let cover = pak.read(&format!("{dir}/client/terrain/cover.ctc")).ok().and_then(|b| Cover::parse(&b).ok().map(|c| c.atlas(0)));
    let built = LayerLibrary::build(WORLD, [CELL].into_iter())?;
    let surfaces = CellSurfaces::new(Some(&built.library), CELL, &heights.surface, Vec3::ONE);
    let objects = load_cell_objects(WORLD, CELL.0, CELL.1);
    let groups = VegGroups::load(WORLD).unwrap_or_default();
    let dummies = pak.read_string(&format!("{dir}/client/entities.xml")).map(|x| parse_dummies(&x)).unwrap_or_default();
    info!(
        "character create: step 3 field loaded in {:.0} ms ({} brushes, {} vegetation, {} dummies)",
        t0.elapsed().as_secs_f32() * 1000.0,
        objects.brushes.len(),
        objects.vegetation.len(),
        dummies.len()
    );
    Some(FieldData { heights: Arc::new(heights), cover, surfaces, arrays: built.images, objects, groups, dummies })
}

/// The loaded field as assets.
struct Field {
    heights: Arc<CellHeights>,
    /// The terrain material (cover, layer arrays, surface map); the tint is set per race.
    material: TerrainMaterial,
    objects: CellObjects,
    groups: VegGroups,
    dummies: Vec<Dummy>,
}

/// What a model is, for the tweaks it gets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dummy,
    Brush,
    /// Grass and bushes (they get the tone curve of `grass.rs`).
    Grass,
    /// Flowers: coloured like the grass, without the curve.
    Flower,
}

/// One thing to spawn.
enum Item {
    Patch { px: usize, py: usize, lod: usize },
    Model { path: String, material: Option<String>, matrix: Mat4, shadows: bool, kind: Kind },
    /// A `ParticleEffect` entity of the cell (aa-fx).
    Effect(PlacedEffect),
}

/// The field of a race while it is up.
struct Shown {
    race: Race,
    root: Entity,
    items: Vec<Item>,
    next: usize,
    terrain: Handle<TerrainMaterial>,
    look: Look,
    /// The grass material made for each of the models' materials.
    grass: HashMap<AssetId<StandardMaterial>, Handle<GrassMaterial>>,
    /// The placed sounds of the field (aa-world `objects::sounds`, top level: they keep their own positions).
    sounds: Vec<Entity>,
}

/// What the race's mood does to the grass and rocks (the terrain's is set on its material).
#[derive(Clone)]
struct Look {
    grass: Vec3,
    brush: f32,
    curve: Vec4,
}

#[derive(Resource)]
pub struct Scenery {
    task: Option<Task<Option<FieldData>>>,
    failed: bool,
    field: Option<Field>,
    shown: Option<Shown>,
    models: Models,
    /// The race whose models are being loaded ahead (while steps 1 and 2 are shown) and how far.
    warm: Option<(Race, Vec<(String, Option<String>, Kind)>, usize)>,
}

/// The models of the field. All of them carry a material or vertex hook (dimmed grass and rocks, the
/// dummy without its shield and arm logs), which aa-cry does not share between loads: kept here.
#[derive(Default)]
struct Models {
    failed: HashSet<String>,
    checked: HashSet<usize>,
    loaded: HashMap<String, Arc<LoadedModel>>,
}

impl Models {
    /// The model of a field item, loaded on first use. `look` is the race's mood for the grass and the rocks.
    fn get(&mut self, cry: &mut CryAssets, path: &str, material: &Option<String>, kind: Kind, look: &Look) -> Option<Arc<LoadedModel>> {
        if self.failed.contains(path) {
            return None;
        }
        let key = format!("{path}|{material:?}");
        if let Some(m) = self.loaded.get(&key) {
            return Some(m.clone());
        }
        let options = ModelOptions {
            material: material.clone(),
            cry_space: true,
            material_hook: match kind {
                Kind::Dummy => Some(skill_prop_hook()),
                Kind::Grass | Kind::Flower => Some(tint_hook(look.grass)),
                Kind::Brush => Some(tint_hook(Vec3::splat(look.brush))),
            },
            vertex_hook: (kind == Kind::Dummy).then(without_arm_logs),
            ..default()
        };
        match cry.load_model(path, &options) {
            Ok(m) => {
                if self.checked.insert(Arc::as_ptr(&m) as usize) {
                    strip_cube_textures(cry, &m);
                }
                self.loaded.insert(key, m.clone());
                Some(m)
            }
            Err(e) => {
                debug!("character create: field model {path}: {e}");
                self.failed.insert(path.to_string());
                None
            }
        }
    }

    /// Forget the models that depend on the race's grass factor.
    fn forget_dimmed(&mut self) {
        self.loaded.retain(|k, _| k.contains("learning_ob01"));
    }
}

impl Scenery {
    /// Despawns the field's placed sounds (they are not under the stage root).
    pub fn despawn_sounds(&self, commands: &mut Commands) {
        for &e in self.shown.iter().flat_map(|s| &s.sounds) {
            commands.entity(e).despawn();
        }
    }

    /// Starts reading the field (call when the screen opens).
    pub fn start() -> Scenery {
        let task = std::env::var_os("AA_CREATE_NO_FIELD").is_none().then(|| AsyncComputeTaskPool::get().spawn(async { load_field() }));
        Scenery { task, failed: false, field: None, shown: None, models: Models::default(), warm: None }
    }
}

/// cry space (cell-local, as the capture's camera) to stage space.
fn to_stage() -> Mat4 {
    Mat4::from_quat(CRY_TO_BEVY) * Mat4::from_translation(-STAGE_ORIGIN)
}

fn layers() -> RenderLayers {
    RenderLayers::layer(STAGE_LAYER)
}

/// Brushes, vegetation and dummies near the camera, nearest first within each kind.
fn plan(field: &Field, race: Race) -> Vec<Item> {
    let cam = poses::pose(3, race, aa_anim::Gender::Male);
    let mut items = Vec::new();
    for py in 0..PATCHES {
        for px in 0..PATCHES {
            let size = PATCH_UNITS as f32 * 2.0;
            let centre = Vec3::new((px as f32 + 0.5) * size, (py as f32 + 0.5) * size, cam.pos.z);
            // the patch's nearest point: its centre minus half a patch, clamped
            let d = (centre - cam.pos).length() - size * 0.7;
            items.push(Item::Patch { px, py, lod: lod_for_distance(d.max(0.0), None).min(LOD_COUNT - 1) });
        }
    }
    let key = race_key(race);
    for d in field.dummies.iter().filter(|d| d.name.starts_with(&format!("target_{key}_"))) {
        // the captured dummies show their arms along the view: the model is a quarter turn off its entity axes
        let spin: f32 = std::env::var("AA_DUMMY_YAW").ok().and_then(|v| v.parse().ok()).unwrap_or(90.0);
        let rot = d.rot * Quat::from_rotation_z(spin.to_radians());
        items.push(Item::Model { path: d.model.clone(), material: None, matrix: Mat4::from_rotation_translation(rot, d.pos), shadows: true, kind: Kind::Dummy });
    }
    // the ground mist, leaves and the like of the race's field (only the effects the capture has switched on)
    for e in field.objects.effects.iter().filter(|_| std::env::var_os("AA_CREATE_NO_FX").is_none()) {
        let to = e.pos - cam.pos;
        if to.length() < EFFECT_RADIUS && to.dot(cam.dir) > -10.0 {
            items.push(Item::Effect(e.clone()));
        }
    }
    let o = &field.objects;
    let mut brushes: Vec<(f32, Item)> = Vec::new();
    for b in &o.brushes {
        let d = (b.translation() - cam.pos).length();
        let Some(model) = o.models.get(b.model as usize) else { continue };
        if d > OBJECT_RADIUS * 2.0 {
            continue;
        }
        let material = usize::try_from(b.material).ok().and_then(|i| o.materials.get(i)).filter(|m| !m.is_empty()).map(|m| format!("{m}.mtl"));
        brushes.push((d, Item::Model { path: model.clone(), material, matrix: b.mat4(), shadows: true, kind: Kind::Brush }));
    }
    brushes.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.extend(brushes.into_iter().map(|b| b.1));
    let mut veg: Vec<(f32, Item)> = Vec::new();
    for v in &o.vegetation {
        let d = (v.pos - cam.pos).length();
        // behind the camera is never seen
        if d > OBJECT_RADIUS || (v.pos - cam.pos).dot(cam.dir) < -6.0 {
            continue;
        }
        let Some(g) = field.groups.groups.get(&v.group) else { continue };
        let material = (!g.material.is_empty()).then(|| format!("{}.mtl", g.material));
        let kind = if g.model.contains("flower") { Kind::Flower } else { Kind::Grass };
        veg.push((d, Item::Model { path: g.model.clone(), material, matrix: v.mat4(), shadows: false, kind }));
    }
    veg.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.extend(veg.into_iter().map(|b| b.1));
    items
}

/// Multiplies the colour of a material by `k` (the field's lights are those of the figure, far brighter than the
/// captured ground).
fn tint_hook(k: Vec3) -> MaterialHook {
    MaterialHook::new(move |_, out| {
        let c = out.base_color.to_linear();
        out.base_color = Color::linear_rgba(c.red * k.x, c.green * k.y, c.blue * k.z, c.alpha);
    })
}

/// The dummy model carries a second part (arms and a painted shield, material `skill_df`) that the game
/// shows only while it demonstrates a skill: the captures show the bare straw doll.
fn skill_prop_hook() -> MaterialHook {
    MaterialHook::new(|m, out| {
        if m.name == "skill_df" {
            out.alpha_mode = AlphaMode::Mask(2.0);
        }
    })
}

/// The captures show the doll with its arms down or foreshortened, never the two logs that stick out in
/// the model's bind pose (the game poses the skeleton): the triangles of the logs (outside the shoulders)
/// are left out.
fn without_arm_logs() -> VertexHook {
    VertexHook::new(|mesh| {
        let keep = |tri: &[u32]| tri.iter().all(|&i| mesh.positions[i as usize].x.abs() < ARM_LOG_X || mesh.positions[i as usize].z < 0.9);
        let mut indices = Vec::with_capacity(mesh.indices.len());
        let mut subsets = mesh.subsets.clone();
        for s in &mut subsets {
            let first = indices.len() as u32;
            for tri in mesh.indices[s.first_index as usize..(s.first_index + s.index_count) as usize].chunks(3) {
                if keep(tri) {
                    indices.extend_from_slice(tri);
                }
            }
            s.first_index = first;
            s.index_count = indices.len() as u32 - first;
        }
        mesh.indices = indices;
        mesh.subsets = subsets;
    })
}

/// Drops cube map textures bound to 2D slots (a wgpu validation error otherwise); see aa-world's
/// `objects.rs`, which does the same for the world's props.
fn strip_cube_textures(cry: &mut CryAssets, loaded: &LoadedModel) {
    use bevy::render::render_resource::TextureViewDimension;
    let is_cube = |images: &Assets<Image>, h: &Option<Handle<Image>>| {
        h.as_ref()
            .and_then(|h| images.get(h))
            .and_then(|i| i.texture_view_descriptor.as_ref())
            .is_some_and(|d| d.dimension == Some(TextureViewDimension::Cube))
    };
    let mut bad = Vec::new();
    for part in loaded.parts.iter().flat_map(|(_, parts)| parts) {
        if let Some(m) = cry.materials.get(&part.material) {
            let slots = [&m.base_color_texture, &m.emissive_texture, &m.metallic_roughness_texture, &m.normal_map_texture, &m.occlusion_texture];
            if slots.into_iter().any(|h| is_cube(&cry.images, h)) {
                bad.push(part.material.clone());
            }
        }
    }
    for handle in bad {
        let Some(mut guard) = cry.materials.get_mut(&handle) else { continue };
        let m: &mut StandardMaterial = &mut guard;
        for slot in [&mut m.base_color_texture, &mut m.emissive_texture, &mut m.metallic_roughness_texture, &mut m.normal_map_texture, &mut m.occlusion_texture] {
            if is_cube(&cry.images, slot) {
                *slot = None;
            }
        }
    }
}

/// Moves the field in and out with step 3 and spawns it over a few frames.
pub fn update(
    mut commands: Commands,
    mut cry: CryAssets,
    screen: Option<Res<CreateScreen>>,
    scenery: Option<ResMut<Scenery>>,
    mut terrain_materials: ResMut<Assets<TerrainMaterial>>,
    mut grass_materials: ResMut<Assets<GrassMaterial>>,
    mut grade: ResMut<BackgroundGrade>,
) {
    let (Some(screen), Some(mut scenery)) = (screen, scenery) else { return };
    let scenery = &mut *scenery;

    // the files arrive from the task
    if let Some(task) = &mut scenery.task {
        if let Some(result) = check_ready(task) {
            scenery.task = None;
            match result {
                Some(data) => {
                    let cover = data.cover.map(|atlas| cry.images.add(cover_image(atlas)));
                    let arrays = data.arrays.map(|i| cry.images.add(i));
                    let surface = cry.images.add(data.surfaces.surface);
                    let material = TerrainMaterial {
                        base: StandardMaterial {
                            base_color: Color::WHITE,
                            base_color_texture: cover,
                            perceptual_roughness: 1.0,
                            reflectance: 0.05,
                            ..default()
                        },
                        extension: TerrainExt::new(data.surfaces.params, arrays, surface),
                    };
                    scenery.field = Some(Field { heights: data.heights, material, objects: data.objects, groups: data.groups, dummies: data.dummies });
                }
                None => {
                    warn!("character create: the step 3 field could not be loaded");
                    scenery.failed = true;
                }
            }
        }
    }

    let (race, step) = {
        let g = lock(&screen.shared);
        (g.cz.cat.race, g.step)
    };
    let want = (step == 3).then_some(race);
    // behind the field the painted sky is brighter than on the other steps (the engine's exposure, +0.92 EV
    // without a tone curve fits the sky of all four races to 3 / 255)
    let sky = if step == 3 { Some(SKY_GRADE) } else { None };
    if grade.0 != sky {
        grade.0 = sky;
    }

    // leave
    if let Some(shown) = &scenery.shown {
        if want != Some(shown.race) {
            commands.entity(shown.root).despawn();
            for &e in &shown.sounds {
                commands.entity(e).despawn();
            }
            commands.entity(screen.stage.camera).remove::<DistanceFog>();
            scenery.shown = None;
        }
    }

    // enter
    if let (None, Some(race), Some(field)) = (&scenery.shown, want, scenery.field.as_ref()) {
        let m = mood(race);
        let root = commands
            .spawn((Name::new("step 3 field"), Transform::default(), Visibility::default(), ChildOf(screen.stage.root)))
            .id();
        commands.entity(screen.stage.camera).insert(DistanceFog {
            color: m.fog,
            directional_light_color: Color::BLACK,
            directional_light_exponent: 30.0,
            falloff: FogFalloff::Linear { start: m.fog_start, end: m.fog_end },
        });
        let mut material = field.material.clone();
        material.extension.params.tint = Vec3::splat(m.ground).extend(DETAIL_DISTANCE);
        let terrain = terrain_materials.add(material);
        let look = Look { grass: m.grass, brush: m.brush, curve: m.curve };
        // the field's winds and birds: the cell's placed sounds in stage space (the cell's cry origin is -STAGE_ORIGIN
        // there, see `to_stage`), heard by the stage camera's listener
        let sounds = aa_world::objects::sounds::spawn_at(&mut commands, -STAGE_ORIGIN, &field.objects.sounds);
        scenery.shown = Some(Shown { race, root, items: plan(field, race), next: 0, terrain, look, grass: HashMap::new(), sounds });
        // models loaded ahead for another race are dimmed for that race
        if scenery.warm.as_ref().is_none_or(|w| w.0 != race) {
            scenery.models.forget_dimmed();
        }
    }

    // while steps 1 and 2 are shown: load the models of the race's field ahead of step 3
    if scenery.shown.is_none() {
        if let Some(field) = scenery.field.as_ref() {
            if scenery.warm.as_ref().is_none_or(|w| w.0 != race) {
                scenery.models.forget_dimmed();
                let mut unique: Vec<(String, Option<String>, Kind)> = Vec::new();
                for item in plan(field, race) {
                    if let Item::Model { path, material, kind, .. } = item {
                        if !unique.iter().any(|u| u.0 == path && u.1 == material) {
                            unique.push((path, material, kind));
                        }
                    }
                }
                scenery.warm = Some((race, unique, 0));
            }
            let m = mood(race);
            let look = Look { grass: m.grass, brush: m.brush, curve: m.curve };
            let start = Instant::now();
            if let Some((_, list, next)) = scenery.warm.as_mut() {
                while *next < list.len() && start.elapsed().as_secs_f32() * 1000.0 <= SPAWN_BUDGET_MS {
                    let (path, material, kind) = &list[*next];
                    *next += 1;
                    scenery.models.get(&mut cry, path, material, *kind, &look);
                }
            }
        }
        return;
    }

    // spawn
    let Some(field) = scenery.field.as_ref() else { return };
    let Some(shown) = scenery.shown.as_mut() else { return };
    let start = Instant::now();
    let frame = to_stage();
    // the terrain comes in at once (it is the first of the items), the props within the frame budget
    while shown.next < shown.items.len() && (matches!(shown.items[shown.next], Item::Patch { .. }) || start.elapsed().as_secs_f32() * 1000.0 <= SPAWN_BUDGET_MS) {
        let item = &shown.items[shown.next];
        shown.next += 1;
        match item {
            Item::Patch { px, py, lod } => {
                let mesh = cry.meshes.add(build_patch_mesh(&field.heights, *px, *py, *lod));
                let size = PATCH_UNITS as f32 * 2.0;
                let origin = Vec3::new(*px as f32 * size, *py as f32 * size, 0.0);
                commands.spawn((
                    Name::new("step 3 terrain"),
                    Mesh3d(mesh),
                    MeshMaterial3d(shown.terrain.clone()),
                    Transform::from_translation(frame.transform_point3(origin)),
                    layers(),
                    ChildOf(shown.root),
                ));
            }
            Item::Effect(e) => {
                let mut effect = Effect::new(e.effect.clone());
                effect.scale = e.scale;
                effect.speed_scale = e.speed_scale;
                effect.count_scale = e.count_scale;
                effect.prime = e.prime;
                if e.effect.contains("fog") {
                    let mist = mist(shown.race);
                    if effect.name.contains("flow_fog") {
                        effect.alpha_scale = mist.alpha;
                        effect.count_scale *= mist.count;
                    }
                    effect.tint = mist.light;
                }
                commands.spawn((
                    Name::new("step 3 effect"),
                    effect,
                    Transform {
                        translation: frame.transform_point3(e.pos),
                        rotation: CRY_TO_BEVY * e.rot * CRY_TO_BEVY.inverse(),
                        scale: Vec3::ONE,
                    },
                    layers(),
                    ChildOf(shown.root),
                ));
            }
            Item::Model { path, material, matrix, shadows, kind } => {
                let Some(loaded) = scenery.models.get(&mut cry, path, material, *kind, &shown.look) else { continue };
                for (node, parts) in &loaded.parts {
                    let full = frame * *matrix * Mat4::from(render_transform(&loaded.model, *node));
                    if !full.is_finite() || full.determinant().abs() < 1e-9 {
                        continue;
                    }
                    for part in parts {
                        let mut e = if *kind == Kind::Grass {
                            let material = shown
                                .grass
                                .entry(part.material.id())
                                .or_insert_with(|| {
                                    let base = cry.materials.get(&part.material).cloned().unwrap_or_default();
                                    grass_materials.add(GrassMaterial { base, extension: GrassExt { gamma: shown.look.curve.x, reference: shown.look.curve.y, floor: shown.look.curve.z, saturation: shown.look.curve.w } })
                                })
                                .clone();
                            commands.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(material), Transform::from_matrix(full), part.aabb, layers(), ChildOf(shown.root)))
                        } else {
                            commands.spawn((
                                Mesh3d(part.mesh.clone()),
                                MeshMaterial3d(part.material.clone()),
                                Transform::from_matrix(full),
                                part.aabb,
                                layers(),
                                ChildOf(shown.root),
                            ))
                        };
                        if !*shadows {
                            e.insert(NotShadowCaster);
                        }
                    }
                }
            }
        }
    }
}
