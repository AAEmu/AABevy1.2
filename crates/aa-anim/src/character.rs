//! Character assembly: a [`CharacterDesc`] (race, gender, face, hair, skin, costume,
//! customisation) is resolved against the game database and the pak into [`CharacterAssets`]
//! (plain data: which files), then spawned as an animated entity hierarchy.
//!
//! ```text
//! Character (Transform = placement, Animator, Character)
//! └─ model root (CRY_TO_BEVY: CryEngine Z up -> Bevy Y up; characters face CryEngine +Y = Bevy -Z)
//!    ├─ Bip01 ... (one entity per bone of the *_base.chr skeleton, animated)
//!    ├─ body        (nude .chr, skinned to the bones; dropped under a one-piece `cosplay` costume)
//!    ├─ face        (face00.cdf: shared head mesh + race material + morphs + composited texture)
//!    ├─ hair        (hair .chr with the .mtl of the chosen colour)
//!    └─ costume pieces (the login-stage cloth pack of the model: cosplay skin, or shirt / pants / gloves / shoes)
//! ```
//!
//! Per-character material work (copies of the cached aa-cry materials, see `restyle`):
//! skin albedo from the `skin_colors` row, composited face texture + normal map + iris colour,
//! hair colour, dyed costume diffuse textures. Skinned parts use the base skeleton's inverse bind
//! matrices so a part made for another race (the Nuian head on Harani bodies) bends correctly.

use std::sync::Arc;

use aa_cry::material::Material as CryMaterial;
use aa_cry::scene::{CryAssets, LoadedModel, MaterialHook, ModelOptions, SkeletonBinding, VertexHook, spawn_loaded};
use aa_cry::shade::skin_base_color;
use aa_cry::{CRY_TO_BEVY, Shading, Skeleton};
use std::collections::HashMap;

use bevy::prelude::*;

use crate::animator::{Animator, ClipLibrary, RandomIdle, spawn_bones};
use crate::cal::Cal;
use crate::cdf::{Cdf, norm};
use crate::custom::Customization;
use crate::cut::{Hull, cut_hulls, cut_mesh};
use crate::db::{GameDb, Gender, HairColor, Race, SkinColor, model_id};
use crate::face::{EyeTint, FaceLayer, abgr, compose_face};
use crate::morph::{FaceMaker, MorphSet};
use crate::pose::{BoundClip, Rig, SampleOptions};
use crate::{Error, Result};

/// What a character wears.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Costume {
    /// Nothing but the nude body.
    None,
    /// The outfit the login / character-create stage dresses the model in
    /// (`characters.preview_cloth_pack_id`, see [`GameDb::login_costume`]).
    #[default]
    Login,
    /// Equipment items (`items.id` of armour); resolved through `item_armors`.
    Items(Vec<u32>),
    /// Explicit mesh paths (`.chr` / `.cdf`).
    Meshes(Vec<String>),
}

/// Everything that makes one character look the way it does.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterDesc {
    pub race: Race,
    pub gender: Gender,
    /// `item_body_parts.item_id` of a fixed face; `None` = the customisable base face.
    pub face: Option<u32>,
    /// `item_body_parts.item_id` of the hair style; `None` = bald.
    pub hair: Option<u32>,
    /// `hair_colors.id`; `None` = first colour of the style.
    pub hair_color: Option<u32>,
    /// `skin_colors.id`
    pub skin: u32,
    pub costume: Costume,
    pub customization: Customization,
}

impl CharacterDesc {
    /// The character-create preset `index` (0-8, `<race>_face000N`) of a race and gender.
    pub fn from_preset(db: &GameDb, race: Race, gender: Gender, index: usize) -> Result<CharacterDesc> {
        let presets = db.presets(model_id(race, gender), 1)?;
        let p = presets
            .get(index)
            .ok_or_else(|| Error::Missing(format!("preset {index} of {race:?} {gender:?}")))?;
        Ok(CharacterDesc {
            race,
            gender,
            face: None,
            hair: (p.hair_item != 0).then_some(p.hair_item),
            hair_color: (p.hair_color_id != 0).then_some(p.hair_color_id),
            skin: p.skin_color_id,
            costume: Costume::Login,
            customization: Customization::from_preset(p),
        })
    }

