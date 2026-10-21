//! Plain data types of the game protocol: characters, equipment, appearance, units, doodads,
//! movement and chat, with their wire encodings.
//!
//! Layouts follow the 1.2 server's `Character.Write`, `UnitCustomModelParams`, `Item.Write` and
//! `SCUnitStatePacket`. Nothing here depends on Bevy.

use crate::buff::Buff;
use crate::wire::{Pos, Reader, Result, WireError, Writer};

/// Race ids as the server stores them (`characters.race`).
pub mod race {
    pub const NUIAN: u8 = 1;
    pub const FAIRY: u8 = 2;
    pub const DWARF: u8 = 3;
    pub const ELF: u8 = 4;
    pub const HARIHARAN: u8 = 5;
    pub const FERRE: u8 = 6;
    pub const RETURNED: u8 = 7;
    pub const WARBORN: u8 = 8;

    /// Name for logs and UI fallbacks.
    pub fn name(race: u8) -> &'static str {
        match race {
            NUIAN => "Nuian",
            FAIRY => "Fairy",
            DWARF => "Dwarf",
            ELF => "Elf",
            HARIHARAN => "Hariharan",
            FERRE => "Ferre",
            RETURNED => "Returned",
            WARBORN => "Warborn",
            _ => "?",
        }
    }
}

pub mod gender {
    pub const MALE: u8 = 1;
    pub const FEMALE: u8 = 2;
}

/// Character archetype ids (`ability1` of a new character): 1..=10; [`NO_ABILITY`] is "none".
pub const NO_ABILITY: u8 = 11;

/// The experience table (`levels.total_exp` of the game database).
///
/// The app reads the table from the server's compact database once at startup and installs it with
/// [`install`]; [`FALLBACK`] is the original 1.2 curve, used when no database is available (tests,
/// a missing `AA_DB`). A server whose `levels` differs (e.g. the 3.0 retail curve for 51-55) is
/// followed, so the experience bar and the derived ability levels stay right above 50.
pub mod exp {
    use std::sync::OnceLock;

    /// Highest level of the 1.2 client.
    pub const MAX_LEVEL: u8 = 55;
    /// Number of entries of an [`ExpTable`] (levels 1..=[`MAX_LEVEL`]).
    pub const LEVELS: usize = MAX_LEVEL as usize;

    /// Total experience at which each level starts, indexed by `level - 1`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ExpTable {
        totals: [i64; LEVELS],
    }

    impl ExpTable {
        /// A table from the totals of levels 1..=[`LEVELS`] (`levels.total_exp`).
        pub const fn from_totals(totals: [i64; LEVELS]) -> Self {
            Self { totals }
        }

        /// The totals of levels 1..=[`MAX_LEVEL`].
        pub fn totals(&self) -> &[i64; LEVELS] {
            &self.totals
        }

        /// Total experience at which `level` starts (0 for level 1 and below; the last entry for
        /// anything above [`MAX_LEVEL`]).
        pub fn total_for_level(&self, level: u8) -> i64 {
            self.totals[usize::from(level.clamp(1, MAX_LEVEL)) - 1]
        }

        /// The level a total experience value corresponds to (abilities use the same table).
        pub fn level_for_total(&self, total: i64) -> u8 {
            self.totals.iter().rposition(|&t| t <= total).map_or(1, |i| i as u8 + 1)
        }

        /// `(earned, needed)` experience inside `level` for a character with `total` experience:
        /// the bar shows `earned / needed`. At the maximum level `needed` is 0.
        pub fn progress(&self, level: u8, total: i64) -> (i64, i64) {
            let level = level.clamp(1, MAX_LEVEL);
            if level == MAX_LEVEL {
                return ((total - self.total_for_level(level)).max(0), 0);
            }
            let start = self.total_for_level(level);
            ((total - start).clamp(0, i64::MAX), self.total_for_level(level + 1) - start)
        }
    }

    /// The 1.2 client's `levels.total_exp` (levels 1..=55).
    pub const FALLBACK: ExpTable = ExpTable::from_totals([
        0, 400, 1400, 3200, 6000, 10000, 15400, 22400, 31200, 42000, 55000, 70400, 88400, 109200,
        133000, 160000, 190400, 224400, 262200, 304000, 350000, 400400, 455400, 515200, 580000,
        650000, 725400, 806400, 893200, 986000, 1085000, 1190400, 1302400, 1421200, 1547000,
        1680000, 1820400, 1968400, 2124200, 2288000, 2460000, 2640400, 2829400, 3027200, 3234000,
        3450000, 3675400, 3910400, 4155200, 4410000, 8082000, 16698960, 36282960, 80346960,
        179307360,
    ]);

    static INSTALLED: OnceLock<ExpTable> = OnceLock::new();

    /// Installs the table the app read from the game database. The first table wins; returns false
    /// when one was already installed.
    pub fn install(table: ExpTable) -> bool {
        INSTALLED.set(table).is_ok()
    }

    /// The installed table, if the app installed one.
    pub fn installed() -> Option<&'static ExpTable> {
        INSTALLED.get()
    }

    /// The table in use: the installed game-database table, or [`FALLBACK`] when none is installed.
    pub fn table() -> &'static ExpTable {
        INSTALLED.get().unwrap_or(&FALLBACK)
    }

    /// Total experience at which `level` starts (see [`ExpTable::total_for_level`]).
    pub fn total_for_level(level: u8) -> i64 {
        table().total_for_level(level)
    }

    /// The level a total experience value corresponds to (see [`ExpTable::level_for_total`]).
    pub fn level_for_total(total: i64) -> u8 {
        table().level_for_total(total)
    }

    /// `(earned, needed)` experience inside `level` (see [`ExpTable::progress`]).
    pub fn progress(level: u8, total: i64) -> (i64, i64) {
        table().progress(level, total)
    }
}

// ---- equipment ------------------------------------------------------------------------------------

/// `EquipmentItemSlot` indices of [`Equipment::slots`].
pub mod slot {
    pub const HEAD: usize = 0;
    pub const NECK: usize = 1;
    pub const CHEST: usize = 2;
    pub const WAIST: usize = 3;
    pub const LEGS: usize = 4;
    pub const HANDS: usize = 5;
    pub const FEET: usize = 6;
    pub const ARMS: usize = 7;
    pub const BACK: usize = 8;
    pub const EAR1: usize = 9;
    pub const EAR2: usize = 10;
    pub const FINGER1: usize = 11;
    pub const FINGER2: usize = 12;
    pub const UNDERSHIRT: usize = 13;
    pub const UNDERPANTS: usize = 14;
    pub const MAINHAND: usize = 15;
    pub const OFFHAND: usize = 16;
    pub const RANGED: usize = 17;
    pub const MUSICAL: usize = 18;
    /// First body part slot (face); body parts carry only a template id.
    pub const FACE: usize = 19;
    pub const HAIR: usize = 20;
    pub const GLASSES: usize = 21;
    pub const RESERVED: usize = 22;
    pub const TAIL: usize = 23;
    pub const BODY: usize = 24;
    pub const BEARD: usize = 25;
    pub const BACKPACK: usize = 26;
    pub const COSPLAY: usize = 27;
    pub const COUNT: usize = 28;

    pub fn is_body_part(slot: usize) -> bool {
        (FACE..=BEARD).contains(&slot)
    }
}

/// Detail block of an equipment item (`detail type 1`, 55 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipDetail {
    /// Template whose look this item wears (look change), 0 for none.
    pub image_item_template_id: u32,
    pub durability: u8,
    pub rune_id: u32,
    pub charge_time: i64,
    pub dye_item_id: u32,
    pub gem_ids: [u32; 7],
    pub temper_physical: u16,
    pub temper_magical: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ItemDetail {
    /// Body parts, NPC gear and items without details.
    #[default]
    None,
    Equip(EquipDetail),
    /// Summoned mate: experience, dead flag, level.
    Mate { exp: i32, dead: u8, level: u8 },
    /// A detail type this client does not interpret; `Item::detail_payload` retains its bytes.
    Other(u8),
}

/// An item as it appears in equipment slots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub template_id: u32,
    /// Database id; 0 for body parts.
    pub id: u64,
    pub grade: u8,
    pub flags: u8,
    pub count: i32,
    pub detail: ItemDetail,
    /// Exact bytes after the detail tag, excluding fixed-block padding. Includes reserved bytes.
    /// `None` for synthetic/template-only items or an undecodable padded detail block.
    pub detail_payload: Option<Vec<u8>>,
    pub create_time: i64,
    pub lifespan_mins: i32,
    pub made_unit_id: u32,
    pub world_id: u8,
    pub unsecure_time: i64,
    pub unpack_time: i64,
}

impl Item {
    /// Only a template id (body parts).
    pub fn template_only(template_id: u32) -> Option<Self> {
        (template_id != 0).then(|| Self {
            template_id,
            ..Self::default()
        })
    }

    /// `Item.Write`: a template id, and for a real item everything else. `None` for template 0.
    pub fn read(r: &mut Reader) -> Result<Option<Self>> {
        let template_id = r.u32()?;
        if template_id == 0 {
            return Ok(None);
        }
        let id = r.u64()?;
        let grade = r.u8()?;
        let flags = r.u8()?;
        let count = r.i32()?;
        let detail_type = r.u8()?;
        let payload_start = r.rest();
        let detail = ItemDetail::read(r, detail_type)
            .map_err(|e| WireError::Invalid(format!("item {template_id}: {e}")))?;
        Ok(Some(Self {
            template_id,
            id,
            grade,
            flags,
            count,
            detail,
            detail_payload: Some(payload_start[..payload_start.len() - r.remaining()].to_vec()),
            create_time: r.i64()?,
            lifespan_mins: r.i32()?,
            made_unit_id: r.u32()?,
            world_id: r.u8()?,
            unsecure_time: r.i64()?,
            unpack_time: r.i64()?,
        }))
    }
}

