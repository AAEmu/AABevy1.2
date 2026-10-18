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

/// Source1.2 controller mode. Socket covers its install/remove skill IDs23728/23729.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemEffectMode {
    Grade { auto_use_aa_point: bool },
    Socket,
    Gem,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedBagItem {
    pub slot: u8,
    pub item: Item,
}
/// Explicit transport request, not an implementation of the protected ordinary skill planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemEffectRequest {
    mode: ItemEffectMode,
    skill_id: u32,
    source: CapturedBagItem,
    target: CapturedBagItem,
    support: Option<CapturedBagItem>,
}
impl ItemEffectRequest {
    pub fn new(
        mode: ItemEffectMode,
        skill_id: u32,
        source: CapturedBagItem,
        target: CapturedBagItem,
        support: Option<CapturedBagItem>,
    ) -> std::result::Result<Self, &'static str> {
        if skill_id == 0 {
            return Err("item effect requires the catalog-resolved use skill");
        }
        if matches!(mode, ItemEffectMode::Socket) && !matches!(skill_id, 23728 | 23729) {
            return Err("native socket skill must be 23728 or 23729");
        }
        if matches!(mode, ItemEffectMode::Gem) && skill_id != 11285 {
            return Err("native gem skill must be 11285");
        }
        if !matches!(mode, ItemEffectMode::Grade { .. }) && support.is_some() {
            return Err("support is only part of grade enchanting");
        }
        if std::iter::once(&source)
            .chain(std::iter::once(&target))
            .chain(support.iter())
            .any(|v| v.item.id == 0 || v.item.template_id == 0)
        {
            return Err("item effect requires captured existing items");
        }
        Ok(Self {
            mode,
            skill_id,
            source,
            target,
            support,
        })
    }
    pub fn mode(&self) -> ItemEffectMode {
        self.mode
    }
    pub fn skill_id(&self) -> u32 {
        self.skill_id
    }
    pub fn source(&self) -> &CapturedBagItem {
        &self.source
    }
    pub fn target(&self) -> &CapturedBagItem {
        &self.target
    }
    pub fn support(&self) -> Option<&CapturedBagItem> {
        self.support.as_ref()
    }
    pub fn captures(&self) -> impl Iterator<Item = &CapturedBagItem> {
        std::iter::once(&self.source).chain(std::iter::once(&self.target)).chain(self.support.iter())
    }
    /// Stronger application stale-input check. Catalog/planner/permission checks remain caller-owned.
    pub fn recheck(&self, inventory: &crate::state::Inventory) -> std::result::Result<(), &'static str> {
        for captured in self.captures() {
            if u32::from(captured.slot) >= inventory.bag_slots || inventory.bag.get(usize::from(captured.slot)).and_then(Option::as_ref) != Some(&captured.item)
            {
                return Err("captured item effect BAG slot changed");
            }
        }
        if let ItemEffectMode::Grade { auto_use_aa_point } = self.mode {
            if auto_use_aa_point != (inventory.auto_use_aa_point_raw == 1) {
                return Err("captured AA point setting changed");
            }
        }
        Ok(())
    }
    pub fn write(&self, w: &mut crate::wire::Writer, self_obj_id: u32) {
        let auto = match self.mode {
            ItemEffectMode::Grade { auto_use_aa_point } => Some(auto_use_aa_point),
            _ => None,
        };
        crate::skill::write_item_effect_start(
            w,
            self.skill_id,
            self_obj_id,
            &self.source.item,
            &self.target.item,
            self.support.as_ref().map(|v| &v.item),
            auto,
        );
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    fn pair() -> (CapturedBagItem, CapturedBagItem) {
        (
            CapturedBagItem {
                slot: 0,
                item: Item {
                    id: 0x0102030405060708,
                    template_id: 0x11223344,
                    grade: 6,
                    made_unit_id: 0x66778899,
                    ..Default::default()
                },
            },
            CapturedBagItem {
                slot: 1,
                item: Item {
                    id: 0x1112131415161718,
                    template_id: 0xaabbccdd,
                    grade: 7,
                    ..Default::default()
                },
            },
        )
    }
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    #[test]
    fn original_complete052_payload_goldens_preserve_caster_target_and_object() {
        let (source, target) = pair();
        let support = CapturedBagItem {
            slot: 2,
            item: Item {
                id: 0x2122232425262728,
                template_id: 0x55667788,
                ..Default::default()
            },
        };
        let base = "b05c0000020302010807060504030201443322110699887766030302011817161514131211ddccbbaa07";
        for (mode, support, suffix) in [
            (ItemEffectMode::Socket, None, "00"),
            (ItemEffectMode::Grade { auto_use_aa_point: false }, None, "0700000000000000000000000000"),
            (ItemEffectMode::Grade { auto_use_aa_point: true }, Some(support), "0788776655282726252423222101"),
        ] {
            let request = ItemEffectRequest::new(mode, 23728, source.clone(), target.clone(), support).unwrap();
            let msg = crate::game::ClientMsg::ItemEffect {
                self_obj_id: 0x010203,
                request,
            };
            assert_eq!(msg.encode_body(), hex(&format!("{base}{suffix}")));
            assert_eq!(&msg.encode(1)[4..6], &0x52u16.to_le_bytes());
        }
    }
    #[test]
    fn captures_refuse_wrong_skill_changed_identity_grade_maker_and_auto_setting() {
        let (source, target) = pair();
        assert!(ItemEffectRequest::new(ItemEffectMode::Socket, 11285, source.clone(), target.clone(), None).is_err());
        assert!(ItemEffectRequest::new(ItemEffectMode::Gem, 23728, source.clone(), target.clone(), None).is_err());
        let request = ItemEffectRequest::new(ItemEffectMode::Grade { auto_use_aa_point: true }, 1, source.clone(), target.clone(), None).unwrap();
        let mut inv = crate::state::Inventory::default();
        inv.bag_slots = 2;
        inv.bag = vec![Some(source.item), Some(target.item)];
        inv.auto_use_aa_point_raw = 1;
        assert!(request.recheck(&inv).is_ok());
        for field in 0..4 {
            let mut changed = inv.clone();
            let item = changed.bag[0].as_mut().unwrap();
            match field {
                0 => item.id += 1,
                1 => item.grade += 1,
                2 => item.made_unit_id += 1,
                _ => item.template_id += 1,
            };
            assert!(request.recheck(&changed).is_err());
        }
        inv.auto_use_aa_point_raw = 0;
        assert!(request.recheck(&inv).is_err());
        inv.auto_use_aa_point_raw = 1;
        inv.bag_slots = 1;
        assert!(request.recheck(&inv).is_err());
    }
}