    /// Face decals, lip colour and face normal map of the customisation.
    fn face_customization(&self, db: &GameDb, dir: &str, prefix: &str) -> (Vec<FaceLayer>, Option<(String, [u8; 3])>, Option<(String, f32)>) {
        let c = &self.customization;
        let face_dir = dir.replace("/nude", "/face");
        let mut layers: Vec<(u32, FaceLayer)> = Vec::new();
        for d in &c.decals {
            let Ok(asset) = db.face_decal(d.asset_id) else { continue };
            let tint = match asset.category {
                4 if c.eyebrow_color != 0 => Some(abgr(c.eyebrow_color)),
                5 if c.deco_color != 0 => Some(abgr(c.deco_color)),
                _ => None,
            };
            let (mx, my) = if asset.movable { (d.move_x, d.move_y) } else { (0, 0) };
            layers.push((
                asset.category,
                FaceLayer {
                    path: asset.path,
                    x: (asset.x + mx) as f32,
                    y: (asset.y + my) as f32,
                    scale: if d.scale > 0.0 { d.scale } else { 1.0 },
                    rotate: d.rotate,
                    weight: if d.weight > 0.0 { d.weight } else { 1.0 },
                    tint,
                },
            ));
        }
        // drawing order: tattoo, makeup, scar, eyebrow, deco
        let rank = |cat: u32| match cat {
            2 => 0,
            3 => 1,
            1 => 2,
            4 => 3,
            _ => 4,
        };
        layers.sort_by_key(|(cat, _)| rank(*cat));
        let lips = (c.lip_color != 0).then(|| (format!("{face_dir}/{prefix}_face_mask_new.dds"), abgr(c.lip_color)));
        let normal = (c.normal_map != 0)
            .then(|| db.face_normal_map(c.normal_map).ok())
            .flatten()
            .filter(|(n, _)| !n.is_empty())
            .map(|(n, _)| (n, c.normal_map_weight));
        (layers.into_iter().map(|(_, l)| l).collect(), lips, normal)
    }

    /// The default look of a race and gender: the first create-screen preset in the login costume.
    pub fn default_for(db: &GameDb, race: Race, gender: Gender) -> Result<CharacterDesc> {
        CharacterDesc::from_preset(db, race, gender, 0)
    }

    pub fn model_id(&self) -> u32 {
        model_id(self.race, self.gender)
    }
}

/// Role of a mesh in the character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Body,
    Face,
    Hair,
    Costume,
}

/// One skinned mesh file of the character.
#[derive(Clone, Debug, PartialEq)]
pub struct PartAsset {
    pub kind: PartKind,
    pub slot: String,
    /// `.chr` path
    pub path: String,
    /// `.mtl` overriding the model's own.
    pub material: Option<String>,
    /// Dye colour (sRGB) for dyeable costume materials.
    pub dye: Option<[u8; 3]>,
}

/// A [`CharacterDesc`] resolved to files.
#[derive(Clone, Debug)]
pub struct CharacterAssets {
    pub desc: CharacterDesc,
    pub model_id: u32,
    /// Skeleton `.chr` (`*_base.chr`).
    pub skeleton: String,
    /// Animation name table (`*_base.cal`).
    pub cal: String,
    pub parts: Vec<PartAsset>,
    pub skin: SkinColor,
    /// Linear albedo multiplier of the skin materials derived from `skin` (see [`skin_albedo`]).
    pub skin_albedo: [f32; 3],
    pub hair_color: Option<HairColor>,
    /// Decals composited on the face texture (see [`crate::face`]), in drawing order.
    pub face_layers: Vec<FaceLayer>,
    /// Lip mask texture and colour.
    pub face_lips: Option<(String, [u8; 3])>,
    /// `face_normal_maps` normal texture and weight.
    pub face_normal: Option<(String, f32)>,
    /// Iris colour (`left_pupil_color`).
    pub pupil_color: Option<[u8; 3]>,
}