impl ItemDetail {
    /// The original wire tag, without interpreting opaque union fields.
    pub fn kind(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Equip(_) => 1,
            Self::Mate { .. } => 3,
            Self::Other(kind) => *kind,
        }
    }

    /// The detail block that follows the detail type byte of an item.
    pub fn read(r: &mut Reader, detail_type: u8) -> Result<Self> {
        Ok(match detail_type {
            0 => Self::None,
            1 => Self::Equip(EquipDetail {
                image_item_template_id: r.u32()?,
                durability: r.u8()?,
                rune_id: {
                    r.skip(2)?;
                    r.u32()?
                },
                charge_time: r.i64()?,
                dye_item_id: r.u32()?,
                gem_ids: {
                    let mut g = [0u32; 7];
                    for v in &mut g {
                        *v = r.u32()?;
                    }
                    g
                },
                temper_physical: r.u16()?,
                temper_magical: r.u16()?,
            }),
            // Summoned slave: type, bc db id, destroyed flag, repair start (an `i32` 0 when
            // unset, else an `i64`), then five `i32`.
            2 => {
                r.skip(1 + 3 + 1)?;
                if r.clone().i32()? == 0 {
                    r.skip(4)?;
                } else {
                    r.skip(8)?;
                }
                r.skip(20)?;
                Self::Other(detail_type)
            }
            3 => Self::Mate {
                exp: r.i32()?,
                dead: r.u8()?,
                level: r.u8()?,
            },
            // Fixed-size details we do not interpret: ucc, treasure map, big fish, music sheet.
            4 => skip_detail(r, 9, detail_type)?,
            5 => skip_detail(r, 24, detail_type)?,
            // Native 3961edf0 also accepts tag 7 (16 bytes) and tag 9 (4 bytes).
            6 | 7 => skip_detail(r, 16, detail_type)?,
            8 => skip_detail(r, 8, detail_type)?,
            9 => skip_detail(r, 4, detail_type)?,
            other => return Err(WireError::Invalid(format!("unsupported item detail type {other}"))),
        })
    }
}

fn skip_detail(r: &mut Reader, n: usize, detail_type: u8) -> Result<ItemDetail> {
    r.skip(n)?;
    Ok(ItemDetail::Other(detail_type))
}

/// The 28 equipment slots of a unit (index with [`slot`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    pub slots: [Option<Item>; slot::COUNT],
}

/// How a unit packet encodes the non-body-part slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipFormat {
    /// Characters, mates, slaves, houses: every slot is `Item.Write`.
    Full,
    /// NPCs: template id, item id and grade; the cosplay slot is a full item.
    Npc,
}

impl Equipment {
    /// Reads the 28 slots. Body part slots (19..=25) are bare template ids in both formats.
    pub fn read(r: &mut Reader, format: EquipFormat) -> Result<Self> {
        let mut eq = Self::default();
        for (i, out) in eq.slots.iter_mut().enumerate() {
            *out = if slot::is_body_part(i) {
                Item::template_only(r.u32()?)
            } else if format == EquipFormat::Npc && i != slot::COSPLAY {
                let template_id = r.u32()?;
                if template_id == 0 {
                    None
                } else {
                    Some(Item {
                        template_id,
                        id: r.u64()?,
                        grade: r.u8()?,
                        ..Item::default()
                    })
                }
            } else {
                Item::read(r)?
            };
        }
        Ok(eq)
    }

    pub fn get(&self, slot: usize) -> Option<&Item> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    /// Template id per slot (0 for empty), handy for appearance lookups.
    pub fn template_ids(&self) -> [u32; slot::COUNT] {
        let mut out = [0u32; slot::COUNT];
        for (o, s) in out.iter_mut().zip(&self.slots) {
            *o = s.as_ref().map_or(0, |i| i.template_id);
        }
        out
    }
}

// ---- appearance -----------------------------------------------------------------------------------

/// Size of the face modifier blob (the character customizer's slider values).
pub const FACE_MODIFIER_LEN: usize = 128;

/// The face part of [`ModelParams`] (`FaceModel`).
#[derive(Clone, Debug, PartialEq)]
pub struct FaceModel {
    pub movable_decal_asset_id: u32,
    pub movable_decal_weight: f32,
    pub movable_decal_scale: f32,
    pub movable_decal_rotate: f32,
    pub movable_decal_move_x: i16,
    pub movable_decal_move_y: i16,
    /// `(asset id, weight)` of the four fixed decals.
    pub fixed_decals: [(u32, f32); 4],
    pub diffuse_map_id: u32,
    pub normal_map_id: u32,
    pub eyelash_map_id: u32,
    pub normal_map_weight: f32,
    pub lip_color: u32,
    pub left_pupil_color: u32,
    pub right_pupil_color: u32,
    pub eyebrow_color: u32,
    pub deco_color: u32,
    /// [`FACE_MODIFIER_LEN`] bytes of customizer data.
    pub modifier: Vec<u8>,
}

impl Default for FaceModel {
    fn default() -> Self {
        Self {
            movable_decal_asset_id: 0,
            movable_decal_weight: 0.0,
            movable_decal_scale: 0.0,
            movable_decal_rotate: 0.0,
            movable_decal_move_x: 0,
            movable_decal_move_y: 0,
            fixed_decals: [(0, 0.0); 4],
            diffuse_map_id: 0,
            normal_map_id: 0,
            eyelash_map_id: 0,
            normal_map_weight: 0.0,
            lip_color: 0,
            left_pupil_color: 0,
            right_pupil_color: 0,
            eyebrow_color: 0,
            deco_color: 0,
            modifier: vec![0; FACE_MODIFIER_LEN],
        }
    }
}

impl FaceModel {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            movable_decal_asset_id: r.u32()?,
            movable_decal_weight: r.f32()?,
            movable_decal_scale: r.f32()?,
            movable_decal_rotate: r.f32()?,
            movable_decal_move_x: r.i16()?,
            movable_decal_move_y: r.i16()?,
            fixed_decals: [
                (r.u32()?, r.f32()?),
                (r.u32()?, r.f32()?),
                (r.u32()?, r.f32()?),
                (r.u32()?, r.f32()?),
            ],
            diffuse_map_id: r.u32()?,
            normal_map_id: r.u32()?,
            eyelash_map_id: r.u32()?,
            normal_map_weight: r.f32()?,
            lip_color: r.u32()?,
            left_pupil_color: r.u32()?,
            right_pupil_color: r.u32()?,
            eyebrow_color: r.u32()?,
            deco_color: r.u32()?,
            modifier: r.blob()?.to_vec(),
        })
    }

    pub fn write(&self, w: &mut Writer) {
        w.u32(self.movable_decal_asset_id)
            .f32(self.movable_decal_weight)
            .f32(self.movable_decal_scale)
            .f32(self.movable_decal_rotate)
            .i16(self.movable_decal_move_x)
            .i16(self.movable_decal_move_y);
        for (asset, weight) in self.fixed_decals {
            w.u32(asset).f32(weight);
        }
        w.u32(self.diffuse_map_id)
            .u32(self.normal_map_id)
            .u32(self.eyelash_map_id)
            .f32(self.normal_map_weight)
            .u32(self.lip_color)
            .u32(self.left_pupil_color)
            .u32(self.right_pupil_color)
            .u32(self.eyebrow_color)
            .u32(self.deco_color)
            .blob(&self.modifier);
    }
}

/// `UnitCustomModelParams`: the appearance of a character, as set in the character creator.
///
/// `kind` gates the fields: 0 nothing, 1 hair color, 2 + skin color and model id, 3 + face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelParams {
    pub kind: u8,
    pub hair_color: u32,
    pub skin_color: u32,
    pub model_id: u32,
    pub face: Option<FaceModel>,
}

impl ModelParams {
    pub const NONE: u8 = 0;
    pub const HAIR: u8 = 1;
    pub const SKIN: u8 = 2;
    pub const FACE: u8 = 3;

    /// A full (kind 3) appearance with all-default values.
    pub fn default_face() -> Self {
        Self {
            kind: Self::FACE,
            face: Some(FaceModel::default()),
            ..Self::default()
        }
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        let mut p = Self {
            kind: r.u8()?,
            ..Self::default()
        };
        if p.kind < Self::HAIR {
            return Ok(p);
        }
        p.hair_color = r.u32()?;
        if p.kind < Self::SKIN {
            return Ok(p);
        }
        p.skin_color = r.u32()?;
        p.model_id = r.u32()?;
        if p.kind < Self::FACE {
            return Ok(p);
        }
        p.face = Some(FaceModel::read(r)?);
        Ok(p)
    }

    pub fn write(&self, w: &mut Writer) {
        w.u8(self.kind);
        if self.kind < Self::HAIR {
            return;
        }
        w.u32(self.hair_color);
        if self.kind < Self::SKIN {
            return;
        }
        w.u32(self.skin_color).u32(self.model_id);
        if self.kind < Self::FACE {
            return;
        }
        self.face.clone().unwrap_or_default().write(w);
    }
}

// ---- characters -----------------------------------------------------------------------------------

/// A character of the character list / `SCCreateCharacterResponse` (`Character.Write`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Character {
    pub id: u32,
    pub name: String,
    pub race: u8,
    pub gender: u8,
    pub level: u8,
    pub hp: i32,
    pub mp: i32,
    pub zone_id: u32,
    pub faction_id: u32,
    pub faction_name: String,
    pub expedition_id: u32,
    pub family: u32,
    pub equipment: Equipment,
    /// The three active ability (archetype) ids; [`NO_ABILITY`] for none.
    pub abilities: [u8; 3],
    pub pos: Pos,
    pub model_params: ModelParams,
    pub labor_power: i16,
    pub labor_power_modified: i64,
    pub dead_count: i16,
    pub dead_time: i64,
    pub rez_wait_duration: i32,
    pub rez_time: i64,
    pub rez_penalty_duration: i32,
    /// Unix time of the last logout.
    pub leave_time: i64,
    pub money: i64,
    pub crime_point: i16,
    /// Raw native crime-state word, following infamy points in Character serialization.
    pub crime_state: i16,
    pub infamy_point: i32,
    /// Unix times of a pending delete / transfer request (0 = none) and the delete delay.
    pub delete_request_time: i64,
    pub transfer_request_time: i64,
    pub delete_time: i64,
    pub consumed_labor_power: i32,
    /// Loyalty tokens.
    pub bm_point: i64,
    pub money2: i64,
    pub auto_use_aa_point: bool,
    pub prev_point: i32,
    pub point: i32,
    pub gift: i32,
    pub updated: i64,
    pub force_name_change: u8,
}

