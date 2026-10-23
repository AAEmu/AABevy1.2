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
use aa_cry::{CRY_TO_BEVY, Shading};
use std::collections::HashMap;

use bevy::prelude::*;

use crate::animator::{Animator, ClipLibrary, RandomIdle, spawn_bones};
use crate::cal::Cal;
use crate::cdf::{Cdf, norm};
use crate::custom::Customization;
use crate::cut::{Hull, cut_hulls, cut_mesh};
use crate::db::{GameDb, Gender, HairColor, ItemVisual, Race, SkinColor, model_id};
use crate::face::{EyeTint, FaceKey, FaceLayer, abgr, take_face};
use crate::morph::{FaceMaker, MorphSet};
use crate::pose::{BoundClip, SampleOptions};
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
        // `<pfx>_face_mask_new.dds` exists for the Nuian male only, the other models have `<pfx>_face_mask.dds`
        let mask = [format!("{face_dir}/{prefix}_face_mask_new.dds"), format!("{face_dir}/{prefix}_face_mask.dds")]
            .into_iter()
            .find(|p| aa_pak::Pak::global().exists(p));
        let lips = mask.filter(|_| c.lip_color != 0).map(|p| (p, abgr(c.lip_color)));
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
    /// Original attachments declared by this part's CDF, for client assembly on shared bones.
    /// Empty for a raw mesh. Paths/materials above still describe the unwrapped part model.
    pub attachments: Vec<crate::cdf::Attachment>,
    /// Equipment slot of a costume part (x2game / protocol numbering, see [`crate::db::wire_slot`]):
    /// 0 head, 2 chest, 4 legs, 5 hands, 6 feet, 7 arms, 8 back, 27 costume. `None` for the body
    /// parts and for explicit meshes.
    pub equip_slot: Option<u8>,
    /// `item_assets.detail` of the visual (0 when unknown): orders the `Cut` hulls, see `cut_lists`.
    pub detail: u32,
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
    /// `item_assets.heel_offset_height` (m) of the costume's visual, else of the shoes' (x2game
    /// `39311310`): heeled shoes are modelled with the sole that far under the floor, and the spawn
    /// lifts the model by it. x2game raises its character and portrait cameras by the same height
    /// (`docs/research/re-equipment.md`).
    pub heel: f32,
    /// Underwear: body diffuse textures that replace the nude skin texture (underpants first, then
    /// the top of the pair, `more_asset_id`). Empty = the nude texture.
    pub underwear: Vec<String>,
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
        let mut body = PartAsset { kind: PartKind::Body, slot: "body".into(), path: body_path.clone(), material: None, dye: None, attachments: Vec::new(), equip_slot: None, detail: 0 };
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
        let mut pieces = self.costume_pieces(db, model)?;
        // a costume (`cosplay`, slot 27) replaces the armour under it: x2game clears 6 slots
        // (`39316BF0`, table at `39AC7358`: head, chest, legs, hands, feet, back) and the one-piece
        // skin carries its own skin geometry, so the nude body would only poke through it
        if pieces.iter().any(|p| p.wire == Some(COSTUME_SLOT) && !p.visual.is_texture()) {
            pieces.retain(|p| !matches!(p.wire, Some(s) if COSTUME_HIDES.contains(&s)));
            parts.retain(|p| p.kind != PartKind::Body);
        }
        // the costume's heel if it has a visual for the model, else the shoes' (x2game `39311310`)
        let heel_of = |wire: u8| pieces.iter().find(|p| p.wire == Some(wire) && !p.visual.is_texture()).map(|p| p.visual.heel);
        let heel = heel_of(COSTUME_SLOT).or_else(|| heel_of(FEET_SLOT)).unwrap_or(0.0);
        let mut underwear: Vec<(u8, String)> = Vec::new();
        for piece in pieces {
            if piece.visual.is_texture() {
                // underwear: a body texture (+ the pair's top), not a mesh
                if parts.iter().any(|p| p.kind == PartKind::Body) {
                    let wire = piece.wire.unwrap_or(0);
                    underwear.push((wire, piece.visual.path.clone()));
                    if piece.visual.more_asset_id != 0
                        && let Ok(top) = db.asset_path(piece.visual.more_asset_id)
                    {
                        underwear.push((wire, top));
                    }
                }
                continue;
            }
            let mut part = resolve_part(PartKind::Costume, &piece.slot, &piece.visual.path)?;
            part.dye = piece.dye;
            part.equip_slot = piece.wire;
            part.detail = piece.visual.detail;
            parts.push(part);
        }
        // underpants (14) before the undershirt (13): the pants texture of a pair carries both pieces
        underwear.sort_by_key(|(wire, _)| std::cmp::Reverse(*wire));
        let underwear: Vec<String> = underwear.into_iter().map(|(_, p)| p).filter(|p| aa_pak::Pak::global().exists(p)).collect();

        let skin_albedo = skin_albedo(&skin, &db.skin_colors(model).unwrap_or_default());
        let (face_layers, face_lips, face_normal) = self.face_customization(db, &dir, &prefix);
        let pupil_color = (self.customization.left_pupil_color != 0).then(|| abgr(self.customization.left_pupil_color));
        Ok(CharacterAssets { desc: self.clone(), model_id: model, skeleton, cal, parts, skin, skin_albedo, hair_color, face_layers, face_lips, face_normal, pupil_color, heel, underwear })
    }

    /// The equipment visuals of the costume with their slots.
    fn costume_pieces(&self, db: &GameDb, model: u32) -> Result<Vec<Piece>> {
        // the slot decides what a piece is, as in x2game: some armour visuals carry the costume type
        // in their `detail` (the ferre login-stage jacket `fe_m_ar_login001`, `0xE000`) and are
        // still worn with trousers, gloves and shoes
        let typed = |slot: &str, wire: Option<u8>, visual: ItemVisual, dye| {
            let slot = wire.and_then(slot_name).map_or(slot.to_string(), str::to_string);
            Piece { slot, wire, visual, dye }
        };
        Ok(match &self.costume {
            Costume::None => Vec::new(),
            Costume::Login => db
                .login_costume(model)?
                .into_iter()
                .map(|p| typed(&p.slot, pack_slot(&p.slot), p.visual, p.dye))
                .collect(),
            Costume::Items(items) => {
                let mut v = Vec::new();
                for &i in items {
                    let dye = db.default_dye(i);
                    let wire = db.item_slot_type(i).and_then(crate::db::wire_slot);
                    v.extend(db.item_visuals(i, model)?.into_iter().map(|visual| typed("item", wire, visual, dye)));
                }
                v
            }
            Costume::Meshes(m) => m
                .iter()
                .map(|p| Piece {
                    slot: "mesh".into(),
                    wire: None,
                    visual: ItemVisual { asset_id: 0, path: norm(p), detail: 0, heel: 0.0, more_asset_id: 0 },
                    dye: None,
                })
                .collect(),
        })
    }
}