impl CharacterDesc {
    /// Looks up every file this character needs. Reads the database and a few small pak files.
    pub fn resolve(&self, db: &GameDb) -> Result<CharacterAssets> {
        let model = self.model_id();
        let (r, g) = (self.race, self.gender);
        let dir = format!("objects/characters/{}/{}/nude", r.folder(), g.folder());
        let prefix = format!("{}_{}", r.prefix(), g.letter());

        let base_cdf = Cdf::load(&format!("{dir}/{prefix}_base.cdf"))?;
        let skeleton = base_cdf.model.clone();
        let cal = format!("{}.cal", skeleton.trim_end_matches(".chr"));
        let body_path = base_cdf
            .attachment("body")
            .map(|a| a.binding.clone())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| format!("{dir}/{prefix}_nude.chr"));

        let skin = db.skin_color(self.skin)?;
        let mut parts = Vec::new();

        // body (+ fur / skin variant materials)
        let mut body = PartAsset { kind: PartKind::Body, slot: "body".into(), path: body_path.clone(), material: None, dye: None };
        body.material = skin_variant(&body_path, &skin);
        parts.push(body);

        // face
        let face_part = match self.face {
            Some(item) => db.body_part(item)?,
            None => db.base_face(model)?,
        };
        let mut face = resolve_part(PartKind::Face, "face", &face_part.path)?;
        if let Some(m) = &face.material.clone().or_else(|| default_mtl(&face.path)) {
            face.material = skin_variant(m, &skin).or(Some(m.clone()));
        }
        parts.push(face);

        // hair
        let mut hair_color = None;
        if let Some(item) = self.hair {
            let hair_part = db.body_part(item)?;
            let mut hair = resolve_part(PartKind::Hair, "hair", &hair_part.path)?;
            let colors = db.hair_colors_for(model, hair_part.asset_ids[0])?;
            let hc = match self.hair_color {
                Some(id) => db.hair_color(id).ok(),
                None => colors.first().cloned(),
            };
            if let Some(hc) = &hc {
                if aa_pak::Pak::global().exists(&hc.material) {
                    hair.material = Some(hc.material.clone());
                }
            }
            hair_color = hc;
            parts.push(hair);
        }

        // costume
        let costume_paths: Vec<(String, String, Option<[u8; 3]>)> = match &self.costume {
            Costume::None => Vec::new(),
            Costume::Login => db.login_costume(model)?.into_iter().map(|p| (p.slot, p.path, p.dye)).collect(),
            Costume::Items(items) => {
                let mut v = Vec::new();
                for &i in items {
                    let dye = db.default_dye(i);
                    v.extend(db.item_meshes(i, model)?.into_iter().map(|p| ("item".to_string(), p, dye)));
                }
                v
            }
            Costume::Meshes(m) => m.iter().map(|p| ("mesh".to_string(), norm(p), None)).collect(),
        };
        // a one-piece `cosplay` skin carries its own skin geometry; the nude body would poke through it
        if costume_paths.iter().any(|(slot, _, _)| slot == "cosplay") {
            parts.retain(|p| p.kind != PartKind::Body);
        }
        for (slot, path, dye) in costume_paths {
            let mut part = resolve_part(PartKind::Costume, &slot, &path)?;
            part.dye = dye;
            parts.push(part);
        }

        let skin_albedo = skin_albedo(&skin, &db.skin_colors(model).unwrap_or_default());
        let (face_layers, face_lips, face_normal) = self.face_customization(db, &dir, &prefix);
        let pupil_color = (self.customization.left_pupil_color != 0).then(|| abgr(self.customization.left_pupil_color));
        Ok(CharacterAssets { desc: self.clone(), model_id: model, skeleton, cal, parts, skin, skin_albedo, hair_color, face_layers, face_lips, face_normal, pupil_color })
    }
}

/// `.cdf` parts are unwrapped to their `.chr` + material; `.chr` parts pass through.
fn resolve_part(kind: PartKind, slot: &str, path: &str) -> Result<PartAsset> {
    if path.ends_with(".cdf") {
        let cdf = Cdf::load(path)?;
        Ok(PartAsset { kind, slot: slot.to_string(), path: cdf.model, material: cdf.material, dye: None })
    } else {
        Ok(PartAsset { kind, slot: slot.to_string(), path: path.to_string(), material: None, dye: None })
    }
}

