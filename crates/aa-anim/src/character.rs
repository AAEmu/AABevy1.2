//! Character assembly: a [`CharacterDesc`] (race, gender, face, hair, skin, costume,
//! customisation) is resolved against the game database and the pak into [`CharacterAssets`]
//! (plain data: which files), then spawned as an animated entity hierarchy.
//!
//! ```text
//! Character (Transform = placement, Animator, Character)
//! └─ model root (CRY_TO_BEVY: CryEngine Z up -> Bevy Y up)
//!    ├─ Bip01 ... (one entity per bone of the *_base.chr skeleton, animated)
//!    ├─ body        (nude .chr, skinned to the bones)
//!    ├─ face        (face00.cdf: shared head mesh + race material + morphs)
//!    ├─ hair        (hair .chr with the .mtl of the chosen colour)
//!    └─ costume pieces (chest, legs, hands, feet)
//! ```

use std::sync::Arc;

use aa_cry::material::Material as CryMaterial;
use aa_cry::mesh::MeshData;
use aa_cry::scene::{CryAssets, LoadedModel, ModelOptions, Part, SkeletonBinding, spawn_loaded, subset_mesh};
use aa_cry::{CRY_TO_BEVY, Skeleton};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;

use crate::animator::{Animator, ClipLibrary, RandomIdle, spawn_bones};
use crate::cal::Cal;
use crate::cdf::{Cdf, norm};
use crate::custom::Customization;
use crate::db::{GameDb, Gender, HairColor, Race, SkinColor, model_id};
use crate::morph::{FaceMaker, MorphSet};
use crate::pose::{BoundClip, Rig, SampleOptions};
use crate::{Error, Result};

/// What a character wears.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Costume {
    /// Nothing but the nude body.
    None,
    /// The login-stage outfit (chest, legs, hands, feet of `armor_assets` 1111..1114).
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
    pub hair_color: Option<HairColor>,
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
        let mut body = PartAsset { kind: PartKind::Body, slot: "body".into(), path: body_path.clone(), material: None };
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
        let costume_paths: Vec<(String, String)> = match &self.costume {
            Costume::None => Vec::new(),
            Costume::Login => db.login_costume(model)?.into_iter().map(|p| (p.slot.to_string(), p.path)).collect(),
            Costume::Items(items) => {
                let mut v = Vec::new();
                for &i in items {
                    v.extend(db.item_meshes(i, model)?.into_iter().map(|p| ("item".to_string(), p)));
                }
                v
            }
            Costume::Meshes(m) => m.iter().map(|p| ("mesh".to_string(), norm(p))).collect(),
        };
        for (slot, path) in costume_paths {
            parts.push(resolve_part(PartKind::Costume, &slot, &path)?);
        }

        Ok(CharacterAssets { desc: self.clone(), model_id: model, skeleton, cal, parts, skin, hair_color })
    }
}

