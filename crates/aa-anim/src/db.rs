//! Character data from the game database (`compact.sqlite3`, read-only).
//!
//! The 1.2 client's own copy is encrypted; the AAEmu server's plain SQLite copy has the same
//! tables (`AA_DB` overrides the default path). Tables used (all keyed by `models.id`, the
//! "model id" of a race / gender, e.g. 10 = nuian male, 11 = nuian female):
//!
//! | table | what |
//! |---|---|
//! | `characters` | one row per race / gender; `creatable = 't'` marks the 8 playable ones |
//! | `models`, `actor_models` | `models.sub_id -> actor_models.model_file` = base `.cdf` |
//! | `item_body_parts` + `item_assets` | face (slot 23) and hair (slot 24) meshes: `asset_id` -> `item_assets.path`; hair also `asset_1..3` (hat variants) and `hair_base` |
//! | `hair_colors` | per (model, hair asset, colour): the `.mtl` that colours that hair |
//! | `skin_colors` | skin tint: bright / middle colour (material `BSkinColor` / `MSkinColor`), diffuse + specular tint, `custom_postfix` (ferre fur patterns) |
//! | `armor_assets` 1111..1114 + `item_armor_assets` + `item_assets` | the **login stage costume**: chest, legs, hands, feet, one asset per model |
//! | `total_character_customs` | named appearance presets: `owner_type_id = 1` are the character-create faces `<race>_face0001..9` (hair, hair colour, skin, normal map, colours, 128 morph bytes) |
//! | `face_normal_maps`, `face_decal_assets`, `face_eyelash_maps`, `face_diffuse_maps` | face texture sets |
//!
//! `equip_pack_body_parts` (`characters.preview_body_pack_id`) has the login-stage face / hair
//! *item* ids, but its `hair_color_id` rows belong to other models, so the defaults used here
//! come from the model-consistent `total_character_customs` presets instead.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::cdf::norm;
use crate::{Error, Result};

pub const DEFAULT_DB: &str = "D:/aaemu-live/rs-game/Data/compact.sqlite3";

/// The four playable races of 1.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Race {
    Nuian,
    Elf,
    Hariharan,
    Ferre,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Gender {
    Male,
    Female,
}

impl Race {
    pub const ALL: [Race; 4] = [Race::Nuian, Race::Elf, Race::Hariharan, Race::Ferre];

    /// Folder name under `objects/characters/` and `animations/`.
    pub fn folder(self) -> &'static str {
        match self {
            Race::Nuian => "nuian",
            Race::Elf => "elf",
            Race::Hariharan => "hariharan",
            Race::Ferre => "ferre",
        }
    }

    /// Two-letter file prefix (`nu`, `el`, `ha`, `fe`).
    pub fn prefix(self) -> &'static str {
        match self {
            Race::Nuian => "nu",
            Race::Elf => "el",
            Race::Hariharan => "ha",
            Race::Ferre => "fe",
        }
    }

    /// `models.id` of the male model; the female one is +1 (10/11, 16/17, 18/19, 20/21).
    pub fn male_model_id(self) -> u32 {
        match self {
            Race::Nuian => 10,
            Race::Elf => 16,
            Race::Hariharan => 18,
            Race::Ferre => 20,
        }
    }

    pub fn from_model_id(model_id: u32) -> Option<(Race, Gender)> {
        Race::ALL.iter().find_map(|&r| {
            let m = r.male_model_id();
            if model_id == m {
                Some((r, Gender::Male))
            } else if model_id == m + 1 {
                Some((r, Gender::Female))
            } else {
                None
            }
        })
    }
}

impl Gender {
    pub const ALL: [Gender; 2] = [Gender::Male, Gender::Female];

    pub fn folder(self) -> &'static str {
        match self {
            Gender::Male => "male",
            Gender::Female => "female",
        }
    }

    /// `m` / `f` as in file names (`nu_m_*`).
    pub fn letter(self) -> char {
        match self {
            Gender::Male => 'm',
            Gender::Female => 'f',
        }
    }
}

/// `(race, gender)` -> `models.id`.
pub fn model_id(race: Race, gender: Gender) -> u32 {
    race.male_model_id() + (gender == Gender::Female) as u32
}