impl Character {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let mut c = Self {
            id: r.u32()?,
            name: r.string()?,
            race: r.u8()?,
            gender: r.u8()?,
            level: r.u8()?,
            hp: r.i32()?,
            mp: r.i32()?,
            zone_id: r.u32()?,
            faction_id: r.u32()?,
            faction_name: r.string()?,
            expedition_id: r.u32()?,
            family: r.u32()?,
            ..Self::default()
        };
        c.equipment = Equipment::read(r, EquipFormat::Full)?;
        c.abilities = [r.u8()?, r.u8()?, r.u8()?];
        c.pos = r.long_position()?;
        c.model_params = ModelParams::read(r)?;
        c.labor_power = r.i16()?;
        c.labor_power_modified = r.i64()?;
        c.dead_count = r.i16()?;
        c.dead_time = r.i64()?;
        c.rez_wait_duration = r.i32()?;
        c.rez_time = r.i64()?;
        c.rez_penalty_duration = r.i32()?;
        c.leave_time = r.i64()?;
        c.money = r.i64()?;
        r.skip(8)?; // moneyAmount
        c.crime_point = r.i16()?;
        c.infamy_point = r.i32()?;
        c.crime_state = r.i16()?;
        c.delete_request_time = r.i64()?;
        c.transfer_request_time = r.i64()?;
        c.delete_time = r.i64()?;
        c.consumed_labor_power = r.i32()?;
        c.bm_point = r.i64()?;
        c.money2 = r.i64()?;
        r.skip(8)?; // moneyAmount
        c.auto_use_aa_point = r.bool()?;
        c.prev_point = r.i32()?;
        c.point = r.i32()?;
        c.gift = r.i32()?;
        c.updated = r.i64()?;
        c.force_name_change = r.u8()?;
        Ok(c)
    }

    pub fn delete_requested(&self) -> bool {
        self.delete_request_time != 0
    }
}

/// What the server tells us when a character is selected (`SCCharacterStatePacket`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterState {
    pub instance_id: u32,
    pub guid: Vec<u8>,
    pub character: Character,
    /// Roll, pitch and yaw in radians.
    pub rotation: [f32; 3],
    pub experience: i32,
    pub recoverable_exp: i32,
    pub return_district_id: i32,
    pub resurrection_district_id: u32,
    pub num_inventory_slots: u8,
    pub num_bank_slots: i16,
    pub created: i64,
    /// Experience of each ability, general first (`0` = general, then the ten archetypes).
    pub ability_exp: [i32; 11],
    /// Unread mail: received, mia received, commercial.
    pub unread_mail: [i32; 3],
    pub jury_point: i32,
    pub jail_seconds:i32,
    pub bounty_money:i64,
    /// Raw native unsigned time field; no epoch interpretation is assumed.
    pub bounty_time:u64,
    /// Seconds played in total.
    pub play_time: i32,
    /// Expert-specialization expansion count; the final byte of SCCharacterState.
    pub expanded_expert: u8,
}

impl CharacterState {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let instance_id = r.u32()?;
        let guid = r.blob()?.to_vec();
        r.skip(4)?; // rwd
        let character = Character::read(r)?;
        let rotation = [r.f32()?, r.f32()?, r.f32()?];
        let experience = r.i32()?;
        let recoverable_exp = r.i32()?;
        r.skip(4)?; // penaltied exp
        let return_district_id = r.i32()?;
        r.skip(4)?; // return district
        let resurrection_district_id = r.u32()?;
        let mut ability_exp = [0i32; 11];
        for v in &mut ability_exp {
            *v = r.i32()?;
        }
        let unread_mail = [r.i32()?, r.i32()?, r.i32()?];
        let num_inventory_slots = r.u8()?;
        let num_bank_slots = r.i16()?;
        // money x2 (the character's own), two reserved i64, auto-use flag, ...
        r.skip(8 * 4 + 1)?;
        let jury_point = r.i32()?;
        let jail_seconds=r.i32()?;
        let bounty_money=r.i64()?;
        let bounty_time=r.u64()?;
        r.skip(4+4)?; // bot report counters
        let play_time = r.i32()?;
        let created = r.i64()?;
        let expanded_expert = r.u8()?;
        Ok(Self {
            instance_id,
            guid,
            character,
            rotation,
            experience,
            recoverable_exp,
            return_district_id,
            resurrection_district_id,
            num_inventory_slots,
            num_bank_slots,
            created,
            ability_exp,
            unread_mail,
            jury_point,
            jail_seconds,bounty_money,bounty_time,
            play_time,
            expanded_expert,
        })
    }
}

/// Request body of `CSCreateCharacterPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateCharacter {
    pub name: String,
    pub race: u8,
    pub gender: u8,
    /// Template ids of the body parts: face, hair, glasses, reserved, tail, body, beard.
    /// 0 lets the server pick the race / gender default.
    pub body_items: [u32; 7],
    pub model_params: ModelParams,
    pub abilities: [u8; 3],
    pub level: u8,
}

impl CreateCharacter {
    /// A valid character of `race` / `gender` with the server's default body parts, a default
    /// face and the first archetype.
    pub fn new(name: impl Into<String>, race: u8, gender: u8) -> Self {
        Self {
            name: name.into(),
            race,
            gender,
            body_items: [0; 7],
            model_params: ModelParams::default_face(),
            abilities: [1, NO_ABILITY, NO_ABILITY],
            level: 1,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        w.string(&self.name).u8(self.race).u8(self.gender);
        for b in self.body_items {
            w.u32(b);
        }
        self.model_params.write(w);
        w.u8(self.abilities[0])
            .u8(self.abilities[1])
            .u8(self.abilities[2])
            .u8(self.level);
    }
}

// ---- units ----------------------------------------------------------------------------------------

/// What kind of unit a `SCUnitStatePacket` announces, with its kind specific ids.
#[derive(Clone, Debug, PartialEq)]
pub enum UnitKind {
    Character { char_id: u32 },
    Npc { template_id: u32, owner_id: u32 },
    Slave { id: u32, tl_id: u16, template_id: u32, summoner_id: u32 },
    Housing { tl_id: u16, template_id: u32, build_step: i16 },
    Transfer { tl_id: u16, template_id: u32 },
    Mate { tl_id: u16, template_id: u32, owner_id: u32 },
    Shipyard { id: u64, template_id: u32 },
}

impl UnitKind {
    /// NPC / slave / house / ... template id (0 for characters).
    pub fn template_id(&self) -> u32 {
        match self {
            Self::Character { .. } => 0,
            Self::Npc { template_id, .. }
            | Self::Slave { template_id, .. }
            | Self::Housing { template_id, .. }
            | Self::Transfer { template_id, .. }
            | Self::Mate { template_id, .. }
            | Self::Shipyard { template_id, .. } => *template_id,
        }
    }
}

/// Radians per step of the unit-state rotation bytes (`2 pi / 127`).
const STATE_ROTATION_STEP: f32 = std::f32::consts::TAU / 127.0;
/// Radians per step of the movement direction byte (`2 pi / 128`).
const MOVE_DIRECTION_STEP: f32 = std::f32::consts::TAU / 128.0;

/// A unit in view (`SCUnitStatePacket`): characters, NPCs, mounts, houses...
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub obj_id: u32,
    pub name: String,
    pub kind: UnitKind,
    pub owner_name: String,
    pub pos: Pos,
    /// Roll, pitch and yaw in radians (yaw 0 faces +y, positive turns left).
    pub rotation: [f32; 3],
    pub scale: f32,
    pub level: u8,
    /// Model (`models` table id) the unit is rendered with.
    pub model_id: u32,
    pub equipment: Equipment,
    pub model_params: ModelParams,
    /// Current health and mana.
    pub hp: i32,
    pub mp: i32,
    /// The highest health and mana seen for this unit (the server sends no maximum; NPCs are
    /// announced at full health, so this is exact for them until they are healed above it).
    pub max_hp: i32,
    pub max_mp: i32,
    /// The unit is dead: announced without health (a corpse in view), or killed since (`UnitDeath`). Cleared by a
    /// resurrection or by health points above zero.
    pub dead: bool,
    /// `16 * gender + race`.
    pub race_gender: u8,
    /// Last movement received for this unit.
    pub movement: Option<Movement>,
    /// Faction id (`factions` table; 0 when the packet did not carry it). Hostile and friendly
    /// factions come from the faction relation packets, not from this value.
    pub faction_id: u32,
    pub expedition_id: u32,
    /// Id of the active appellation (title), 0 for none.
    pub appellation: u32,
    /// Unit flag bits as sent (`0x2000` on NPCs).
    pub flags: u16,
    /// First common UnitState PISC integer, expanded in native actor byte order.
    pub gm_modes: [u8; 4],
    /// Only genuine SCUnitBountyMoney reports hydrate this actor value.
    pub bounty_money:Option<i64>,
    /// Honor gained in combat and hostile faction kills (unit state, then SCUnitPvPPointsChanged).
    pub pvp_honor: i32,
    pub pvp_kills: i32,
    /// Characters: learned skills as `(skill id, level)`.
    pub skills: Vec<(u32, u8)>,
    /// Characters: ids of the passive buffs.
    pub passive_buffs: Vec<u32>,
    /// Characters: experience and order of each ability, in ability id order starting at 1
    /// (order 255 = not active).
    pub ability_exp: Vec<(i32, u8)>,
    /// Characters: the active ability (archetype) ids.
    pub active_abilities: Vec<u8>,
    /// Buffs on the unit when it was announced (later changes arrive as buff events).
    pub buffs: Vec<Buff>,
    /// `anim_actions.id` of the idle pose an NPC stands in (a merchant's, a guard's); 0 for none. Changes arrive as
    /// `Event::UnitPosture`.
    pub anim_action: u32,
    /// Received house posture; absent until a full unit tail or house posture report supplies it.
    pub house_posture: Option<crate::unit_msg::HousePosture>,
    /// Raw lane attachment fields in the unit-state snapshot, distinct from UnitAttached notifications.
    pub state_attachments: Option<UnitStateAttachments>,
    /// What the unit sits on (a mount, a vehicle, a ship) and where, set while `UnitAttached` is in force.
    pub attached_to: Option<Attachment>,
}

/// Non-character lane unit-state attachment fields. These do not imply a local rider/seat event.
/// Native first/second attachment serializers differ; no native layout inference is performed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitStateAttachments {
    pub first_point: u8,
    /// Slave owner object or transfer bonding object, only when the first point encodes one.
    pub first_obj_id: Option<u32>,
    pub bonding_obj_id: Option<u32>,
}

/// A rider's seat: the object it is attached to and the attach point (`1` driver, `2` and up passengers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub obj_id: u32,
    pub point: u8,
}

impl Unit {
    /// Apply source-backed received faction/membership changes to existing actors.
    /// Membership requires a matching persistent Character identity.
    pub fn apply_expedition(&mut self, event:&crate::ExpeditionEvent)->bool {
        use crate::ExpeditionEvent as E;
        match event {
            E::UnitFactionChanged{obj_id,old_faction_id,new_faction_id,..} => {
                if self.obj_id!=*obj_id || self.faction_id!=*old_faction_id || self.faction_id==*new_faction_id {return false;}
                self.faction_id=*new_faction_id;true
            }
            E::MembershipChanged{obj_id,character_id,new_expedition_id,..} => self.set_expedition(*obj_id,*character_id,*new_expedition_id),
            E::Created{faction,owner_obj_id,members} if faction.id!=0 && faction.political_system==0 => {
                let mut changed=self.set_expedition(*owner_obj_id,faction.owner_id,faction.id);
                for row in members.iter().take(4) {changed|=self.set_expedition(row.obj_id,row.character_id,faction.id);}
                changed
            }
            _=>false,
        }
    }
    fn set_expedition(&mut self,obj_id:u32,character_id:u32,new_expedition_id:u32)->bool {
        if character_id==0 || self.obj_id!=obj_id || !matches!(self.kind,UnitKind::Character{char_id} if char_id==character_id) {return false;}
        if self.expedition_id==new_expedition_id {return false;}
        self.expedition_id=new_expedition_id;true
    }

