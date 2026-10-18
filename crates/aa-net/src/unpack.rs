//! Original ITEM_UNPACK dialog acceptance: an identity-qualified request, never an equipment continuation.
use crate::{items::container, state::Inventory, wire::Writer};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnpackItemRequest {
    container: u8,
    slot: u8,
    item_id: u64,
}
impl UnpackItemRequest {
    pub fn new(container: u8, slot: u8, item_id: u64) -> Result<Self, &'static str> {
        if item_id == 0 {
            return Err("unpack requires an existing item database identity");
        }
        Ok(Self { container, slot, item_id })
    }
    pub fn container(&self) -> u8 {
        self.container
    }
    pub fn slot(&self) -> u8 {
        self.slot
    }
    pub fn item_id(&self) -> u64 {
        self.item_id
    }
    /// Stronger app preflight than the native dialog's slot-only check. No local item mutation.
    pub fn recheck(&self, inventory: &Inventory) -> Result<(), &'static str> {
        let index = usize::from(self.slot);
        let current = match self.container {
            container::INVENTORY if u32::from(self.slot) < inventory.bag_slots => inventory.bag.get(index),
            container::BANK if u32::from(self.slot) < inventory.bank_slots => inventory.bank.get(index),
            container::EQUIPMENT => inventory.equipment.slots.get(index),
            _ => return Err("unpack location is not present in the received owned inventory"),
        };
        if current
            .and_then(Option::as_ref)
            .is_some_and(|item| item.id == self.item_id && item.template_id != 0)
        {
            Ok(())
        } else {
            Err("unpack item identity or location changed")
        }
    }
    pub(crate) fn write(&self, w: &mut Writer) {
        w.u8(self.container).u8(self.slot).u64(self.item_id);
    }
}
#[cfg(test)]
mod tests;