/// One equipment visual of a [`Costume`], before it is resolved to a part.
struct Piece {
    slot: String,
    wire: Option<u8>,
    visual: ItemVisual,
    dye: Option<[u8; 3]>,
}

/// The costume (`cosplay`) equipment slot.
const COSTUME_SLOT: u8 = 27;
/// Wire slot of the shoes (`slot_type_id` 7).
const FEET_SLOT: u8 = 6;
/// The armour slots a costume hides: head, chest, legs, hands, feet, back (x2game `39316BF0` walks
/// the 6 slots of the table at `39AC7358`, read from a running client: `docs/research/re-equipment.md`;
/// waist, arms and underwear stay).
const COSTUME_HIDES: [u8; 6] = [0, 2, 4, 5, 6, 8];

/// Attachment name of an equipment slot (the base `.cdf` slots).
fn slot_name(wire: u8) -> Option<&'static str> {
    Some(match wire {
        0 => "head",
        1 => "neck",
        2 => "chest",
        3 => "waist",
        4 => "legs",
        5 => "hands",
        6 => "feet",
        7 => "arms",
        8 => "back",
        13 => "undershirt",
        14 => "underpants",
        27 => "cosplay",
        _ => return None,
    })
}

/// Equipment slot of an `equip_pack_cloths` column ([`crate::db::CLOTH_PACK_SLOTS`]).
fn pack_slot(column: &str) -> Option<u8> {
    Some(match column {
        "headgear" => 0,
        "necklace" => 1,
        "shirt" => 2,
        "belt" => 3,
        "pants" => 4,
        "glove" => 5,
        "shoes" => 6,
        "bracelet" => 7,
        "back" => 8,
        "undershirt" => 13,
        "underpants" => 14,
        "cosplay" => 27,
        _ => return None,
    })
}

