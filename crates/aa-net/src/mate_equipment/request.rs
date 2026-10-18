//! Full native CSChangeMateEquipment echoes. This packet does not predict a swap.
use super::{MAX_MATE_EQUIPMENT_ENTRIES, MateEquipmentEntry};
use crate::{
    model::{Item, ItemDetail},
    wire::{Reader, Writer},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MateEquipmentRequest {
    /// Stored character ID, not runtime unit ID.
    pub character_id: u32,
    pub mate_tl_id: u16,
    pub passenger_id: u32,
    pub bts: bool,
    entries: Vec<MateEquipmentEntry>,
}
impl MateEquipmentRequest {
    /// First location is physical bag2; second is mate equipmentfc. Slots are zero-based.
    /// Item echoes must be actual full equipment with retained55-byte detail payload.
    pub fn new(character_id: u32, mate_tl_id: u16, passenger_id: u32, bts: bool, entries: Vec<MateEquipmentEntry>) -> Result<Self, &'static str> {
        let r = Self {
            character_id,
            mate_tl_id,
            passenger_id,
            bts,
            entries,
        };
        r.validate()?;
        Ok(r)
    }
    pub fn entries(&self) -> &[MateEquipmentEntry] {
        &self.entries
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.character_id == 0 || self.mate_tl_id == 0 {
            return Err("mate equipment requires stored owner and timeline identity");
        }
        if self.entries.is_empty() || self.entries.len() > MAX_MATE_EQUIPMENT_ENTRIES {
            return Err("mate equipment needs one or two entries");
        }
        for e in &self.entries {
            if e.first.container != 2 || e.second.container != 0xfc || e.second.slot >= 28 {
                return Err("mate equipment locations require physical bag2 and raw mate slot0..27 in fc");
            }
            if e.first.item.is_none() && e.second.item.is_none() {
                return Err("mate equipment entry has no existing item");
            }
            for item in [&e.first.item, &e.second.item].into_iter().flatten() {
                validate_item(item)?;
            }
        }
        Ok(())
    }
    pub(crate) fn write(&self, w: &mut Writer) {
        w.u32(self.character_id)
            .u16(self.mate_tl_id)
            .u32(self.passenger_id)
            .bool(self.bts)
            .u8(self.entries.len() as u8);
        for e in &self.entries {
            write_item(w, e.first.item.as_ref());
            write_item(w, e.second.item.as_ref());
            w.u8(e.first.container).u8(e.first.slot).u8(e.second.container).u8(e.second.slot);
        }
    }
}
fn validate_item(i: &Item) -> Result<(), &'static str> {
    if i.template_id == 0 || i.id == 0 || !matches!(i.detail, ItemDetail::Equip(_)) {
        return Err("mate equipment echoes require an existing full equipment item");
    }
    let Some(raw) = i.detail_payload.as_deref() else {
        return Err("mate equipment echo lacks received detail bytes");
    };
    if raw.len() != 55 {
        return Err("mate equipment detail must be exactly55 bytes");
    }
    let mut r = Reader::new(raw);
    if !ItemDetail::read(&mut r, 1).is_ok_and(|d| d == i.detail && r.is_empty()) {
        return Err("mate equipment detail disagrees with its received bytes");
    }
    Ok(())
}
fn write_item(w: &mut Writer, i: Option<&Item>) {
    let Some(i) = i else {
        w.u32(0);
        return;
    };
    w.u32(i.template_id)
        .u64(i.id)
        .u8(i.grade)
        .u8(i.flags)
        .i32(i.count)
        .u8(1)
        .raw(i.detail_payload.as_deref().expect("validated immutable equipment echo"))
        .i64(i.create_time)
        .i32(i.lifespan_mins)
        .u32(i.made_unit_id)
        .u8(i.world_id)
        .i64(i.unsecure_time)
        .i64(i.unpack_time);
}
#[cfg(test)]
mod tests;
