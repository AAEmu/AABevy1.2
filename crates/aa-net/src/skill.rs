//! Skills: who casts what at whom, and what came of it.
//!
//! Layouts follow the server's `systems/skills/packets.rs` and `types.rs` (`SCSkillStarted`,
//! `SCSkillFired`, `SCUnitDamaged`, `SCUnitHealed`, `SCPlotEvent` ...) and the reader of
//! `CSStartSkill`. Cast/stop times use 10 ms wire units (some with an offset of 10);
//! exposed values are milliseconds. Native `SCCooldowns` entries already use milliseconds.

mod detail;
#[cfg(test)]
pub(crate) mod damage_tests;
pub use detail::{SkillCasterData,SkillTargetData,SkillPosition};

use crate::buff::{caster_kind, BuffCaster};
use crate::wire::{Pos, Reader, Result, Writer};

/// Native fixed capacity of each skill/tag cooldown list.
pub const COOLDOWN_LIMIT: usize = 150;

/// One native cooldown entry, in its original wire order. No timestamp or scaling is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cooldown {
    /// Skill ID in `SkillCooldowns::skills`, cooldown-tag ID in `SkillCooldowns::tags`.
    pub id: u32,
    pub total_ms: u32,
    pub remaining_ms: u32,
}

/// `SCCooldowns` (0x04a), received for the current player. Empty lists do not mean reset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillCooldowns {
    pub skills: Vec<Cooldown>,
    pub tags: Vec<Cooldown>,
}

impl SkillCooldowns {
    pub fn read(r: &mut Reader) -> Result<Self> {
        fn list(r: &mut Reader) -> Result<Vec<Cooldown>> {
            let count = r.i32()?;
            if !(0..=COOLDOWN_LIMIT as i32).contains(&count) {
                return Err(crate::wire::WireError::Invalid(format!(
                    "cooldown count {count} outside native capacity 0..={COOLDOWN_LIMIT}"
                )));
            }
            (0..count)
                .map(|_| {
                    Ok(Cooldown {
                        id: r.u32()?,
                        total_ms: r.u32()?,
                        remaining_ms: r.u32()?,
                    })
                })
                .collect()
        }
        Ok(Self {
            skills: list(r)?,
            tags: list(r)?,
        })
    }
}

/// `SkillCastTargetType`.
pub mod target_kind {
    pub const UNIT: u8 = 0;
    pub const POSITION: u8 = 1;
    pub const POSITION2: u8 = 2;
    pub const ITEM: u8 = 3;
    pub const DOODAD: u8 = 4;
    pub const POSITION3: u8 = 5;
}

/// `SkillResult`: why a skill did not start (`SkillStarted::result`, 0 = it started).
pub mod result {
    pub const SUCCESS: u8 = 0x00;
    pub const FAILURE: u8 = 0x01;
    pub const SOURCE_DIED: u8 = 0x02;
    pub const SOURCE_ALIVE: u8 = 0x03;
    pub const TARGET_DIED: u8 = 0x04;
    pub const TARGET_DESTROYED: u8 = 0x05;
    pub const TARGET_ALIVE: u8 = 0x06;
    pub const ON_CASTING: u8 = 0x07;
    pub const COOLDOWN_TIME: u8 = 0x08;
    pub const NO_TARGET: u8 = 0x09;
    pub const LACK_HEALTH: u8 = 0x0A;
    pub const LACK_MANA: u8 = 0x0B;
    pub const OBSTACLE: u8 = 0x0C;
    pub const OUTOF_HEIGHT: u8 = 0x0D;
    pub const TOO_CLOSE_RANGE: u8 = 0x0E;
    pub const TOO_FAR_RANGE: u8 = 0x0F;
    pub const OUTOF_ANGLE: u8 = 0x10;
    pub const CANNOT_CAST_IN_COMBAT: u8 = 0x11;
    pub const CANNOT_CAST_WHILE_MOVING: u8 = 0x12;
    pub const CANNOT_CAST_IN_STUN: u8 = 0x13;
    pub const CANNOT_CAST_WHILE_WALKING: u8 = 0x14;
    pub const CANNOT_CAST_IN_SWIMMING: u8 = 0x15;
    pub const BLANK_MINDED: u8 = 0x16;
    pub const SILENCE: u8 = 0x17;
    pub const CRIPPLED: u8 = 0x18;
    pub const CANNOT_CAST_IN_CHANNELING: u8 = 0x19;
    pub const CANNOT_CAST_IN_PRISON: u8 = 0x1A;
    pub const NEED_STEALTH: u8 = 0x1B;
    pub const NEED_NOCOMBAT_TARGET: u8 = 0x1C;
    pub const TARGET_IMMUNE: u8 = 0x1D;
    pub const INVALID_SKILL: u8 = 0x1E;
    pub const INACTIVE_ABILITY: u8 = 0x1F;
    pub const NOT_ENOUGH_ABILITY_LEVEL: u8 = 0x20;
    pub const INVALID_SOURCE: u8 = 0x21;
    pub const INVALID_TARGET: u8 = 0x22;
    pub const INVALID_LOCATION: u8 = 0x23;
    pub const NEED_REAGENT: u8 = 0x24;
    pub const ITEM_LOCKED: u8 = 0x25;
    pub const NEED_MONEY: u8 = 0x26;
    pub const NEED_LABOR_POWER: u8 = 0x27;
    pub const NOT_NOW: u8 = 0x2C;
    pub const NO_PERM: u8 = 0x2D;
    pub const BAG_FULL: u8 = 0x2E;
    pub const SKILL_REQ_FAIL: u8 = 0x32;
    /// `skill_urk_combat`: the unit requirement "not in combat" (Dash, Recall, the dances) failed.
    pub const URK_COMBAT: u8 = 0x48;