/// `.cdf` parts are unwrapped to their `.chr` + material; `.chr` parts pass through.
fn resolve_part(kind: PartKind, slot: &str, path: &str) -> Result<PartAsset> {
    if path.ends_with(".cdf") {
        let cdf = Cdf::load(path)?;
        Ok(PartAsset { kind, slot: slot.to_string(), path: cdf.model, material: cdf.material, dye: None, attachments: cdf.attachments, equip_slot: None, detail: 0 })
    } else {
        Ok(PartAsset { kind, slot: slot.to_string(), path: path.to_string(), material: None, dye: None, attachments: Vec::new(), equip_slot: None, detail: 0 })
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
    spawn_character_inner(commands, cry, clips, assets, transform, false)
}

/// Like [`spawn_character`], with the original face00 expression morphs overlaid on a verified
/// compatible customisable head. Add [`crate::facial::FacialPlugin`] to play its matching FSQs.
pub fn spawn_character_with_facial(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    assets: CharacterAssets,
    transform: Transform,
) -> Result<Entity> {
    spawn_character_inner(commands, cry, clips, assets, transform, true)
}

fn spawn_character_inner(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    assets: CharacterAssets,
    transform: Transform,
    facial: bool,
) -> Result<Entity> {
    let t_skeleton = aa_cry::prof::scope("char.skeleton");
    // one rig per skeleton file: clip bindings and skeleton-bound models are shared between the characters
    let rig = crate::pose::shared_rig(&assets.skeleton)?;
    let skeleton = rig.skeleton.clone();
    drop(t_skeleton);

    let root = commands.spawn((Name::new(format!("{:?} {:?}", assets.desc.race, assets.desc.gender)), transform, Visibility::default())).id();
    let model_root = commands
        .spawn((Name::new("model"), model_transform(&assets), Visibility::default(), ChildOf(root)))
        .id();
    let bones = spawn_bones(commands, model_root, &rig);

    // the parts hide each other's (and the body's) triangles inside their `Cut` hulls
    let t_hulls = aa_cry::prof::scope("char.hulls");
    let cuts = cut_lists(cry, &assets);
    drop(t_hulls);

    let mut part_entities = Vec::new();
    for (part, cut) in assets.parts.iter().zip(&cuts) {
        let options = ModelOptions {
            material: part.material.clone(),
            skeleton: Some(SkeletonBinding { skeleton: skeleton.clone(), joints: bones.clone() }),
            cry_space: true,
            shading: Shading::Full,
            ..default()
        };
        let loaded = match load_part(cry, &assets, part, &options, cut.as_ref()) {
            Ok(l) => l,
            Err(e) => {
                warn!("aa-anim: {}: {e}", part.path);
                continue;
            }
        };
        if facial && part.kind == PartKind::Face {
            match crate::facial::prepare_face(cry, &assets, part, &loaded) {
                Ok(face) => {
                    if let Ok(weights) = bevy::mesh::morph::MorphWeights::new(vec![0.0; face.targets.len()], None) {
                        commands.entity(root).insert((face, weights));
                    }
                }
                Err(error) => warn!("aa-anim: facial overlay for {}: {error}", part.path),
            }
        }
        let t_spawn = aa_cry::prof::scope("char.spawn_loaded");
        let e = spawn_loaded(commands, &part.path, &loaded, &options);
        drop(t_spawn);
        commands.entity(e).insert(ChildOf(model_root));
        part_entities.push(e);
    }

    // animation
    let _t_idle = aa_cry::prof::scope("char.idle");
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

/// The transform of the model root under the character's: CryEngine's axes to Bevy's, and up by the
/// heel of the shoes so their soles stand on the floor.
pub fn model_transform(assets: &CharacterAssets) -> Transform {
    Transform::from_rotation(CRY_TO_BEVY).with_translation(Vec3::Y * assets.heel)
}

fn start_idle(animator: &mut Animator, clips: &ClipLibrary, assets: &CharacterAssets) -> Result<()> {
    let cal = Cal::shared(&assets.cal)?;
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
    cutters: Option<&CutList>,
) -> Result<Arc<LoadedModel>> {
    // the plain model (cached): material path and mesh data for the face texture
    let t_plain = aa_cry::prof::scope("char.part.plain");
    let plain = cry.load_model(&part.path, options)?;
    drop(t_plain);
    let mtl = plain.material_path.as_deref().and_then(|p| cry.mtl(p));
    let t_look = aa_cry::prof::scope("char.part.face_look");
    let face = if part.kind == PartKind::Face { face_look(cry, assets, mtl.as_deref(), &plain) } else { FaceLook::default() };
    drop(t_look);
    let t_dyes = aa_cry::prof::scope("char.part.dyes");
    let dyes = dyed_diffuses(cry, mtl.as_deref(), part);
    drop(t_dyes);
    let morph = if part.kind == PartKind::Face { face_morph(assets, part) } else { None };
    let cut = cutters.map(|c| c.hulls.clone());
    // the body under underwear: its texture replaces the nude skin (`_ddn` its normal map)
    let (body_diffuse, body_normal) = match assets.underwear.first().filter(|_| part.kind == PartKind::Body) {
        Some(df) => {
            let ddn = df.replace("_df.dds", "_ddn.dds");
            (cry.texture(df, true), aa_pak::Pak::global().exists(&ddn).then(|| cry.texture(&ddn, false)).flatten())
        }
        None => (None, None),
    };

    let skin = part.kind != PartKind::Costume || mtl.as_deref().is_some_and(|m| m.sub_materials.iter().any(|s| s.shader.eq_ignore_ascii_case("humanskin")));
    let look = Arc::new(Look { kind: part.kind, skin: assets.skin.clone(), skin_tune: body_or_face_tune(part.kind, assets.desc.race, assets.desc.gender),
        fur: assets.desc.race == Race::Ferre,
        face,
        dyes,
        body_diffuse,
        body_normal,
    });
    let needs_materials = skin || !look.dyes.is_empty();
    if !needs_materials && morph.is_none() && cut.is_none() {
        return Ok(plain);
    }
    // a part that is only cut (plain armour) is the same for every character wearing the same
    // pieces: share it
    let shared_key = (!needs_materials && morph.is_none())
        .then(|| cutters.map(|c| format!("{}|{:?}|{}|{}", part.path, part.material, assets.skeleton, c.key)))
        .flatten();
    if let Some(key) = &shared_key
        && let Some(model) = CUT_MODELS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key).cloned())
    {
        return Ok(model);
    }
    let mut hooked = options.clone();
    if needs_materials {
        let look = look.clone();
        hooked.material_hook = Some(MaterialHook::new(move |sub, m| restyle(sub, m, &look)));
    }
    if morph.is_some() || cut.is_some() {
        let path = part.path.clone();
        hooked.vertex_hook = Some(VertexHook::new(move |mesh| {
            if let Some((set, weights)) = &morph {
                set.apply(&mut mesh.positions, weights);
            }
            if let Some(hulls) = &cut {
                let n = cut_mesh(mesh, hulls);
                debug!("aa-anim: {path}: {n} of {} triangles cut by {} hulls", mesh.indices.len() / 3, hulls.len());
            }
        }));
    }
    let _t_hooked = aa_cry::prof::scope("char.part.hooked");
    let model = cry.load_model(&part.path, &hooked)?;
    if let Some(key) = shared_key {
        let mut cache = CUT_MODELS.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(Default::default);
        if cache.len() >= CUT_MODELS_MAX {
            cache.clear();
        }
        cache.insert(key, model.clone());
    }
    Ok(model)
}