/// `.cdf` parts are unwrapped to their `.chr` + material; `.chr` parts pass through.
fn resolve_part(kind: PartKind, slot: &str, path: &str) -> Result<PartAsset> {
    if path.ends_with(".cdf") {
        let cdf = Cdf::load(path)?;
        Ok(PartAsset { kind, slot: slot.to_string(), path: cdf.model, material: cdf.material })
    } else {
        Ok(PartAsset { kind, slot: slot.to_string(), path: path.to_string(), material: None })
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
    let candidate = format!("{stem}_{}.mtl", skin.material_postfix);
    aa_pak::Pak::global().exists(&candidate).then_some(candidate)
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

    let mut part_entities = Vec::new();
    for part in &assets.parts {
        let options = ModelOptions {
            material: part.material.clone(),
            skeleton: Some(SkeletonBinding { skeleton: skeleton.clone(), joints: bones.clone() }),
            cry_space: true,
            ..default()
        };
        let loaded = match load_part(cry, &assets, &skeleton, part, &options) {
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
/// skin / hair / eye / costume material tuning (materials are copied, the shared cache entries
/// stay untouched).
fn load_part(
    cry: &mut CryAssets,
    assets: &CharacterAssets,
    base: &Skeleton,
    part: &PartAsset,
    options: &ModelOptions,
) -> Result<Arc<LoadedModel>> {
    let loaded = cry.load_model(&part.path, options)?;
    let morph = if part.kind == PartKind::Face { face_morph(assets, part) } else { None };
    let mtl_path = loaded.material_path.clone();
    let mtl = mtl_path.as_deref().and_then(|p| cry.mtl(p));

    let mut out = (*loaded).clone();
    out.parts.clear();
    // Skin vertices are expressed in the *character's* bind pose, not the part's: use the base
    // skeleton's inverse bind matrices for the joints the part maps to (identical for parts made
    // for the same race; the Nuian head on Harani bodies relies on it plus `hariharan_Morph`).
    if let Some(part_skeleton) = &loaded.skeleton {
        let inverse: Vec<Mat4> = part_skeleton
            .bones
            .iter()
            .zip(part_skeleton.map_to(base))
            .map(|(b, m)| m.map(|j| base.bones[j].inverse_bind()).unwrap_or_else(|| b.inverse_bind()))
            .collect();
        out.inverse_bindposes = Some(cry.bindposes.add(SkinnedMeshInverseBindposes::from(inverse)));
    }
    for (ni, parts) in &loaded.parts {
        let Some(mi) = loaded.model.nodes[*ni].mesh else { continue };
        let source = &loaded.model.meshes[mi];
        let morphed: Option<MeshData> = morph.as_ref().map(|(set, weights)| {
            let mut m = source.clone();
            set.apply(&mut m.positions, weights);
            m
        });
        let data = morphed.as_ref().unwrap_or(source);
        let skinned = parts.first().is_some_and(|p| p.skinned);
        let mut new_parts = Vec::new();
        for subset in &source.subsets {
            let Some(base) = cry.material(mtl_path.as_deref(), subset.material_id as usize) else { continue };
            let sub = mtl.as_deref().and_then(|m| m.sub(subset.material_id as usize));
            let material = match sub {
                Some(sub) => restyle(cry, &base, sub, assets, part),
                None => base,
            };
            let Some((mesh, aabb)) = subset_mesh(data, subset, skinned, RenderAssetUsages::RENDER_WORLD) else { continue };
            new_parts.push(Part { mesh: cry.meshes.add(mesh), material, aabb, skinned });
        }
        if !new_parts.is_empty() {
            out.parts.push((*ni, new_parts));
        }
    }
    Ok(Arc::new(out))
}

/// Morph targets of the customisable head and the weights of this character, if any.
fn face_morph(assets: &CharacterAssets, part: &PartAsset) -> Option<(MorphSet, Vec<(String, f32)>)> {
    let c = &assets.desc.customization;
    let set = MorphSet::load(&part.path).ok().flatten()?;
    let d = &assets.desc;
    let targets = format!("objects/characters/{}/{}/face/{}_{}_targets.xml", d.race.folder(), d.gender.folder(), d.race.prefix(), d.gender.letter());
    let maker = FaceMaker::load(&targets).ok()?;
    let mut weights = FaceMaker::race_weights(d.race);
    weights.extend(maker.weights(&c.modifier));
    Some((set, weights))
}

/// Per-shader tuning of one sub-material for the character (see the module docs of `aa-anim`).
fn restyle(cry: &mut CryAssets, base: &Handle<StandardMaterial>, sub: &CryMaterial, assets: &CharacterAssets, _part: &PartAsset) -> Handle<StandardMaterial> {
    let shader = sub.shader.to_ascii_lowercase();
    let Some(src) = cry.materials.get(base) else { return base.clone() };
    let mut m = src.clone();
    match shader.as_str() {
        "humanskin" => {
            // the textures carry the colour; the engine's `Diffuse` (0.2) and SSS colours are lighting terms
            m.base_color = Color::WHITE;
            m.emissive = LinearRgba::BLACK;
            m.perceptual_roughness = 0.55;
            m.reflectance = 0.3;
            let _ = &assets.skin;
        }
        "hair" => {
            let color = sub
                .public_param("HairColor")
                .and_then(parse_rgb)
                .map(|c| Color::srgb(c[0], c[1], c[2]))
                .unwrap_or(Color::srgb(0.4, 0.3, 0.2));
            m.base_color = color;
            m.emissive = LinearRgba::BLACK;
            m.perceptual_roughness = 0.6;
            m.reflectance = 0.25;
        }
        "eye" => {
            m.base_color = Color::WHITE;
            m.emissive = LinearRgba::BLACK;
            m.perceptual_roughness = 0.12;
            m.reflectance = 0.5;
        }
        "illum" => {
            m.emissive = LinearRgba::BLACK;
        }
        _ => return base.clone(),
    }
    cry.materials.add(m)
}

fn parse_rgb(s: &str) -> Option<[f32; 3]> {
    let mut it = s.split(',').map(|v| v.trim().parse::<f32>());
    Some([it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?])
}