/// `x/y/foo.chr` -> `x/y/foo.mtl` when that exists.
fn default_mtl(chr: &str) -> Option<String> {
    let m = format!("{}.mtl", chr.rsplit_once('.')?.0);
    aa_pak::Pak::global().exists(&m).then_some(m)
}

/// Ferre skins select a material file by postfix (`fe_f_nude.mtl` -> `fe_f_nude_brop01.mtl`).
fn skin_variant(mtl_or_chr: &str, skin: &SkinColor) -> Option<String> {
    if skin.material_postfix.is_empty() {
        return None;
    }
    let (stem, _) = mtl_or_chr.rsplit_once('.')?;
    // `fe_m_nude.mtl` -> `fe_m_nude_brop01.mtl`; `fe_m_face00.mtl` / `fe_m_face01.chr` -> `fe_m_face_brop01.mtl`
    let bare = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let pak = aa_pak::Pak::global();
    [stem, bare]
        .into_iter()
        .map(|s| format!("{s}_{}.mtl", skin.material_postfix))
        .find(|c| pak.exists(c))
}

/// Marks the root entity of a spawned character.
#[derive(Component, Clone, Debug)]
pub struct Character {
    pub assets: Arc<CharacterAssets>,
    /// The entity holding bones and parts (rotated into Bevy space).
    pub model_root: Entity,
    /// Spawned part roots, in [`CharacterAssets::parts`] order.
    pub parts: Vec<Entity>,
}

/// Names of the animations the login stage uses.
pub const STAGE_IDLE: &str = "fist_ac_stage_idle";
pub const STAGE_VARIANT: &str = "fist_ac_stage_rand_1";

/// Spawns an animated character playing its login-stage idle (with the occasional variant).
pub fn spawn_character(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    assets: CharacterAssets,
    transform: Transform,
) -> Result<Entity> {
    let skeleton = Arc::new(Skeleton::load(&assets.skeleton)?);
    let rig = Arc::new(Rig::new(skeleton.clone()));

    let root = commands.spawn((Name::new(format!("{:?} {:?}", assets.desc.race, assets.desc.gender)), transform, Visibility::default())).id();
    let model_root = commands
        .spawn((Name::new("model"), Transform::from_rotation(CRY_TO_BEVY), Visibility::default(), ChildOf(root)))
        .id();
    let bones = spawn_bones(commands, model_root, &rig);

    // costumes hide the nude body triangles inside their `Cut` hulls
    let hulls = Arc::new(costume_hulls(cry, &assets));

    let mut part_entities = Vec::new();
    for part in &assets.parts {
        let options = ModelOptions {
            material: part.material.clone(),
            skeleton: Some(SkeletonBinding { skeleton: skeleton.clone(), joints: bones.clone() }),
            cry_space: true,
            shading: Shading::Full,
            ..default()
        };
        let loaded = match load_part(cry, &assets, part, &options, &hulls) {
            Ok(l) => l,
            Err(e) => {
                warn!("aa-anim: {}: {e}", part.path);
                continue;
            }
        };
        let e = spawn_loaded(commands, &part.path, &loaded, &options);
        commands.entity(e).insert(ChildOf(model_root));
        part_entities.push(e);
    }

    // animation
    let mut animator = Animator::new(rig.clone(), bones);
    match start_idle(&mut animator, clips, &assets) {
        Ok(()) => {}
        Err(e) => warn!("aa-anim: idle animation for {:?} {:?}: {e}", assets.desc.race, assets.desc.gender),
    }

    commands.entity(root).insert((
        animator,
        Character { assets: Arc::new(assets), model_root, parts: part_entities },
    ));
    Ok(root)
}