/// Cut-only parts by part, skeleton and cutters (see `load_part`). Bounded: when full it starts
/// over (the spawned parts keep their own handles).
static CUT_MODELS: std::sync::Mutex<Option<HashMap<String, Arc<LoadedModel>>>> = std::sync::Mutex::new(None);
const CUT_MODELS_MAX: usize = 256;

/// The `Cut` hulls that hide triangles of one part, with a key naming the parts they come from.
#[derive(Clone)]
struct CutList {
    hulls: Arc<Vec<Hull>>,
    key: String,
}

/// The `Cut` hulls of one part's model (see [`crate::cut`]); empty for a part without any.
fn part_hulls(cry: &mut CryAssets, part: &PartAsset) -> Vec<Hull> {
    let Ok(model) = aa_cry::Model::load(&part.path) else { return Vec::new() };
    let mtl_path = part.material.clone().or_else(|| aa_cry::scene::find_mtl(&part.path, &model));
    let Some(mtl) = mtl_path.and_then(|p| cry.mtl(&p)) else { return Vec::new() };
    cut_hulls(&model, &mtl)
}

/// Sort key of an equipment part for the cut order: `item_assets.detail`, with the region bits the
/// gloves, arms and boots take over from the chest / legs piece (x2game `39320FC0`; without a chest
/// the chest value is `0x4402`, without legs the legs value `0x4800`).
fn cut_key(part: &PartAsset, chest: u32, legs: u32) -> u32 {
    part.detail
        | match part.equip_slot {
            Some(5) => chest & 0x1C7E,
            Some(7) => chest & 0x1FF0,
            Some(6) => legs & 0x1F8E,
            _ => 0,
        }
}

/// Which `Cut` hulls cut which part, per part of `assets.parts` (`None`: not cut).
///
/// The rule of x2game's `CharacterCustomizer` (`393211A0`): the equipment parts are sorted by
/// [`cut_key`], highest first, and each one is cut by the hulls of the parts of its group sorted
/// before it, then adds its own hulls to the group. The helmet and the costume form the `head`
/// group, all other equipment the `body` group. Afterwards face, hair, glasses and beard are cut
/// by the `head` group, the nude body by the `body` group; the tail is never cut.
fn cut_lists(cry: &mut CryAssets, assets: &CharacterAssets) -> Vec<Option<CutList>> {
    let parts = &assets.parts;
    let detail = |slot: u8, default: u32| {
        parts.iter().find(|p| p.kind == PartKind::Costume && p.equip_slot == Some(slot)).map_or(default, |p| p.detail)
    };
    let (chest, legs) = (detail(2, 0x4402), detail(4, 0x4800));
    let mut order: Vec<(usize, u32)> = parts
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == PartKind::Costume && body_part_slot(p).is_none())
        .map(|(i, p)| (i, cut_key(p, chest, legs)))
        .collect();
    // stable: equal keys keep the drawing order
    order.sort_by_key(|&(_, key)| std::cmp::Reverse(key));

    #[derive(Default)]
    struct Group {
        hulls: Vec<Hull>,
        key: String,
    }
    impl Group {
        fn list(&self) -> Option<CutList> {
            (!self.hulls.is_empty()).then(|| CutList { hulls: Arc::new(self.hulls.clone()), key: self.key.clone() })
        }
    }
    let (mut head, mut body) = (Group::default(), Group::default());
    let mut out: Vec<Option<CutList>> = vec![None; parts.len()];
    for (i, _) in order {
        let part = &parts[i];
        let group = if matches!(part.equip_slot, Some(0 | COSTUME_SLOT)) { &mut head } else { &mut body };
        out[i] = group.list();
        let own = part_hulls(cry, part);
        if !own.is_empty() {
            group.hulls.extend(own);
            group.key.push_str(&part.path);
            group.key.push(';');
        }
    }
    for (i, part) in parts.iter().enumerate() {
        out[i] = match (part.kind, body_part_slot(part)) {
            (PartKind::Body, _) => body.list(),
            (PartKind::Face | PartKind::Hair, _) | (_, Some("glasses" | "beard")) => head.list(),
            (_, Some(_)) => None,
            (PartKind::Costume, None) => continue,
        };
    }
    out
}