    /// Every result number the 1.2 client knows (`SkillResultHelper.SkillResultErrorKeyToIdFor_r208022` of the server), by number.
    const KEYS: [&str; 131] = [
        "success", "failure", "source died", "source alive", "target died", "target destroyed", "target alive", "already casting",
        "cooldown", "no target", "not enough health", "not enough mana", "obstacle", "out of height", "too close", "too far", "out of angle",
        "not in combat", "not while moving", "stunned", "not while walking", "not while swimming", "blank minded", "silenced", "crippled",
        "not while channeling", "not in prison", "needs stealth", "needs a non combat target", "target immune", "invalid skill",
        "ability not active", "ability level too low", "invalid source", "invalid target", "invalid location", "reagent missing", "item locked",
        "not enough money", "not enough labor", "source is hanging", "source is riding", "higher buff", "not a pvp area", "not now",
        "not permitted", "bag full", "protected faction", "protected level", "unit requirements (or) failed", "requirements not met",
        "backpack occupied", "obstacle for the doodad", "no doodad in a house", "cannot use on self", "not preoccupied", "not my npc",
        "not checked (second pass)", "cannot use here", "invalid enchant support item", "character stat too low", "character stat too high",
        "item secured", "invalid account attribute", "req: level", "req: ability", "req: race", "req: gender", "req: equip slot",
        "req: equip item", "req: own item", "req: trained skill", "req: not in combat", "req: stealth", "req: health", "req: buff",
        "req: target buff", "req: target in combat", "req: can learn craft", "req: doodad range", "req: shield", "req: no buff",
        "req: target buff tag", "req: corpse range", "req: weapon type", "req: target health below", "req: target npc", "req: target doodad",
        "req: ranged weapon", "req: no buff tag", "req: completed quest context", "req: quest context in progress", "req: quest context ready",
        "req: target npc group", "req: area sphere", "req: quest context not completed", "req: quest context precomplete", "req: target owner type",
        "req: not under water", "req: faction match", "req: time of day", "req: mother faction", "req: actability point", "req: crime point",
        "req: honor point", "req: living point", "req: crime record", "req: jury point", "req: source owner type", "req: appellation",
        "req: in zone", "req: out of zone", "req: dominion owner", "req: verdict only", "req: faction match only", "req: mother faction only",
        "req: nation owner", "req: faction match only (not)", "req: mother faction only (not)", "req: nation member", "req: not nation member",
        "req: nation owner at position", "req: dominion owner at position", "req: housing", "req: health margin", "req: mana margin",
        "req: labor margin", "req: unknown", "req: max level", "req: expedition owner", "req: expedition member",
    ];

    /// The server's name of a result, for logs (the UI has localized texts).
    pub fn name(r: u8) -> &'static str {
        match r {
            SUCCESS => "success",
            FAILURE => "failure",
            SOURCE_DIED => "source died",
            TARGET_DIED => "target died",
            ON_CASTING => "already casting",
            COOLDOWN_TIME => "cooldown",
            NO_TARGET => "no target",
            LACK_HEALTH => "not enough health",
            LACK_MANA => "not enough mana",
            OBSTACLE => "obstacle",
            TOO_CLOSE_RANGE => "too close",
            TOO_FAR_RANGE => "too far",
            OUTOF_ANGLE => "out of angle",
            CANNOT_CAST_IN_COMBAT => "not in combat",
            CANNOT_CAST_WHILE_MOVING => "not while moving",
            CANNOT_CAST_IN_STUN => "stunned",
            SILENCE => "silenced",
            INVALID_SKILL => "invalid skill",
            INACTIVE_ABILITY => "ability not active",
            NOT_ENOUGH_ABILITY_LEVEL => "ability level too low",
            INVALID_SOURCE => "invalid source",
            INVALID_TARGET => "invalid target",
            INVALID_LOCATION => "invalid location",
            NEED_REAGENT => "reagent missing",
            NEED_MONEY => "not enough money",
            NEED_LABOR_POWER => "not enough labor",
            NOT_NOW => "not now",
            NO_PERM => "not permitted",
            BAG_FULL => "bag full",
            SKILL_REQ_FAIL => "requirements not met",
            other => KEYS.get(usize::from(other)).copied().unwrap_or("?"),
        }
    }
}

/// `SkillHitType`: how an attack landed (`UnitDamage::hit_type`).
pub mod hit_type {
    pub const INVALID: u8 = 0x00;
    pub const MELEE_HIT: u8 = 0x01;
    pub const MELEE_CRITICAL: u8 = 0x03;
    pub const MELEE_MISS: u8 = 0x04;
    pub const MELEE_DODGE: u8 = 0x05;
    pub const MELEE_BLOCK: u8 = 0x06;
    pub const MELEE_PARRY: u8 = 0x07;
    pub const RANGED_HIT: u8 = 0x09;
    pub const RANGED_MISS: u8 = 0x0A;
    pub const RANGED_CRITICAL: u8 = 0x0B;
    pub const SPELL_HIT: u8 = 0x0D;
    pub const SPELL_MISS: u8 = 0x0E;
    pub const SPELL_CRITICAL: u8 = 0x0F;
    pub const RANGED_DODGE: u8 = 0x10;
    pub const RANGED_BLOCK: u8 = 0x11;
    pub const IMMUNE: u8 = 0x12;
    pub const SPELL_RESIST: u8 = 0x13;
    pub const RANGED_PARRY: u8 = 0x14;

    pub fn is_critical(h: u8) -> bool {
        matches!(h, MELEE_CRITICAL | RANGED_CRITICAL | SPELL_CRITICAL)
    }

    /// Dodged, blocked, parried, resisted, missed or immune: no damage was dealt.
    pub fn is_miss(h: u8) -> bool {
        matches!(
            h,
            MELEE_MISS
                | MELEE_DODGE
                | MELEE_BLOCK
                | MELEE_PARRY
                | RANGED_MISS
                | RANGED_DODGE
                | RANGED_BLOCK
                | RANGED_PARRY
                | SPELL_MISS
                | SPELL_RESIST
                | IMMUNE
        )
    }
}

/// What a skill is aimed at (`SkillCastTarget`): a unit, a doodad, a spot or an item.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkillTarget {
    /// See [`target_kind`].
    pub kind: u8,
    /// Object id for units and doodads (also the optional object of the other kinds).
    pub obj_id: u32,
    /// The spot, for the position kinds.
    pub pos: Option<Pos>,
    /// The spot's facing (`POSITION` only; the summon capture sends the ship yaw).
    pub rot: f32,
}

impl SkillTarget {
    pub fn unit(obj_id: u32) -> Self {
        Self {
            kind: target_kind::UNIT,
            obj_id,
            pos: None,
            rot: 0.0,
        }
    }