    /// SCUnitPvPPointsChanged: kind 0 honor gained in combat, kind 1 hostile faction kills.
    pub fn apply_pvp_points(&mut self, change: &crate::unit_msg::UnitPvpPointsChanged) -> bool {
        if self.obj_id != change.obj_id { return false; }
        let value = match change.kind {
            0 => &mut self.pvp_honor,
            1 => &mut self.pvp_kills,
            _ => return false,
        };
        if *value == change.point { return false; }
        *value = change.point;
        true
    }
    pub fn apply_gm_mode(&mut self, change: &crate::unit_msg::UnitGmModeChanged) -> bool {
        if self.obj_id != change.obj_id { return false; }
        let Ok(index) = usize::try_from(change.mode) else { return false };
        let Some(value) = self.gm_modes.get_mut(index) else { return false };
        if *value == change.value { return false; }
        *value = change.value;
        true
    }
    pub fn is_character(&self) -> bool {
        matches!(self.kind, UnitKind::Character { .. })
    }

    pub fn is_npc(&self) -> bool {
        matches!(self.kind, UnitKind::Npc { .. })
    }

    pub fn race(&self) -> u8 {
        self.race_gender & 0x0F
    }

    pub fn gender(&self) -> u8 {
        self.race_gender >> 4
    }

    pub fn yaw(&self) -> f32 {
        self.rotation[2]
    }

    /// `SCUnitStatePacket`. The core fields must decode; the trailing, kind specific fields
    /// (health, rotation, race) are read on a best-effort basis and left at their defaults when
    /// the packet ends or has an unexpected shape.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let obj_id = r.bc()?;
        let name = r.string()?;
        let kind_byte = r.u8()?;
        let (kind, format) = match kind_byte {
            0 => {
                let char_id = r.u32()?;
                r.skip(8)?;
                (UnitKind::Character { char_id }, EquipFormat::Full)
            }
            1 => {
                r.skip(3)?; // object id again
                let template_id = r.u32()?;
                let owner_id = r.u32()?;
                r.skip(1)?; // client driven
                (UnitKind::Npc { template_id, owner_id }, EquipFormat::Npc)
            }
            2 => (
                UnitKind::Slave {
                    id: r.u32()?,
                    tl_id: r.u16()?,
                    template_id: r.u32()?,
                    summoner_id: r.u32()?,
                },
                EquipFormat::Full,
            ),
            3 => (
                UnitKind::Housing {
                    tl_id: r.u16()?,
                    template_id: r.u32()?,
                    build_step: r.i16()?,
                },
                EquipFormat::Full,
            ),
            4 => (
                UnitKind::Transfer {
                    tl_id: r.u16()?,
                    template_id: r.u32()?,
                },
                EquipFormat::Full,
            ),
            5 => (
                UnitKind::Mate {
                    tl_id: r.u16()?,
                    template_id: r.u32()?,
                    owner_id: r.u32()?,
                },
                EquipFormat::Full,
            ),
            6 => (
                UnitKind::Shipyard {
                    id: r.u64()?,
                    template_id: r.u32()?,
                },
                EquipFormat::Full,
            ),
            other => return Err(WireError::Invalid(format!("unit kind {other}"))),
        };
        let owner_name = r.string()?;
        let pos = r.position()?;
        let scale = r.f32()?;
        let level = r.u8()?;
        let model_id = r.u32()?;
        // Transfers and shipyards carry exactly the seven bare body-part template IDs.
        let equipment = if matches!(kind_byte, 4 | 6) {
            let mut equipment = Equipment::default();
            for index in 19..=25 { equipment.slots[index] = Item::template_only(r.u32()?); }
            equipment
        } else { Equipment::read(r, format)? };
        let model_params = ModelParams::read(r)?;
        let mut unit = Self {
            obj_id,
            name,
            kind,
            owner_name,
            pos,
            rotation: [0.0; 3],
            scale,
            level,
            model_id,
            equipment,
            model_params,
            hp: 0,
            mp: 0,
            max_hp: 0,
            max_mp: 0,
            dead: false,
            race_gender: 0,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            bounty_money:None,
            pvp_honor: 0,
            pvp_kills: 0,
            skills: Vec::new(),
            passive_buffs: Vec::new(),
            ability_exp: Vec::new(),
            active_abilities: Vec::new(),
            buffs: Vec::new(),
            anim_action: 0,
            house_posture: None,
            state_attachments: None,
            attached_to: None,
        };
        // The lane's slave/house/transfer/shipyard tails have exact boundaries.
        // Reject truncation instead of treating their suffix as the next DD04 opcode.
        let tail_read = if matches!(kind_byte, 2 | 3 | 4 | 6) {
            unit.read_tail(r, kind_byte)?;
            true
        } else { unit.read_tail(r, kind_byte).is_ok() };
        (unit.max_hp, unit.max_mp) = (unit.hp, unit.mp);
        // a character or monster announced with no health is a corpse (a failed tail leaves the health at 0 as well)
        unit.dead = tail_read && unit.hp <= 0 && (unit.is_npc() || unit.is_character());
        Ok(unit)
    }

    /// bc, hp, mp, attach points, posture, weapon, learned skills, rotation, race / gender.
    fn read_tail(&mut self, r: &mut Reader, kind_byte: u8) -> Result<()> {
        r.skip(3)?;
        self.hp = r.i32()? / 100;
        self.mp = r.i32()? / 100;
        let attach1 = r.u8()?;
        let strict = matches!(kind_byte, 2 | 3 | 4 | 6);
        let first_obj_id = match kind_byte {
            2 if (attach1 as i8) > -1 => Some(r.bc()?),
            4 if attach1 != 255 => Some(r.bc()?),
            3 | 6 if attach1 != 255 => return Err(WireError::Invalid(format!("unit kind {kind_byte} first attach point {attach1}"))),
            0 | 2 | 3 | 4 | 6 => None,
            1 | 5 if attach1 == 255 => None,
            _ => return Ok(()),
        };
        // Second attachment: -1, a character's longer bond, or a noncharacter BC.
        let bond = r.u8()?;
        let bonding_obj_id = if bond != 0xff {
            if kind_byte == 0 { r.skip(12)?; None }
            else { Some(u32::from(bond) | (u32::from(r.u16()?) << 8)) }
        } else { None };
        if strict { self.state_attachments = Some(UnitStateAttachments { first_point: attach1, first_obj_id, bonding_obj_id }); }
        let posture = r.u8()?;
        let has_loot_left = r.bool()?;
        match posture {
            0 => {}
            1 => self.house_posture = Some(crate::unit_msg::HousePosture::read(r, has_loot_left)?),
            4 => {
                self.anim_action = r.u32()?;
                r.skip(1)?; // activate
            }
            7 => r.skip(14)?,
            8 => r.skip(8)?,
            other if strict => return Err(WireError::Invalid(format!("unit kind {kind_byte} model posture {other}"))),
            _ => return Ok(()),
        }
        r.skip(1)?; // active weapon
        if kind_byte == 0 {
            let skills = r.u8()? as usize;
            for _ in 0..skills {
                self.skills.push((r.u32()?, r.u8()?));
            }
            let passives = r.i32()?.max(0) as usize;
            for _ in 0..passives {
                self.passive_buffs.push(r.u32()?);
            }
        } else if strict {
            let skills = r.u8()?;
            let buffs = r.i32()?;
            if skills != 0 || buffs != 0 { return Err(WireError::Invalid(format!("unit kind {kind_byte} learned counts {skills}/{buffs}"))); }
        } else {
            r.skip(1 + 4)?; // learned skill and buff counts (always 0)
        }
        self.rotation = [
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
        ];
        self.race_gender = r.u8()?;
        self.read_extras(r, kind_byte)
    }

    /// After the race: appellation, faction, honor, flags, (characters: abilities and visual
    /// options) and the three buff lists.
    fn read_extras(&mut self, r: &mut Reader, kind_byte: u8) -> Result<()> {
        let common = r.pisc(4)?;
        self.gm_modes = (common[0] as u32).to_le_bytes();
        self.appellation = common[2] as u32;
        let [faction, expedition, ..] = r.pisc(4)?;
        (self.faction_id, self.expedition_id) = (faction as u32, expedition as u32);
        let pvp = r.pisc(2)?; // honor gained in combat, hostile faction kills
        (self.pvp_honor, self.pvp_kills) = (pvp[0] as i32, pvp[1] as i32);
        self.flags = r.u16()?;
        if kind_byte != 0 {
            self.buffs = Buff::read_lists(r)?;
            return Ok(());
        }
        // Characters: one `exp (i32), order (u8)` per ability, the active ability ids, a bc, the
        // visual options (6 byte stp + 4 flags), premium and 6 stat values, then the buffs. The
        // number of abilities is not on the wire (10 for the server's character model, 11 when
        // the general ability is listed): take the count that makes the packet end exactly.
        let start = r.clone();
        let mut first_ok = None;
        for n in [10usize, 11, 9, 12] {
            let mut t = start.clone();
            match Self::read_character_extras(&mut t, n) {
                Ok(x) if t.is_empty() => {
                    *r = t;
                    (self.ability_exp, self.active_abilities, self.buffs) = x;
                    return Ok(());
                }
                Ok(x) => {
                    first_ok.get_or_insert((t, x));
                }
                Err(_) => {}
            }
        }
        // Nothing fits exactly: keep the first parse that worked (trailing bytes are ignored).
        match first_ok {
            Some((t, x)) => {
                *r = t;
                (self.ability_exp, self.active_abilities, self.buffs) = x;
                Ok(())
            }
            None => Err(WireError::Invalid("character unit state tail".into())),
        }
    }

    #[allow(clippy::type_complexity)]
    fn read_character_extras(r: &mut Reader, abilities: usize) -> Result<(Vec<(i32, u8)>, Vec<u8>, Vec<Buff>)> {
        let mut exp = Vec::with_capacity(abilities);
        for _ in 0..abilities {
            exp.push((r.i32()?, r.u8()?));
        }
        let n = r.u8()? as usize;
        let mut active = Vec::with_capacity(n);
        for _ in 0..n {
            active.push(r.u8()?);
        }
        r.skip(3)?; // bc 0
        r.skip(6 + 4)?; // stp, helmet / back holdable / cosplay / cosplay backpack
        r.skip(4 + 6 * 4)?; // premium, six stat values
        let buffs = Buff::read_lists(r)?;
        Ok((exp, active, buffs))
    }
}

