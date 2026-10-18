//! Full-item mate equipment notifications; no inferred swap or success settlement.

use crate::model::Item;
use crate::wire::{Reader, Result, WireError};

/// Native 397d1340 reads at most two inline transaction entries.
pub const MAX_MATE_EQUIPMENT_ENTRIES: usize = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MateItemLocation {
    pub item: Option<Item>,
    pub container: u8,
    pub slot: u8,
}

/// Preserve wire order: the lane's first/second item names do not reliably denote destinations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateEquipmentEntry {
    pub first: MateItemLocation,
    pub second: MateItemLocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateEquipmentChange {
    /// Stored character ID, not its runtime object ID.
    pub character_id: u32,
    pub mate_tl_id: u16,
    pub passenger_id: u32,
    /// Raw native flag; no inferred passenger or transfer semantics.
    pub bts: bool,
    pub entries: Vec<MateEquipmentEntry>,
    /// Wire flag only. The lane writes true even for rejected moves.
    pub success: bool,
}

impl MateEquipmentChange {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let character_id = r.u32()?;
        let mate_tl_id = r.u16()?;
        let passenger_id = r.u32()?;
        let bts = r.bool()?;
        let count = usize::from(r.u8()?);
        if count > MAX_MATE_EQUIPMENT_ENTRIES || count > r.remaining().saturating_sub(1) / 12 {
            return Err(WireError::Invalid(
                "invalid mate equipment entry count".into(),
            ));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let first = Item::read(r)?;
            let second = Item::read(r)?;
            entries.push(MateEquipmentEntry {
                first: MateItemLocation {
                    item: first,
                    container: r.u8()?,
                    slot: r.u8()?,
                },
                second: MateItemLocation {
                    item: second,
                    container: r.u8()?,
                    slot: r.u8()?,
                },
            });
        }
        Ok(Self {
            character_id,
            mate_tl_id,
            passenger_id,
            bts,
            entries,
            success: r.bool()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::mate::{MateEvent, Mates};
    use crate::model::CharacterState;
    use crate::session::Event;
    use crate::wire::Writer;

    fn body(count: u8, success: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(42).u16(0x102).u32(0xf1234567).bool(true).u8(count);
        for n in 0..count {
            // One full tag-9 opaque item and an empty item, then both raw locations.
            w.u32(123)
                .u64(0x1122334455667788 + u64::from(n))
                .u8(2)
                .u8(3)
                .i32(4);
            w.u8(9).raw(&[0x81, 0x82, 0x83, 0x84]);
            w.i64(10).i32(11).u32(12).u8(13).i64(14).i64(15);
            w.u32(0).u8(2).u8(n).u8(252).u8(3 + n);
        }
        w.bool(success);
        w.finish()
    }

    #[test]
    fn native_two_entry_bound_full_items_and_trailing_flag_preserve_dd04() {
        for count in 0..=2 {
            for success in [false, true] {
                let raw = body(count, success);
                let mut r = Reader::new(&raw);
                let change = MateEquipmentChange::read(&mut r).unwrap();
                assert!(r.is_empty());
                assert_eq!(
                    (
                        change.character_id,
                        change.mate_tl_id,
                        change.passenger_id,
                        change.bts
                    ),
                    (42, 0x102, 0xf1234567, true)
                );
                assert_eq!(change.success, success);
                assert_eq!(change.entries.len(), usize::from(count));
                for (n, entry) in change.entries.iter().enumerate() {
                    let item = entry.first.item.as_ref().unwrap();
                    assert_eq!(item.id, 0x1122334455667788 + n as u64);
                    assert_eq!(
                        item.detail_payload.as_deref(),
                        Some(&[0x81, 0x82, 0x83, 0x84][..])
                    );
                    assert_eq!((entry.first.container, entry.first.slot), (2, n as u8));
                    assert_eq!(
                        entry.second,
                        MateItemLocation {
                            item: None,
                            container: 252,
                            slot: 3 + n as u8
                        }
                    );
                }
                let mut inner = Writer::new();
                inner
                    .u16(0)
                    .u16(0x126)
                    .raw(&raw)
                    .u16(0)
                    .u16(crate::opcodes::sc::SCOnOffSnowPacket)
                    .bool(true);
                let inner = inner.finish();
                let mut outer = Writer::new();
                let len = inner.len() as u16;
                outer
                    .raw(&[0xdd, 4])
                    .u16(2)
                    .u8(1)
                    .u16(len)
                    .u16(!len)
                    .raw(&inner);
                assert_eq!(
                    ServerMsg::decode(&outer.finish()).unwrap(),
                    ServerMsg::Batch(vec![
                        ServerMsg::Mate(MateEvent::EquipmentChanged(change)),
                        ServerMsg::Snow(true)
                    ])
                );
                for n in 0..raw.len() {
                    assert!(
                        MateEquipmentChange::read(&mut Reader::new(&raw[..n])).is_err(),
                        "cut {n}"
                    );
                }
            }
        }
        for count in [3, 255] {
            let mut raw = body(0, true);
            raw[11] = count;
            assert!(MateEquipmentChange::read(&mut Reader::new(&raw)).is_err());
        }
    }

    #[test]
    fn only_matching_owner_and_active_timeline_cache_without_settlement() {
        let mut state = crate::state::WorldState::default();
        let mut selected = CharacterState::default();
        selected.character.id = 42;
        state.apply(&Event::CharacterSelected(Box::new(selected)));
        state.apply(&Event::Mate(MateEvent::Spawned(
            crate::mate::test_spawn_fixture(),
        )));
        let change = MateEquipmentChange::read(&mut Reader::new(&body(1, true))).unwrap();
        let before_inventory = state.inventory.clone();
        state.apply(&Event::Mate(MateEvent::EquipmentChanged(change.clone())));
        assert_eq!(state.mates.last_equipment_change, Some(change.clone()));
        assert_eq!(state.inventory.bag, before_inventory.bag);
        assert_eq!(state.inventory.equipment, before_inventory.equipment);
        assert!(state.units.by_id.is_empty());
        for mismatch in [
            MateEquipmentChange {
                character_id: 99,
                ..change.clone()
            },
            MateEquipmentChange {
                mate_tl_id: 999,
                ..change.clone()
            },
        ] {
            assert!(
                !state
                    .mates
                    .apply(&Event::Mate(MateEvent::EquipmentChanged(mismatch)))
            );
            assert_eq!(state.mates.last_equipment_change, Some(change.clone()));
        }
        state.apply(&Event::Mate(MateEvent::Spawned(
            crate::mate::test_spawn_fixture(),
        )));
        assert_eq!(state.mates.last_equipment_change, None);
        state
            .mates
            .apply(&Event::Mate(MateEvent::EquipmentChanged(change)));
        state.mates.apply(&Event::Disconnected {
            message: "test".into(),
        });
        assert_eq!(state.mates, Mates::default());
    }
}