    /// A real nonzero 24-bit doodad ID; invalid IDs must not be truncated or changed to self.
    pub fn doodad(obj_id: u32) -> std::result::Result<Self, &'static str> {
        if obj_id == 0 || obj_id > 0xff_ffff {
            return Err("doodad object ID must be nonzero and fit 24 bits");
        }
        Ok(Self {
            kind: target_kind::DOODAD,
            obj_id,
            pos: None,
            rot: 0.0,
        })
    }

    pub fn position(pos: Pos) -> Self {
        Self {
            kind: target_kind::POSITION,
            obj_id: 0,
            pos: Some(pos),
            rot: 0.0,
        }
    }

    /// A spot with its facing (`POSITION`; the summon item sends the ship's yaw).
    pub fn position_rot(pos: Pos, rot: f32) -> Self {
        Self {
            kind: target_kind::POSITION,
            obj_id: 0,
            pos: Some(pos),
            rot,
        }
    }

    /// The target kind byte and its fields.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = r.u8()?;
        Ok(match kind {
            target_kind::UNIT | target_kind::DOODAD => Self {
                kind,
                obj_id: r.bc()?,
                pos: None,
                rot: 0.0,
            },
            target_kind::POSITION => {
                let pos = r.long_position()?;
                let rot = r.f32()?;
                r.skip(3 + 3)?; // two object ids
                Self {
                    kind,
                    obj_id: 0,
                    pos: Some(pos),
                    rot,
                }
            }
            target_kind::POSITION2 => {
                let pos = r.long_position()?;
                r.skip(6 * 4)?; // end point and normal
                Self {
                    kind,
                    obj_id: 0,
                    pos: Some(pos),
                    rot: 0.0,
                }
            }
            target_kind::ITEM => {
                let obj_id = r.bc()?;
                r.skip(8 + 4 + 1)?; // item id, two types
                Self {
                    kind,
                    obj_id,
                    pos: None,
                    rot: 0.0,
                }
            }
            target_kind::POSITION3 => {
                let pos = r.long_position()?;
                r.skip(4)?; // pitch
                Self {
                    kind,
                    obj_id: 0,
                    pos: Some(pos),
                    rot: 0.0,
                }
            }
            other => return Err(crate::wire::WireError::Invalid(format!("skill target kind {other}"))),
        })
    }

    /// Writes the kinds a client sends: unit, doodad and position.
    pub fn write(&self, w: &mut Writer) {
        w.u8(self.kind);
        match (self.kind, self.pos) {
            (target_kind::POSITION, Some(p)) => {
                w.long_position(p).f32(self.rot).bc(0).bc(0);
            }
            (target_kind::POSITION3, Some(p)) => {
                w.long_position(p).f32(0.0);
            }
            (target_kind::POSITION2, Some(p)) => {
                w.long_position(p);
                for _ in 0..6 {
                    w.f32(0.0);
                }
            }
            (target_kind::ITEM, _) => {
                w.bc(self.obj_id).u64(0).u32(0).u8(0);
            }
            _ => {
                w.bc(self.obj_id);
            }
        }
    }
}

/// Reads past a `SkillObject` (flag byte and its fields).
fn skip_skill_object(r: &mut Reader) -> Result<()> {
    match r.u8()? {
        1 => r.skip(1 + 4 + 8 + 8 + 4),         // portal info
        2 => r.skip(4).and_then(|()| r.string().map(|_| ())), // save portal info
        3 | 6 => r.string().map(|_| ()),        // text
        4 => r.skip(8 + 8 + 4),                 // position
        5 => r.skip(4),
        7 => r.skip(4 + 8 + 1),                 // enchanting support
        _ => Ok(()),
    }
}

/// The unit, bag item or mate/slave a client starts a skill with (`SkillCaster`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartCaster {
    /// The player's own unit (object id filled in by the session).
    Unit,
    /// An item of the bag: `item_id` is its database id (`Item::id`), `template_id` the template;
    /// the skill id must be the item's `use_skill_id`.
    Item { item_id: u64, template_id: u32 },
    /// A mate/slave runtime object plus the separate mount_skills.id (not combat skill ID).
    Mount { obj_id: u32, mount_skill_id: u32 },
}

/// `CSStartSkill`: `skill_id`, caster, target and no skill object.
pub fn write_start_skill(w: &mut Writer, skill_id: u32, self_obj_id: u32, caster: StartCaster, target: &SkillTarget) {
    w.u32(skill_id);
    match caster {
        StartCaster::Unit => {
            w.u8(caster_kind::UNIT).bc(self_obj_id);
        }
        StartCaster::Item { item_id, template_id } => {
            w.u8(caster_kind::ITEM).bc(self_obj_id).u64(item_id).u32(template_id).u8(0).u32(0);
        }
        StartCaster::Mount { obj_id, mount_skill_id } => {
            w.u8(caster_kind::MOUNT).bc(obj_id).u32(mount_skill_id);
        }
    }
    target.write(w);
    w.u8(0); // no skill object
}

/// Native397b1ef0 result framing. Flags select independent fields; unknown bits are retained.
/// The native reader clears code/extra/value before reading the selected fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillResult {
    pub raw_flags:u8,
    pub code:u8,
    pub extra:u16,
    pub value:u32,
}
impl SkillResult {
    pub fn read(r:&mut Reader)->Result<Self> {
        let raw_flags=r.u8()?;
        Ok(Self {
            raw_flags,
            code:if raw_flags&1!=0 {r.u8()?} else {0},
            extra:if raw_flags&2!=0 {r.u16()?} else {0},
            value:if raw_flags&4!=0 {r.u32()?} else {0},
        })
    }
}

/// `SCSkillStarted`: a unit begins casting, or (with a non zero `result`, `tl` 0) a skill the
/// client asked for was refused.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillStarted {
    pub skill_id: u32,
    /// Timeline id: names this cast in `SkillFired`, `SkillEnded` and `CastingStopped`.
    pub tl: u16,
    pub caster: BuffCaster,
    pub target: SkillTarget,
    pub raw_caster:SkillCasterData,
    pub raw_target:SkillTargetData,
    /// Cast time with haste, ms (0 = instant), and without.
    pub cast_ms: u32,
    pub base_cast_ms: u32,
    pub synergy: u8,
    /// `result::*`; 0 when the skill started.
    pub result: u8,
    /// Detail of a failure (for example the missing amount).
    pub result_value: u32,
    /// Complete native result; preserves simultaneous u16/u32 fields and unknown flag bits.
    pub raw_result:SkillResult,
}

impl SkillStarted {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let skill_id = r.u32()?;
        let tl = r.u16()?;
        let raw_caster=SkillCasterData::read(r)?;
        let raw_target=SkillTargetData::read(r)?;
        let caster=raw_caster.projection();
        let target=raw_target.projection();
        skip_skill_object(r)?;
        let cast_ms = u32::from(r.u16()?) * 10;
        let base_cast_ms = u32::from(r.u16()?) * 10;
        let synergy = r.u8()?;
        let raw_result=SkillResult::read(r)?;
        let result=raw_result.code;
        // Preserve the historical app field: u32 takes precedence over the u16 detail.
        let result_value=if raw_result.raw_flags&4!=0 {raw_result.value} else {u32::from(raw_result.extra)};
        Ok(Self { skill_id, tl, caster, target, cast_ms, base_cast_ms, synergy, result, result_value, raw_result, raw_caster, raw_target })
    }

    /// The skill did not start.
    pub fn failed(&self) -> bool {
        self.result != result::SUCCESS
    }
}

