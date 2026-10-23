//! Captured native SaveNotes item cast. Score staging and the fired callback stay in the app.

use crate::{NativeItemRecords, item_effect::CapturedBagItem, items::container, wire::Writer};

/// Native Music Paper (`items.id` 27902) use skill. Opening its composer is local.
pub const COMPOSE_SKILL_ID: u32 = 22215;

/// Explicit SaveNotes cast, separate from uploading the locally staged score.
///
/// The caller selects `grade` from `music_note_limits.step`: the first row whose
/// `note_length >= XlStringLen(notes)`. Compilation, proficiency and confirmation
/// remain caller-owned. This request neither uploads notes nor acknowledges a save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicComposeRequest {
    source: CapturedBagItem,
    grade: u8,
}

impl MusicComposeRequest {
    pub fn new(source: CapturedBagItem, grade: u8) -> Result<Self, &'static str> {
        if source.item.id == 0 || source.item.template_id != 27902 {
            return Err("music composition requires captured Music Paper 27902");
        }
        // Deliberate transport boundary for the verified original rows, not a
        // replacement for selecting the grade from the actual static table.
        if grade > 10 {
            return Err("music composition grade must be a verified limit step 0..10");
        }
        Ok(Self { source, grade })
    }

    pub fn source(&self) -> &CapturedBagItem {
        &self.source
    }

    pub fn grade(&self) -> u8 {
        self.grade
    }

    pub fn skill_id(&self) -> u32 {
        COMPOSE_SKILL_ID
    }

    /// Recheck the complete received item at its physical main-bag slot.
    /// Unknown records never fall back to legacy inventory or a UID-only lookup.
    /// The session separately guards active character, native owner and own actor.
    pub fn recheck_native(&self, records: &NativeItemRecords) -> Result<(), &'static str> {
        if records.item(container::INVENTORY, self.source.slot) != Some(&self.source.item) {
            return Err("captured native music paper BAG slot is unknown or changed");
        }
        Ok(())
    }

    /// Exact native CS052 body. Callers bypassing the session must check identity
    /// and `recheck_native` themselves. The item grade and composition grade differ.
    pub fn write(&self, w: &mut Writer, self_obj_id: u32) {
        let item = &self.source.item;
        w.u32(COMPOSE_SKILL_ID)
            .u8(crate::buff::caster_kind::ITEM)
            .bc(self_obj_id)
            .u64(item.id)
            .u32(item.template_id)
            .u8(item.grade)
            .u32(item.made_unit_id);
        crate::skill::SkillTarget::unit(self_obj_id).write(w);
        // Original 397bfd50 type5 ->397bff73 writes a 32-bit integer, not u8.
        w.u8(5).u32(u32::from(self.grade));
    }
}