fn start_idle(animator: &mut Animator, clips: &ClipLibrary, assets: &CharacterAssets) -> Result<()> {
    let cal = Cal::load(&assets.cal)?;
    let rig = animator.rig.clone();
    let bind = |name: &str| -> Result<BoundClip> {
        let entry = cal.animation(name).ok_or_else(|| Error::Missing(format!("animation {name} in {}", assets.cal)))?;
        let opts = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
        clips.bound(&rig, &entry.path, opts)
    };
    let idle = bind(STAGE_IDLE)?;
    animator.play(idle, true, 0.0);
    if let Ok(v) = bind(STAGE_VARIANT) {
        let seed = assets.model_id * 7919 + assets.desc.skin;
        animator.random_idle = Some(RandomIdle::new(vec![v], 2, seed));
    }
    Ok(())
}

/// Loads a part's Bevy assets with the character's look applied: face morphs on the vertices and
/// skin / eye / costume material tuning, done by the model hooks of aa-cry (the converted
/// materials of the `.mtl` stay untouched in the shared cache; parts without a per-character
/// change are shared between characters).
fn load_part(
    cry: &mut CryAssets,
    assets: &CharacterAssets,
    part: &PartAsset,
    options: &ModelOptions,
    hulls: &Arc<Vec<Hull>>,
) -> Result<Arc<LoadedModel>> {
    // the plain model (cached): material path and mesh data for the face texture
    let plain = cry.load_model(&part.path, options)?;
    let mtl = plain.material_path.as_deref().and_then(|p| cry.mtl(p));
    let face = if part.kind == PartKind::Face { face_look(cry, assets, mtl.as_deref(), &plain) } else { FaceLook::default() };
    let dyes = dyed_diffuses(cry, mtl.as_deref(), part);
    let morph = if part.kind == PartKind::Face { face_morph(assets, part) } else { None };
    let cut = (part.kind == PartKind::Body && !hulls.is_empty()).then(|| hulls.clone());

    let skin = part.kind != PartKind::Costume || mtl.as_deref().is_some_and(|m| m.sub_materials.iter().any(|s| s.shader.eq_ignore_ascii_case("humanskin")));
    let look = Arc::new(Look { kind: part.kind, skin: assets.skin.clone(), face, dyes });
    let needs_materials = skin || !look.dyes.is_empty();
    if !needs_materials && morph.is_none() && cut.is_none() {
        return Ok(plain);
    }
    let mut hooked = options.clone();
    if needs_materials {
        let look = look.clone();
        hooked.material_hook = Some(MaterialHook::new(move |sub, m| restyle(sub, m, &look)));
    }
    if morph.is_some() || cut.is_some() {
        hooked.vertex_hook = Some(VertexHook::new(move |mesh| {
            if let Some((set, weights)) = &morph {
                set.apply(&mut mesh.positions, weights);
            }
            if let Some(hulls) = &cut {
                cut_mesh(mesh, hulls);
            }
        }));
    }
    Ok(cry.load_model(&part.path, &hooked)?)
}

/// The `Cut` hulls of the costume parts of a character (see [`crate::cut`]).
fn costume_hulls(cry: &mut CryAssets, assets: &CharacterAssets) -> Vec<Hull> {
    let mut hulls = Vec::new();
    for part in assets.parts.iter().filter(|p| p.kind == PartKind::Costume) {
        let Ok(model) = aa_cry::Model::load(&part.path) else { continue };
        let mtl_path = part.material.clone().or_else(|| aa_cry::scene::find_mtl(&part.path, &model));
        let Some(mtl) = mtl_path.and_then(|p| cry.mtl(&p)) else { continue };
        hulls.extend(cut_hulls(&model, &mtl));
    }
    hulls
}

/// The per-character textures of the customisable face.
#[derive(Default)]
struct FaceLook {
    /// Pak path (lower case) of the material texture the composed texture replaces: only
    /// sub-materials using this file get it (ferre heads have a separate fur sub-material).
    replaces: String,
    /// Base skin texture with decals and lip colour composited ([`crate::face`]).
    texture: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
}

