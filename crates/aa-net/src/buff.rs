//! Buffs and debuffs on units: how a unit-state packet lists them and how `SCBuffCreated` adds one.
//!
//! Layouts follow the server's `Buff.write_net` (`SCUnitStatePacket.WriteBuff`) and
//! `SCBuffCreatedPacket`. The server sends no stack count with a new buff: `charge` is the stack /
//! charge count used by the active lane convenience view. Native stacks require static catalog
//! rules and received history; see `state::NativeBuffs`. Charge alone is not a native stack count.

use crate::wire::{Reader, Result};

/// `SkillCasterType` of the unit that applied a buff.
pub mod caster_kind {
    pub const UNIT: u8 = 0;
    pub const UNK1: u8 = 1;
    pub const ITEM: u8 = 2;
    pub const MOUNT: u8 = 3;
    pub const DOODAD: u8 = 4;
}

/// Who applied a buff (`SkillCaster`): the kind (see [`caster_kind`]) and the object id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuffCaster {
    pub kind: u8,
    pub obj_id: u32,
}

impl BuffCaster {
    /// `SkillCaster.Write`: kind byte, object id, and the extra fields of item and mount casters.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = r.u8()?;
        let obj_id = r.bc()?;
        match kind {
            caster_kind::ITEM => r.skip(8 + 4 + 1 + 4)?, // item id, template id, type 1, type 2
            caster_kind::MOUNT => r.skip(4)?,            // mount skill template id
            _ => {}
        }
        Ok(Self { kind, obj_id })
    }
}

/// Which list of a unit-state packet a buff came in (`good`, `bad`, `hidden`); buffs from
/// `SCBuffCreated` carry no list and are [`BuffKind::Unknown`] (the `buffs` table has `kind_id`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuffKind {
    #[default]
    Unknown,
    Good,
    Bad,
    Hidden,
}

/// An active effect on a unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buff {
    /// Per-unit instance id (`SCBuffRemoved` names a buff by it).
    pub index: u32,
    /// `buffs.id` (icon, name and description come from that table).
    pub buff_id: u32,
    pub caster: BuffCaster,
    /// Character id of the caster (0 for NPCs).
    pub caster_char_id: u32,
    pub caster_level: u8,
    pub ability_level: i16,
    /// The skill whose toggle this buff is (`SCBuffCreated` only).
    pub skill_id: u32,
    /// Exact snapshot cooldownSkill or corresponding Created skill field; no timer inferred.
    pub raw_cooldown_skill: Option<u32>,
    /// Total duration in ms, 0 for a permanent buff.
    pub duration_ms: i32,
    /// Time already elapsed in ms when the packet was written (0 for a new buff).
    pub elapsed_ms: u32,
    pub tick_ms: u32,
    /// Exact signed NetBuff tickIndex; Created has no corresponding wire field.
    pub raw_tick_index: Option<i32>,
    /// Snapshot value or native Created constructor projection; no ongoing tick advancement.
    pub native_tick_index: Option<i32>,
    /// Genuine snapshot stack count; Created uses the lane charge convenience value.
    /// For original-client modifiers use the catalog-aware `state::NativeBuffs` view.
    pub stacks: i32,
    pub charged: i32,
    pub kind: BuffKind,
}

impl Buff {
    pub(crate) fn created_tick_index(elapsed_ms: u32, tick_ms: u32) -> i32 {
        if tick_ms == 0 { 0 } else { (elapsed_ms / tick_ms) as i32 }
    }

    /// One entry of a unit-state buff list (`NetBuff`).
    pub fn read_net(r: &mut Reader, kind: BuffKind) -> Result<Self> {
        let index = r.u32()?;
        let buff_id = r.u32()?;
        let caster = BuffCaster::read(r)?;
        let caster_char_id = r.u32()?;
        let caster_level = r.u8()?;
        let ability_level = r.i16()?;
        let duration_ms = r.i32()?;
        let elapsed_ms = r.u32()?;
        let tick_ms = r.u32()?;
        let raw_tick_index = Some(r.i32()?);
        let stacks = r.i32()?;
        let charged = r.i32()?;
        let raw_cooldown_skill = Some(r.u32()?);
        Ok(Self {
            index,
            buff_id,
            caster,
            caster_char_id,
            caster_level,
            ability_level,
            skill_id: 0,
            raw_cooldown_skill,
            duration_ms,
            elapsed_ms,
            tick_ms,
            raw_tick_index,
            native_tick_index: raw_tick_index,
            stacks,
            charged,
            kind,
        })
    }