/// An 8-bit RGB triple.
pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct SkinColor {
    pub id: u32,
    pub model_id: u32,
    pub name: String,
    /// `BSkinColor` of the HumanSkin shader.
    pub bright: Rgb,
    /// `MSkinColor`.
    pub middle: Rgb,
    pub diffuse: Rgb,
    pub specular: Rgb,
    pub glossiness: f32,
    pub specular_level: f32,
    /// Selects `<name>_<postfix>.mtl` variants (ferre fur patterns), empty otherwise.
    pub material_postfix: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HairColor {
    pub id: u32,
    pub model_id: u32,
    /// `item_assets.id` of the hair style this colour belongs to.
    pub asset_id: u32,
    pub name: String,
    /// `.mtl` colouring this hair style.
    pub material: String,
    pub diffuse: Rgb,
    pub base: Rgb,
}

/// A face or hair body part (`item_body_parts` row with its meshes resolved).
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPart {
    pub item_id: u32,
    pub slot_type: u32,
    pub model_id: u32,
    /// `item_assets` ids of the normal mesh and the hat variants (`asset_1..4`; 0 = none).
    pub asset_ids: [u32; 5],
    /// Mesh path of `asset_ids[0]`.
    pub path: String,
    /// Mesh paths of the hat variants (empty = none).
    pub hat_paths: [String; 4],
    pub face_mask: String,
    pub hair_base: String,
}

/// Character-create preset (`total_character_customs`).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u32,
    pub model_id: u32,
    pub name: String,
    pub owner_type: u32,
    pub hair_item: u32,
    pub hair_color_id: u32,
    pub skin_color_id: u32,
    pub face_normal_map_id: u32,
    pub face_normal_map_weight: f32,
    pub face_diffuse_map_id: u32,
    pub face_eyelash_map_id: u32,
    pub movable_decal: Decal,
    pub fixed_decals: [Decal; 4],
    /// `0xAARRGGBB`
    pub lip_color: u32,
    pub left_pupil_color: u32,
    pub right_pupil_color: u32,
    pub eyebrow_color: u32,
    pub deco_color: u32,
    /// 128 signed morph weights (`custom_face_presets` / `<race>_targets.xml` indices).
    pub modifier: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Decal {
    pub asset_id: u32,
    pub weight: f32,
    pub scale: f32,
    pub rotate: f32,
    pub move_x: i32,
    pub move_y: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayableModel {
    pub character_id: u32,
    pub model_id: u32,
    pub race: Race,
    pub gender: Gender,
    pub model_name: String,
    /// `actor_models.model_file`: the base `.cdf`.
    pub cdf: String,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CostumePiece {
    /// Slot column of `equip_pack_cloths`: `shirt`, `pants`, `glove`, `shoes`, `cosplay`, ...
    pub slot: String,
    /// `.chr` or `.cdf`, normalised.
    pub path: String,
    /// Default dye colour of the item (sRGB), if it is dyeable.
    pub dye: Option<[u8; 3]>,
}

/// Item slots of an `equip_pack_cloths` row, in draw order.
pub const CLOTH_PACK_SLOTS: [&str; 12] = [
    "underpants", "undershirt", "shirt", "pants", "glove", "shoes", "belt", "bracelet", "necklace", "headgear", "back", "cosplay",
];

pub struct GameDb {
    conn: Connection,
}

impl GameDb {
    /// Opens the database (`AA_DB` or [`DEFAULT_DB`]).
    pub fn open() -> Result<GameDb> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| DEFAULT_DB.to_string());
        GameDb::open_path(path)
    }

    pub fn open_path(path: impl AsRef<Path>) -> Result<GameDb> {
        let conn = Connection::open_with_flags(path.as_ref(), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
            .map_err(db_err)?;
        Ok(GameDb { conn })
    }

    /// Whether the database file exists (tests skip otherwise).
    pub fn available() -> bool {
        Path::new(&std::env::var("AA_DB").unwrap_or_else(|_| DEFAULT_DB.to_string())).exists()
    }

    /// The eight creatable race / gender models, ordered by race then gender.
    pub fn playable(&self) -> Result<Vec<PlayableModel>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT c.id, c.model_id, m.name, a.model_file, a.height \
                 FROM characters c JOIN models m ON m.id = c.model_id \
                 JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel' \
                 WHERE c.creatable = 't'",
            )
            .map_err(db_err)?;
        let mut out: Vec<PlayableModel> = st
            .query_map([], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, f32>(4)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .filter_map(|(character_id, model_id, model_name, cdf, height)| {
                let (race, gender) = Race::from_model_id(model_id)?;
                Some(PlayableModel { character_id, model_id, race, gender, model_name, cdf: norm(&cdf), height })
            })
            .collect();
        out.sort_by_key(|p| (p.race, p.gender));
        Ok(out)
    }

    /// The costume the login / character-create stage dresses a model in: the model's
    /// `characters.preview_cloth_pack_id` -> `equip_pack_cloths` items -> their meshes for the model.
    /// Nuian and Hariharan wear a one-piece `cosplay` skin (with cape), Elf and Ferre the
    /// "login stage" shirt / pants / gloves / shoes.
    pub fn login_costume(&self, model_id: u32) -> Result<Vec<CostumePiece>> {
        let cols = CLOTH_PACK_SLOTS.map(|c| format!("p.{c}_id")).join(", ");
        let sql = format!(
            "SELECT {cols} FROM characters ch JOIN equip_pack_cloths p ON p.id = ch.preview_cloth_pack_id              WHERE ch.model_id = ?1 AND ch.creatable = 't' LIMIT 1"
        );
        let items: Vec<u32> = self
            .conn
            .query_row(&sql, params![model_id], |r| {
                (0..CLOTH_PACK_SLOTS.len()).map(|i| Ok(r.get::<_, Option<u32>>(i)?.unwrap_or(0))).collect()
            })
            .map_err(|e| Error::Db(format!("preview cloth pack of model {model_id}: {e}")))?;
        let mut out = Vec::new();
        for (slot, item) in CLOTH_PACK_SLOTS.iter().zip(items) {
            if item == 0 {
                continue;
            }
            let dye = self.default_dye(item);
            for path in self.item_meshes(item, model_id)? {
                out.push(CostumePiece { slot: slot.to_string(), path, dye });
            }
        }
        Ok(out)
    }

    /// The colour a dyeable item is sold / worn with by default:
    /// `dyeable_items.default_dyeing_item_id` -> `item_dyeings` -> `dyeing_colors.color_rgb` (sRGB bytes).
    pub fn default_dye(&self, item_id: u32) -> Option<[u8; 3]> {
        let color: u32 = self
            .conn
            .query_row(
                "SELECT dc.color_rgb FROM dyeable_items di JOIN item_dyeings idy ON idy.item_id = di.default_dyeing_item_id                  JOIN dyeing_colors dc ON dc.id = idy.dyeing_color_id WHERE di.item_id = ?1 LIMIT 1",
                params![item_id],
                |r| r.get(0),
            )
            .ok()?;
        Some(crate::dye::dye_rgb(color))
    }

    /// Meshes of an equipment item for a model (`item_armors` -> `item_armor_assets` -> `item_assets`).
    pub fn item_meshes(&self, item_id: u32, model_id: u32) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT ia.path FROM item_armors arm JOIN item_armor_assets iaa ON iaa.armor_asset_id = arm.asset_id \
                 JOIN item_assets ia ON ia.id = iaa.asset_id \
                 WHERE arm.item_id = ?1 AND ia.model_id = ?2 ORDER BY ia.id",
            )
            .map_err(db_err)?;
        let rows = st.query_map(params![item_id, model_id], |r| r.get::<_, String>(0)).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).map(|p| norm(&p)).collect())
    }

    /// A face / hair body part by `item_body_parts.item_id`.
    pub fn body_part(&self, item_id: u32) -> Result<BodyPart> {
        self.conn
            .query_row(
                "SELECT item_id, slot_type_id, model_id, asset_id, asset_1_id, asset_2_id, asset_3_id, asset_4_id, \
                 face_mask, hair_base FROM item_body_parts WHERE item_id = ?1",
                params![item_id],
                |r| {
                    let ids: [u32; 5] = [r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?];
                    Ok((
                        r.get::<_, u32>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, u32>(2)?,
                        ids,
                        r.get::<_, Option<String>>(8)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                    ))
                },
            )
            .map_err(|e| Error::Db(format!("item_body_parts {item_id}: {e}")))
            .and_then(|(item_id, slot_type, model_id, asset_ids, face_mask, hair_base)| {
                let path = self.asset_path(asset_ids[0])?;
                let mut hat_paths: [String; 4] = Default::default();
                for (i, p) in hat_paths.iter_mut().enumerate() {
                    if asset_ids[i + 1] != 0 {
                        *p = self.asset_path(asset_ids[i + 1]).unwrap_or_default();
                    }
                }
                Ok(BodyPart {
                    item_id,
                    slot_type,
                    model_id,
                    asset_ids,
                    path,
                    hat_paths,
                    face_mask: norm(&face_mask),
                    hair_base: norm(&hair_base),
                })
            })
    }

    /// `item_assets.path` (normalised).
    pub fn asset_path(&self, asset_id: u32) -> Result<String> {
        self.conn
            .query_row("SELECT path FROM item_assets WHERE id = ?1", params![asset_id], |r| r.get::<_, String>(0))
            .map(|p| norm(&p))
            .map_err(|e| Error::Db(format!("item_assets {asset_id}: {e}")))
    }

    /// Body parts of a slot type for a model (23 face, 24 hair), by item id.
    pub fn body_parts(&self, model_id: u32, slot_type: u32) -> Result<Vec<BodyPart>> {
        let mut st = self
            .conn
            .prepare("SELECT item_id FROM item_body_parts WHERE model_id = ?1 AND slot_type_id = ?2 ORDER BY item_id")
            .map_err(db_err)?;
        let ids: Vec<u32> = st
            .query_map(params![model_id, slot_type], |r| r.get(0))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids.into_iter().filter_map(|i| self.body_part(i).ok()).collect())
    }

    /// The customisable base face of a model: the `*_face00.cdf` body part.
    pub fn base_face(&self, model_id: u32) -> Result<BodyPart> {
        self.body_parts(model_id, 23)?
            .into_iter()
            .find(|b| b.path.ends_with("face00.cdf"))
            .ok_or_else(|| Error::Missing(format!("face00.cdf body part of model {model_id}")))
    }

    pub fn skin_color(&self, id: u32) -> Result<SkinColor> {
        self.conn
            .query_row(
                "SELECT id, model_id, COMMENT, bright_skin_color_r, bright_skin_color_g, bright_skin_color_b, \
                 middle_skin_color_r, middle_skin_color_g, middle_skin_color_b, diffuse_color_r, diffuse_color_g, \
                 diffuse_color_b, specular_color_r, specular_color_g, specular_color_b, glossness, specular_level, \
                 custom_postfix FROM skin_colors WHERE id = ?1",
                params![id],
                |r| {
                    let c = |i: usize| -> rusqlite::Result<Rgb> {
                        Ok([r.get::<_, i64>(i)? as u8, r.get::<_, i64>(i + 1)? as u8, r.get::<_, i64>(i + 2)? as u8])
                    };
                    Ok(SkinColor {
                        id: r.get(0)?,
                        model_id: r.get(1)?,
                        name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        bright: c(3)?,
                        middle: c(6)?,
                        diffuse: c(9)?,
                        specular: c(12)?,
                        glossiness: r.get::<_, f64>(15)? as f32,
                        specular_level: r.get::<_, f64>(16)? as f32,
                        material_postfix: r.get::<_, Option<String>>(17)?.unwrap_or_default(),
                    })
                },
            )
            .map_err(|e| Error::Db(format!("skin_colors {id}: {e}")))
    }

    /// Skin colours selectable for a model (`npc_only = 'f'`), by id.
    pub fn skin_colors(&self, model_id: u32) -> Result<Vec<SkinColor>> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM skin_colors WHERE model_id = ?1 AND npc_only = 'f' ORDER BY COMMENT, id")
            .map_err(db_err)?;
        let ids: Vec<u32> = st.query_map(params![model_id], |r| r.get(0)).map_err(db_err)?.filter_map(|r| r.ok()).collect();
        Ok(ids.into_iter().filter_map(|i| self.skin_color(i).ok()).collect())
    }

    pub fn hair_color(&self, id: u32) -> Result<HairColor> {
        self.conn
            .query_row(
                "SELECT id, model_id, asset_id, comment, hair_material, hair_diffuse_color_r, hair_diffuse_color_g, \
                 hair_diffuse_color_b, hair_base_color_r, hair_base_color_g, hair_base_color_b FROM hair_colors WHERE id = ?1",
                params![id],
                |r| {
                    Ok(HairColor {
                        id: r.get(0)?,
                        model_id: r.get(1)?,
                        asset_id: r.get(2)?,
                        name: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        material: norm(&r.get::<_, Option<String>>(4)?.unwrap_or_default()),
                        diffuse: [r.get::<_, i64>(5)? as u8, r.get::<_, i64>(6)? as u8, r.get::<_, i64>(7)? as u8],
                        base: [r.get::<_, i64>(8)? as u8, r.get::<_, i64>(9)? as u8, r.get::<_, i64>(10)? as u8],
                    })
                },
            )
            .map_err(|e| Error::Db(format!("hair_colors {id}: {e}")))
    }

    /// Colours of one hair style (`item_assets.id`), ordered by name (`custom01`, `custom02`, ...).
    pub fn hair_colors_for(&self, model_id: u32, asset_id: u32) -> Result<Vec<HairColor>> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM hair_colors WHERE model_id = ?1 AND asset_id = ?2 AND npc_only = 'f' ORDER BY comment, id")
            .map_err(db_err)?;
        let ids: Vec<u32> =
            st.query_map(params![model_id, asset_id], |r| r.get(0)).map_err(db_err)?.filter_map(|r| r.ok()).collect();
        Ok(ids.into_iter().filter_map(|i| self.hair_color(i).ok()).collect())
    }

    /// `(normal map, specular map)` paths of a `face_normal_maps` row.
    pub fn face_normal_map(&self, id: u32) -> Result<(String, String)> {
        self.conn
            .query_row("SELECT normal, specular FROM face_normal_maps WHERE id = ?1", params![id], |r| {
                Ok((
                    norm(&r.get::<_, Option<String>>(0)?.unwrap_or_default()),
                    norm(&r.get::<_, Option<String>>(1)?.unwrap_or_default()),
                ))
            })
            .map_err(|e| Error::Db(format!("face_normal_maps {id}: {e}")))
    }

    /// `face_decal_assets` row: `(texture path, default x, default y)`.
    pub fn face_decal(&self, id: u32) -> Result<(String, i32, i32)> {
        self.conn
            .query_row("SELECT asset_path, defaultX, defaultY FROM face_decal_assets WHERE id = ?1", params![id], |r| {
                Ok((norm(&r.get::<_, String>(0)?), r.get::<_, i32>(1)?, r.get::<_, i32>(2)?))
            })
            .map_err(|e| Error::Db(format!("face_decal_assets {id}: {e}")))
    }

    /// Presets of a model; `owner_type` 1 = the character-create faces, 2 = NPCs, 3 = named NPCs.
    pub fn presets(&self, model_id: u32, owner_type: u32) -> Result<Vec<Preset>> {
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT {PRESET_COLUMNS} FROM total_character_customs WHERE model_id = ?1 AND owner_type_id = ?2 ORDER BY name, id"
            ))
            .map_err(db_err)?;
        let rows = st.query_map(params![model_id, owner_type], preset_row).map_err(db_err)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn preset(&self, id: u32) -> Result<Preset> {
        self.conn
            .query_row(
                &format!("SELECT {PRESET_COLUMNS} FROM total_character_customs WHERE id = ?1"),
                params![id],
                preset_row,
            )
            .map_err(|e| Error::Db(format!("total_character_customs {id}: {e}")))
    }
}