/// UV bounding box `[u0, v0, u1, v1]` of the geometry drawn with the `Eye` shader.
fn eye_uv_box(loaded: &LoadedModel, mtl: &CryMaterial) -> Option<[f32; 4]> {
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut found = false;
    for mesh in &loaded.model.meshes {
        for subset in &mesh.subsets {
            if !mtl.sub(subset.material_id as usize).is_some_and(|s| s.shader.eq_ignore_ascii_case("eye")) {
                continue;
            }
            let first = subset.first_index as usize;
            for &i in mesh.indices.get(first..first + subset.index_count as usize)? {
                if let Some(uv) = mesh.uvs.get(i as usize) {
                    bounds = [bounds[0].min(uv.x), bounds[1].min(uv.y), bounds[2].max(uv.x), bounds[3].max(uv.y)];
                    found = true;
                }
            }
        }
    }
    found.then_some(bounds)
}

/// Does the sub-material's diffuse texture match `path` (lower-case pak path)?
fn uses_texture(sub: &CryMaterial, path: &str) -> bool {
    use aa_cry::material::{TextureMap, texture_pak_path};
    !path.is_empty() && sub.texture(&TextureMap::Diffuse).is_some_and(|t| texture_pak_path(&t.file).to_ascii_lowercase() == path)
}

/// Builds the face texture and normal map of the character for a face material.
fn face_look(cry: &mut CryAssets, assets: &CharacterAssets, mtl: Option<&CryMaterial>, loaded: &LoadedModel) -> FaceLook {
    use aa_cry::material::{TextureMap, texture_pak_path};
    let mut look = FaceLook::default();
    let Some(skin) = mtl.and_then(|m| m.sub_materials.iter().find(|s| s.shader.eq_ignore_ascii_case("humanskin"))) else { return look };
    let Some(diffuse) = skin.texture(&TextureMap::Diffuse) else { return look };
    // `face00_df` (named by the material; `... copy.dds` does not exist) is a placeholder: the
    // customisable base is the `face01` texture of the same race
    let named = texture_pak_path(&diffuse.file);
    let pak = aa_pak::Pak::global();
    let base = [named.replace("face00", "face01"), named.replace(" copy", "")].into_iter().find(|p| pak.exists(p));
    let Some(base) = base else { return look };
    look.replaces = named.to_ascii_lowercase();
    if let Some((path, _)) = &assets.face_normal {
        look.normal = cry.texture(path, false);
    }
    let eye = assets.pupil_color.and_then(|color| eye_uv_box(loaded, mtl?).map(|uv| EyeTint { uv, color }));
    if assets.face_layers.is_empty() && assets.face_lips.is_none() && eye.is_none() && base == named {
        return look;
    }
    let lips = assets.face_lips.as_ref().map(|(p, c)| (p.as_str(), *c));
    match compose_face(&base, &assets.face_layers, lips, eye) {
        Ok(img) => look.texture = Some(cry.images.add(img)),
        Err(e) => warn!("aa-anim: face texture {base}: {e}"),
    }
    look
}

/// Morph targets of the customisable head and the weights of this character, if any.
fn face_morph(assets: &CharacterAssets, part: &PartAsset) -> Option<(MorphSet, Vec<(String, f32)>)> {
    let c = &assets.desc.customization;
    let set = MorphSet::load(&part.path).ok().flatten()?;
    let d = &assets.desc;
    let targets = format!("objects/characters/{}/{}/face/{}_{}_targets.xml", d.race.folder(), d.gender.folder(), d.race.prefix(), d.gender.letter());
    let maker = FaceMaker::load(&targets).ok()?;
    let mut weights = FaceMaker::race_gender_weights(d.race, d.gender);
    weights.extend(maker.weights(&c.modifier));
    Some((set, weights))
}

/// What differs per character in the materials of one part.
struct Look {
    kind: PartKind,
    /// The chosen `skin_colors` row: it feeds the `HumanSkin` shader parameters.
    skin: SkinColor,
    face: FaceLook,
    /// Dyed copies of dyeable diffuse textures, by pak path of the original.
    dyes: HashMap<String, Handle<Image>>,
}