// ---- movement -------------------------------------------------------------------------------------

/// Stance byte of a unit movement (`GameStanceType`).
pub mod stance {
    pub const COMBAT: u8 = 0;
    pub const RELAXED: u8 = 1;
    pub const SWIM: u8 = 2;
    /// Swimming with a weapon drawn.
    pub const CO_SWIM: u8 = 3;
    pub const COMBAT2: u8 = 4;
    pub const STEALTH: u8 = 5;
    pub const CROUCH: u8 = 6;
    pub const PRONE: u8 = 7;
    pub const FLY: u8 = 8;
}

/// Movement flags (`MoveTypeFlags`).
pub mod move_flags {
    pub const MOVING: u8 = 0x02;
    /// "Braking": the last packet of a movement, or a unit standing still.
    pub const STOPPING: u8 = 0x04;
    pub const IN_COMBAT: u8 = 0x08;
    pub const HAS_SC_TYPE_AND_PHASE: u8 = 0x10;
    /// `Moving | Stopping`: the flags of a jump.
    pub const JUMPING: u8 = MOVING | STOPPING;
}

/// Actor flags of a unit movement (`MoveTypeActorFlags`).
pub mod actor_flags {
    /// Feet on terrain or a static collider.
    pub const STANDING_ON_SOLID: u8 = 0x04;
    /// In the air after a jump.
    pub const JUMPING: u8 = 0x10;
    /// Standing on a ship, vehicle or doodad; a block naming the object follows (not modelled).
    pub const STANDING_ON_OBJECT: u8 = 0x20;
    /// Hanging from an object; climb data follows (not modelled).
    pub const HANGING_FROM_OBJECT: u8 = 0x40;
    /// A `u16` fall velocity follows.
    pub const FALL_VELOCITY: u8 = 0x80;
}

/// Alertness byte of a unit movement (`MoveTypeAlertness`).
pub mod alertness {
    pub const IDLE: u8 = 0;
    pub const ALERT: u8 = 1;
    pub const COMBAT: u8 = 2;
}

/// Wire units of a fall velocity per m/s (the server maps 32000 to the fatal 63.25 m/s). A fall is
/// reported once, in the movement packet of the landing; the server ignores anything under 20 m/s.
pub const FALL_VELOCITY_PER_MPS: f32 = 32000.0 / 63.25;

/// A unit movement: what `CSMoveUnit` sends and `SCOneUnitMovement` broadcasts.
#[derive(Clone, Debug, PartialEq)]
pub struct Movement {
    /// Movement type (1 = unit; other types are vehicles and ships).
    pub move_type: u8,
    /// Sender's physics time in milliseconds.
    pub time: u32,
    /// [`move_flags`].
    pub flags: u8,
    pub pos: Pos,
    /// Velocity in 1/1024 m/s per axis.
    pub velocity: [i16; 3],
    /// Facing direction byte (`2 pi / 128` per step, positive turns left, 0 faces +y).
    pub direction: i8,
    /// Roll and pitch bytes of the rotation.
    pub tilt: [i8; 2],
    pub delta_movement: [i8; 3],
    pub stance: u8,
    /// [`alertness`] byte.
    pub alertness: u8,
    /// [`actor_flags`] byte.
    pub actor_flags: u8,
    /// Fall velocity in wire units ([`FALL_VELOCITY_PER_MPS`]); sent when `actor_flags` has
    /// [`actor_flags::FALL_VELOCITY`].
    pub fall_vel: u16,
    /// The object the unit stands on or hangs from (a ship deck, a vehicle, a mount): the blocks that follow the actor flags
    /// when [`actor_flags::STANDING_ON_OBJECT`] or [`actor_flags::HANGING_FROM_OBJECT`] is set.
    pub support: Option<MoveSupport>,
    /// Facing of a vehicle, ship or transfer movement in radians (they carry a rotation, not a direction byte).
    pub vehicle_yaw: Option<f32>,
}

/// What a unit stands on or hangs from, as a unit movement reports it. While it stands on something, the movement's `pos` is
/// relative to that object and [`MoveSupport::world`] is where the unit really is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoveSupport {
    /// Object id (`gcId`).
    pub obj_id: u32,
    /// Which id space `obj_id` is in (the lowest set bit wins: 1 unit, 2 gimmick, 4 doodad); 0 when only hanging.
    pub kind: u8,
    /// Part of the object (`gcPartId`).
    pub part: u16,
    /// World position and rotation bytes (roll, pitch, yaw), when standing on the object.
    pub world: Option<(Pos, [i8; 3])>,
    /// Climbing data, when hanging from the object.
    pub climb: Option<u32>,
}

impl Movement {
    /// A standing / walking unit movement at `pos` facing `yaw` radians.
    pub fn unit(pos: Pos, yaw: f32, time: u32) -> Self {
        Self {
            move_type: 1,
            time,
            flags: move_flags::MOVING,
            pos,
            velocity: [0; 3],
            direction: radians_to_direction(yaw),
            tilt: [0; 2],
            delta_movement: [0; 3],
            stance: stance::RELAXED,
            alertness: 0,
            actor_flags: actor_flags::STANDING_ON_SOLID,
            fall_vel: 0,
            support: None,
            vehicle_yaw: None,
        }
    }

    /// Facing in radians.
    pub fn yaw(&self) -> f32 {
        self.vehicle_yaw.unwrap_or_else(|| f32::from(self.direction) * MOVE_DIRECTION_STEP)
    }

    /// Body after the object id: type byte and `MoveType.Write`.
    pub fn write(&self, w: &mut Writer) {
        // The supporting-object and climbing blocks are not modelled: never announce them.
        let af = self.actor_flags & !(actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT);
        w.u8(self.move_type).u32(self.time).u8(self.flags);
        if self.flags & move_flags::HAS_SC_TYPE_AND_PHASE != 0 {
            w.u32(0).u8(0);
        }
        w.position(self.pos)
            .i16(self.velocity[0])
            .i16(self.velocity[1])
            .i16(self.velocity[2])
            .i8(self.tilt[0])
            .i8(self.tilt[1])
            .i8(self.direction)
            .i8(self.delta_movement[0])
            .i8(self.delta_movement[1])
            .i8(self.delta_movement[2])
            .u8(self.stance)
            .u8(self.alertness)
            .u8(af);
        if af & actor_flags::FALL_VELOCITY != 0 {
            w.u16(self.fall_vel);
        }
    }

    /// Reads the type byte and `MoveType.Write` (see [`Self::read_typed`]).
    pub fn read(r: &mut Reader) -> Result<Self> {
        let move_type = r.u8()?;
        Self::read_typed(move_type, r)
    }

    /// `MoveType.Write` of a movement of `move_type` (0 / 1 unit, 2 vehicle, 3 ship, 4 ship request, 5 transfer; other types
    /// have the layout of a unit's first fields). `SCUnitMovements` has no type byte per entry: the type is the object's.
    pub fn read_typed(move_type: u8, r: &mut Reader) -> Result<Self> {
        let time = r.u32()?;
        let flags = r.u8()?;
        if flags & move_flags::HAS_SC_TYPE_AND_PHASE != 0 {
            r.skip(5)?;
        }
        let mut m = Self {
            move_type,
            time,
            flags,
            pos: Pos::default(),
            velocity: [0; 3],
            direction: 0,
            tilt: [0; 2],
            delta_movement: [0; 3],
            stance: 0,
            alertness: 0,
            actor_flags: 0,
            fall_vel: 0,
            support: None,
            vehicle_yaw: None,
        };
        if move_type == 4 {
            // Ship request (throttle, steering): no position.
            r.skip(2)?;
            return Ok(m);
        }
        m.pos = r.position()?;
        if move_type == 5 {
            // Transfer: velocity / 50 as shorts, rotation quaternion (x, y, z as shorts), angular velocity, steering, path point,
            // speed, reverse.
            m.velocity = [r.i16()?, r.i16()?, r.i16()?].map(|v| (f32::from(v) / 32767.0 * 50.0 * 1024.0).clamp(-32767.0, 32767.0) as i16);
            m.vehicle_yaw = Some(quat_yaw(short_quat(r)?));
            r.skip(12 + 4 + 4 + 4 + 1)?;
            return Ok(m);
        }
        m.velocity = [r.i16()?, r.i16()?, r.i16()?];
        match move_type {
            2 | 3 => {
                // Vehicle and ship: rotation as three shorts (a quaternion without its scalar), angular velocity, then the
                // vehicle's steering (f32) and wheel speeds, or the ship's steering, throttle, zone and stuck flag.
                m.vehicle_yaw = Some(quat_yaw(short_quat(r)?));
                r.skip(12)?;
                if move_type == 2 {
                    r.skip(4)?;
                    let wheels = r.u8()?;
                    r.skip(usize::from(wheels) * 4)?;
                } else {
                    r.skip(1 + 1 + 2 + 1)?;
                }
            }
            _ => {
                m.tilt = [r.i8()?, r.i8()?];
                m.direction = r.i8()?;
                m.delta_movement = [r.i8()?, r.i8()?, r.i8()?];
                m.stance = r.u8()?;
                m.alertness = r.u8()?;
                m.actor_flags = r.u8()?;
                let af = m.actor_flags;
                if af & actor_flags::FALL_VELOCITY != 0 {
                    m.fall_vel = r.u16()?;
                }
                let mut support = MoveSupport::default();
                if af & actor_flags::STANDING_ON_OBJECT != 0 {
                    support.kind = r.u8()?;
                    support.part = r.u16()?;
                    let world = r.position()?;
                    support.world = Some((world, [r.i8()?, r.i8()?, r.i8()?]));
                }
                if af & (actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT) != 0 {
                    support.obj_id = r.u32()?;
                }
                if af & actor_flags::HANGING_FROM_OBJECT != 0 {
                    support.climb = Some(r.u32()?);
                }
                if af & (actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT) != 0 {
                    m.support = Some(support);
                }
            }
        }
        Ok(m)
    }
}

/// A rotation quaternion from three shorts (x, y, z; the scalar is rebuilt as the server does).
fn short_quat(r: &mut Reader) -> Result<[f32; 4]> {
    const SCALE: f32 = 1.0 / 32767.0;
    let [x, y, z] = [r.i16()?, r.i16()?, r.i16()?].map(|v| f32::from(v) * SCALE);
    let norm = x * x + y * y + z * z;
    let w = if norm < 0.9975 { (1.0 - f64::from(norm)).sqrt() as f32 } else { 0.0 };
    Ok([x, y, z, w])
}

