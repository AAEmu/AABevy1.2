//! Native bounded craft-failure notice payload; entry semantics stay uninterpreted.

use crate::wire::{Reader, Result};

/// Native reader397b8110 and handler394d2370 both cap the inline list at twenty entries.
pub const MAX_CRAFT_FAILURE_ENTRIES: i32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CraftFailureNotice {
    /// First u32 is not used by native OnCraftFailed; no recipe/object role is inferred.
    pub raw_id: u32,
    /// Original signed count survives even when negative or larger than the native inline cap.
    pub raw_count: i32,
    /// Exactly max(0, min(raw_count, 20)) native u32 values, without filtering or deduplication.
    pub entries: Vec<u32>,
}

impl CraftFailureNotice {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let raw_id = r.u32()?;
        let raw_count = r.i32()?;
        let entries = (0..raw_count.clamp(0, MAX_CRAFT_FAILURE_ENTRIES))
            .map(|_| r.u32())
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            raw_id,
            raw_count,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::opcodes::sc;
    use crate::wire::Writer;

    #[test]
    fn native_signed_count_and_inline_cap_preserve_the_following_dd04_packet() {
        for count in [i32::MIN, -1, 0, 1, 20, 21, i32::MAX] {
            let values: Vec<u32> = (0..count.clamp(0, 20))
                .map(|n| match n % 3 {
                    0 => 0,
                    1 => u32::MAX,
                    _ => 0x80000000 + n as u32,
                })
                .collect();
            let mut w = Writer::new();
            w.u32(0xfedcba98).i32(count);
            for value in &values {
                w.u32(*value);
            }
            let raw = w.finish();
            let expected = CraftFailureNotice {
                raw_id: 0xfedcba98,
                raw_count: count,
                entries: values,
            };
            let mut r = Reader::new(&raw);
            assert_eq!(CraftFailureNotice::read(&mut r).unwrap(), expected);
            assert!(r.is_empty());
            for n in 0..raw.len() {
                assert!(
                    CraftFailureNotice::read(&mut Reader::new(&raw[..n])).is_err(),
                    "count {count} cut {n}"
                );
            }
            let mut inner = Writer::new();
            inner
                .u16(0)
                .u16(0x1bf)
                .raw(&raw)
                .u16(0)
                .u16(sc::SCOnOffSnowPacket)
                .bool(true);
            let inner = inner.finish();
            let len = inner.len() as u16;
            let mut outer = Writer::new();
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
                    ServerMsg::CraftFailureNotice(expected),
                    ServerMsg::Snow(true),
                ])
            );
        }
    }

    #[test]
    fn lane_unused_writer_golden_is_preserved_without_claiming_recipe_semantics() {
        // economy/craft_store_tests.rs::sc_craft_failed_bytes independent golden.
        let raw = [7, 0, 0, 0, 2, 0, 0, 0, 0x34, 0x12, 0, 0, 0x34, 0x12, 0, 0];
        let notice = CraftFailureNotice::read(&mut Reader::new(&raw)).unwrap();
        assert_eq!(
            notice,
            CraftFailureNotice {
                raw_id: 7,
                raw_count: 2,
                entries: vec![0x1234, 0x1234]
            }
        );
    }
}
