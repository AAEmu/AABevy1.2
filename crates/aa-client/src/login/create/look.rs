//! The look being created: what the customizing screens edit, in the screen's own terms (1-based
//! indices into the per-model lists the client shows), and its translation to the game's data
//! (`CharacterDesc` for the preview, `CreateCharacter` for the server).
//!
//! The lists come from the game database and keep the client's order, which was checked against
//! the real captures: hair styles sorted by their icon number (`customizing_<race>_<g>/hair_NNN`),
//! hair colours by name (`custom01` .. `custom60`), decals / wrinkles by name, skins by name.
//! The default look of every race / gender (hair 10 / colour 6 for a Nuian male, 1 / 33 for an
//! Elf female, ...) is the first premade face, and matches the highlighted cells of
//! `reference/character_create/step2_*`.

use std::collections::HashMap;

use aa_anim::custom::Customization;
use aa_anim::db::{Decal, GameDb, Preset, model_id};
use aa_anim::morph::FaceMaker;
use aa_anim::{CharacterDesc, Costume, Gender, Race};
use rusqlite::{Connection, OpenFlags};

/// Size of the face modifier blob.
pub const MODIFIER_LEN: usize = 128;

/// `face_decal_assets.category_id`.
pub mod decal {
    pub const SCAR: u32 = 1;
    pub const TATTOO: u32 = 2;
    pub const MAKEUP: u32 = 3;
    pub const EYEBROW: u32 = 4;
    pub const DECO: u32 = 5;
}

/// The custom styles of the screen (`SetCustomizing<Name>` / `GetCustom<Name>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    Hair,
    HairColor,
    Eyebrow,
    EyebrowColor,
    PupilColor,
    MakeUp,
    LipColor,
    Deco,
    DecoColor,
    FaceDiffuse,
    SkinColor,
    FaceNormal,
    Tattoo,
    Scar,
}

impl Style {
    pub fn parse(name: &str) -> Option<Style> {
        Some(match name {
            "hair" => Style::Hair,
            "hair_color" => Style::HairColor,
            "eyebrow" => Style::Eyebrow,
            "eyebrow_color" => Style::EyebrowColor,
            "pupil_color" => Style::PupilColor,
            "makeup" => Style::MakeUp,
            "lip_color" => Style::LipColor,
            "deco" => Style::Deco,
            "deco_color" => Style::DecoColor,
            "diffuse" => Style::FaceDiffuse,
            "skin" => Style::SkinColor,
            "wrinkle" => Style::FaceNormal,
            "tattoo" => Style::Tattoo,
            "scar" => Style::Scar,
            _ => return None,
        })
    }
}

/// A hair style of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct HairStyle {
    /// `item_body_parts.item_id` (what the server stores as the hair item).
    pub item: u32,
    /// `item_assets.id` of its mesh: the key of its colours.
    pub asset: u32,
    /// Icon file below `ui/icon/` (`customizing_elf_f/hair_001.dds`).
    pub icon: String,
}

/// The selectable things of one race and gender, in screen order.
pub struct Catalog {
    pub race: Race,
    pub gender: Gender,
    pub model: u32,
    pub hair: Vec<HairStyle>,
    /// `face_decal_assets` ids.
    pub eyebrow: Vec<u32>,
    pub makeup: Vec<u32>,
    pub deco: Vec<u32>,
    pub tattoo: Vec<u32>,
    pub scar: Vec<u32>,
    /// `face_normal_maps` ids.
    pub wrinkle: Vec<u32>,
    /// `skin_colors` ids.
    pub skin: Vec<u32>,
    /// The nine premade faces (`total_character_customs`, owner 1) by name.
    pub presets: Vec<Preset>,
    /// `custom_face_presets` modifiers of the premade eyes, nose, mouth and head shape.
    pub parts: [Vec<Vec<u8>>; 4],
    /// The race's slider table (`<race>_<g>_targets.xml`).
    pub maker: FaceMaker,
    /// `item_body_parts.item_id` of the customisable base face.
    pub face_item: u32,
    /// Category of every decal id of the model.
    pub decal_category: HashMap<u32, u32>,
    /// `hair_colors_for` per hair style asset (a query of 60 rows, asked on every rebuild).
    hair_color_cache: std::sync::Mutex<HashMap<u32, std::sync::Arc<Vec<aa_anim::db::HairColor>>>>,
}