/// The yaw (radians, 0 faces +y, positive turns left) of a rotation quaternion `[x, y, z, w]`.
fn quat_yaw([x, y, z, w]: [f32; 4]) -> f32 {
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// The direction byte for a yaw in radians (inverse of [`Movement::yaw`]).
pub fn radians_to_direction(yaw: f32) -> i8 {
    let step = MOVE_DIRECTION_STEP;
    let mut d = (yaw / step).round() as i32;
    d = d.rem_euclid(128);
    if d > 85 {
        d -= 128;
    }
    d as i8
}

// ---- doodads --------------------------------------------------------------------------------------

/// A doodad (static or interactive world object) in view.
#[derive(Clone, Debug, PartialEq)]
pub struct Doodad {
    pub obj_id: u32,
    pub template_id: u32,
    pub owner_obj_id: u32,
    pub parent_obj_id: u32,
    pub attach_point: u8,
    pub pos: Pos,
    /// Rotation quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub scale: f32,
    pub has_loot_item: bool,
    /// Current phase (function group).
    pub func_group_id: u32,
    pub owner_id: u32,
    pub ucc_id: u64,
    pub item_template_id: u32,
    pub type2: u32,
    /// Milliseconds remaining as last announced; the reducer does not invent a growth countdown.
    pub time_left: u32,
    pub plant_time: i64,
    pub quest_glow: u32,
    pub family: i32,
    pub puzzle_group: i32,
    pub owner_type: u8,
    pub owner_db_id: u32,
    pub data: i32,
}

impl Doodad {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let obj_id = r.bc()?;
        let template_id = r.u32()?;
        let owner_obj_id = r.bc()?;
        let parent_obj_id = r.bc()?;
        let attach_point = r.u8()?;
        let pos = r.position()?;
        let scale_short = |v: i16| f32::from(v) / f32::from(i16::MAX);
        let (x, y, z) = (scale_short(r.i16()?), scale_short(r.i16()?), scale_short(r.i16()?));
        let norm = x * x + y * y + z * z;
        let w = if norm < 1.0 { (1.0 - norm).sqrt() } else { 0.0 };
        Ok(Self {
            obj_id,
            template_id,
            owner_obj_id,
            parent_obj_id,
            attach_point,
            pos,
            rotation: [x, y, z, w],
            scale: r.f32()?,
            has_loot_item: r.bool()?,
            func_group_id: r.u32()?,
            owner_id: r.u32()?,
            ucc_id: r.u64()?,
            item_template_id: r.u32()?,
            type2: r.u32()?,
            time_left: r.u32()?,
            plant_time: r.i64()?,
            quest_glow: r.u32()?,
            family: r.i32()?,
            puzzle_group: r.i32()?,
            owner_type: r.u8()?,
            owner_db_id: r.u32()?,
            data: r.i32()?,
        })
    }
}

// ---- chat -----------------------------------------------------------------------------------------

/// Chat channels (`ChatType`).
pub mod chat_type {
    pub const WHISPERED: i16 = -4;
    pub const WHISPER: i16 = -3;
    pub const SYSTEM: i16 = -2;
    pub const NOTICE: i16 = -1;
    /// Say, to everyone nearby.
    pub const SAY: i16 = 0;
    pub const SHOUT: i16 = 1;
    pub const TRADE: i16 = 2;
    pub const GROUP_FIND: i16 = 3;
    pub const PARTY: i16 = 4;
    pub const RAID: i16 = 5;
    pub const REGION: i16 = 6;
    pub const CLAN: i16 = 7;
    pub const SYSTEM2: i16 = 8;
    pub const FAMILY: i16 = 9;
    pub const RAID_LEADER: i16 = 10;
    pub const JUDGE: i16 = 11;
    pub const ALLY: i16 = 14;
    /// A player created channel.
    pub const USER: i16 = 15;

    /// The channel's name, for logs and fallbacks (the UI has localized names).
    pub fn name(chat_type: i16) -> &'static str {
        match chat_type {
            WHISPERED => "Whispered",
            WHISPER => "Whisper",
            SYSTEM => "System",
            NOTICE => "Notice",
            SAY => "Say",
            SHOUT => "Shout",
            TRADE => "Trade",
            GROUP_FIND => "Looking for group",
            PARTY => "Party",
            RAID => "Raid",
            REGION => "Nation",
            CLAN => "Guild",
            SYSTEM2 => "System",
            FAMILY => "Family",
            RAID_LEADER => "Raid leader",
            JUDGE => "Judge",
            ALLY => "Ally",
            USER => "User channel",
            _ => "?",
        }
    }
}

/// `SCChatMessagePacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub chat_type: i16,
    /// The channel's sub type (the zone group id for shout / trade / group-find channels, 0 for
    /// say, whispers and system text) and faction: with `chat_type` they name the channel the
    /// message came through (see `SCJoinedChatChannel`).
    pub sub_type: i16,
    pub channel_faction: u32,
    /// Sender's object id and character id (0 for system messages).
    pub sender_obj_id: u32,
    pub sender_id: u32,
    pub sender_race: u8,
    pub faction_id: u32,
    pub sender: String,
    pub message: String,
    pub ability: i32,
}