const PRESET_COLUMNS: &str = "id, model_id, name, owner_type_id, hair_id, hair_color_id, skin_color_id, face_normal_map_id, \
    face_normal_map_weight, face_diffuse_map_id, face_eyelash_map_id, face_movable_decal_asset_id, face_movable_decal_weight, \
    face_movable_decal_scale, face_movable_decal_rotate, face_movable_decal_move_x, face_movable_decal_move_y, \
    face_fixed_decal_asset_0_id, face_fixed_decal_asset_0_weight, face_fixed_decal_asset_1_id, face_fixed_decal_asset_1_weight, \
    face_fixed_decal_asset_2_id, face_fixed_decal_asset_2_weight, face_fixed_decal_asset_3_id, face_fixed_decal_asset_3_weight, \
    lip_color, left_pupil_color, right_pupil_color, eyebrow_color, deco_color, modifier";
fn preset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Preset> {
    let u = |i: usize| -> rusqlite::Result<u32> { Ok(r.get::<_, Option<i64>>(i)?.unwrap_or(0) as u32) };
    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
    let fixed = |i: usize| -> rusqlite::Result<Decal> {
        Ok(Decal { asset_id: u(i)?, weight: f(i + 1)?, ..Default::default() })
    };
    Ok(Preset {
        id: u(0)?,
        model_id: u(1)?,
        name: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        owner_type: u(3)?,
        hair_item: u(4)?,
        hair_color_id: u(5)?,
        skin_color_id: u(6)?,
        face_normal_map_id: u(7)?,
        face_normal_map_weight: f(8)?,
        face_diffuse_map_id: u(9)?,
        face_eyelash_map_id: u(10)?,
        movable_decal: Decal {
            asset_id: u(11)?,
            weight: f(12)?,
            scale: f(13)?,
            rotate: f(14)?,
            move_x: r.get::<_, Option<i64>>(15)?.unwrap_or(0) as i32,
            move_y: r.get::<_, Option<i64>>(16)?.unwrap_or(0) as i32,
        },
        fixed_decals: [fixed(17)?, fixed(19)?, fixed(21)?, fixed(23)?],
        lip_color: u(25)?,
        left_pupil_color: u(26)?,
        right_pupil_color: u(27)?,
        eyebrow_color: u(28)?,
        deco_color: u(29)?,
        modifier: r.get::<_, Option<Vec<u8>>>(30)?.unwrap_or_default(),
    })
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::Db(e.to_string())
}

