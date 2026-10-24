//! Appearance requests with source-qualified, captured BAG records.
//!
//! Native `39047650`/`390474a0` sends CS049 (base ID, image ID). Cleanup and
//! Skinize use the item-to-item CS052 shape. The isolated native cursor oracle
//! captures inputs at protected planner `39396e20`, then separately executes
//! the original serializers; it does not execute that planner's admission.
//! Catalog, planner, permission and UI mode gates remain caller-owned.

use crate::{NativeItemRecords, item_effect::CapturedBagItem, wire::Writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemLookOperation {
    Convert,
    Cleanup,
    Skinize,
}

/// A transport request, not a successful conversion or an ordinary skill planner.
/// Full-record capture equality is a stronger application stale-input boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemLookRequest {
    operation: ItemLookOperation,
    skill_id: Option<u32>,
    source: CapturedBagItem,
    target: CapturedBagItem,
}

impl ItemLookRequest {
    /// Apply `image` to `base`. Only their ordered u64 IDs go on the wire.
    pub fn convert(base: CapturedBagItem, image: CapturedBagItem) -> Result<Self, &'static str> {
        Self::new(ItemLookOperation::Convert, None, image, base)
    }

    /// Clear a target's image using an explicitly catalog-resolved powder skill.
    pub fn cleanup(
        skill_id: u32,
        source: CapturedBagItem,
        target: CapturedBagItem,
    ) -> Result<Self, &'static str> {
        Self::new(ItemLookOperation::Cleanup, Some(skill_id), source, target)
    }

    /// Make the target an image using an explicitly catalog-resolved powder skill.
    pub fn skinize(
        skill_id: u32,
        source: CapturedBagItem,
        target: CapturedBagItem,
    ) -> Result<Self, &'static str> {
        Self::new(ItemLookOperation::Skinize, Some(skill_id), source, target)
    }

    fn new(
        operation: ItemLookOperation,
        skill_id: Option<u32>,
        source: CapturedBagItem,
        target: CapturedBagItem,
    ) -> Result<Self, &'static str> {
        if skill_id == Some(0) {
            return Err("item look requires the catalog-resolved use skill");
        }
        if [&source, &target]
            .into_iter()
            .any(|v| v.item.id == 0 || v.item.template_id == 0)
        {
            return Err("item look requires captured existing items");
        }
        Ok(Self {
            operation,
            skill_id,
            source,
            target,
        })
    }

    pub fn operation(&self) -> ItemLookOperation {
        self.operation
    }

    /// Dedicated conversion has no skill ID.
    pub fn skill_id(&self) -> Option<u32> {
        self.skill_id
    }

    /// Conversion's image, or the cleanup/Skinize powder.
    pub fn source(&self) -> &CapturedBagItem {
        &self.source
    }

    /// Conversion's base, or the cleanup/Skinize recipient.
    pub fn target(&self) -> &CapturedBagItem {
        &self.target
    }

    pub fn captures(&self) -> impl Iterator<Item = &CapturedBagItem> {
        [&self.source, &self.target].into_iter()
    }

    /// Recheck full records at the same physical slots. Unknown records never
    /// fall back to the legacy inventory. The session separately checks owner.
    pub fn recheck_native(&self, records: &NativeItemRecords) -> Result<(), &'static str> {
        for captured in self.captures() {
            if records.item(2, captured.slot) != Some(&captured.item) {
                return Err("captured native item look BAG slot is unknown or changed");
            }
        }
        Ok(())
    }

    pub(crate) fn opcode(&self) -> u16 {
        match self.operation {
            ItemLookOperation::Convert => crate::opcodes::cs::CSConvertItemLookPacket,
            ItemLookOperation::Cleanup | ItemLookOperation::Skinize => {
                crate::opcodes::cs::CSStartSkillPacket
            }
        }
    }

    pub(crate) fn write(&self, w: &mut Writer, self_obj_id: u32) {
        match self.skill_id {
            None => {
                w.u64(self.target.item.id).u64(self.source.item.id);
            }
            Some(skill_id) => crate::skill::write_item_effect_start(
                w,
                skill_id,
                self_obj_id,
                &self.source.item,
                &self.target.item,
                None,
                None,
            ),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