/// `SCSkillFired`: the cast completed and the effect happens.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillFired {
    pub skill_id: u32,
    pub tl: u16,
    pub caster: BuffCaster,
    pub target: SkillTarget,
    pub raw_caster:SkillCasterData,
    pub raw_target:SkillTargetData,
    /// Delay until the effects land, ms.
    pub delay_ms: i32,
    /// Channeling time, ms (0 for plain skills).
    pub channeling_ms: i32,
    pub fire_anim_id: u32,
    pub result:SkillResult,
    /// Final wire byte, preserved without assigning a general cooldown meaning.
    /// The active lane writes zero. Native serializer maps bit1 to its cooldown suppression gate.
    pub raw_final_flag: u8,
}

impl SkillFired {
    /// Native397ca4aa..bd unpacks bit1 into packet+160, tested by3939b35b.
    /// This is one producer gate, not a claim that the remaining planner admits the cast.
    pub fn native_cooldown_suppressed(&self)->bool {self.raw_final_flag&2!=0}
    pub fn read(r: &mut Reader) -> Result<Self> {
        let skill_id = r.u32()?;
        let tl = r.u16()?;
        let raw_caster=SkillCasterData::read(r)?;
        let raw_target=SkillTargetData::read(r)?;
        let caster=raw_caster.projection();
        let target=raw_target.projection();
        skip_skill_object(r)?;
        let delay = i32::from(r.i16()?);
        let channeling = i32::from(r.i16()?);
        let result=SkillResult::read(r)?;
        let fire_anim_id = r.u32()?;
        let raw_final_flag = r.u8()?;
        Ok(Self {
            skill_id,
            tl,
            caster,
            target,
            raw_caster,
            raw_target,
            delay_ms: (delay - 10) * 10,
            channeling_ms: (channeling - 10) * 10,
            fire_anim_id,
            result,
            raw_final_flag,
        })
    }
}

/// `CastAction`: what caused a damage or heal: a skill, a plot of a skill, or a buff tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CastAction {
    /// 0 skill, 1 plot, 2 buff, 3 and 4 other.
    pub kind: u8,
    pub skill_id: u32,
    pub tl: u16,
    /// The buff, for buff ticks (damage over time).
    pub buff_id: u32,
}

impl CastAction {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = r.u8()?;
        let mut a = Self { kind, ..Self::default() };
        match kind {
            0 => {
                a.skill_id = r.u32()?;
                a.tl = r.u16()?;
            }
            1 => {
                r.skip(4)?; // plot id
                a.tl = r.u16()?;
                r.skip(4)?; // event id
                a.skill_id = r.u32()?;
            }
            2 => {
                a.buff_id = r.u32()?;
                r.skip(3 + 4 + 1 + 1)?; // owner, index, two flags
            }
            3 => r.skip(4 + 4 + 3)?,
            4 => r.skip(4 + 3)?,
            other => return Err(crate::wire::WireError::Invalid(format!("cast action {other}"))),
        }
        Ok(a)
    }
}

/// `SCUnitDamaged`: a hit on `target_obj_id`.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitDamage {
    pub action: CastAction,
    pub caster: BuffCaster,
    pub caster_obj_id: u32,
    pub target_obj_id: u32,
    pub damage: i32,
    /// Damage absorbed by shields.
    pub absorbed: i32,
    pub mana_burn: i32,
    /// Exact received `holdables.id` byte, independent of the caster's current mainhand.
    pub holdable_id: u8,
    /// Exact packed wire word: low five bits are `hit_type`, bits 5..7 and 8..11
    /// are separate native context fields, and bits 12..15 are the pierce selector.
    pub raw_hit: u16,
    /// `hit_type::*` (the low five bits of the wire value).
    pub hit_type: u8,
}

impl UnitDamage {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let action = CastAction::read(r)?;
        let caster = BuffCaster::read(r)?;
        let caster_obj_id = r.bc()?;
        let target_obj_id = r.bc()?;
        r.skip(1)?; // crime state
        let [damage, absorbed, ..] = r.pisc(3)?;
        let mana_burn = r.pisc(3)?[2];
        let holdable_id = r.u8()?;
        let raw_hit = r.u16()?;
        r.skip(2)?; // flag, result
        Ok(Self {
            action,
            caster,
            caster_obj_id,
            target_obj_id,
            damage: damage as i32,
            absorbed: absorbed as i32,
            mana_burn: mana_burn as i32,
            holdable_id,
            raw_hit,
            hit_type: (raw_hit & 0x1F) as u8,
        })
    }

    /// Raw high nibble retained by native397c8760 at damage context +0x64.
    /// This is a selector, not a boolean or a derived damage type.
    pub fn pierce_selector(&self) -> u8 {
        (self.raw_hit >> 12) as u8
    }
}

/// `SCUnitHealed`.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitHeal {
    pub action: CastAction,
    pub caster: BuffCaster,
    pub target_obj_id: u32,
    /// 0 health, 1 mana.
    pub heal_type: u8,
    /// 11 critical heal, 13 plain heal.
    pub hit_type: u8,
    pub value: i32,
}

impl UnitHeal {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let action = CastAction::read(r)?;
        let caster = BuffCaster::read(r)?;
        let target_obj_id = r.bc()?;
        let heal_type = r.u8()?;
        let hit_type = r.u8()?;
        let value = r.i32()?;
        r.skip(4 + 1)?;
        Ok(Self { action, caster, target_obj_id, heal_type, hit_type, value })
    }

    pub fn is_critical(&self) -> bool {
        self.hit_type == 11
    }
}

/// `SCPlotEvent`: a step of a plot skill (most player skills are plots).
#[derive(Clone, Debug, PartialEq)]
pub struct PlotEvent {
    pub tl: u16,
    pub event_id: u32,
    pub skill_id: u32,
    /// Object ids of the source and target (0 when it is a position).
    pub caster_obj_id: u32,
    pub target_obj_id: u32,
    /// The spot, when the source / target of this step is a position and not a unit (ground
    /// targeted skills, area effects).
    pub caster_pos: Option<Pos>,
    pub target_pos: Option<Pos>,
    /// The next step's cast time, ms.
    pub casting_ms: u32,
}