/// `0xAARRGGBB` to `[r, g, b]`.
pub fn argb_rgb(c: u32) -> Rgb {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids() {
        assert_eq!(model_id(Race::Elf, Gender::Female), 17);
        assert_eq!(Race::from_model_id(21), Some((Race::Ferre, Gender::Female)));
        assert_eq!(Race::from_model_id(13), None);
        assert_eq!(argb_rgb(0xFF10_2030), [0x10, 0x20, 0x30]);
    }

    #[test]
    fn real_db_playable_and_costumes() {
        if !GameDb::available() {
            return;
        }
        let db = GameDb::open().unwrap();
        let p = db.playable().unwrap();
        assert_eq!(p.len(), 8, "{p:?}");
        assert_eq!(p[0].race, Race::Nuian);
        assert!(p[0].cdf.ends_with("nuian/male/nude/nu_m.cdf"), "{}", p[0].cdf);
        for m in &p {
            let c = db.login_costume(m.model_id).unwrap();
            assert!(!c.is_empty(), "{}: {c:?}", m.model_name);
            let face = db.base_face(m.model_id).unwrap();
            assert!(face.path.ends_with("face00.cdf"), "{}", face.path);
            let presets = db.presets(m.model_id, 1).unwrap();
            assert_eq!(presets.len(), 9, "{}", m.model_name);
            assert_eq!(presets[0].modifier.len(), 128);
            let hair = db.body_part(presets[0].hair_item).unwrap();
            assert_eq!(hair.model_id, m.model_id);
            let sk = db.skin_color(presets[0].skin_color_id).unwrap();
            assert_eq!(sk.model_id, m.model_id);
            let hc = db.hair_color(presets[0].hair_color_id).unwrap();
            // the preset's colour may belong to another style of the model; the model always matches
            assert_eq!(hc.model_id, m.model_id, "{}", m.model_name);
        }
    }
}