fn db_path() -> String {
    std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string())
}

/// The number in a name or file name (`hair_017.dds` -> 17), for the client's list order.
fn number_in(s: &str) -> u32 {
    let digits: String = s.chars().rev().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    digits.chars().rev().collect::<String>().parse().unwrap_or(u32::MAX)
}

impl Catalog {
    pub fn load(conn: &Connection, db: &GameDb, race: Race, gender: Gender) -> anyhow::Result<Catalog> {
        let model = model_id(race, gender);

        let mut st = conn.prepare(
            "SELECT b.item_id, b.asset_id, COALESCE(ic.filename, '') FROM item_body_parts b \
             JOIN items i ON i.id = b.item_id LEFT JOIN icons ic ON ic.id = i.icon_id \
             WHERE b.model_id = ?1 AND b.slot_type_id = 24 AND b.npc_only = 'f' AND b.beautyshop_only = 'f'",
        )?;
        let mut hair: Vec<HairStyle> = st
            .query_map([model], |r| Ok(HairStyle { item: r.get(0)?, asset: r.get(1)?, icon: r.get(2)? }))?
            .filter_map(|r| r.ok())
            .collect();
        hair.sort_by_key(|h| (number_in(&h.icon), h.item));

        // decals: everything of the category by name; only the scars are movable
        let mut decal_category = HashMap::new();
        let mut decals = |category: u32| -> anyhow::Result<Vec<u32>> {
            let mut st = conn.prepare(
                "SELECT id, name, movable = 't' FROM face_decal_assets WHERE model_id = ?1 AND category_id = ?2 ORDER BY name, id",
            )?;
            let rows: Vec<(u32, bool)> = st
                .query_map([model, category], |r| Ok((r.get(0)?, r.get(2)?)))?
                .filter_map(|r| r.ok())
                .collect();
            let wanted = category == decal::SCAR;
            let ids: Vec<u32> = rows.iter().filter(|(_, movable)| *movable == wanted).map(|(id, _)| *id).collect();
            for id in &ids {
                decal_category.insert(*id, category);
            }
            Ok(ids)
        };
        let scar = decals(decal::SCAR)?;
        let tattoo = decals(decal::TATTOO)?;
        let makeup = decals(decal::MAKEUP)?;
        let eyebrow = decals(decal::EYEBROW)?;
        let deco = decals(decal::DECO)?;

        let mut st = conn.prepare("SELECT id FROM face_normal_maps WHERE model_id = ?1 AND npc_only = 'f' ORDER BY name, id")?;
        let wrinkle: Vec<u32> = st.query_map([model], |r| r.get(0))?.filter_map(|r| r.ok()).collect();

        let skin: Vec<u32> = db.skin_colors(model)?.into_iter().map(|s| s.id).collect();
        let presets = db.presets(model, 1)?;

        let mut parts: [Vec<Vec<u8>>; 4] = Default::default();
        let mut st = conn.prepare("SELECT face_morph_type_id, modifier FROM custom_face_presets WHERE model_id = ?1 ORDER BY name, id")?;
        for row in st.query_map([model], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Vec<u8>>(1)?)))?.flatten() {
            if let Some(list) = parts.get_mut((row.0 as usize).wrapping_sub(1)) {
                list.push(row.1);
            }
        }

        let maker = FaceMaker::load(&format!(
            "objects/characters/{}/{}/face/{}_{}_targets.xml",
            race.folder(),
            gender.folder(),
            race.prefix(),
            gender.letter()
        ))?;
        let face_item = db.base_face(model)?.item_id;

        Ok(Catalog { race, gender, model, hair, eyebrow, makeup, deco, tattoo, scar, wrinkle, skin, presets, parts, maker, face_item, decal_category, hair_color_cache: Default::default() })
    }

    /// The colours of hair style `style` (1-based), in the DB's order.
    pub fn hair_colors_of(&self, db: &GameDb, style: usize) -> std::sync::Arc<Vec<aa_anim::db::HairColor>> {
        let Some(h) = self.hair.get(style.wrapping_sub(1)) else { return Default::default() };
        let mut cache = self.hair_color_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.entry(h.asset).or_insert_with(|| std::sync::Arc::new(db.hair_colors_for(self.model, h.asset).unwrap_or_default())).clone()
    }

    /// The sliders of a face part (1 eyes, 2 nose, 3 mouth, 4 head shape), in file order.
    pub fn targets_of(&self, part: usize) -> Vec<&aa_anim::morph::TargetDef> {
        let category = match part {
            1 => "eyes",
            2 => "nose",
            3 => "mouth",
            4 => "shape",
            _ => return Vec::new(),
        };
        self.maker.targets.iter().filter(|t| t.category.eq_ignore_ascii_case(category)).collect()
    }
}