/// The body-part slots a client may add as costume-kind parts without an equipment slot
/// (`aa-client` draws the ferre tail and the beard that way): `tail`, `glasses` or `beard`.
fn body_part_slot(part: &PartAsset) -> Option<&'static str> {
    if part.equip_slot.is_some() {
        return None;
    }
    ["tail", "glasses", "beard"].into_iter().find(|s| part.slot == *s)
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

/// UV rectangles `[u0, v0, u1, v1]` of the eyeballs: the geometry drawn with the `Eye` shader.
///
/// The ferre `Eye` sub-material also draws teeth and tongue of the same atlas, so the union box
/// spans the whole strip. The eyeballs are the two separate meshes of the shader that are square
/// and of the same size: both eyes use one rectangle (elf, nuian: returned once) or one each
/// (ferre). If there is no such pair the union box is used.
fn eye_uv_boxes(model: &aa_cry::Model, mtl: &CryMaterial) -> Vec<[f32; 4]> {
    let union = |a: [f32; 4], b: [f32; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
    let mut all: Option<[f32; 4]> = None;
    // uv box of every connected piece of eye geometry
    let mut pieces: Vec<[f32; 4]> = Vec::new();
    for mesh in &model.meshes {
        // union-find over vertices touched by eye triangles
        let mut parent: Vec<u32> = (0..mesh.positions.len() as u32).collect();
        fn find(p: &mut [u32], mut i: u32) -> u32 {
            while p[i as usize] != i {
                p[i as usize] = p[p[i as usize] as usize];
                i = p[i as usize];
            }
            i
        }
        let mut used: Vec<u32> = Vec::new();
        for subset in &mesh.subsets {
            if !mtl.sub(subset.material_id as usize).is_some_and(|s| s.shader.eq_ignore_ascii_case("eye")) {
                continue;
            }
            let first = subset.first_index as usize;
            let Some(idx) = mesh.indices.get(first..first + subset.index_count as usize) else { continue };
            for tri in idx.chunks_exact(3) {
                let r = find(&mut parent, tri[0]);
                for &v in &tri[1..] {
                    let o = find(&mut parent, v);
                    parent[o as usize] = r;
                }
                used.extend_from_slice(tri);
            }
        }
        let mut boxes: HashMap<u32, [f32; 4]> = HashMap::new();
        for v in used {
            let Some(uv) = mesh.uvs.get(v as usize) else { continue };
            let root = find(&mut parent, v);
            let b = [uv.x, uv.y, uv.x, uv.y];
            all = Some(all.map_or(b, |a| union(a, b)));
            boxes.entry(root).and_modify(|e| *e = union(*e, b)).or_insert(b);
        }
        pieces.extend(boxes.into_values());
    }
    let iou = |a: [f32; 4], b: [f32; 4]| {
        let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
        let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
        let inter = w * h;
        let uni = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
        if uni > 0.0 { inter / uni } else { 0.0 }
    };
    let size = |a: [f32; 4]| (a[2] - a[0], a[3] - a[1]);
    // the largest pair of square pieces of one size
    let mut best: Option<(f32, [f32; 4], [f32; 4])> = None;
    for (i, &a) in pieces.iter().enumerate() {
        for &b in &pieces[i + 1..] {
            let ((aw, ah), (bw, bh)) = (size(a), size(b));
            let similar = (aw - bw).abs() <= 0.1 * aw.max(bw) && (ah - bh).abs() <= 0.1 * ah.max(bh);
            let square = ah > 0.0 && (0.8..=1.25).contains(&(aw / ah));
            if similar && square && best.is_none_or(|(s, ..)| aw * ah > s) {
                best = Some((aw * ah, a, b));
            }
        }
    }
    match best {
        Some((_, a, b)) if iou(a, b) > 0.7 => vec![union(a, b)],
        Some((_, a, b)) => vec![a, b],
        None => all.into_iter().collect(),
    }
}

/// Does the sub-material's diffuse texture match `path` (lower-case pak path)?
fn uses_texture(sub: &CryMaterial, path: &str) -> bool {
    use aa_cry::material::{TextureMap, texture_pak_path};
    !path.is_empty() && sub.texture(&TextureMap::Diffuse).is_some_and(|t| texture_pak_path(&t.file).to_ascii_lowercase() == path)
}

/// What the face texture of a character is made of: the texture the material names (replaced by
/// the composed one), and the [`FaceKey`] of the composition, `None` when the customisation changes
/// nothing (the base texture is used as is).
struct FaceInputs {
    named: String,
    base: String,
    key: Option<FaceKey>,
}

fn face_inputs(assets: &CharacterAssets, mtl: Option<&CryMaterial>, model: &aa_cry::Model) -> Option<FaceInputs> {
    use aa_cry::material::{TextureMap, texture_pak_path};
    let skin = mtl?.sub_materials.iter().find(|s| s.shader.eq_ignore_ascii_case("humanskin"))?;
    let diffuse = skin.texture(&TextureMap::Diffuse)?;
    // `face00_df` (named by the material; `... copy.dds` does not exist) is a placeholder: the
    // customisable base is the `face01` texture of the same race
    let named = texture_pak_path(&diffuse.file);
    let pak = aa_pak::Pak::global();
    let base = [named.replace("face00", "face01"), named.replace(" copy", "")].into_iter().find(|p| pak.exists(p))?;
    let eyes: Vec<EyeTint> = match (assets.pupil_color, mtl) {
        (Some(color), Some(mtl)) => eye_uv_boxes(model, mtl).into_iter().map(|uv| EyeTint { uv, color }).collect(),
        _ => Vec::new(),
    };
    let changed = !(assets.face_layers.is_empty() && assets.face_lips.is_none() && eyes.is_empty() && base == named);
    let key = changed.then(|| FaceKey { base: base.clone(), layers: assets.face_layers.clone(), lips: assets.face_lips.clone(), eyes });
    Some(FaceInputs { named, base, key })
}

/// The key of the composed face texture of `assets` for its face part's model and material, to
/// [`prewarm_face`](crate::face::prewarm_face) it on a worker thread. `None` without a customisable
/// face. Pure CPU: callable from any thread.
pub fn face_key(assets: &CharacterAssets, mtl: Option<&CryMaterial>, model: &aa_cry::Model) -> Option<FaceKey> {
    face_inputs(assets, mtl, model)?.key
}

/// Composes the face texture of `assets` ahead of [`spawn_character`] (20-30 ms of CPU: BC decode,
/// decals, lips, eyes, mips) so that the spawn on the main thread finds it ready: loads the face
/// model and material itself and calls [`prewarm_face`](crate::face::prewarm_face). Callable from
/// any thread; returns whether it composed something (false: nothing to compose, or already done).
pub fn prewarm_character_face(assets: &CharacterAssets) -> Result<bool> {
    let Some(part) = assets.parts.iter().find(|p| p.kind == PartKind::Face) else { return Ok(false) };
    let model = aa_cry::Model::load(&part.path)?;
    let mtl_path = part.material.clone().or_else(|| aa_cry::scene::find_mtl(&part.path, &model));
    let mtl = mtl_path.and_then(|p| aa_cry::material::Material::load(&p).ok());
    match face_key(assets, mtl.as_ref(), &model) {
        Some(key) => crate::face::prewarm_face(&key),
        None => Ok(false),
    }
}

/// Ids of the composed face assets (`FaceKey::id` -> asset): identical faces share one texture
/// while some character still uses it (the map holds ids, not handles: a face nobody shows any
/// more, e.g. one of the create screen's slider steps, is freed).
static FACE_ASSETS: std::sync::Mutex<Option<HashMap<String, AssetId<Image>>>> = std::sync::Mutex::new(None);

/// Weighted maps share one asset for equal path / strength while a character uses it. Store IDs
/// so discarded create-screen slider steps do not keep textures alive.
static FACE_NORMAL_ASSETS: std::sync::Mutex<Option<HashMap<(String, u32), AssetId<Image>>>> = std::sync::Mutex::new(None);

fn face_normal_texture(cry: &mut CryAssets, path: &str, weight: f32) -> Option<Handle<Image>> {
    let weight = crate::face_normal::strength(weight);
    if weight == 1.0 {
        return cry.texture(path, false);
    }
    let key = (norm(path).to_ascii_lowercase(), weight.to_bits());
    let known = FACE_NORMAL_ASSETS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key).copied());
    if let Some(handle) = known.and_then(|id| cry.images.get_strong_handle(id)) {
        return Some(handle);
    }
    match crate::face_normal::bake(path, weight) {
        Ok(image) => {
            let handle = cry.images.add(image);
            FACE_NORMAL_ASSETS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(key, handle.id());
            Some(handle)
        }
        Err(e) => {
            warn!("aa-anim: weighted face normal {path}: {e}");
            cry.texture(path, false)
        }
    }
}

