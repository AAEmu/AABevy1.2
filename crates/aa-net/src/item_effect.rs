//! Received grade-enchant results. Inline item bytes remain available without rewriting them.
use crate::{
    model::Item,
    wire::{Reader, Result},
};

#[derive(Clone, Debug, PartialEq)]
pub struct GradeEnchantResult {
    pub raw_result: u8,
    pub item: Option<Item>,
    pub item_wire: Vec<u8>,
    pub before_grade: u8,
    pub after_grade: u8,
}
impl GradeEnchantResult {
    pub fn read(r: &mut Reader<'_>) -> Result<Self> {
        let raw_result = r.u8()?;
        let before = r.rest();
        let item = Item::read(r)?;
        let item_wire = before[..before.len() - r.remaining()].to_vec();
        Ok(Self {
            raw_result,
            item,
            item_wire,
            before_grade: r.u8()?,
            after_grade: r.u8()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct GradeEnchantBroadcast {
    pub raw_name: Vec<u8>,
    pub result: GradeEnchantResult,
}
impl GradeEnchantBroadcast {
    pub fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            raw_name: r.blob()?.to_vec(),
            result: GradeEnchantResult::read(r)?,
        })
    }
    /// Native CString projection. Refuse names exceeding its 128-byte destination capacity;
    /// this safety boundary never changes framing or the retained received bytes.
    pub fn native_name(&self) -> Option<&[u8]> {
        if self.raw_name.len() > 128 {
            return None;
        }
        Some(&self.raw_name[..self.raw_name.iter().position(|&b| b == 0).unwrap_or(self.raw_name.len())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_null_item_goldens_and_name_safety() {
        // Original397c2070/397c2100 +3961f500 executed-byte writer oracle.
        for raw in [0, 4, 255] {
            let bytes = [raw, 0, 0, 0, 0, 1, 6];
            let result = GradeEnchantResult::read(&mut Reader::new(&bytes)).unwrap();
            assert_eq!(result.raw_result, raw);
            assert_eq!(result.item, None);
            assert_eq!(result.item_wire, [0; 4]);
            assert_eq!((result.before_grade, result.after_grade), (1, 6));
            for n in 0..bytes.len() {
                assert!(GradeEnchantResult::read(&mut Reader::new(&bytes[..n])).is_err());
            }
            for name in [vec![], vec![255, b'A', 0, b'B'], vec![b'A'; 128], vec![b'A'; 129]] {
                let mut wire = (name.len() as u16).to_le_bytes().to_vec();
                wire.extend_from_slice(&name);
                wire.extend_from_slice(&bytes);
                let value = GradeEnchantBroadcast::read(&mut Reader::new(&wire)).unwrap();
                assert_eq!(value.raw_name, name);
                assert_eq!(value.result, result);
                assert_eq!(value.native_name().is_some(), name.len() <= 128);
                if name.first() == Some(&255) {
                    assert_eq!(value.native_name(), Some(&[255, b'A'][..]));
                }
                for n in 0..wire.len() {
                    assert!(GradeEnchantBroadcast::read(&mut Reader::new(&wire[..n])).is_err());
                }
            }
        }
    }
}