/// The movable decal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scar {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rotate: f32,
}

impl Default for Scar {
    fn default() -> Self {
        Scar { index: 0, x: 0.0, y: 0.0, scale: 1.0, rotate: 0.0 }
    }
}

/// What the customizing screens edit (indices are 1-based, 0 = none).
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub hair: usize,
    pub hair_color: usize,
    pub skin: usize,
    pub wrinkle: (usize, f32),
    pub eyebrow: usize,
    pub makeup: (usize, f32),
    pub deco: (usize, f32),
    pub tattoo: usize,
    pub scar: Scar,
    /// `0xAABBGGRR`
    pub eyebrow_color: u32,
    pub pupil_color: u32,
    pub lip_color: u32,
    pub deco_color: u32,
    pub modifier: Vec<u8>,
    pub smile: bool,
}

impl Look {
    /// The look of a premade face: everything it sets.
    pub fn from_preset(cat: &Catalog, db: &GameDb, p: &Preset) -> Look {
        let hair = cat.hair.iter().position(|h| h.item == p.hair_item).map_or(0, |i| i + 1);
        let hair_color = cat.hair_colors_of(db, hair).iter().position(|c| c.id == p.hair_color_id).map_or(1, |i| i + 1);
        let skin = cat.skin.iter().position(|&s| s == p.skin_color_id).map_or(1, |i| i + 1);
        let pos = |list: &[u32], id: u32| list.iter().position(|&x| x == id).map_or(0, |i| i + 1);
        let mut look = Look {
            hair,
            hair_color,
            skin,
            wrinkle: (pos(&cat.wrinkle, p.face_normal_map_id), p.face_normal_map_weight),
            eyebrow: 0,
            makeup: (0, 1.0),
            deco: (0, 1.0),
            tattoo: 0,
            scar: Scar::default(),
            eyebrow_color: p.eyebrow_color,
            pupil_color: p.left_pupil_color,
            lip_color: p.lip_color,
            deco_color: p.deco_color,
            modifier: pad(&p.modifier),
            smile: false,
        };
        let movable = std::iter::once(p.movable_decal).chain(p.fixed_decals);
        for d in movable.filter(|d| d.asset_id != 0) {
            let weight = if d.weight > 0.0 { d.weight } else { 1.0 };
            match cat.decal_category.get(&d.asset_id).copied() {
                Some(decal::SCAR) => {
                    look.scar = Scar {
                        index: pos(&cat.scar, d.asset_id),
                        x: d.move_x as f32,
                        y: d.move_y as f32,
                        scale: if d.scale > 0.0 { d.scale } else { 1.0 },
                        rotate: d.rotate,
                    }
                }
                Some(decal::TATTOO) => look.tattoo = pos(&cat.tattoo, d.asset_id),
                Some(decal::MAKEUP) => look.makeup = (pos(&cat.makeup, d.asset_id), weight),
                Some(decal::EYEBROW) => look.eyebrow = pos(&cat.eyebrow, d.asset_id),
                Some(decal::DECO) => look.deco = (pos(&cat.deco, d.asset_id), weight),
                _ => {}
            }
        }
        look
    }

    /// The hair colour ids of the current hair style (60 of them).
    pub fn hair_colors(&self, cat: &Catalog, db: &GameDb) -> std::sync::Arc<Vec<aa_anim::db::HairColor>> {
        cat.hair_colors_of(db, self.hair)
    }