impl PlotEvent {
    /// A `PlotObject`: a unit (kind 1, object id) or a position (kind 2).
    fn plot_object(r: &mut Reader) -> Result<(u32, Option<Pos>)> {
        match r.u8()? {
            1 => Ok((r.bc()?, None)),
            _ => {
                let pos = r.position()?;
                r.skip(3)?; // three rotation bytes
                Ok((0, Some(pos)))
            }
        }
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        let tl = r.u16()?;
        let event_id = r.u32()?;
        let skill_id = r.u32()?;
        let (caster_obj_id, caster_pos) = Self::plot_object(r)?;
        let (target_obj_id, target_pos) = Self::plot_object(r)?;
        r.skip(8 + 3)?; // item id, object id
        let casting_ms = u32::from(r.u16()?) * 10;
        r.skip(3 + 2)?;
        let n = r.u8()?;
        r.skip(usize::from(n) * 3)?;
        let flag = r.u8()?;
        if (flag >> 3) & 1 == 1 {
            r.skip(13 * 4)?;
        }
        Ok(Self { tl, event_id, skill_id, caster_obj_id, target_obj_id, caster_pos, target_pos, casting_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caster(w: &mut Writer, obj: u32) {
        w.u8(caster_kind::UNIT).bc(obj);
    }

    /// Refusals seen on the lane (Dash and Recall in combat answered 72, the dances in the water 21, Revive on a live unit 6)
    /// have names, and the table lines up with the server's numbering at both ends.
    #[test]
    fn every_server_result_number_has_a_name() {
        assert_eq!(result::name(result::URK_COMBAT), "req: not in combat");
        assert_eq!(result::name(result::TARGET_ALIVE), "target alive");
        assert_eq!(result::name(result::CANNOT_CAST_IN_SWIMMING), "not while swimming");
        assert_eq!(result::name(result::SKILL_REQ_FAIL), "requirements not met");
        assert_eq!(result::name(0x33), "backpack occupied");
        assert_eq!(result::name(0x40), "req: level");
        assert_eq!(result::name(130), "req: expedition member");
        assert_eq!(result::name(131), "?");
        assert!((0..=130u8).all(|r| r == 0 || result::name(r) != "?"));
    }

    #[test]
    fn mount_caster_writes_distinct_actor_mount_template_and_combat_skill_ids() {
        let message = crate::game::ClientMsg::StartSkill {
            skill_id: 20001,
            self_obj_id: 0x112233,
            caster: StartCaster::Mount {
                obj_id: 0x445566,
                mount_skill_id: 77,
            },
            target: SkillTarget::unit(0x445566),
        };
        // Same separate ID roles as lane handler_permission_tests::mate_cast_packet.
        let body = message.encode_body();
        assert_eq!(
            body,
            [
                0x21, 0x4e, 0, 0, 3, 0x66, 0x55, 0x44, 77, 0, 0, 0, 0, 0x66, 0x55, 0x44, 0
            ]
        );
        let mut reader = Reader::new(&body);
        assert_eq!(reader.u32().unwrap(), 20001);
        assert_eq!(reader.u8().unwrap(), 3);
        assert_eq!(
            reader.bc().unwrap(),
            0x445566,
            "caster is the mate, not the player"
        );
        assert_eq!(reader.u32().unwrap(), 77);
        assert_eq!(
            SkillTarget::read(&mut reader).unwrap(),
            SkillTarget::unit(0x445566)
        );
        assert_eq!(reader.u8().unwrap(), 0);
        assert!(reader.is_empty());
    }

    #[test]
    fn start_skill_bytes_follow_the_server_reader() {
        // CSStartSkill: u32 skill, caster kind + bc, target kind + bc, flag.
        let mut w = Writer::new();
        write_start_skill(&mut w, 18132, 0x0102, StartCaster::Unit, &SkillTarget::unit(0x0A0B0C));
        assert_eq!(w.finish(), [0xD4, 0x46, 0, 0, 0, 0x02, 0x01, 0, 0, 0x0C, 0x0B, 0x0A, 0]);
        // An item caster adds item id, template id, type 1 and type 2.
        let mut w = Writer::new();
        write_start_skill(&mut w, 25202, 7, StartCaster::Item { item_id: 99, template_id: 18791 }, &SkillTarget::unit(7));
        let bytes = w.finish();
        assert_eq!(bytes.len(), 4 + 1 + 3 + 8 + 4 + 1 + 4 + 4 + 1);
        assert_eq!(bytes[4], caster_kind::ITEM);
        // The server's reader accepts it: read it back with ours.
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u32().unwrap(), 25202);
        assert_eq!(BuffCaster::read(&mut r).unwrap(), BuffCaster { kind: 2, obj_id: 7 });
        assert_eq!(SkillTarget::read(&mut r).unwrap(), SkillTarget::unit(7));
        assert_eq!(r.u8().unwrap(), 0);
        assert!(r.is_empty());
    }

    /// A ground cast (Missile Rain on the lane, 2026-10): the server's `SkillCastPositionTarget` reader takes two `<< 32` fixed point
    /// longs (1/4096 m), a float height, a float rotation and two object ids, and echoes the spot back in `SCPlotEvent` rounded to 1/4096 m.
    #[test]
    fn ground_cast_bytes_follow_the_server_reader() {
        let pos = Pos::new(15572.556, 15393.606, 126.4835);
        let mut w = Writer::new();
        write_start_skill(&mut w, 13281, 20032, StartCaster::Unit, &SkillTarget::position(pos));
        let body = w.finish();
        let long = |v: f32| (((v * 4096.0) as i64) << 32).to_le_bytes();
        let mut expected = vec![0xE1, 0x33, 0, 0, 0, 0x40, 0x4E, 0x00, 1];
        expected.extend(long(15572.556));
        expected.extend(long(15393.606));
        expected.extend(126.4835f32.to_le_bytes());
        expected.extend(0f32.to_le_bytes()); // rotation
        expected.extend([0u8; 7]); // two 3 byte object ids (0), then the "no skill object" flag
        assert_eq!(body, expected);
        let mut r = Reader::new(&body);
        assert_eq!(r.u32().unwrap(), 13281);
        assert_eq!(BuffCaster::read(&mut r).unwrap(), BuffCaster { kind: 0, obj_id: 20032 });
        let back = SkillTarget::read(&mut r).unwrap();
        assert_eq!(back.kind, target_kind::POSITION);
        let p = back.pos.unwrap();
        // the lane's PlotEvent echo of this cast: 15572.555 / 15393.605
        assert!((p.x - 15572.555).abs() < 0.001 && (p.y - 15393.605).abs() < 0.001 && (p.z - 126.4835).abs() < 1e-4);
        assert_eq!(r.u8().unwrap(), 0);
        assert!(r.is_empty());
    }

    /// The 2026-10-30 group-A2 capture: Eznan Cutter item 5223 (item id 16777310) casts skill
    /// 15802 at the spot the client searched, with the ship yaw (player yaw + pi/2) as rotation.
    #[test]
    fn captured_summon_cs_start_skill_carries_the_chosen_position_and_rotation() {
        let target = SkillTarget::position_rot(Pos::new(12000.0, 9030.0, 99.872_765), 1.570_796_4);
        let body = crate::game::ClientMsg::StartSkill {
            skill_id: 15802,
            self_obj_id: 31221,
            caster: StartCaster::Item {
                item_id: 16_777_310,
                template_id: 5223,
            },
            target,
        }
        .encode_body();
        let expected: [u8; 57] = [
            0xba, 0x3d, 0, 0, 2, 0xf5, 0x79, 0, 0x5e, 0, 0, 1, 0, 0, 0, 0, 0x67, 0x14, 0, 0, 0, 0,
            0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0xee, 2, 0, 0, 0, 0, 0, 0x60, 0x34, 2, 0xdb, 0xbe, 0xc7,
            0x42, 0xdb, 0x0f, 0xc9, 0x3f, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            body, expected,
            "the captured CSStartSkill body, byte for byte"
        );
        let mut r = Reader::new(&body);
        assert_eq!(r.u32().unwrap(), 15802);
        assert_eq!(r.u8().unwrap(), caster_kind::ITEM);
        assert_eq!(r.bc().unwrap(), 31221);
        assert_eq!(r.u64().unwrap(), 16_777_310);
        assert_eq!(r.u32().unwrap(), 5223);
        assert_eq!(r.u8().unwrap(), 0);
        assert_eq!(r.u32().unwrap(), 0);
        let back = SkillTarget::read(&mut r).unwrap();
        assert_eq!(back.kind, target_kind::POSITION);
        let pos = back.pos.unwrap();
        assert_eq!((pos.x, pos.y, pos.z), (12000.0, 9030.0, 99.872_765));
        assert_eq!(back.rot, 1.570_796_4);
        assert_eq!(r.u8().unwrap(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn doodad_skill_target_is_kind_four_with_exact_nonzero_24_bit_identity() {
        let target = SkillTarget::doodad(0x445566).unwrap();
        let mut w = Writer::new();
        write_start_skill(&mut w, 18132, 0x112233, StartCaster::Unit, &target);
        let body = w.finish();
        assert_eq!(
            body,
            [
                0xd4, 0x46, 0, 0, 0, 0x33, 0x22, 0x11, 4, 0x66, 0x55, 0x44, 0
            ]
        );
        let mut r = Reader::new(&body);
        assert_eq!(r.u32().unwrap(), 18132);
        assert_eq!(
            BuffCaster::read(&mut r).unwrap(),
            BuffCaster {
                kind: 0,
                obj_id: 0x112233
            }
        );
        assert_eq!(SkillTarget::read(&mut r).unwrap(), target);
        assert_eq!(r.u8().unwrap(), 0);
        assert!(r.is_empty());
        for id in [1, 0xff_ffff] {
            assert_eq!(SkillTarget::doodad(id).unwrap().obj_id, id);
        }
        for id in [0, 0x100_0000, u32::MAX] {
            assert!(SkillTarget::doodad(id).is_err());
        }
    }

    #[test]
    fn position_targets_roundtrip() {
        let t = SkillTarget::position(Pos::new(100.5, 200.25, 50.0));
        let mut w = Writer::new();
        t.write(&mut w);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 1 + 8 + 8 + 4 + 4 + 3 + 3);
        let back = SkillTarget::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(back.kind, target_kind::POSITION);
        let p = back.pos.unwrap();
        assert_eq!((p.x, p.y, p.z), (100.5, 200.25, 50.0));
    }

    /// The bytes of a position target as the server's own reader takes them (`SkillCastTarget::read` of
    /// `aaemu-game/src/systems/skills/types.rs`, `POSITION`: i64 x, i64 y, f32 z, f32 rot, bc, bc;
    /// `convert_long_back(v) = (v >> 32) as f32 / 4096`).
    #[test]
    fn position_target_decodes_as_the_server_reads_it() {
        fn convert_long_back(v: i64) -> f32 {
            (v >> 32) as f32 / 4096.0
        }
        let mut w = Writer::new();
        write_start_skill(&mut w, 10664, 0x0102, StartCaster::Unit, &SkillTarget::position(Pos::new(15579.5, 15390.25, 126.5)));
        let b = w.finish();
        // u32 skill, caster kind + bc(0x0102 = 3 bytes), then the target
        let t = 4 + 1 + 3;
        assert_eq!(b[t], 1, "SkillCastTargetType::POSITION");
        let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        let f32_at = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        assert_eq!(convert_long_back(i64_at(t + 1)), 15579.5);
        assert_eq!(convert_long_back(i64_at(t + 9)), 15390.25);
        assert_eq!(f32_at(t + 17), 126.5);
        assert_eq!(f32_at(t + 21), 0.0, "rot");
        assert_eq!(&b[t + 25..t + 31], &[0, 0, 0, 0, 0, 0], "two object ids (bc 0)");
    }

    #[test]
    fn native_skill_result_executed_x86_goldens_preserve_independent_fields_and_unknown_bits() {
        // Original397b1ef0 executed via mocked abstract-reader primitives; self-contained outputs.
        let cases: &[(&[u8],SkillResult)]=&[
            (&[0],SkillResult{raw_flags:0,code:0,extra:0,value:0}),
            (&[1,73],SkillResult{raw_flags:1,code:73,extra:0,value:0}),
            (&[2,0x34,0x12],SkillResult{raw_flags:2,code:0,extra:0x1234,value:0}),
            (&[3,73,0x34,0x12],SkillResult{raw_flags:3,code:73,extra:0x1234,value:0}),
            (&[4,0xef,0xcd,0xab,0x89],SkillResult{raw_flags:4,code:0,extra:0,value:0x89abcdef}),
            (&[5,73,0xef,0xcd,0xab,0x89],SkillResult{raw_flags:5,code:73,extra:0,value:0x89abcdef}),
            (&[6,0x34,0x12,0xef,0xcd,0xab,0x89],SkillResult{raw_flags:6,code:0,extra:0x1234,value:0x89abcdef}),
            (&[7,73,0x34,0x12,0xef,0xcd,0xab,0x89],SkillResult{raw_flags:7,code:73,extra:0x1234,value:0x89abcdef}),
            (&[255,73,0x34,0x12,0xef,0xcd,0xab,0x89],SkillResult{raw_flags:255,code:73,extra:0x1234,value:0x89abcdef}),
            (&[128],SkillResult{raw_flags:128,code:0,extra:0,value:0}),
        ];
        for (bytes,expected) in cases {
            let mut r=Reader::new(bytes);assert_eq!(SkillResult::read(&mut r).unwrap(),*expected);assert!(r.is_empty());
            for n in 0..bytes.len() {assert!(SkillResult::read(&mut Reader::new(&bytes[..n])).is_err());}
        }
        assert_eq!(SkillResult::default(),cases[0].1);
    }
    #[test]
    fn fired_and_started_result_framing_all_flags_and_compressed_tail() {
        use std::io::Write;
        for flags in 0..=u8::MAX {
            let mut result=Writer::new();result.u8(flags);
            if flags&1!=0 {result.u8(73);}
            if flags&2!=0 {result.u16(0x1234);}
            if flags&4!=0 {result.u32(0x89abcdef);}
            let result=result.finish();
            let mut started=Writer::new();started.u32(18132).u16(3).u8(0).bc(1).u8(0).bc(2).u8(0).u16(15).u16(20).u8(0).raw(&result);
            let started=started.finish();let mut sr=Reader::new(&started);let started_result=SkillStarted::read(&mut sr).unwrap();assert!(sr.is_empty());
            assert_eq!(started_result.raw_result.raw_flags,flags);assert_eq!(started_result.result,if flags&1!=0 {73}else{0});
            assert_eq!(started_result.raw_result.extra,if flags&2!=0 {0x1234}else{0});
            assert_eq!(started_result.raw_result.value,if flags&4!=0 {0x89abcdef}else{0});
            assert_eq!(started_result.result_value,if flags&4!=0 {0x89abcdef}else if flags&2!=0 {0x1234}else{0});
            let mut fired=Writer::new();fired.u32(18132).u16(3).u8(0).bc(1).u8(0).bc(2).u8(0).i16(40).i16(10).raw(&result).u32(296).u8(flags);
            let fired=fired.finish();let mut fr=Reader::new(&fired);let fired_result=SkillFired::read(&mut fr).unwrap();assert!(fr.is_empty());
            assert_eq!(fired_result.result,started_result.raw_result);assert_eq!(fired_result.fire_anim_id,296);assert_eq!(fired_result.raw_final_flag,flags);
            assert_eq!(fired_result.native_cooldown_suppressed(),flags&2!=0);
            for n in 0..fired.len() {assert!(SkillFired::read(&mut Reader::new(&fired[..n])).is_err());}
            for n in 0..started.len() {assert!(SkillStarted::read(&mut Reader::new(&started[..n])).is_err());}
            let mut raw=Writer::new();raw.u16(0).u16(0xa1).raw(&started).u16(0).u16(0xa2).raw(&fired).u16(0).u16(0xa3).u16(3);
            let mut compressor=flate2::write::DeflateEncoder::new(Vec::new(),flate2::Compression::fast());compressor.write_all(&raw.finish()).unwrap();
            let mut batch=Writer::new();batch.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(3).raw(&compressor.finish().unwrap());
            let crate::game::ServerMsg::Batch(messages)=crate::game::ServerMsg::decode(&batch.finish()).unwrap() else{panic!()};
            assert_eq!(messages.len(),3);assert!(matches!(&messages[0],crate::game::ServerMsg::SkillStarted(s) if s.raw_result==fired_result.result));
            assert!(matches!(&messages[1],crate::game::ServerMsg::SkillFired(f) if f.result==fired_result.result &&f.raw_final_flag==flags));
            assert!(matches!(messages[2],crate::game::ServerMsg::SkillEnded{tl:3}));
        }
    }

    #[test]
    fn skill_started_decodes_the_server_layout() {
        // The server's `skill_started_layout_with_error`: id 0x01020304, tl 0x0A0B, unit caster
        // 0x112233, unit target 0x445566, no skill object, cast 150 / 150 (x10 ms), synergy 0,
        // flags 5 (result byte + uint): result 0x0F, value 7.
        let mut w = Writer::new();
        w.u32(0x0102_0304).u16(0x0A0B).u8(0).bc(0x112233).u8(0).bc(0x445566).u8(0);
        w.u16(150).u16(150).u8(0).u8(5).u8(0x0F).u32(7);
        let s = SkillStarted::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!((s.skill_id, s.tl), (0x0102_0304, 0x0A0B));
        assert_eq!(s.caster, BuffCaster { kind: 0, obj_id: 0x112233 });
        assert_eq!(s.target, SkillTarget::unit(0x445566));
        assert_eq!((s.cast_ms, s.base_cast_ms), (1500, 1500));
        assert_eq!((s.result, s.result_value), (0x0F, 7));
        assert!(s.failed());
        assert_eq!(result::name(s.result), "too far");

        // A cast that started has no result flags.
        let mut w = Writer::new();
        w.u32(18132).u16(3).u8(0).bc(1).u8(0).bc(2).u8(0).u16(0).u16(0).u8(0).u8(0);
        let s = SkillStarted::read(&mut Reader::new(&w.finish())).unwrap();
        assert!(!s.failed());
        assert_eq!((s.tl, s.cast_ms), (3, 0));
    }

    #[test]
    fn skill_fired_decodes_the_server_layout() {
        // delay and channeling are written as value / 10 + 10.
        let mut w = Writer::new();
        w.u32(18132).u16(3).u8(0).bc(1).u8(0).bc(2).u8(0);
        w.i16(30 + 10).i16(0 + 10).u8(0).u32(296).u8(0);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let f = SkillFired::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((f.skill_id, f.tl, f.delay_ms, f.channeling_ms, f.fire_anim_id), (18132, 3, 300, 0, 296));
        assert_eq!(f.raw_final_flag, 0);
        for end in 0..bytes.len() {
            assert!(SkillFired::read(&mut Reader::new(&bytes[..end])).is_err(), "truncation {end}");
        }
        for flag in [1, 2, 255] {
            let mut native_bytes = bytes.clone();
            *native_bytes.last_mut().unwrap() = flag;
            let mut r = Reader::new(&native_bytes);
            assert_eq!(SkillFired::read(&mut r).unwrap().raw_final_flag, flag);
            assert!(r.is_empty());
        }
    }

    #[test]
    fn damage_and_heal_decode_the_server_layout() {
        // SCUnitDamaged: CastAction::Skill, unit caster, bc caster, bc target, crime state,
        // pisc(damage, absorbed, 0), pisc(0, 0, mana burn), holdable, u16 (288 | hit), flag, result.
        let mut w = Writer::new();
        w.u8(0).u32(18132).u16(3); // cast action
        caster(&mut w, 0x10); // skill caster
        w.bc(0x10).bc(0x20).u8(0);
        w.pisc(&[1234, 5, 0]).pisc(&[0, 0, 9]);
        w.u8(0).u16(288 | 0x03).u8(1).u8(1);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let d = UnitDamage::read(&mut r).unwrap();
        assert!(r.is_empty(), "{} bytes left", r.remaining());
        assert_eq!((d.action.skill_id, d.action.tl, d.caster_obj_id, d.target_obj_id), (18132, 3, 0x10, 0x20));
        assert_eq!((d.damage, d.absorbed, d.mana_burn), (1234, 5, 9));
        assert_eq!(d.hit_type, hit_type::MELEE_CRITICAL);
        assert_eq!((d.holdable_id, d.raw_hit, d.pierce_selector()), (0, 0x0123, 0));
        assert!(hit_type::is_critical(d.hit_type) && !hit_type::is_miss(d.hit_type));

        // A buff tick names the buff, not a skill.
        let mut w = Writer::new();
        w.u8(2).u32(555).bc(0x20).u32(4).bool(true).bool(false);
        caster(&mut w, 0x10);
        w.bc(0x10).bc(0x20).u8(0).pisc(&[7, 0, 0]).pisc(&[0, 0, 0]).u8(0).u16(288).u8(1).u8(1);
        let d = UnitDamage::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!((d.action.kind, d.action.buff_id, d.action.skill_id, d.damage), (2, 555, 0, 7));
        assert_eq!((d.holdable_id, d.raw_hit, d.pierce_selector()), (0, 0x0120, 0));

        // SCUnitHealed: cast action, caster, bc target, type, hit, i32 value, i32, result.
        let mut w = Writer::new();
        w.u8(0).u32(25202).u16(9);
        caster(&mut w, 0x20);
        w.bc(0x20).u8(0).u8(13).i32(300).i32(0).u8(1);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let h = UnitHeal::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((h.target_obj_id, h.heal_type, h.value, h.is_critical()), (0x20, 0, 300, false));
    }

    #[test]
    fn plot_event_decodes_the_server_layout() {
        // SCPlotEvent: tl, event id, skill id, caster (1 + bc), target (1 + bc), item id, bc obj,
        // u16 casting time, bc 0, i16 0, count, count x bc, flag (bit 3: 13 x i32).
        let mut w = Writer::new();
        w.u16(3).u32(77).u32(18132);
        w.u8(1).bc(0x10).u8(1).bc(0x20);
        w.u64(0).bc(0x10).u16(50).bc(0).i16(0).u8(2).bc(0x20).bc(0x20).u8(2);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let e = PlotEvent::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((e.tl, e.event_id, e.skill_id, e.caster_obj_id, e.target_obj_id, e.casting_ms), (3, 77, 18132, 0x10, 0x20, 500));

        // A position target and the long flag form.
        let mut w = Writer::new();
        w.u16(3).u32(77).u32(18132);
        w.u8(1).bc(0x10).u8(2).position(Pos::new(10.0, 20.0, 30.0)).i8(0).i8(0).i8(0);
        w.u64(0).bc(0x10).u16(0).bc(0).i16(0).u8(0).u8(8);
        for _ in 0..13 {
            w.i32(0);
        }
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let e = PlotEvent::read(&mut r).unwrap();
        assert!(r.is_empty(), "{} bytes left", r.remaining());
        assert_eq!((e.caster_obj_id, e.target_obj_id), (0x10, 0));
        assert_eq!(e.caster_pos, None);
        let p = e.target_pos.expect("a position target");
        assert!((p.x - 10.0).abs() < 0.01 && (p.y - 20.0).abs() < 0.01 && (p.z - 30.0).abs() < 0.1, "{p:?}");
    }
}

#[cfg(test)]
mod cooldown_tests {
    use super::*;

    #[test]
    fn native_cooldowns_keep_both_lists_and_original_values() {
        // Native 397b2350: i32 skill count, triples, i32 tag count, triples. No 10ms scale.
        let body = [
            2, 0, 0, 0, 4, 3, 2, 1, 0xe8, 3, 0, 0, 0xfa, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 1, 0, 0,
            0, 0xff, 0xff, 0xff, 0xff, 1, 0, 0, 0, 77, 0, 0, 0, 0x20, 0x4e, 0, 0, 0x88, 0x13, 0, 0,
            0xab,
        ];
        let mut r = Reader::new(&body);
        let decoded = SkillCooldowns::read(&mut r).unwrap();
        assert_eq!(
            decoded,
            SkillCooldowns {
                skills: vec![
                    Cooldown {
                        id: 0x0102_0304,
                        total_ms: 1000,
                        remaining_ms: 250
                    },
                    Cooldown {
                        id: u32::MAX,
                        total_ms: 1,
                        remaining_ms: u32::MAX
                    }
                ],
                tags: vec![Cooldown {
                    id: 77,
                    total_ms: 20_000,
                    remaining_ms: 5000
                }],
            }
        );
        assert_eq!(r.rest(), &[0xab], "do not consume the next DD04 packet");
        for len in 0..body.len() - 1 {
            assert!(
                SkillCooldowns::read(&mut Reader::new(&body[..len])).is_err(),
                "truncated at {len}"
            );
        }
        assert_eq!(
            SkillCooldowns::read(&mut Reader::new(&[0; 8])).unwrap(),
            SkillCooldowns::default()
        );
    }

    #[test]
    fn cooldown_counts_reject_negative_or_over_native_capacity_in_either_list() {
        for count in [-1, i32::MIN, 151, i32::MAX] {
            for second_list in [false, true] {
                let mut w = Writer::new();
                if second_list {
                    w.i32(0);
                }
                w.i32(count).raw(&[0; 24]);
                assert!(
                    matches!(
                        SkillCooldowns::read(&mut Reader::new(&w.finish())),
                        Err(crate::wire::WireError::Invalid(_))
                    ),
                    "{count}, tags={second_list}"
                );
            }
        }
    }

    #[test]
    fn cooldown_native_capacity_and_independent_empty_lists_are_framed() {
        for (skills, tags) in [(150, 150), (0, 150), (150, 0)] {
            let mut w = Writer::new();
            for count in [skills, tags] {
                w.i32(count);
                for id in 0..count {
                    w.u32(id as u32).u32(123_456).u32(9999);
                }
            }
            w.u16(0xbeef);
            let bytes = w.finish();
            let mut r = Reader::new(&bytes);
            let value = SkillCooldowns::read(&mut r).unwrap();
            assert_eq!(
                (value.skills.len(), value.tags.len()),
                (skills as usize, tags as usize)
            );
            assert_eq!(r.u16().unwrap(), 0xbeef);
            assert!(r.is_empty());
        }
    }
}


/// Exact native397cad20 item-caster/item-target path; separate from generic no-object casts.
pub(crate) fn write_item_effect_start(w:&mut Writer,skill_id:u32,self_obj_id:u32,source:&crate::model::Item,target:&crate::model::Item,support:Option<&crate::model::Item>,auto_aa:Option<bool>) {
    w.u32(skill_id).u8(2).bc(self_obj_id).u64(source.id).u32(source.template_id).u8(source.grade).u32(source.made_unit_id);
    w.u8(3).bc(self_obj_id).u64(target.id).u32(target.template_id).u8(target.grade);
    if let Some(auto)=auto_aa {w.u8(7).u32(support.map_or(0,|v|v.template_id)).u64(support.map_or(0,|v|v.id)).bool(auto);} else {w.u8(0);}
}