    /// The three lists (good, bad, hidden) at the end of a unit-state packet.
    pub fn read_lists(r: &mut Reader) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        for kind in [BuffKind::Good, BuffKind::Bad, BuffKind::Hidden] {
            let n = r.u8()?;
            for _ in 0..n {
                out.push(Self::read_net(r, kind)?);
            }
        }
        Ok(out)
    }

    /// Body of `SCBuffCreatedPacket`: the buff and the unit it is on.
    pub fn read_created(r: &mut Reader) -> Result<(u32, Self)> {
        let caster = BuffCaster::read(r)?;
        let caster_char_id = r.u32()?;
        let owner = r.bc()?;
        let index = r.u32()?;
        let buff_id = r.u32()?;
        let caster_level = r.u8()?;
        let ability_level = r.i16()?;
        let skill_id = r.u32()?;
        // Charge, duration / 10, 0, tick / 10.
        let [charge, duration, elapsed, tick] = r.pisc(4)?;
        Ok((
            owner,
            Self {
                index,
                buff_id,
                caster,
                caster_char_id,
                caster_level,
                ability_level,
                skill_id,
                raw_cooldown_skill: Some(skill_id),
                duration_ms: (duration * 10) as i32,
                elapsed_ms: (elapsed * 10) as u32,
                tick_ms: (tick * 10) as u32,
                raw_tick_index: None,
                native_tick_index: Some(Self::created_tick_index((elapsed * 10) as u32, (tick * 10) as u32)),
                stacks: charge.max(1) as i32,
                charged: charge as i32,
                kind: BuffKind::Unknown,
            },
        ))
    }

    /// Time left in ms after `since_ms` milliseconds counted from the moment the packet arrived;
    /// `None` for a permanent buff.
    pub fn remaining_ms(&self, since_ms: u64) -> Option<u64> {
        if self.duration_ms <= 0 {
            return None;
        }
        let used = u64::from(self.elapsed_ms) + since_ms;
        Some((self.duration_ms as u64).saturating_sub(used))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::wire::Writer;

    /// `Buff.write_net` of the server, field by field.
    pub(crate) fn net_buff(w: &mut Writer, index: u32, buff_id: u32) {
        w.u32(index).u32(buff_id);
        w.u8(0).bc(0x0102); // caster
        w.u32(77).u8(5).i16(3); // caster char id, level, ability level
        w.i32(30_000).u32(5_000).u32(1_000); // duration, elapsed, tick
        w.i32(0).i32(2).i32(0).u32(0); // tick index, stack, charged, cooldown skill
    }

    #[test]
    fn created_tick_projection_matches_executed_constructor_fragment() {
        for (elapsed, tick, expected) in [(0,0,0),(25,7,3),(999,100,9),(1000,100,10),
            (u32::MAX,1,-1),(0x80000000,3,715827882),(u32::MAX,u32::MAX,1)] {
            assert_eq!(Buff::created_tick_index(elapsed,tick),expected);
        }
        assert_eq!(Buff::default().native_tick_index,None);
    }

    #[test]
    fn snapshot_cooldown_skill_retains_all_bits_without_changing_legacy_skill_id() {
        for id in [0,1,0x80000000,u32::MAX] {
            let mut w=Writer::new();net_buff(&mut w,2,77);let mut bytes=w.finish();
            let n=bytes.len();bytes[n-4..].copy_from_slice(&id.to_le_bytes());bytes.push(0xa5);
            let mut r=Reader::new(&bytes);let buff=Buff::read_net(&mut r,BuffKind::Good).unwrap();
            assert_eq!((buff.raw_cooldown_skill,buff.skill_id),(Some(id),0));assert_eq!(r.u8().unwrap(),0xa5);
            for n in 0..bytes.len()-1 {assert!(Buff::read_net(&mut Reader::new(&bytes[..n]),BuffKind::Good).is_err());}
        }
        assert_eq!(Buff::default().raw_cooldown_skill,None);
    }

    #[test]
    fn snapshot_tick_index_preserves_signed_bits_and_alignment() {
        for index in [i32::MIN, -1, 0, 123, i32::MAX] {
            let mut w = Writer::new();
            net_buff(&mut w, 2, 77);
            let mut bytes = w.finish();
            let offset = bytes.len() - 16;
            bytes[offset..offset + 4].copy_from_slice(&index.to_le_bytes());
            bytes.push(0xa5);
            let mut r = Reader::new(&bytes);
            let b = Buff::read_net(&mut r, BuffKind::Good).unwrap();
            assert_eq!((b.raw_tick_index, b.native_tick_index), (Some(index), Some(index)));
            assert_eq!(r.u8().unwrap(), 0xa5);
            for n in 0..bytes.len() - 1 {
                assert!(Buff::read_net(&mut Reader::new(&bytes[..n]), BuffKind::Good).is_err());
            }
        }
    }

    #[test]
    fn net_buff_lists() {
        let mut w = Writer::new();
        w.u8(2);
        net_buff(&mut w, 2, 8_000_011);
        net_buff(&mut w, 3, 2423);
        w.u8(0); // bad
        w.u8(1);
        net_buff(&mut w, 9, 4);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let buffs = Buff::read_lists(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(buffs.len(), 3);
        assert_eq!(buffs[0].buff_id, 8_000_011);
        assert_eq!(buffs[0].kind, BuffKind::Good);
        assert_eq!(buffs[0].caster, BuffCaster { kind: 0, obj_id: 0x0102 });
        assert_eq!((buffs[0].caster_char_id, buffs[0].caster_level, buffs[0].ability_level), (77, 5, 3));
        assert_eq!((buffs[0].duration_ms, buffs[0].elapsed_ms, buffs[0].tick_ms), (30_000, 5_000, 1_000));
        assert_eq!(buffs[0].raw_tick_index, Some(0));
        assert_eq!(buffs[0].stacks, 2);
        assert_eq!(buffs[2].kind, BuffKind::Hidden);
        assert_eq!(buffs[0].remaining_ms(1_000), Some(24_000));
        assert_eq!(buffs[0].remaining_ms(60_000), Some(0));
    }

    #[test]
    fn buff_created_golden_bytes() {
        // Unit caster, caster char 0, owner 0x0A0B0C, index 2, buff 8000011, level 1, ability level
        // 1, no skill, pisc(charge 0, duration 3000 (30 s / 10 ms), 0, tick 0).
        let mut w = Writer::new();
        w.u8(0).bc(0x0A0B0C); // SkillCaster::Unit
        w.u32(0).bc(0x0A0B0C).u32(2).u32(8_000_011).u8(1).i16(1).u32(0);
        w.pisc(&[0, 3000, 0, 0]);
        let bytes = w.finish();
        // 4 caster + 4 + 3 + 4 + 4 + 1 + 2 + 4, then the pisc: header + 1 + 2 + 1 + 1
        assert_eq!(bytes.len(), 26 + 6);
        let (owner, b) = Buff::read_created(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(owner, 0x0A0B0C);
        assert_eq!((b.index, b.buff_id, b.duration_ms, b.stacks), (2, 8_000_011, 30_000, 1));
        assert_eq!(b.kind, BuffKind::Unknown);
        assert_eq!(b.raw_tick_index, None);
        assert_eq!(b.native_tick_index, Some(0));
        assert_eq!(b.raw_cooldown_skill, Some(0));
    }

    #[test]
    fn created_retains_elapsed_and_charge_without_consuming_next_packet() {
        let mut w = Writer::new();
        w.u8(0).bc(1).u32(10).bc(2).u32(3).u32(77).u8(4).i16(5).u32(6);
        w.pisc(&[9, 100, 25, 7]);
        w.u8(0xaa);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let (_, b) = Buff::read_created(&mut r).unwrap();
        assert_eq!((b.charged, b.duration_ms, b.elapsed_ms, b.tick_ms), (9, 1000, 250, 70));
        assert_eq!((b.raw_tick_index, b.native_tick_index), (None, Some(3)));
        assert_eq!(b.raw_cooldown_skill, Some(6));
        assert_eq!(r.u8().unwrap(), 0xaa);
    }

    #[test]
    fn item_and_mount_casters_carry_extra_fields() {
        let mut w = Writer::new();
        w.u8(caster_kind::ITEM).bc(1).u64(99).u32(1234).u8(0).u32(0);
        w.u8(caster_kind::MOUNT).bc(2).u32(55);
        w.u8(caster_kind::DOODAD).bc(3);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert_eq!(BuffCaster::read(&mut r).unwrap(), BuffCaster { kind: 2, obj_id: 1 });
        assert_eq!(BuffCaster::read(&mut r).unwrap(), BuffCaster { kind: 3, obj_id: 2 });
        assert_eq!(BuffCaster::read(&mut r).unwrap(), BuffCaster { kind: 4, obj_id: 3 });
        assert!(r.is_empty());
    }
}