    /// The preview character of this look, in the login-stage costume.
    pub fn desc(&self, cat: &Catalog, db: &GameDb) -> CharacterDesc {
        let hair = cat.hair.get(self.hair.wrapping_sub(1));
        let hair_color = hair.and_then(|_| self.hair_colors(cat, db).get(self.hair_color.wrapping_sub(1)).map(|c| c.id));
        let id = |list: &[u32], i: usize| list.get(i.wrapping_sub(1)).copied();
        let mut decals = Vec::new();
        if let Some(asset_id) = id(&cat.scar, self.scar.index) {
            decals.push(Decal {
                asset_id,
                weight: 1.0,
                scale: self.scar.scale,
                rotate: self.scar.rotate,
                move_x: self.scar.x as i32,
                move_y: self.scar.y as i32,
            });
        }
        let mut fixed = |asset: Option<u32>, weight: f32| {
            if let Some(asset_id) = asset {
                decals.push(Decal { asset_id, weight, ..Default::default() });
            }
        };
        fixed(id(&cat.tattoo, self.tattoo), 1.0);
        fixed(id(&cat.makeup, self.makeup.0), self.makeup.1);
        fixed(id(&cat.eyebrow, self.eyebrow), 1.0);
        fixed(id(&cat.deco, self.deco.0), self.deco.1);
        let normal = id(&cat.wrinkle, self.wrinkle.0);
        CharacterDesc {
            race: cat.race,
            gender: cat.gender,
            face: None,
            hair: hair.map(|h| h.item),
            hair_color,
            skin: id(&cat.skin, self.skin).unwrap_or(1),
            costume: Costume::Login,
            customization: Customization {
                modifier: self.modifier.clone(),
                normal_map: normal.unwrap_or(0),
                normal_map_weight: if normal.is_some() { self.wrinkle.1 } else { 1.0 },
                lip_color: self.lip_color,
                left_pupil_color: self.pupil_color,
                right_pupil_color: self.pupil_color,
                eyebrow_color: self.eyebrow_color,
                deco_color: self.deco_color,
                decals,
            },
        }
    }

    /// Premade eyes / nose / mouth / head shape `index` (0-based): only the sliders of that part.
    pub fn apply_part(&mut self, cat: &Catalog, part: usize, index: usize) {
        let Some(preset) = cat.parts.get(part.wrapping_sub(1)).and_then(|l| l.get(index)) else { return };
        for t in cat.targets_of(part) {
            if let Some(m) = self.modifier.get_mut(t.idx) {
                *m = preset.get(t.idx).copied().unwrap_or(0);
            }
        }
    }

    /// Mixes part `part` (1..4) of the look with the same part of premade `index`: `value` 0..100 is
    /// the share of the premade one.
    pub fn mix_part(&mut self, cat: &Catalog, base: &Look, part: usize, index: usize, value: f32) {
        let Some(preset) = cat.parts.get(part.wrapping_sub(1)).and_then(|l| l.get(index)) else { return };
        let k = (value / 100.0).clamp(0.0, 1.0);
        for t in cat.targets_of(part) {
            let a = base.modifier.get(t.idx).copied().unwrap_or(0) as i8 as f32;
            let b = preset.get(t.idx).copied().unwrap_or(0) as i8 as f32;
            if let Some(m) = self.modifier.get_mut(t.idx) {
                *m = (a + (b - a) * k).round() as i8 as u8;
            }
        }
    }
}

fn pad(m: &[u8]) -> Vec<u8> {
    let mut v = m.to_vec();
    v.resize(MODIFIER_LEN, 0);
    v
}

/// The editor: the model's catalog, the current look and the bookkeeping the natives need.
pub struct Customizer {
    pub db: GameDb,
    conn: Connection,
    pub cat: Catalog,
    pub look: Look,
    /// "Keep Appearance": a premade face only sets the sliders.
    pub fix: bool,
    snapshots: HashMap<u32, Look>,
    /// The premade (0-based) the mix controls work with.
    mixed: usize,
    /// Bumped by every change of the look; the preview follows it.
    pub revision: u64,
}

