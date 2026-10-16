//! Plain data types of the game protocol: characters, equipment, appearance, units, doodads,
//! movement and chat, with their wire encodings.
//!
//! Layouts follow the 1.2 server's `Character.Write`, `UnitCustomModelParams`, `Item.Write` and
//! `SCUnitStatePacket`. Nothing here depends on Bevy.

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
#[derive(Clone, Debug, Default, PartialEq)]
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

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ItemDetail {
    /// Body parts, NPC gear and items without details.
    #[default]
    None,
    Equip(EquipDetail),
    /// Summoned mate: experience, dead flag, level.
    Mate { exp: i32, dead: u8, level: u8 },
    /// A detail type this client does not interpret (its bytes were skipped).
    Other(u8),
}

/// An item as it appears in equipment slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub template_id: u32,
    /// Database id; 0 for body parts.
    pub id: u64,
    pub grade: u8,
    pub flags: u8,
    pub count: i32,
    pub detail: ItemDetail,
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
        let detail = match detail_type {
            0 => ItemDetail::None,
            1 => ItemDetail::Equip(EquipDetail {
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
            3 => ItemDetail::Mate {
                exp: r.i32()?,
                dead: r.u8()?,
                level: r.u8()?,
            },
            // Fixed-size details we do not interpret: ucc, treasure map, big fish, music sheet.
            4 => skip_detail(r, 9, detail_type)?,
            5 => skip_detail(r, 24, detail_type)?,
            6 => skip_detail(r, 16, detail_type)?,
            8 => skip_detail(r, 8, detail_type)?,
            other => {
                return Err(WireError::Invalid(format!(
                    "item {template_id}: unsupported detail type {other}"
                )))
            }
        };
        Ok(Some(Self {
            template_id,
            id,
            grade,
            flags,
            count,
            detail,
            create_time: r.i64()?,
            lifespan_mins: r.i32()?,
            made_unit_id: r.u32()?,
            world_id: r.u8()?,
            unsecure_time: r.i64()?,
            unpack_time: r.i64()?,
        }))
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
        r.skip(2)?; // crime score
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
        r.skip(11 * 4)?; // ability exp
        r.skip(3 * 4)?; // unread mail
        let num_inventory_slots = r.u8()?;
        let num_bank_slots = r.i16()?;
        // money x2, two reserved i64, auto-use flag, jury point, jail seconds, bounty money and
        // time, bot report counters, play time.
        r.skip(8 * 4 + 1 + 4 + 4 + 8 + 8 + 4 + 4 + 4)?;
        let created = r.i64()?;
        r.skip(1)?; // expanded expert
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
    /// `16 * gender + race`.
    pub race_gender: u8,
    /// Last movement received for this unit.
    pub movement: Option<Movement>,
}

impl Unit {
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
        // Transfers and shipyards only carry the body part slots: they have no usable equipment.
        let (equipment, model_params, tail_ok) = match kind_byte {
            4 | 6 => (Equipment::default(), ModelParams::default(), false),
            _ => {
                let eq = Equipment::read(r, format)?;
                (eq, ModelParams::read(r)?, true)
            }
        };
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
            race_gender: 0,
            movement: None,
        };
        if tail_ok {
            // Ignore failures: a short or unexpected tail only costs the optional fields.
            let _ = unit.read_tail(r, kind_byte);
        }
        Ok(unit)
    }

    /// bc, hp, mp, attach points, posture, weapon, learned skills, rotation, race / gender.
    fn read_tail(&mut self, r: &mut Reader, kind_byte: u8) -> Result<()> {
        r.skip(3)?;
        self.hp = r.i32()? / 100;
        self.mp = r.i32()? / 100;
        let attach1 = r.u8()?;
        // Slaves and transfers add an attach target to the first attach point; only characters,
        // NPCs and mates have the plain form decoded here.
        if !matches!(kind_byte, 0 | 1 | 5) || (attach1 != 255 && kind_byte != 0) {
            return Ok(());
        }
        // Second attach point: -1, or (characters) a 13 byte bond, or (NPCs) a bc.
        let bond = r.u8()?;
        if bond != 0xFF {
            r.skip(if kind_byte == 0 { 12 } else { 2 })?;
        }
        let posture = r.u8()?;
        r.skip(1)?; // looted flag
        match posture {
            0 => {}
            1 => r.skip(8)?,
            4 => r.skip(5)?,
            7 => r.skip(14)?,
            8 => r.skip(8)?,
            _ => return Ok(()),
        }
        r.skip(1)?; // active weapon
        if kind_byte == 0 {
            let skills = r.u8()? as usize;
            r.skip(skills * 5)?;
            let passives = r.i32()?.max(0) as usize;
            r.skip(passives * 4)?;
        } else {
            r.skip(1 + 4)?; // learned skill and buff counts (always 0)
        }
        self.rotation = [
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
            f32::from(r.i8()?) * STATE_ROTATION_STEP,
        ];
        self.race_gender = r.u8()?;
        Ok(())
    }
}