impl ChatMessage {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let chat_type = r.i16()?;
        let sub_type = r.i16()?;
        let channel_faction = r.u32()?;
        let sender_obj_id = r.bc()?;
        let sender_id = r.u32()?;
        r.skip(1)?; // language
        let sender_race = r.u8()?;
        let faction_id = r.u32()?;
        let sender = r.string()?;
        let message = r.string()?;
        let ability = r.i32()?;
        Ok(Self {
            chat_type,
            sub_type,
            channel_faction,
            sender_obj_id,
            sender_id,
            sender_race,
            faction_id,
            sender,
            message,
            ability,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_character_default_matches_the_testclient_bytes() {
        // aaemu-testclient `create_character`: name, race 1, gender 1, seven zero body items, a
        // type 3 model with an all-zero face (128 byte modifier), ability 1 / 11 / 11, level 1.
        let mut expect = vec![2, 0, b'A', b'b', 1, 1];
        expect.extend([0u8; 28]);
        expect.push(3);
        expect.extend([0u8; 12]); // hair color, skin color, model id
        expect.extend([0u8; 4 + 12 + 4]); // decal asset, weight, scale, rotate, move x/y
        expect.extend([0u8; 4 * 8]); // fixed decals
        expect.extend([0u8; 12 + 4 + 20]); // maps, normal weight, five colors
        expect.extend([128, 0]);
        expect.extend([0u8; 128]);
        expect.extend([1, 11, 11, 1]);

        let mut w = Writer::new();
        CreateCharacter::new("Ab", race::NUIAN, gender::MALE).write(&mut w);
        assert_eq!(w.finish(), expect);
    }

    #[test]
    fn model_params_roundtrip_all_kinds() {
        let mut face = FaceModel {
            lip_color: 0x1122_3344,
            movable_decal_move_x: -3,
            ..FaceModel::default()
        };
        face.fixed_decals[2] = (9, 0.5);
        face.modifier[5] = 77;
        for params in [
            ModelParams::default(),
            ModelParams { kind: 1, hair_color: 0xAA, ..Default::default() },
            ModelParams { kind: 2, hair_color: 1, skin_color: 2, model_id: 3, face: None },
            ModelParams { kind: 3, hair_color: 1, skin_color: 2, model_id: 3, face: Some(face) },
        ] {
            let mut w = Writer::new();
            params.write(&mut w);
            let bytes = w.finish();
            let mut r = Reader::new(&bytes);
            assert_eq!(ModelParams::read(&mut r).unwrap(), params);
            assert!(r.is_empty());
        }
        // 1 + 4 + 8 + (16 + 4 + 32 + 16 + 20 + 2 + 128)
        let mut w = Writer::new();
        ModelParams::default_face().write(&mut w);
        assert_eq!(w.finish().len(), 1 + 4 + 8 + 16 + 4 + 32 + 16 + 20 + 2 + 128);
    }

    #[test]
    fn exact_detail_payload_includes_reserved_and_all_native_fixed_tags() {
        // Payload widths from native 3961edf0, independently of the parsed enum variants.
        for (kind, size) in [(0, 0), (1, 55), (2, 29), (3, 6), (4, 9), (5, 24),
                             (6, 16), (7, 16), (8, 8), (9, 4), (2, 33)] {
            let mut payload: Vec<u8> = (0..size).map(|n| n as u8 + 1).collect();
            if kind == 2 && size == 29 {
                payload[5..9].fill(0); // Lane's unset repair timestamp is four bytes.
            }
            let mut w = Writer::new();
            w.u32(123).u64(456).u8(2).u8(3).i32(4).u8(kind).raw(&payload);
            w.i64(10).i32(11).u32(12).u8(13).i64(14).i64(15).u32(0xaabbccdd);
            let bytes = w.finish();
            let mut r = Reader::new(&bytes);
            let item = Item::read(&mut r).unwrap().unwrap();
            assert_eq!(item.detail.kind(), kind);
            assert_eq!(item.detail_payload.as_deref(), Some(payload.as_slice()));
            assert_eq!((item.create_time, item.unpack_time), (10, 15));
            assert_eq!(r.u32().unwrap(), 0xaabbccdd);
            assert!(r.is_empty());
            for n in 4..bytes.len() - 4 {
                assert!(Item::read(&mut Reader::new(&bytes[..n])).is_err(), "tag {kind} cut {n}");
            }
        }
        assert_eq!(Item::template_only(1).unwrap().detail_payload, None);
    }

    fn equip_item(w: &mut Writer, template: u32, id: u64) {
        w.u32(template).u64(id).u8(2).u8(1).i32(1).u8(1);
        // 55 byte equipment detail
        w.u32(777).u8(90).i16(0).u32(5).i64(123).u32(42);
        for i in 0..7u32 {
            w.u32(i);
        }
        w.u16(3).u16(4);
        w.i64(10).i32(20).u32(30).u8(1).i64(40).i64(50);
    }

    #[test]
    fn full_item_and_equipment() {
        let mut w = Writer::new();
        equip_item(&mut w, 0x1000, 0x0102_0304_0506_0708);
        let bytes = w.finish();
        let item = Item::read(&mut Reader::new(&bytes)).unwrap().unwrap();
        assert_eq!(item.template_id, 0x1000);
        assert_eq!(item.id, 0x0102_0304_0506_0708);
        assert_eq!((item.grade, item.flags, item.count), (2, 1, 1));
        let ItemDetail::Equip(d) = &item.detail else { panic!("not equipment") };
        assert_eq!((d.image_item_template_id, d.durability, d.rune_id), (777, 90, 5));
        assert_eq!((d.charge_time, d.dye_item_id), (123, 42));
        assert_eq!(d.gem_ids[6], 6);
        assert_eq!((d.temper_physical, d.temper_magical), (3, 4));
        assert_eq!((item.create_time, item.lifespan_mins, item.world_id), (10, 20, 1));
        assert_eq!((item.unsecure_time, item.unpack_time), (40, 50));

        // 28 slots: chest is a full item, face / hair body parts only a template id.
        let mut w = Writer::new();
        for i in 0..slot::COUNT {
            match i {
                slot::CHEST => equip_item(&mut w, 0x1000, 1),
                slot::FACE => {
                    w.u32(4001);
                }
                slot::HAIR => {
                    w.u32(4002);
                }
                _ => {
                    w.u32(0);
                }
            }
        }
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let eq = Equipment::read(&mut r, EquipFormat::Full).unwrap();
        assert!(r.is_empty());
        assert_eq!(eq.get(slot::CHEST).unwrap().template_id, 0x1000);
        assert_eq!(eq.get(slot::FACE).unwrap().template_id, 4001);
        assert_eq!(eq.get(slot::HAIR).unwrap().id, 0);
        assert!(eq.get(slot::HEAD).is_none());
        assert_eq!(eq.template_ids()[slot::HAIR], 4002);
    }

    #[test]
    fn npc_equipment_uses_the_short_form() {
        let mut w = Writer::new();
        for i in 0..slot::COUNT {
            match i {
                slot::MAINHAND => {
                    w.u32(500).u64(9).u8(3);
                }
                slot::FACE => {
                    w.u32(4001);
                }
                _ => {
                    w.u32(0);
                }
            }
        }
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let eq = Equipment::read(&mut r, EquipFormat::Npc).unwrap();
        assert!(r.is_empty());
        let main = eq.get(slot::MAINHAND).unwrap();
        assert_eq!((main.template_id, main.id, main.grade), (500, 9, 3));
    }

    fn character_bytes() -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(77).string("Bob").u8(1).u8(2).u8(5).i32(1000).i32(500);
        w.u32(8).u32(101).string("").u32(0).u32(0);
        for i in 0..slot::COUNT {
            if i == slot::FACE {
                w.u32(4001);
            } else {
                w.u32(0);
            }
        }
        w.u8(1).u8(11).u8(11);
        w.long_position(Pos::new(100.5, 200.25, 30.0));
        ModelParams::default_face().write(&mut w);
        w.i16(50).i64(1).i16(0).i64(0).i32(0).i64(0).i32(0).i64(1234);
        w.i64(9999).i64(0).i16(0).i32(0).i16(-123);
        w.i64(0).i64(0).i64(0).i32(0).i64(11).i64(22).i64(0);
        w.bool(true).i32(0).i32(0).i32(0).i64(5).u8(0);
        w.finish()
    }

    #[test]
    fn character_decodes() {
        let bytes = character_bytes();
        let mut r = Reader::new(&bytes);
        let c = Character::read(&mut r).unwrap();
        assert!(r.is_empty(), "{} bytes left", r.remaining());
        assert_eq!((c.id, c.name.as_str(), c.race, c.gender, c.level), (77, "Bob", 1, 2, 5));
        assert_eq!((c.hp, c.mp, c.zone_id, c.faction_id), (1000, 500, 8, 101));
        assert_eq!(c.abilities, [1, 11, 11]);
        assert_eq!(c.pos, Pos::new(100.5, 200.25, 30.0));
        assert_eq!(c.equipment.get(slot::FACE).unwrap().template_id, 4001);
        assert_eq!((c.labor_power, c.leave_time, c.money), (50, 1234, 9999));
        assert_eq!((c.bm_point, c.money2, c.updated), (11, 22, 5));
        assert_eq!(c.crime_state,-123);
        assert!(c.auto_use_aa_point);
        assert!(!c.delete_requested());
    }

    #[test]
    fn character_state_decodes() {
        let mut w = Writer::new();
        w.u32(1).blob(&[7; 16]).i32(0);
        w.raw(&character_bytes());
        w.f32(0.0).f32(0.0).f32(1.5);
        w.i32(100).i32(10).u32(0).i32(0).u32(0).u32(3);
        for _ in 0..11 {
            w.u32(0);
        }
        w.i32(0).i32(0).i32(0).u8(50).i16(200);
        w.i64(1).i64(2).i64(0).i64(0).bool(true).i32(0).i32(i32::MIN).i64(i64::MIN).u64(u64::MAX);
        w.i32(0).i32(0).i32(600).i64(1_700_000_000).u8(7);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let s = CharacterState::read(&mut r).unwrap();
        assert!(r.is_empty(), "{} bytes left", r.remaining());
        assert_eq!((s.jail_seconds,s.bounty_money,s.bounty_time),(i32::MIN,i64::MIN,u64::MAX));
        assert_eq!(s.instance_id, 1);
        assert_eq!(s.guid, vec![7; 16]);
        assert_eq!(s.character.name, "Bob");
        assert_eq!(s.rotation, [0.0, 0.0, 1.5]);
        assert_eq!((s.experience, s.recoverable_exp), (100, 10));
        assert_eq!((s.num_inventory_slots, s.num_bank_slots), (50, 200));
        assert_eq!(s.created, 1_700_000_000);
        assert_eq!(s.expanded_expert, 7);
    }

    #[test]
    fn character_unit_state_decodes() {
        let mut w = Writer::new();
        w.bc(0x112233).string("Bob").u8(0).u32(77).i64(0).string("");
        w.position(Pos::new(10.0, 20.0, 5.0)).f32(1.0).u8(5).u32(1234);
        for i in 0..slot::COUNT {
            w.u32(if i == slot::HAIR { 4002 } else { 0 });
        }
        ModelParams::default_face().write(&mut w);
        w.bc(0).i32(100_000).i32(50_000).u8(255).i8(-1);
        w.u8(0).bool(false).u8(0); // posture, looted, weapon
        w.u8(1).u32(5).u8(3); // one skill
        w.i32(1).u32(99); // one passive buff
        w.i8(0).i8(0).i8(32).u8(0x12);
        let bytes = w.finish();
        let u = Unit::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(u.obj_id, 0x112233);
        assert_eq!(u.kind, UnitKind::Character { char_id: 77 });
        assert_eq!((u.level, u.model_id), (5, 1234));
        assert_eq!(u.equipment.get(slot::HAIR).unwrap().template_id, 4002);
        assert_eq!((u.hp, u.mp), (1000, 500));
        assert!((u.yaw() - 32.0 * std::f32::consts::TAU / 127.0).abs() < 1e-5);
        assert_eq!((u.race(), u.gender()), (2, 1));
        assert!((u.pos.x - 10.0).abs() < 0.01 && (u.pos.y - 20.0).abs() < 0.01);
    }

    /// The part of `SCUnitStatePacket` the character branch writes after the race byte: pisc
    /// groups, flags, abilities, visual options, premium, stats and the three buff lists.
    fn character_extras(w: &mut Writer, abilities: usize) {
        w.pisc(&[0, 0, 321, 0]); // appellation
        w.pisc(&[101, 7, 0, 0]); // faction, expedition
        w.pisc(&[300, 2]); // honor, hostile kills
        w.u16(0);
        for i in 0..abilities {
            w.i32(1000 * i as i32).u8(if i == 0 { 0 } else { 255 });
        }
        w.u8(1).u8(1); // one active ability: 1
        w.bc(0);
        w.raw(&[0; 6]).raw(&[1, 1, 1, 1]); // stp, visual options
        w.i32(1);
        for _ in 0..6 {
            w.i32(0);
        }
        w.u8(2); // good
        crate::buff::tests::net_buff(w, 2, 8_000_011);
        crate::buff::tests::net_buff(w, 3, 2423);
        w.u8(0); // bad
        w.u8(0); // hidden
    }

    #[test]
    fn character_unit_state_carries_faction_skills_abilities_and_buffs() {
        for abilities in [10usize, 11] {
            let mut w = Writer::new();
            w.bc(0x112233).string("Bob").u8(0).u32(77).i64(0).string("");
            w.position(Pos::new(10.0, 20.0, 5.0)).f32(1.0).u8(5).u32(1234);
            for _ in 0..slot::COUNT {
                w.u32(0);
            }
            ModelParams::default_face().write(&mut w);
            w.bc(0).i32(100_000).i32(50_000).u8(255).i8(-1);
            w.u8(0).bool(false).u8(0); // posture, looted, weapon
            w.u8(2).u32(5).u8(3).u32(9).u8(1); // two skills
            w.i32(1).u32(99); // one passive buff
            w.i8(0).i8(0).i8(32).u8(0x12);
            character_extras(&mut w, abilities);
            let bytes = w.finish();
            let u = Unit::read(&mut Reader::new(&bytes)).unwrap();
            assert_eq!((u.hp, u.mp, u.max_hp), (1000, 500, 1000));
            assert_eq!(u.skills, vec![(5, 3), (9, 1)], "{abilities} abilities");
            assert_eq!(u.passive_buffs, vec![99]);
            assert_eq!((u.faction_id, u.expedition_id, u.appellation), (101, 7, 321));
            assert_eq!(u.ability_exp.len(), abilities);
            assert_eq!(u.ability_exp[2], (2000, 255));
            assert_eq!(u.active_abilities, vec![1]);
            assert_eq!(u.buffs.len(), 2);
            assert_eq!(u.buffs[0].buff_id, 8_000_011);
            assert_eq!(u.buffs[1].index, 3);
        }
    }

    #[test]
    fn npc_unit_state_carries_faction_and_buffs() {
        let mut w = Writer::new();
        w.bc(0x000101).string("").u8(1);
        w.bc(0x000101).u32(5001).u32(0).u8(0);
        w.string("");
        w.position(Pos::new(1.0, 2.0, 3.0)).f32(1.0).u8(10).u32(321);
        for _ in 0..slot::COUNT {
            w.u32(0);
        }
        ModelParams { kind: 2, ..ModelParams::default() }.write(&mut w);
        w.bc(0).i32(20_000).i32(0).u8(255).i8(-1).u8(0).bool(false).u8(0);
        w.u8(0).i32(0).i8(0).i8(0).i8(-10).u8(0x21);
        w.pisc(&[0, 0, 0, 0]).pisc(&[161, 0, 0, 0]).pisc(&[300, 7]).u16(0x2000);
        w.u8(1);
        crate::buff::tests::net_buff(&mut w, 2, 4);
        w.u8(0).u8(0);
        let u = Unit::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!((u.name.as_str(), u.hp, u.max_hp, u.faction_id, u.flags), ("", 200, 200, 161, 0x2000));
        assert_eq!((u.pvp_honor, u.pvp_kills), (300, 7));
        assert_eq!(u.kind.template_id(), 5001);
        assert_eq!(u.buffs.len(), 1);
        assert_eq!(u.buffs[0].buff_id, 4);
    }

    #[test]
    fn npc_unit_state_decodes_and_survives_a_truncated_tail() {
        let mut w = Writer::new();
        w.bc(0x000101).string("Guard").u8(1);
        w.bc(0x000101).u32(5001).u32(0).u8(0);
        w.string("");
        w.position(Pos::new(1.0, 2.0, 3.0)).f32(1.0).u8(10).u32(321);
        for _ in 0..slot::COUNT {
            w.u32(0);
        }
        ModelParams { kind: 2, ..ModelParams::default() }.write(&mut w);
        let head = w.clone().finish();

        // Truncated right after the model: core fields only.
        let u = Unit::read(&mut Reader::new(&head)).unwrap();
        assert_eq!(u.kind, UnitKind::Npc { template_id: 5001, owner_id: 0 });
        assert_eq!((u.name.as_str(), u.level, u.model_id, u.hp), ("Guard", 10, 321, 0));

        // Full tail.
        w.bc(0).i32(20_000).i32(0).u8(255).i8(-1).u8(0).bool(false).u8(0);
        w.u8(0).i32(0).i8(0).i8(0).i8(-10).u8(0x21);
        let u = Unit::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!(u.hp, 200);
        assert_eq!(u.race_gender, 0x21);
        assert!(u.yaw() < 0.0);
    }

    #[test]
    fn movement_roundtrips_and_matches_the_testclient_layout() {
        let m = Movement::unit(Pos::new(100.0, 200.0, 30.0), 0.0, 700);
        let mut w = Writer::new();
        w.bc(0x000102);
        m.write(&mut w);
        let bytes = w.finish();
        // bc, type 1, time, flags 0x02, 9 position bytes, 3 x i16 velocity, 6 x i8, stance 1,
        // alertness 0, actor flags 0x04 (standing on solid ground).
        assert_eq!(bytes.len(), 3 + 1 + 4 + 1 + 9 + 6 + 6 + 3);
        assert_eq!(&bytes[..3], &[2, 1, 0]);
        assert_eq!(&bytes[3..9], &[1, 0xBC, 0x02, 0, 0, 0x02]);
        assert_eq!(bytes[bytes.len() - 3], 1);
        assert_eq!(bytes[bytes.len() - 1], actor_flags::STANDING_ON_SOLID);

        let mut r = Reader::new(&bytes);
        assert_eq!(r.bc().unwrap(), 0x102);
        let back = Movement::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(back.time, 700);
        assert!((back.pos.x - 100.0).abs() < 0.01);
        assert_eq!(back.stance, stance::RELAXED);
    }

    #[test]
    fn movement_state_bytes_follow_the_server_layout() {
        // `UnitMove.Read`: ... delta movement x3, stance, alertness, actor flags, then `fallVel`
        // (u16) when actor flag 0x80 is set.
        let mut m = Movement::unit(Pos::new(10.0, 20.0, 30.0), 0.0, 5);
        m.delta_movement = [-127, 127, 3];
        m.alertness = alertness::COMBAT;
        m.actor_flags = actor_flags::STANDING_ON_SOLID | actor_flags::FALL_VELOCITY;
        m.fall_vel = 0x1234;
        let mut w = Writer::new();
        m.write(&mut w);
        let bytes = w.finish();
        let tail = &bytes[bytes.len() - 8..];
        assert_eq!(tail, &[0x81, 127, 3, stance::RELAXED, 2, 0x84, 0x34, 0x12]);
        let back = Movement::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(back.delta_movement, [-127, 127, 3]);
        assert_eq!((back.alertness, back.fall_vel), (2, 0x1234));
        assert_eq!(back.actor_flags, 0x84);

        // The fall velocity block is only written when its flag is set, and the object / climb
        // blocks (not modelled) are never announced.
        m.actor_flags = actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT | actor_flags::JUMPING;
        let mut w = Writer::new();
        m.write(&mut w);
        let bytes = w.finish();
        assert_eq!(*bytes.last().unwrap(), actor_flags::JUMPING);
        assert_eq!(Movement::read(&mut Reader::new(&bytes)).unwrap().fall_vel, 0);
    }

    #[test]
    fn direction_bytes() {
        use std::f32::consts::FRAC_PI_2;
        assert_eq!(radians_to_direction(0.0), 0);
        assert_eq!(radians_to_direction(FRAC_PI_2), 32);
        assert_eq!(radians_to_direction(-FRAC_PI_2), -32);
        assert_eq!(radians_to_direction(std::f32::consts::PI), 64);
        let m = Movement::unit(Pos::default(), FRAC_PI_2, 0);
        assert!((m.yaw() - FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn doodad_decodes() {
        let mut w = Writer::new();
        w.bc(0x000555).u32(1234).bc(0).bc(0).u8(0);
        w.position(Pos::new(50.0, 60.0, 7.0));
        w.i16(0).i16(0).i16(i16::MAX).f32(1.0).bool(false);
        w.u32(11).u32(0).u64(0).u32(0).u32(0).u32(0).i64(0).u32(0).i32(0).i32(0).u8(0).u32(0).i32(0);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let d = Doodad::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((d.obj_id, d.template_id, d.func_group_id), (0x555, 1234, 11));
        assert!((d.rotation[2] - 1.0).abs() < 1e-3);
        assert!(d.rotation[3].abs() < 0.05);
    }

    #[test]
    fn chat_decodes() {
        let mut w = Writer::new();
        w.i16(1).i16(7).u32(0).bc(0x000102).u32(77).u8(0).u8(1).u32(101);
        w.string("Bob").string("hello").i32(0).i32(0);
        let bytes = w.finish();
        let c = ChatMessage::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!((c.chat_type, c.sender.as_str(), c.message.as_str()), (1, "Bob", "hello"));
        assert_eq!((c.sub_type, c.channel_faction), (7, 0));
        assert_eq!((c.sender_obj_id, c.sender_id, c.sender_race), (0x102, 77, 1));
    }
}

#[cfg(test)]
mod exp_tests {
    use super::exp::{ExpTable, FALLBACK, LEVELS, MAX_LEVEL};
    use std::path::PathBuf;

    /// A synthetic server table: the 1.2 curve with the 3.0 retail values for levels 51-55.
    fn table_3_0() -> ExpTable {
        let mut totals = *FALLBACK.totals();
        totals[50..].copy_from_slice(&[5_174_400, 6_550_320, 8_751_792, 12_164_073, 17_453_110]);
        ExpTable::from_totals(totals)
    }

    #[test]
    fn fallback_is_the_1_2_table() {
        assert_eq!(FALLBACK.totals().len(), LEVELS);
        assert_eq!(FALLBACK.total_for_level(1), 0);
        assert_eq!(FALLBACK.total_for_level(2), 400);
        assert_eq!(FALLBACK.total_for_level(50), 4_410_000);
        assert_eq!(FALLBACK.total_for_level(55), 179_307_360);
        assert_eq!(FALLBACK.level_for_total(179_307_360), 55);
        assert_eq!(FALLBACK.level_for_total(4_409_999), 49);
        // Before the app installs a table (tests, no database) the free functions use it.
        assert_eq!(super::exp::table().total_for_level(55), 179_307_360);
        assert_eq!(super::exp::level_for_total(4_410_000), 50);
    }

    #[test]
    fn custom_table_uses_the_3_0_values_at_51_to_55() {
        let table = table_3_0();
        for (level, total) in [
            (51, 5_174_400),
            (52, 6_550_320),
            (53, 8_751_792),
            (54, 12_164_073),
            (55, 17_453_110),
        ] {
            assert_eq!(table.total_for_level(level), total);
            assert_eq!(table.level_for_total(total), level);
            assert_eq!(table.level_for_total(total - 1), level - 1);
            assert_eq!(table.level_for_total(total + 1), level);
        }
        assert_eq!(table.level_for_total(4_410_000), 50);
        assert_eq!(table.level_for_total(17_453_109), 54);
        assert_eq!(table.level_for_total(i64::MAX), MAX_LEVEL);
        // Bar inside level 51: 5,174,400 .. 6,550,320 (1,375,920 long), 0 needed at 55.
        assert_eq!(table.progress(51, 5_174_400), (0, 1_375_920));
        assert_eq!(table.progress(51, 5_174_500), (100, 1_375_920));
        assert_eq!(table.progress(55, 17_453_110 + 5), (5, 0));
    }

    #[test]
    fn real_table_from_the_game_database() {
        let path = std::env::var("AA_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"D:\aaemu-live\rs-game\Data\compact.sqlite3"));
        if !path.exists() {
            return;
        }
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open compact.sqlite3");
        let mut query = conn
            .prepare("SELECT id, total_exp FROM levels WHERE id BETWEEN 1 AND 55 ORDER BY id")
            .expect("levels table");
        let rows = query
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .expect("query levels")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("read levels");
        assert_eq!(rows.len(), LEVELS);
        let mut totals = [0i64; LEVELS];
        for (id, total) in rows {
            assert!((1..=i64::from(MAX_LEVEL)).contains(&id));
            totals[(id - 1) as usize] = total;
        }
        let table = ExpTable::from_totals(totals);
        // Servers with the 3.0 patch store the retail totals for 51-55; older compacts keep 1.2's.
        if table.total_for_level(51) == 5_174_400 {
            assert_eq!(table.totals()[50..], [5_174_400, 6_550_320, 8_751_792, 12_164_073, 17_453_110]);
        }
        for level in 1..=MAX_LEVEL {
            assert_eq!(table.level_for_total(table.total_for_level(level)), level);
        }
    }
}

#[cfg(test)]
mod gm_snapshot_tests {
    use super::*;
    #[test]
    fn common_first_pisc_integer_hydrates_four_mode_bytes_without_reusing_u16_flags() {
        for word in [0u32, 0x04030201, 0xff00fe80, u32::MAX] {
            let mut w=crate::wire::Writer::new();
            w.pisc(&[i64::from(word),9,77,8]).pisc(&[13,14,0,0]).pisc(&[0,0]).u16(0x2000).raw(&[0,0,0]);
            let bytes=w.finish(); let mut r=Reader::new(&bytes);
            let mut unit=crate::state::tests::unit(7);
            unit.read_extras(&mut r,1).unwrap();
            assert!(r.is_empty()); assert_eq!(unit.gm_modes,word.to_le_bytes());
            assert_eq!((unit.appellation,unit.flags),(77,0x2000));
        }
    }
}