/// Builds the face texture and normal map of the character for a face material. The composed
/// texture is shared by every character with the same face inputs and may have been composed on a
/// worker thread ([`prewarm_character_face`]).
fn face_look(cry: &mut CryAssets, assets: &CharacterAssets, mtl: Option<&CryMaterial>, loaded: &LoadedModel) -> FaceLook {
    let mut look = FaceLook::default();
    let Some(inputs) = face_inputs(assets, mtl, &loaded.model) else { return look };
    look.replaces = inputs.named.to_ascii_lowercase();
    if let Some((path, weight)) = &assets.face_normal {
        look.normal = face_normal_texture(cry, path, *weight);
    }
    let Some(key) = inputs.key else { return look };
    let id = key.id();
    let known = FACE_ASSETS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&id).copied());
    if let Some(handle) = known.and_then(|asset| cry.images.get_strong_handle(asset)) {
        crate::face::discard_face(&key);
        look.texture = Some(handle);
        return look;
    }
    match take_face(&key) {
        Ok(img) => {
            let handle = cry.images.add(img);
            FACE_ASSETS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(id, handle.id());
            look.texture = Some(handle);
        }
        Err(e) => warn!("aa-anim: face texture {}: {e}", inputs.base),
    }
    look
}

/// Morph targets of the customisable head and the weights of this character, if any.
pub(crate) fn face_morph(assets: &CharacterAssets, part: &PartAsset) -> Option<(MorphSet, Vec<(String, f32)>)> {
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
    /// Per model colour calibration of the skin, see [`skin_tune`].
    skin_tune: Vec3,
    /// Fur instead of skin (ferre): no skin sheen, the dark fur would only mirror the sky.
    fur: bool,
    face: FaceLook,
    /// Dyed copies of dyeable diffuse textures, by pak path of the original.
    dyes: HashMap<String, Handle<Image>>,
    /// Underwear texture replacing the body's skin texture, and its normal map.
    body_diffuse: Option<Handle<Image>>,
    body_normal: Option<Handle<Image>>,
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
            if look.kind == PartKind::Body {
                if let Some(t) = &look.body_diffuse {
                    m.base_color_texture = Some(t.clone());
                }
                if let Some(n) = &look.body_normal {
                    m.normal_map_texture = Some(n.clone());
                }
            }
            // the character's skin colour: the row's bright colour (stored b, g, r like the
            // material's `BSkinColor`) and diffuse colour replace the file's own (the first skin)
            let row = &look.skin;
            let bright = Vec3::new(row.bright[2] as f32, row.bright[1] as f32, row.bright[0] as f32) / 255.0;
            let diffuse = Vec3::new(row.diffuse[0] as f32, row.diffuse[1] as f32, row.diffuse[2] as f32) / 255.0;
            let c = skin_base_color(diffuse, bright).to_linear();
            let t = look.skin_tune;
            m.base_color = Color::linear_rgb(c.red * t.x, c.green * t.y, c.blue * t.z);
            if look.kind != PartKind::Face {
                // the body and the costumes' skin parts: see `BODY_SKIN_ROUGHNESS`
                m.perceptual_roughness = BODY_SKIN_ROUGHNESS;
                m.reflectance = BODY_SKIN_REFLECTANCE;
            }
            if look.fur {
                m.reflectance = 0.12;
                m.perceptual_roughness = m.perceptual_roughness.max(0.75);
            }
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

/// Sheen of the skin of the body and of the costumes' skin parts (chest cutouts, bare arms), the
/// face keeps the conversion's roughness 0.5 / reflectance 0.4. The captures show soft highlights
/// where the body turns away from the key light (elf female chest cutout); at the face's roughness
/// the body skin of the same lighting is a flat patch of one colour.
const BODY_SKIN_ROUGHNESS: f32 = 0.3;
const BODY_SKIN_REFLECTANCE: f32 = 0.7;

/// Per model colour multiplier of the skin (linear rgb) on top of `aa_cry::shade::skin_base_color`.
///
/// The generic conversion has one pink pull for all skins; the models differ (the elf's pale face
/// texture needs the pink, nuian male skin is already pink and turned red). Each entry was fitted
/// to the face of the model's `reference/character_create/step1_*` capture (mean linear colour
/// of the eye-to-chin region, `compare --costume none --no-hair`), see `docs/aa-anim.md`.
pub fn skin_tune(race: Race, gender: Gender) -> Vec3 {
    // BEGIN SKIN TUNE
    let t = match (race, gender) {
        (Race::Nuian, Gender::Male) => [0.820, 1.195, 1.181],
        (Race::Nuian, Gender::Female) => [0.981, 1.552, 1.502],
        (Race::Elf, Gender::Male) => [0.683, 0.816, 0.689],
        (Race::Elf, Gender::Female) => [1.219, 1.228, 1.355],
        (Race::Hariharan, Gender::Male) => [0.795, 0.792, 0.654],
        (Race::Hariharan, Gender::Female) => [0.486, 0.525, 0.505],
        (Race::Ferre, Gender::Male) => [0.222, 0.244, 0.222],
        (Race::Ferre, Gender::Female) => [0.467, 0.491, 0.418],
    };
    // END SKIN TUNE
    Vec3::from(t)
}

/// The skin tune of a part: [`skin_tune`] on the face; on the body (hands, arms, the costumes' skin parts) only its
/// brightness, times [`body_skin_tune`]. The face tune's tint corrects the face texture (yellowish where the captures
/// are pink); the body textures are pink already, and with the tint the hands came out a flat lavender white in game.
pub fn body_or_face_tune(kind: PartKind, race: Race, gender: Gender) -> Vec3 {
    let t = skin_tune(race, gender);
    if kind == PartKind::Face {
        t
    } else {
        Vec3::splat(t.dot(Vec3::new(0.2126, 0.7152, 0.0722))) * body_skin_tune(race, gender)
    }
}

/// Extra multiplier of [`skin_tune`] for the skin of the body (and of the costumes' skin parts).
///
/// `skin_tune` is fitted to the face region of the captures, so it carries the face texture's
/// colour. A body texture that is much brighter or more saturated than the face's needs its own
/// tune:
///
/// * ferre male: his body fur texture is greyer than his face texture while the capture is dark
///   brown all over, so the body gets a warm, darker tint;
/// * elf female: the nude texture is peach `(0.75, 0.41, 0.32)` linear, the face's grey pink
///   `(0.37, 0.26, 0.26)`, so the face tune (1.6) pushed the red of the costume's chest cutout
///   (`el_f_login001`, a `HumanSkin` sub-mesh with the nude texture) into the tone curve's shoulder:
///   a flat salmon patch `(243, 161, 148)` where the capture shows pale pink `(221, 165, 163)`. The
///   row is the face / body texture ratio `(0.50, 0.63, 0.83)`, refitted (`(0.43, 0.54, 0.69)`) after the stage-frame fix, and scaled by 1.06 after the stage rig refit, to the mean
///   of the two cutout lobes of `step2_elf_female` (`compare`, same pixels).
///
/// * nuian male / female: the nude textures (pink, the hands about `(0.60, 0.29, 0.31)` linear) are
///   2.6 / 1.4 times as bright as the `face01` textures the faces are composed on (mean linear luma
///   of the skin, face / body 0.38 / 0.71), so with the face's tune the hands came out a flat white in
///   the sun where the real client's are pink skin about as bright as the face
///   (`reference/ingame/combat_melee/real_*`); the rows are those ratios.
///
/// The other bodies are not tinted: their textures are within 5 % of the faces' (elf male 0.95,
/// hariharan 1.0 / 0.98).
pub fn body_skin_tune(race: Race, gender: Gender) -> Vec3 {
    match (race, gender) {
        (Race::Ferre, Gender::Male) => Vec3::new(0.75, 0.52, 0.45),
        (Race::Elf, Gender::Female) => Vec3::new(0.456, 0.572, 0.731),
        (Race::Nuian, Gender::Male) => Vec3::splat(0.38),
        (Race::Nuian, Gender::Female) => Vec3::splat(0.71),
        _ => Vec3::ONE,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn part(slot: u8, detail: u32) -> PartAsset {
        PartAsset {
            kind: PartKind::Costume,
            slot: String::new(),
            path: String::new(),
            material: None,
            dye: None,
            attachments: Vec::new(),
            equip_slot: Some(slot),
            detail,
        }
    }

    #[test]
    fn cut_order_of_the_starter_set() {
        // nu_m starter jacket 0x5500, trousers 0x5020, boots 0x4030, gloves 0x4180 (x2game `39320FC0`)
        let (chest, legs) = (0x5500, 0x5020);
        assert_eq!(cut_key(&part(6, 0x4030), chest, legs), 0x5030);
        assert_eq!(cut_key(&part(5, 0x4180), chest, legs), 0x5580);
        assert_eq!(cut_key(&part(2, 0x5500), chest, legs), 0x5500);
        // no chest / legs: the defaults
        assert_eq!(cut_key(&part(5, 0x4080), 0x4402, 0x4800), 0x4482);
        assert_eq!(cut_key(&part(6, 0x4010), 0x4402, 0x4800), 0x4810);
        // gloves before the jacket, the jacket before the boots, the boots before the trousers
        assert!(0x5580 > 0x5500 && 0x5500 > 0x5030 && 0x5030 > 0x5020);
    }

    #[test]
    fn costume_items_hide_body_and_covered_armour_and_underwear_is_a_texture() {
        if !GameDb::available() {
            return;
        }
        let db = GameDb::open().unwrap();
        let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Female).unwrap();
        // cap, jacket, trousers, boots, butler / maid costume
        desc.costume = Costume::Items(vec![6252, 23387, 23388, 23390, 28439]);
        let a = desc.resolve(&db).unwrap();
        let worn: Vec<_> = a.parts.iter().filter(|p| p.kind == PartKind::Costume).map(|p| p.equip_slot).collect();
        assert_eq!(worn, [Some(27)]);
        assert!(a.parts.iter().all(|p| p.kind != PartKind::Body));
        // underwear pair: the pants texture, then the top (more_asset_id); no mesh part
        desc.costume = Costume::Items(vec![30721]);
        let a = desc.resolve(&db).unwrap();
        assert!(a.parts.iter().all(|p| p.kind != PartKind::Costume));
        assert_eq!(a.underwear.len(), 2, "{:?}", a.underwear);
        assert!(a.underwear[0].ends_with("nu_f_in_pants002_df.dds") && a.underwear[1].ends_with("nu_f_in_shirt002_df.dds"), "{:?}", a.underwear);
        // heeled shoes carry their heel height, and the model stands on it
        desc.costume = Costume::Items(vec![18180]);
        let a = desc.resolve(&db).unwrap();
        assert!((a.heel - 0.09).abs() < 1e-4);
        assert!((model_transform(&a).translation.y - 0.09).abs() < 1e-4);
        // a costume's own heel wins over the shoes' (the nu_f login stage costume: 0.08)
        desc.costume = Costume::Items(vec![18180, 8000028]);
        assert!((desc.resolve(&db).unwrap().heel - 0.08).abs() < 1e-4);
    }
}