// ---- movement -------------------------------------------------------------------------------------

/// Stance byte of a unit movement (`GameStanceType`).
pub mod stance {
    pub const COMBAT: u8 = 0;
    pub const RELAXED: u8 = 1;
    pub const SWIM: u8 = 2;
    pub const COMBAT2: u8 = 4;
    pub const STEALTH: u8 = 5;
    pub const CROUCH: u8 = 6;
    pub const PRONE: u8 = 7;
    pub const FLY: u8 = 8;
}

/// Movement flags (`MoveTypeFlags`).
pub mod move_flags {
    pub const MOVING: u8 = 0x02;
    pub const STOPPING: u8 = 0x04;
    pub const IN_COMBAT: u8 = 0x08;
    pub const HAS_SC_TYPE_AND_PHASE: u8 = 0x10;
}

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
    pub alertness: u8,
    pub actor_flags: u8,
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
            actor_flags: 0,
        }
    }

    /// Facing in radians.
    pub fn yaw(&self) -> f32 {
        f32::from(self.direction) * MOVE_DIRECTION_STEP
    }

    /// Body after the object id: type byte and `MoveType.Write`.
    pub fn write(&self, w: &mut Writer) {
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
            .u8(self.actor_flags);
    }

    /// Reads the type byte and `MoveType.Write`. Unit movements are decoded in full; vehicle,
    /// ship and transfer movements only yield time, position and velocity.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let move_type = r.u8()?;
        let time = r.u32()?;
        let flags = r.u8()?;
        if flags & move_flags::HAS_SC_TYPE_AND_PHASE != 0 {
            r.skip(5)?;
        }
        if move_type == 4 {
            // Ship request: no position.
            return Ok(Self {
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
            });
        }
        let pos = r.position()?;
        let velocity = [r.i16()?, r.i16()?, r.i16()?];
        let mut m = Self {
            move_type,
            time,
            flags,
            pos,
            velocity,
            direction: 0,
            tilt: [0; 2],
            delta_movement: [0; 3],
            stance: 0,
            alertness: 0,
            actor_flags: 0,
        };
        if move_type == 1 || move_type == 0 {
            m.tilt = [r.i8()?, r.i8()?];
            m.direction = r.i8()?;
            m.delta_movement = [r.i8()?, r.i8()?, r.i8()?];
            m.stance = r.u8()?;
            m.alertness = r.u8()?;
            m.actor_flags = r.u8()?;
            // Optional blocks the actor flags announce are not needed here.
        }
        Ok(m)
    }
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
}

/// `SCChatMessagePacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub chat_type: i16,
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
        r.skip(2 + 4)?; // sub type, faction channel
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
        w.i64(9999).i64(0).i16(0).i32(0).i16(0);
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
        w.i64(1).i64(2).i64(0).i64(0).bool(true).i32(0).i32(0).i64(0).i64(0);
        w.i32(0).i32(0).i32(600).i64(1_700_000_000).u8(0);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let s = CharacterState::read(&mut r).unwrap();
        assert!(r.is_empty(), "{} bytes left", r.remaining());
        assert_eq!(s.instance_id, 1);
        assert_eq!(s.guid, vec![7; 16]);
        assert_eq!(s.character.name, "Bob");
        assert_eq!(s.rotation, [0.0, 0.0, 1.5]);
        assert_eq!((s.experience, s.recoverable_exp), (100, 10));
        assert_eq!((s.num_inventory_slots, s.num_bank_slots), (50, 200));
        assert_eq!(s.created, 1_700_000_000);
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
        // alertness 0, actor flags 0.
        assert_eq!(bytes.len(), 3 + 1 + 4 + 1 + 9 + 6 + 6 + 3);
        assert_eq!(&bytes[..3], &[2, 1, 0]);
        assert_eq!(&bytes[3..9], &[1, 0xBC, 0x02, 0, 0, 0x02]);
        assert_eq!(bytes[bytes.len() - 3], 1);

        let mut r = Reader::new(&bytes);
        assert_eq!(r.bc().unwrap(), 0x102);
        let back = Movement::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(back.time, 700);
        assert!((back.pos.x - 100.0).abs() < 0.01);
        assert_eq!(back.stance, stance::RELAXED);
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
        w.i16(0).i16(0).u32(0).bc(0x000102).u32(77).u8(0).u8(1).u32(101);
        w.string("Bob").string("hello").i32(0).i32(0);
        let bytes = w.finish();
        let c = ChatMessage::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!((c.chat_type, c.sender.as_str(), c.message.as_str()), (0, "Bob", "hello"));
        assert_eq!((c.sender_obj_id, c.sender_id, c.sender_race), (0x102, 77, 1));
    }
}