/// Per-shader tuning of one converted sub-material for the character: aa-cry's conversion
/// already follows the shaders (skin tone from `BSkinColor`, hair colour from `HairColor`, eye,
/// Illum specular); only what the customisation changes is applied here.
fn restyle(sub: &CryMaterial, m: &mut StandardMaterial, look: &Look) {
    let shader = sub.shader.to_ascii_lowercase();
    match shader.as_str() {
        "humanskin" => {
            if look.kind == PartKind::Face && uses_texture(sub, &look.face.replaces) {
                if let Some(t) = &look.face.texture {
                    m.base_color_texture = Some(t.clone());
                }
                if let Some(n) = &look.face.normal {
                    m.normal_map_texture = Some(n.clone());
                }
            }
            // the character's skin colour: the row's bright colour (stored b, g, r like the
            // material's `BSkinColor`) and diffuse colour replace the file's own (the first skin)
            let row = &look.skin;
            let bright = Vec3::new(row.bright[2] as f32, row.bright[1] as f32, row.bright[0] as f32) / 255.0;
            let diffuse = Vec3::new(row.diffuse[0] as f32, row.diffuse[1] as f32, row.diffuse[2] as f32) / 255.0;
            m.base_color = skin_base_color(diffuse, bright);
        }
        "eye" => {
            if let Some(t) = look.face.texture.as_ref().filter(|_| uses_texture(sub, &look.face.replaces)) {
                // the iris lives in the face texture atlas
                m.base_color_texture = Some(t.clone());
            }
        }
        "illum" => {
            if let Some(image) = sub.texture(&aa_cry::material::TextureMap::Diffuse).and_then(|t| look.dyes.get(&t.pak_path())) {
                m.base_color_texture = Some(image.clone());
                if m.emissive_texture.is_some() {
                    m.emissive_texture = Some(image.clone());
                }
            }
        }
        _ => {}
    }
}

/// Albedo multiplier of the HumanSkin materials for a `skin_colors` row. The body / face
/// textures are the same for every skin of a race; the row's `diffuse_color` (a few dozen of 255,
/// per model) sets how dark the skin is and its hue (relative to the model's brightest skin), e.g.
/// Nuian male skin 1 `(22,22,16)` vs skin 8000002 `(3,4,4)`. `all` are the selectable skins of the
/// model (empty = no normalisation). Calibrated against the real character-create screens.
pub fn skin_albedo(skin: &SkinColor, all: &[SkinColor]) -> [f32; 3] {
    let luma = |c: [u8; 3]| c[0] as f32 * 0.3 + c[1] as f32 * 0.59 + c[2] as f32 * 0.11;
    let brightest = all.iter().map(|s| luma(s.diffuse)).fold(luma(skin.diffuse), f32::max).max(1.0);
    let l = luma(skin.diffuse).max(1.0);
    let level = 0.9 * (l / brightest).powf(0.7);
    // hue of the diffuse colour, damped
    let hue = skin.diffuse.map(|c| 1.0 + (c as f32 / l - 1.0).clamp(-0.5, 0.5) * 0.3);
    hue.map(|h| (level * h).clamp(0.0, 1.0))
}

/// The dyed copies of the dyeable diffuse textures of a costume part (see [`crate::dye`]), by the
/// pak path of the original texture.
fn dyed_diffuses(cry: &mut CryAssets, mtl: Option<&CryMaterial>, part: &PartAsset) -> HashMap<String, Handle<Image>> {
    use aa_cry::material::{TextureMap, texture_pak_path};
    let mut out = HashMap::new();
    let Some(mtl) = mtl else { return out };
    for sub in &mtl.sub_materials {
        let Some(channel) = sub.public_param("DyeingChannelMask").map(crate::dye::mask_channel) else { continue };
        let (Some(mask), Some(diffuse)) = (sub.texture(&TextureMap::Custom1), sub.texture(&TextureMap::Diffuse)) else { continue };
        let Some(dye) = part.dye.or_else(|| {
            sub.public_param("DyeingColor").and_then(parse_rgb).map(|c| [c[0], c[1], c[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
        }) else {
            continue;
        };
        match crate::dye::bake_dyed_diffuse(&texture_pak_path(&diffuse.file), &texture_pak_path(&mask.file), channel, dye) {
            Ok(img) => {
                out.insert(diffuse.pak_path(), cry.images.add(img));
            }
            Err(e) => warn!("aa-anim: dyeing {}: {e}", diffuse.file),
        }
    }
    out
}

fn parse_rgb(s: &str) -> Option<[f32; 3]> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>());
    Some([it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?])
}