impl Customizer {
    pub fn open(race: Race, gender: Gender) -> anyhow::Result<Customizer> {
        let db = GameDb::open().map_err(|e| anyhow::anyhow!("{e}"))?;
        let conn = Connection::open_with_flags(db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)?;
        let cat = Catalog::load(&conn, &db, race, gender)?;
        let look = Customizer::default_look(&cat, &db);
        Ok(Customizer { db, conn, cat, look, fix: false, snapshots: HashMap::new(), mixed: 0, revision: 1 })
    }

    fn default_look(cat: &Catalog, db: &GameDb) -> Look {
        match cat.presets.first() {
            Some(p) => Look::from_preset(cat, db, p),
            None => Look {
                hair: 1,
                hair_color: 1,
                skin: 1,
                wrinkle: (0, 1.0),
                eyebrow: 0,
                makeup: (0, 1.0),
                deco: (0, 1.0),
                tattoo: 0,
                scar: Scar::default(),
                eyebrow_color: 0,
                pupil_color: 0,
                lip_color: 0,
                deco_color: 0,
                modifier: vec![0; MODIFIER_LEN],
                smile: false,
            },
        }
    }

    /// A different race / gender: a fresh catalog and the first premade face.
    pub fn select(&mut self, race: Race, gender: Gender) -> anyhow::Result<()> {
        if (self.cat.race, self.cat.gender) == (race, gender) {
            return Ok(());
        }
        self.cat = Catalog::load(&self.conn, &self.db, race, gender)?;
        self.look = Customizer::default_look(&self.cat, &self.db);
        self.snapshots.clear();
        self.revision += 1;
        Ok(())
    }

    /// Back to the first premade face of the current model (a new character).
    pub fn reset(&mut self) {
        self.look = Customizer::default_look(&self.cat, &self.db);
        self.snapshots.clear();
        self.revision += 1;
    }

    pub fn desc(&self) -> CharacterDesc {
        self.look.desc(&self.cat, &self.db)
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    pub(super) fn conn(&self) -> &Connection {
        &self.conn
    }

    // ---- styles -----------------------------------------------------------------------------

    /// Number of entries of a list style.
    pub fn count(&self, style: Style) -> usize {
        match style {
            Style::Hair => self.cat.hair.len(),
            Style::HairColor => self.look.hair_colors(&self.cat, &self.db).len(),
            Style::Eyebrow => self.cat.eyebrow.len(),
            Style::MakeUp => self.cat.makeup.len(),
            Style::Deco => self.cat.deco.len(),
            Style::SkinColor => self.cat.skin.len(),
            Style::FaceNormal => self.cat.wrinkle.len(),
            Style::Tattoo => self.cat.tattoo.len(),
            Style::Scar => self.cat.scar.len(),
            _ => 0,
        }
    }

    /// `(index, weight)` of a list style.
    pub fn get(&self, style: Style) -> (usize, Option<f32>) {
        let l = &self.look;
        match style {
            Style::Hair => (l.hair, None),
            Style::HairColor => (l.hair_color, None),
            Style::Eyebrow => (l.eyebrow, None),
            Style::MakeUp => (l.makeup.0, Some(l.makeup.1)),
            Style::Deco => (l.deco.0, Some(l.deco.1)),
            Style::SkinColor => (l.skin, None),
            Style::FaceNormal => (l.wrinkle.0, Some(l.wrinkle.1)),
            Style::Tattoo => (l.tattoo, None),
            Style::Scar => (l.scar.index, None),
            _ => (0, None),
        }
    }

    pub fn set(&mut self, style: Style, index: usize, weight: Option<f32>) {
        let w = weight.unwrap_or(1.0).clamp(0.0, 1.0);
        let n = self.count(style);
        let index = if index > n { 0 } else { index };
        let l = &mut self.look;
        match style {
            Style::Hair => l.hair = index,
            Style::HairColor => l.hair_color = index.max(1),
            Style::Eyebrow => l.eyebrow = index,
            Style::MakeUp => l.makeup = (index, w),
            Style::Deco => l.deco = (index, w),
            Style::SkinColor => l.skin = index.max(1),
            Style::FaceNormal => l.wrinkle = (index, w),
            Style::Tattoo => l.tattoo = index,
            Style::Scar => l.scar.index = index,
            _ => return,
        }
        self.changed();
    }

    pub fn color(&self, style: Style) -> [f32; 3] {
        let c = match style {
            Style::EyebrowColor => self.look.eyebrow_color,
            Style::PupilColor => self.look.pupil_color,
            Style::LipColor => self.look.lip_color,
            Style::DecoColor => self.look.deco_color,
            _ => 0,
        };
        [(c & 0xff) as f32 / 255.0, ((c >> 8) & 0xff) as f32 / 255.0, ((c >> 16) & 0xff) as f32 / 255.0]
    }

    pub fn set_color(&mut self, style: Style, rgb: [f32; 3]) {
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        let c = 0xff00_0000 | (b(rgb[2]) << 16) | (b(rgb[1]) << 8) | b(rgb[0]);
        match style {
            Style::EyebrowColor => self.look.eyebrow_color = c,
            Style::PupilColor => self.look.pupil_color = c,
            Style::LipColor => self.look.lip_color = c,
            Style::DecoColor => self.look.deco_color = c,
            _ => return,
        }
        self.changed();
    }

    pub fn set_scar(&mut self, index: usize, x: f32, y: f32, scale: f32, rotate: f32) {
        let index = if index > self.cat.scar.len() { 0 } else { index };
        self.look.scar = Scar { index, x, y, scale, rotate };
        self.changed();
    }

    // ---- face sliders -----------------------------------------------------------------------

    /// `(min, max)` of slider `idx`.
    pub fn slider_range(&self, idx: usize) -> (i32, i32) {
        self.cat.maker.targets.iter().find(|t| t.idx == idx).map_or((-100, 100), |t| (t.min, t.max))
    }

    pub fn slider_name(&self, idx: usize) -> Option<&str> {
        self.cat.maker.targets.iter().find(|t| t.idx == idx).map(|t| t.name.as_str())
    }

    pub fn slider(&self, idx: usize) -> i32 {
        self.look.modifier.get(idx).map_or(0, |&b| b as i8 as i32)
    }

    pub fn set_slider(&mut self, idx: usize, value: f32) {
        if idx >= MODIFIER_LEN {
            return;
        }
        let (min, max) = self.slider_range(idx);
        let v = (value.round() as i32).clamp(min, max);
        if self.look.modifier[idx] != v as i8 as u8 {
            self.look.modifier[idx] = v as i8 as u8;
            self.changed();
        }
    }

    // ---- premade faces ----------------------------------------------------------------------

    /// `SetCustomizingParam(part, index, fixed)`: part 0 = the whole premade face, 1 eyes, 2 nose,
    /// 3 mouth, 4 head shape; a negative index only refreshes.
    pub fn apply_preset(&mut self, part: usize, index: i32, fixed: bool) {
        if index < 0 {
            self.changed();
            return;
        }
        let index = index as usize;
        if part == 0 {
            let Some(p) = self.cat.presets.get(index) else { return };
            let full = Look::from_preset(&self.cat, &self.db, p);
            if fixed {
                self.look.modifier = full.modifier;
            } else {
                self.look = full;
            }
        } else {
            let cat = &self.cat;
            self.look.apply_part(cat, part, index);
        }
        self.changed();
    }

    pub fn save_slot(&mut self, slot: u32) {
        self.snapshots.insert(slot, self.look.clone());
    }

    pub fn load_slot(&mut self, slot: u32) {
        if let Some(l) = self.snapshots.get(&slot) {
            self.look = l.clone();
            self.changed();
        }
    }

    /// `SetMixedParam(index)`: the premade the mix works with (1-based).
    pub fn set_mixed(&mut self, index: usize) {
        self.mixed = index.saturating_sub(1);
    }

    /// `MixParam(type, value, random)`: `part` 0 mixes the whole face, else one part.
    pub fn mix(&mut self, part: usize, value: f32) {
        let base = self.snapshots.get(&0).cloned().unwrap_or_else(|| self.look.clone());
        let cat = &self.cat;
        let mut look = self.look.clone();
        if part == 0 {
            if let Some(p) = cat.presets.get(self.mixed) {
                let preset = pad(&p.modifier);
                let k = (value / 100.0).clamp(0.0, 1.0);
                for i in 0..MODIFIER_LEN {
                    let a = base.modifier[i] as i8 as f32;
                    let b = preset[i] as i8 as f32;
                    look.modifier[i] = (a + (b - a) * k).round() as i8 as u8;
                }
            }
        } else {
            look.mix_part(cat, &base, part, self.mixed, value);
        }
        self.look = look;
        self.changed();
    }

    /// What `CSCreateCharacter` carries for this look: body parts (face, hair), the appearance
    /// params (hair / skin colour, decals, maps, colours, the 128 slider bytes) and the skillset.
    pub fn create_request(&self, name: &str, ability: u32) -> aa_net::model::CreateCharacter {
        use aa_net::model::{CreateCharacter, FaceModel, ModelParams, NO_ABILITY, gender, race};
        let cat = &self.cat;
        let l = &self.look;
        let id = |list: &[u32], i: usize| list.get(i.wrapping_sub(1)).copied().unwrap_or(0);
        let hair = cat.hair.get(l.hair.wrapping_sub(1));
        let hair_color = self.look.hair_colors(cat, &self.db).get(l.hair_color.wrapping_sub(1)).map_or(0, |c| c.id);
        let mut face = FaceModel {
            modifier: l.modifier.clone(),
            lip_color: l.lip_color,
            left_pupil_color: l.pupil_color,
            right_pupil_color: l.pupil_color,
            eyebrow_color: l.eyebrow_color,
            deco_color: l.deco_color,
            ..FaceModel::default()
        };
        face.modifier.resize(MODIFIER_LEN, 0);
        face.movable_decal_asset_id = id(&cat.scar, l.scar.index);
        if face.movable_decal_asset_id != 0 {
            face.movable_decal_weight = 1.0;
            face.movable_decal_scale = l.scar.scale;
            face.movable_decal_rotate = l.scar.rotate;
            face.movable_decal_move_x = l.scar.x as i16;
            face.movable_decal_move_y = l.scar.y as i16;
        }
        // the four fixed decals are tattoo, makeup, eyebrow and deco, in the order of their categories
        face.fixed_decals = [
            (id(&cat.tattoo, l.tattoo), 1.0),
            (id(&cat.makeup, l.makeup.0), l.makeup.1),
            (id(&cat.eyebrow, l.eyebrow), 1.0),
            (id(&cat.deco, l.deco.0), l.deco.1),
        ];
        face.normal_map_id = id(&cat.wrinkle, l.wrinkle.0);
        face.normal_map_weight = if face.normal_map_id != 0 { l.wrinkle.1 } else { 0.0 };
        let race_id = match cat.race {
            Race::Nuian => race::NUIAN,
            Race::Elf => race::ELF,
            Race::Hariharan => race::HARIHARAN,
            Race::Ferre => race::FERRE,
        };
        let gender_id = if cat.gender == Gender::Female { gender::FEMALE } else { gender::MALE };
        let mut req = CreateCharacter::new(name, race_id, gender_id);
        // face, hair, glasses, reserved, tail, body, beard (0 = the race's default)
        req.body_items = [cat.face_item, hair.map_or(0, |h| h.item), 0, 0, 0, 0, 0];
        req.model_params = ModelParams {
            kind: ModelParams::FACE,
            hair_color,
            skin_color: id(&cat.skin, l.skin),
            model_id: cat.model,
            face: Some(face),
        };
        req.abilities = [ability.min(u32::from(NO_ABILITY)) as u8, NO_ABILITY, NO_ABILITY];
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "timing probe, needs the game data"]
    fn resolve_cost() {
        let cz = Customizer::open(Race::Nuian, Gender::Male).unwrap();
        let desc = cz.desc();
        for i in 0..6 {
            let t = web_time::Instant::now();
            let a = desc.resolve(&cz.db).unwrap();
            println!("resolve {i}: {:.1} ms ({} parts)", t.elapsed().as_secs_f32() * 1000.0, a.parts.len());
        }
        let mut plain = desc.clone();
        plain.customization = Default::default();
        for i in 0..3 {
            let t = web_time::Instant::now();
            plain.resolve(&cz.db).unwrap();
            println!("resolve (no customization) {i}: {:.1} ms", t.elapsed().as_secs_f32() * 1000.0);
        }
    }
}
