//! The hotbar, the bag and bank, and the equipment of the local player.

use crate::items::{container, ActionSlot, ItemMove, ItemTask, ACTION_PAGES, ACTION_PAGE_SLOTS, ACTION_SLOTS};
use crate::model::{Equipment, Item, slot};
use crate::session::Event;

resource! {
    /// The hotbar: 85 slots, index = the `CSUpdateActionSlot` slot number (1 based, 0 unused),
    /// and the current page (0 based; the scripts count pages from 1).
    pub struct ActionBar {
        pub slots: Vec<ActionSlot>,
        pub page: u8,
    }
}

impl ActionBar {
    pub fn get(&self, slot: usize) -> Option<&ActionSlot> {
        self.slots.get(slot).filter(|s| !s.is_empty())
    }

    /// The current page, 0 based (Lua's `GetActionBarPage` adds one).
    pub fn page(&self) -> u8 {
        self.page
    }

    /// `FUN_393f6910`: page >= 4 is refused (the page stays where it was).
    pub fn set_page(&mut self, page: u8) -> bool {
        if page >= ACTION_PAGES {
            return false;
        }
        self.page = page;
        true
    }

    /// The first global slot a page shows: 1, 49, 61, 73 (`FUN_392c5a50`, `(page + 3) * 12`).
    pub fn page_base(page: u8) -> Option<usize> {
        match page {
            0 => Some(1),
            1..=3 => Some(49 + usize::from(page - 1) * ACTION_PAGE_SLOTS),
            _ => None,
        }
    }

    /// The global slot of the 0-based `index` of the page-managed main bar.
    pub fn page_slot(page: u8, index: usize) -> Option<usize> {
        if index >= ACTION_PAGE_SLOTS {
            return None;
        }
        let slot = Self::page_base(page)? + index;
        (slot < ACTION_SLOTS).then_some(slot)
    }

    /// The global slot of the 0-based `display` index of the action bar: its first 12 slots
    /// follow the page, the two 6-slot sub bars stay at 13..24, later bars keep their own slot.
    pub fn display_slot(&self, display: usize) -> Option<usize> {
        Self::display_slot_on(self.page, display)
    }

    pub fn display_slot_on(page: u8, display: usize) -> Option<usize> {
        if display < ACTION_PAGE_SLOTS {
            Self::page_slot(page, display)
        } else {
            (display + 1 < ACTION_SLOTS).then_some(display + 1)
        }
    }

    /// The count an item slot shows (the potions on the bottom bar): all of the template in the
    /// bag for an `ItemType` slot, the stack for an `ItemId` slot. `None` for other slots.
    pub fn item_count(&self, slot: usize, inventory: &Inventory) -> Option<i64> {
        let s = self.slots.get(slot)?;
        if let Some(template) = s.item_template_id() {
            return Some(inventory.count_of(template));
        }
        let id = s.item_id()?;
        Some(inventory.find_by_id(id).map_or(0, |(_, _, item)| i64::from(item.count)))
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::ActionSlots(slots) => self.slots = slots.clone(),
            Event::ActionSlotChanged { slot, action } => {
                let slot = usize::from(*slot);
                if slot >= ACTION_SLOTS {
                    return false;
                }
                if self.slots.len() <= slot {
                    self.slots.resize(slot + 1, ActionSlot::default());
                }
                self.slots[slot] = *action;
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.slots.is_empty() && self.page == 0 {
                    return false;
                }
                self.slots.clear();
                self.page = 0;
            }
            _ => return false,
        }
        true
    }
}

resource! {
    /// The player's items. `bag` and `bank` are indexed by slot (`None` = empty); items carry the
    /// template id, database id, count, grade and details.
    pub struct Inventory {
        pub bag_slots: u32,
        pub bank_slots: u32,
        /// Last server value; native getter is true only for exactly one.
        pub auto_use_aa_point_raw: u8,
        /// Sent request awaiting an authoritative item task.
        pub pending_auto_use_aa_point: Option<bool>,
        /// Native physical-slot pending keys. Explicit UI operations register their proven locks;
        /// this is not an assertion that every outgoing command has a lock producer wired.
        pending_item_slots: std::collections::BTreeSet<(u8, u8)>,
        pub bag: Vec<Option<Item>>,
        pub bank: Vec<Option<Item>>,
        /// The coffer the player has open (container [`container::TRADE`] in moves and tasks).
        pub coffer: crate::coffer::Coffer,
        /// Worn items by `model::slot`.
        pub equipment: Equipment,
        /// The player's object id (set by the spawn), to pick its equipment changes.
        own_obj_id: u32,
    }
}

impl Inventory {
    /// Register a native physical-slot lock after its operation has been accepted locally.
    /// Container4 is deliberately excluded by native397d2200; setting/currency keys are separate.
    /// This changes neither item contents nor the received result of the operation.
    pub fn mark_item_slot_pending(&mut self, kind: u8, slot: u8) -> bool {
        matches!(kind, container::EQUIPMENT | container::INVENTORY | container::BANK)
            && self.pending_item_slots.insert((kind, slot))
    }

    pub fn item_slot_pending(&self, kind: u8, slot: u8) -> bool {
        self.pending_item_slots.contains(&(kind, slot))
    }

    /// Native physical keys are packed as [0, container, 0, slot].
    pub fn pending_item_slot_keys(&self) -> impl Iterator<Item = u32> + '_ {
        self.pending_item_slots.iter().map(|&(kind, slot)| u32::from_le_bytes([0, kind, 0, slot]))
    }

    /// Release a proven local rejection/cancellation. Server reports release through `apply`.
    pub fn clear_item_slot_pending(&mut self, kind: u8, slot: u8) -> bool {
        self.pending_item_slots.remove(&(kind, slot))
    }

    pub fn auto_use_aa_point(&self) -> bool { self.auto_use_aa_point_raw == 1 }

    /// Native UI change/pending guard; the command itself is a transport request.
    pub fn can_set_auto_use_aa_point(&self, value: bool) -> bool {
        self.auto_use_aa_point_raw != u8::from(value) && self.pending_auto_use_aa_point.is_none()
    }

    /// The bag's items with their slot index.
    pub fn bag_items(&self) -> impl Iterator<Item = (usize, &Item)> {
        self.bag.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|it| (i, it)))
    }

    /// The bank's items with zero-based slot indices, loaded at login like the bag.
    pub fn bank_items(&self) -> impl Iterator<Item = (usize, &Item)> {
        self.bank.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|it| (i, it)))
    }

    /// Total count of a template in the bag (the number shown on a potion slot).
    pub fn count_of(&self, template_id: u32) -> i64 {
        self.bag_items()
            .filter(|(_, it)| it.template_id == template_id)
            .map(|(_, it)| i64::from(it.count))
            .sum()
    }

    /// The first stack of a template in the bag: `(slot, item)`.
    pub fn find_template(&self, template_id: u32) -> Option<(usize, &Item)> {
        self.bag_items().find(|(_, it)| it.template_id == template_id)
    }

    /// An item by its database id in the bag, bank or equipment: `(container, slot, item)`.
    pub fn find_by_id(&self, id: u64) -> Option<(u8, usize, &Item)> {
        if id == 0 {
            return None;
        }
        let find = |list: &[Option<Item>]| list.iter().position(|it| it.as_ref().is_some_and(|it| it.id == id));
        if let Some(i) = find(&self.bag) {
            return self.bag[i].as_ref().map(|it| (container::INVENTORY, i, it));
        }
        if let Some(i) = find(&self.bank) {
            return self.bank[i].as_ref().map(|it| (container::BANK, i, it));
        }
        let i = find(&self.equipment.slots)?;
        self.equipment.slots[i].as_ref().map(|it| (container::EQUIPMENT, i, it))
    }

    pub fn free_bag_slots(&self) -> usize {
        self.bag.iter().filter(|s| s.is_none()).count()
    }

    /// The slot of `container` (grown on demand for the bag and bank).
    fn slot_mut(&mut self, container: u8, index: usize) -> Option<&mut Option<Item>> {
        let list = match container {
            container::INVENTORY => &mut self.bag,
            container::BANK => &mut self.bank,
            container::TRADE if self.coffer.is_open() => &mut self.coffer.slots,
            container::EQUIPMENT => return self.equipment.slots.get_mut(index),
            _ => return None,
        };
        if list.len() <= index {
            list.resize(index + 1, None);
        }
        list.get_mut(index)
    }

    /// Finds the slot of an item by id when the packet's slot does not hold it.
    fn locate(&mut self, container: u8, slot: usize, item_id: u64) -> Option<&mut Option<Item>> {
        let holds = |s: &Option<Item>| s.as_ref().is_some_and(|it| item_id == 0 || it.id == item_id);
        if self.slot_mut(container, slot).is_some_and(|s| holds(s)) {
            return self.slot_mut(container, slot);
        }
        let list: &mut [Option<Item>] = match container {
            container::INVENTORY => &mut self.bag,
            container::BANK => &mut self.bank,
            container::TRADE => &mut self.coffer.slots,
            container::EQUIPMENT => &mut self.equipment.slots,
            _ => return None,
        };
        list.iter_mut().find(|s| s.as_ref().is_some_and(|it| it.id == item_id))
    }

    fn apply_task(&mut self, task: &ItemTask) -> bool {
        match task {
            ItemTask::Add { container, slot, item, .. } => {
                let Some(s) = self.slot_mut(*container, usize::from(*slot)) else { return false };
                *s = Some(item.clone());
            }
            ItemTask::Count { container, slot, item_id, delta, .. } => {
                let Some(s) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                let Some(item) = s.as_mut() else { return false };
                item.count += delta;
                if item.count <= 0 {
                    *s = None;
                }
            }
            ItemTask::Grade { container, slot, item_id, grade } => {
                let Some(Some(item)) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                item.grade = *grade;
            }
            ItemTask::Flags { container, slot, item_id, bits } => {
                let Some(Some(item)) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                // Native action10/397cdc90 sets bits; action11 is a separate full replacement.
                item.flags |= *bits;
            }
            ItemTask::Security { container,slot,item_id,bits,unsecure_time,unpack_time,.. } => {
                if *item_id==0 {return false}
                let list: &mut [Option<Item>] = match *container {
                    container::INVENTORY=>&mut self.bag,container::BANK=>&mut self.bank,
                    container::EQUIPMENT=>&mut self.equipment.slots,_=>return false,
                };
                let Some(Some(item))=list.get_mut(usize::from(*slot)) else{return false};
                if item.id!=*item_id {return false}
                item.flags=*bits;item.unsecure_time=*unsecure_time;item.unpack_time=*unpack_time;
            }
            ItemTask::Update { container, slot, item_id, detail, detail_payload, .. } => {
                let Some(Some(item)) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                item.detail = detail.clone();
                item.detail_payload = detail_payload.clone();
            }
            ItemTask::Remove { container, slot, item_id, .. } | ItemTask::RemoveSlot { container, slot, item_id } => {
                // Mail finalization has already moved the server item into escrow before
                // constructing its Remove task, so its address is MAIL, not the old bag slot.
                // The unique ID authoritatively removes that item from the visible inventory.
                if *container == container::MAIL && *item_id != 0 {
                    return self.remove_by_id(*item_id);
                }
                let Some(s) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                *s = None;
            }
            ItemTask::RemoveCrafting { item_id } => return self.remove_by_id(*item_id),
            ItemTask::Move { from_container, from_slot, from_item_id, to_container, to_slot, to_item_id } => {
                // Take both ends first, then put them back crossed: handles empty targets,
                // swaps and moves between the bag, bank and equipment alike. An end with item id 0 is an
                // empty slot (do not look for it: the body parts worn have id 0 too).
                let from = if *from_item_id == 0 {
                    None
                } else {
                    self.locate(*from_container, usize::from(*from_slot), *from_item_id).and_then(Option::take)
                };
                let to = if *to_item_id == 0 {
                    None
                } else {
                    self.locate(*to_container, usize::from(*to_slot), *to_item_id).and_then(Option::take)
                };
                if from.is_none() && to.is_none() {
                    return false;
                }
                if let Some(s) = self.slot_mut(*to_container, usize::from(*to_slot)) {
                    *s = from;
                }
                if let Some(s) = self.slot_mut(*from_container, usize::from(*from_slot)) {
                    *s = to;
                }
            }
            ItemTask::AutoUseAaPoint(value) => {
                self.auto_use_aa_point_raw = *value;
            }
            // Purse and points are the player stats'.
            ItemTask::Money(_)
            | ItemTask::BankMoney(_)
            | ItemTask::GamePoint { .. }
            | ItemTask::AaPoint(_)
            | ItemTask::BankAaPoint(_) => return false,
        }
        true
    }

    fn remove_by_id(&mut self, id: u64) -> bool {
        let mut removed = false;
        for list in [&mut self.bag[..], &mut self.bank[..], &mut self.coffer.slots[..], &mut self.equipment.slots[..]] {
            for s in list.iter_mut() {
                if s.as_ref().is_some_and(|it| it.id == id) {
                    *s = None;
                    removed = true;
                }
            }
        }
        removed
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterSelected(cs) => {
                *self = Self {
                    bag_slots: u32::from(cs.num_inventory_slots),
                    bank_slots: cs.num_bank_slots.max(0) as u32,
                    auto_use_aa_point_raw: u8::from(cs.character.auto_use_aa_point),
                    pending_auto_use_aa_point: None,
                    pending_item_slots: Default::default(),
                    bag: vec![None; usize::from(cs.num_inventory_slots)],
                    bank: vec![None; cs.num_bank_slots.max(0) as usize],
                    coffer: Default::default(),
                    equipment: cs.character.equipment.clone(),
                    own_obj_id: 0,
                };
            }
            Event::AutoUseAaPointRequested(value) => self.pending_auto_use_aa_point = Some(*value),
            Event::CraftItemUnlocked(unlock) if unlock.unlocks_auto_use_aa_point() => {
                return self.pending_auto_use_aa_point.take().is_some();
            }
            Event::CraftItemUnlocked(unlock) => {
                return self.clear_item_slot_pending(unlock.kind, unlock.slot);
            }
            Event::PlayerSpawn(s) => self.own_obj_id = s.obj_id,
            Event::Coffer(_) => return self.coffer.apply(e),
            Event::DoodadsRemoved(_) => return self.coffer.apply(e),
            Event::InventoryInit { bag_slots, bank_slots } => {
                self.bag_slots = *bag_slots;
                self.bank_slots = *bank_slots;
                self.bag.resize(*bag_slots as usize, None);
                self.bank.resize(*bank_slots as usize, None);
            }
            Event::InventoryChunk(chunk) => {
                let first = chunk.first_slot();
                for (i, item) in chunk.items.iter().enumerate() {
                    let Some(s) = self.slot_mut(chunk.container, first + i) else { return false };
                    *s = item.clone();
                }
            }
            Event::InventoryExpanded { container, slots } => {
                let slots = usize::from(*slots);
                match *container {
                    container::INVENTORY => {
                        self.bag_slots = slots as u32;
                        self.bag.resize(slots, None);
                    }
                    container::BANK => {
                        self.bank_slots = slots as u32;
                        self.bank.resize(slots, None);
                    }
                    _ => return false,
                }
            }
            Event::ItemsChanged(changes) => {
                let mut changed = false;
                let mut unlocked = Vec::new();
                for task in &changes.tasks {
                    let applied = self.apply_task(task);
                    changed |= applied;
                    if !applied {
                        continue;
                    }
                    match task {
                        ItemTask::Add { container, slot, .. } | ItemTask::Count { container, slot, .. }
                        | ItemTask::Grade { container, slot, .. } | ItemTask::Remove { container, slot, .. }
                        | ItemTask::RemoveSlot { container, slot, .. } | ItemTask::Update { container, slot, .. }
                        | ItemTask::Flags { container, slot, .. } | ItemTask::Security { container, slot, .. } => unlocked.push((*container, *slot)),
                        ItemTask::Move { from_container, from_slot, to_container, to_slot, .. } => {
                            unlocked.extend([(*from_container, *from_slot), (*to_container, *to_slot)]);
                        }
                        _ => {}
                    }
                }
                // Native sink map unlocks reported keys after the complete apply loop.
                for (kind, slot) in unlocked {
                    changed |= self.clear_item_slot_pending(kind, slot);
                }
                if changes.tasks.iter().any(|task| matches!(task, ItemTask::AutoUseAaPoint(_))) {
                    self.pending_auto_use_aa_point = None;
                }
                for id in &changes.force_remove {
                    changed |= self.remove_by_id(*id);
                }
                return changed;
            }
            Event::EquipmentChanged { obj_id, items } if self.own_obj_id != 0 && *obj_id == self.own_obj_id => {
                for (index, item) in items {
                    if let Some(s) = self.equipment.slots.get_mut(usize::from(*index)) {
                        *s = item.clone();
                    }
                }
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.bag.is_empty() && self.bank.is_empty() && self.bag_slots == 0
                    && self.auto_use_aa_point_raw == 0 && self.pending_auto_use_aa_point.is_none()
                    && self.pending_item_slots.is_empty() && self.coffer == crate::coffer::Coffer::default()
                    && self.equipment.slots.iter().all(Option::is_none) && self.own_obj_id == 0 {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }

    /// The worn item in a `model::slot`.
    pub fn worn(&self, equip_slot: usize) -> Option<&Item> {
        debug_assert!(equip_slot < slot::COUNT);
        self.equipment.get(equip_slot)
    }

    /// The item in a slot of the bag, bank or equipment (`container` as in [`container`]).
    pub fn item_at(&self, container: u8, index: usize) -> Option<&Item> {
        match container {
            container::INVENTORY => self.bag.get(index),
            container::BANK => self.bank.get(index),
            container::TRADE => self.coffer.slots.get(index),
            container::EQUIPMENT => self.equipment.slots.get(index),
            _ => None,
        }?
        .as_ref()
    }

    fn has_slot(&self, container: u8, index: usize) -> bool {
        match container {
            container::INVENTORY => index < self.bag.len(),
            container::BANK => index < self.bank.len(),
            container::TRADE => index < self.coffer.slots.len(),
            container::EQUIPMENT => index < self.equipment.slots.len(),
            _ => false,
        }
    }

    /// The first empty slot of the bag.
    pub fn first_free_bag_slot(&self) -> Option<usize> {
        self.bag.iter().position(Option::is_none)
    }

    pub fn first_free_bank_slot(&self) -> Option<usize> {
        self.bank.iter().position(Option::is_none)
    }

    /// The `CSSwapItems` block that moves what is in `from` onto `to` (and what is there back): bag to bag, bag to bank,
    /// worn to bag. Nothing when both slots are empty or either is not a real slot.
    pub fn plan_move(&self, from: (u8, usize), to: (u8, usize)) -> Option<ItemMove> {
        if !self.has_slot(from.0, from.1) || !self.has_slot(to.0, to.1) {
            return None;
        }
        let slot = |(c, i): (u8, usize)| (c, u8::try_from(i).ok());
        let ((fc, fs), (tc, ts)) = (slot(from), slot(to));
        let (fs, ts) = (fs?, ts?);
        let (a, b) = (self.item_at(fc, from.1), self.item_at(tc, to.1));
        if a.is_none() && b.is_none() || from == to {
            return None;
        }
        Some(ItemMove {
            from_item_id: a.map_or(0, |i| i.id),
            to_item_id: b.map_or(0, |i| i.id),
            from_container: fc,
            from_slot: fs,
            to_container: tc,
            to_slot: ts,
        })
    }

    /// Wear the bag item in `bag_slot` in the worn slot `equip_slot` (a `model::slot`), swapping with what is worn there.
    /// The real client sends it from the worn slot (its item id 0 when empty) to the bag item.
    pub fn plan_equip(&self, bag_slot: usize, equip_slot: usize) -> Option<ItemMove> {
        if !self.has_slot(container::EQUIPMENT, equip_slot) {
            return None;
        }
        let item = self.item_at(container::INVENTORY, bag_slot)?;
        Some(ItemMove {
            from_item_id: self.item_at(container::EQUIPMENT, equip_slot).map_or(0, |i| i.id),
            to_item_id: item.id,
            from_container: container::EQUIPMENT,
            from_slot: u8::try_from(equip_slot).ok()?,
            to_container: container::INVENTORY,
            to_slot: u8::try_from(bag_slot).ok()?,
        })
    }

    /// Take the worn item in `equip_slot` off into the bag slot `bag_slot` (swapping with the item there when it fits).
    pub fn plan_unequip(&self, equip_slot: usize, bag_slot: usize) -> Option<ItemMove> {
        self.item_at(container::EQUIPMENT, equip_slot)?;
        self.plan_move((container::EQUIPMENT, equip_slot), (container::INVENTORY, bag_slot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{action, ActionKind, InvenChunk, ItemTasks};
    use crate::model::{Character, CharacterState, ItemDetail};

    fn item(template_id: u32, id: u64, count: i32) -> Item {
        Item { template_id, id, count, grade: 1, ..Item::default() }
    }

    fn selected(bag: u8) -> Event {
        let mut character = Character::default();
        character.equipment.slots[slot::MAINHAND] = Some(item(5569, 900, 1));
        Event::CharacterSelected(Box::new(CharacterState {
            character,
            num_inventory_slots: bag,
            num_bank_slots: 20,
            ..CharacterState::default()
        }))
    }

    #[test]
    fn physical_slot_pending_keys_release_only_reported_slots_and_preserve_mate_state() {
        let mut inv = Inventory::default();
        inv.apply(&selected(3));
        let mut pet = item(42, 123, 1);
        pet.detail = ItemDetail::Mate { exp: 17, dead: 7, level: 28 };
        inv.bag[0] = Some(pet.clone());
        assert!(inv.mark_item_slot_pending(container::INVENTORY, 0));
        assert!(!inv.mark_item_slot_pending(container::INVENTORY, 0));
        assert!(inv.mark_item_slot_pending(container::BANK, 2));
        assert!(!inv.mark_item_slot_pending(4, 0));
        assert!(!inv.mark_item_slot_pending(0xf0, 0));
        assert_eq!(inv.pending_item_slot_keys().collect::<Vec<_>>(), vec![0x200, 0x02000300]);
        assert_eq!(inv.bag[0], Some(pet.clone()));

        assert!(!inv.apply(&Event::CraftItemUnlocked(crate::craft::CraftItemUnlock { kind: 2, slot: 1 })));
        assert!(inv.item_slot_pending(2, 0));
        assert!(!inv.apply(&Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Update { container: 2, slot: 0, item_id: 999,
                detail: ItemDetail::Mate { exp: 0, dead: 0, level: 1 }, detail_payload: None, raw_detail_block: None }],
            ..Default::default() })));
        assert!(inv.item_slot_pending(2, 0));
        assert_eq!(inv.bag[0], Some(pet));
        let update = Event::ItemsChanged(ItemTasks { reason: crate::mate::REPAIR_PETS_TASK,
            tasks: vec![ItemTask::Update { container: 2, slot: 0, item_id: 123,
                detail: ItemDetail::Mate { exp: 17, dead: 0, level: 28 }, detail_payload: None, raw_detail_block: None }],
            ..Default::default() });
        assert!(inv.apply(&update));
        assert!(!inv.item_slot_pending(2, 0));
        assert!(inv.item_slot_pending(3, 2));
        assert_eq!(inv.bag[0].as_ref().unwrap().detail, ItemDetail::Mate { exp: 17, dead: 0, level: 28 });
        assert!(inv.apply(&Event::CraftItemUnlocked(crate::craft::CraftItemUnlock { kind: 3, slot: 2 })));
        assert!(!inv.apply(&Event::CraftItemUnlocked(crate::craft::CraftItemUnlock { kind: 3, slot: 2 })));
        assert_eq!(inv.bag[0].as_ref().unwrap().id, 123);
    }

    #[test]
    fn physical_slot_pending_lifecycle_and_explicit_rejection_are_independent_of_aa_setting() {
        let mut inv = Inventory::default();
        inv.mark_item_slot_pending(2, 4);
        inv.apply(&Event::AutoUseAaPointRequested(true));
        assert!(inv.clear_item_slot_pending(2, 4));
        assert_eq!(inv.pending_auto_use_aa_point, Some(true));
        inv.mark_item_slot_pending(3, 5);
        inv.apply(&selected(1));
        assert_eq!(inv.pending_item_slot_keys().count(), 0);
        inv.mark_item_slot_pending(2, 0);
        inv.apply(&Event::LeftWorld { target: 1 });
        assert_eq!(inv.pending_item_slot_keys().count(), 0);
    }

    fn tasks(reason: u8, tasks: Vec<ItemTask>) -> Event {
        Event::ItemsChanged(ItemTasks { reason, tasks, force_remove: vec![], truncated: false })
    }

    #[test]
    fn inventory_unlock_releases_only_the_matching_pending_key_without_changing_value() {
        use crate::craft::CraftItemUnlock;
        let mut inv = Inventory::default();
        inv.apply(&Event::AutoUseAaPointRequested(true));
        for (kind, slot) in [(2, 0), (0xf0, 1)] {
            assert!(!inv.apply(&Event::CraftItemUnlocked(CraftItemUnlock { kind, slot })));
            assert_eq!(inv.pending_auto_use_aa_point, Some(true));
        }
        assert!(inv.apply(&Event::CraftItemUnlocked(CraftItemUnlock { kind: 0xf0, slot: 0 })));
        assert_eq!(inv.pending_auto_use_aa_point, None);
        assert!(!inv.auto_use_aa_point());
        assert!(!inv.apply(&Event::CraftItemUnlocked(CraftItemUnlock { kind: 0xf0, slot: 0 })));
    }

    #[test]
    fn aa_point_request_wire_is_native_single_byte() {
        use crate::game::ClientMsg;
        for value in [false, true] {
            let packet = ClientMsg::SetAutoUseAaPoint(value);
            assert_eq!(packet.opcode(), 0x133);
            assert_eq!(packet.encode_body(), vec![u8::from(value)]);
        }
    }

    #[test]
    fn aa_point_setting_is_received_state_with_repeat_reports_and_raw_values() {
        let mut inv = Inventory::default();
        let mut character = CharacterState::default();
        character.character.auto_use_aa_point = true;
        inv.apply(&Event::CharacterSelected(Box::new(character)));
        assert!(inv.auto_use_aa_point());
        assert!(!inv.can_set_auto_use_aa_point(true));
        assert!(inv.can_set_auto_use_aa_point(false));
        inv.apply(&Event::AutoUseAaPointRequested(false));
        assert!(inv.auto_use_aa_point());
        assert!(!inv.can_set_auto_use_aa_point(false));
        for raw in [0, 0, 2, 255, 1] {
            assert!(inv.apply(&tasks(0, vec![ItemTask::AutoUseAaPoint(raw)])));
            assert_eq!(inv.auto_use_aa_point_raw, raw);
            assert_eq!(inv.auto_use_aa_point(), raw == 1);
            assert_eq!(inv.pending_auto_use_aa_point, None);
        }
        inv.apply(&Event::Disconnected { message: String::new() });
        assert!(!inv.auto_use_aa_point());
        assert_eq!(inv.auto_use_aa_point_raw, 0);
    }

    #[test]
    fn the_action_bar_follows_the_server_and_local_changes() {
        let mut bar = ActionBar::default();
        let mut slots = vec![ActionSlot::default(); ACTION_SLOTS];
        slots[1] = ActionSlot { kind: ActionKind::Spell, id: 18132 };
        slots[11] = ActionSlot { kind: ActionKind::ItemType, id: 8001 };
        assert!(bar.apply(&Event::ActionSlots(slots)));
        assert_eq!(bar.get(1).unwrap().skill_id(), Some(18132));
        assert!(bar.get(2).is_none());
        assert!(bar.apply(&Event::ActionSlotChanged {
            slot: 2,
            action: ActionSlot { kind: ActionKind::ItemId, id: 77 }
        }));
        assert_eq!(bar.get(2).unwrap().item_id(), Some(77));
        assert!(!bar.apply(&Event::ActionSlotChanged { slot: 200, action: ActionSlot::default() }));
        // The item slots show counts from the bag.
        let mut inv = Inventory::default();
        inv.apply(&selected(10));
        inv.apply(&tasks(9, vec![
            ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 3, item: item(8001, 77, 12) },
            ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 5, item: item(8001, 78, 8) },
        ]));
        assert_eq!(bar.item_count(11, &inv), Some(20));
        assert_eq!(bar.item_count(2, &inv), Some(12));
        assert_eq!(bar.item_count(1, &inv), None);
        assert!(bar.apply(&Event::LeftWorld { target: 1 }));
        assert!(bar.slots.is_empty());
    }

    #[test]
    fn action_bar_pages_map_the_main_bar_and_reset_with_the_world() {
        let mut bar = ActionBar::default();
        assert_eq!(bar.page(), 0);
        assert!(bar.set_page(3));
        assert_eq!(bar.page(), 3);
        assert!(!bar.set_page(4));
        assert_eq!(bar.page(), 3);
        // The main 12 slots are page managed; the two 6-slot sub bars stay at 13..24.
        assert_eq!(ActionBar::page_slot(0, 0), Some(1));
        assert_eq!(ActionBar::page_slot(0, 11), Some(12));
        assert_eq!(ActionBar::page_slot(1, 0), Some(49));
        assert_eq!(ActionBar::page_slot(1, 11), Some(60));
        assert_eq!(ActionBar::page_slot(2, 0), Some(61));
        assert_eq!(ActionBar::page_slot(3, 11), Some(84));
        assert_eq!(ActionBar::page_slot(3, 12), None);
        assert_eq!(ActionBar::page_slot(4, 0), None);
        bar.set_page(1);
        assert_eq!(bar.display_slot(0), Some(49));
        assert_eq!(bar.display_slot(11), Some(60));
        assert_eq!(bar.display_slot(12), Some(13));
        assert_eq!(bar.display_slot(23), Some(24));
        assert_eq!(bar.display_slot(24), Some(25));
        assert_eq!(bar.display_slot(ACTION_SLOTS - 2), Some(ACTION_SLOTS - 1));
        assert_eq!(bar.display_slot(ACTION_SLOTS - 1), None);
        let mut slots = vec![ActionSlot::default(); ACTION_SLOTS];
        slots[49] = ActionSlot { kind: ActionKind::Spell, id: 18132 };
        assert!(bar.apply(&Event::ActionSlots(slots)));
        assert_eq!(bar.get(bar.display_slot(0).unwrap()).unwrap().skill_id(), Some(18132));
        assert!(bar.apply(&Event::LeftWorld { target: 1 }));
        assert_eq!(bar.page(), 0);
    }

    #[test]
    fn the_bag_follows_chunks_and_item_tasks() {
        let mut inv = Inventory::default();
        assert!(inv.apply(&selected(20)));
        assert_eq!((inv.bag.len(), inv.bank.len()), (20, 20));
        assert_eq!(inv.worn(slot::MAINHAND).unwrap().template_id, 5569);
        // The second chunk of the bag: a potion stack in its fourth slot.
        let mut items = vec![None; 10];
        items[3] = Some(item(8001, 555, 20));
        assert!(inv.apply(&Event::InventoryChunk(InvenChunk {
            container: container::INVENTORY,
            chunks: 1,
            start_chunk: 1,
            items,
            truncated: false,
        })));
        assert_eq!(inv.find_template(8001).map(|(slot, _)| slot), Some(13));
        assert_eq!(inv.count_of(8001), 20);
        // Use one, pick up 5 more in a new stack, then move the new stack onto slot 0.
        assert!(inv.apply(&tasks(13, vec![ItemTask::Count { container: 2, slot: 13, item_id: 555, delta: -1, template_id: 8001 }])));
        assert_eq!(inv.count_of(8001), 19);
        assert!(inv.apply(&tasks(9, vec![ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 4, item: item(8001, 600, 5) }])));
        assert_eq!(inv.count_of(8001), 24);
        assert!(inv.apply(&tasks(5, vec![ItemTask::Move {
            from_container: 2,
            from_slot: 4,
            from_item_id: 600,
            to_container: 2,
            to_slot: 0,
            to_item_id: 0,
        }])));
        assert!(inv.bag[4].is_none());
        assert_eq!(inv.bag[0].as_ref().unwrap().id, 600);
        // A swap of two occupied slots.
        assert!(inv.apply(&tasks(5, vec![ItemTask::Move {
            from_container: 2,
            from_slot: 0,
            from_item_id: 600,
            to_container: 2,
            to_slot: 13,
            to_item_id: 555,
        }])));
        assert_eq!(inv.bag[0].as_ref().unwrap().id, 555);
        assert_eq!(inv.bag[13].as_ref().unwrap().id, 600);
        // Equip it: bag slot 13 to the head slot (a bag slot swapped with an empty worn slot).
        assert!(inv.apply(&tasks(5, vec![ItemTask::Move {
            from_container: 2,
            from_slot: 13,
            from_item_id: 600,
            to_container: 1,
            to_slot: slot::HEAD as u8,
            to_item_id: 0,
        }])));
        assert_eq!(inv.worn(slot::HEAD).unwrap().id, 600);
        assert!(inv.bag[13].is_none());
        // The last of a stack goes away, a grade change and details stick.
        assert!(inv.apply(&tasks(1, vec![ItemTask::Count { container: 2, slot: 0, item_id: 555, delta: -19, template_id: 8001 }])));
        assert!(inv.bag[0].is_none());
        assert!(inv.apply(&tasks(97, vec![
            ItemTask::Grade { container: 1, slot: slot::HEAD as u8, item_id: 600, grade: 3 },
            ItemTask::Update {
                container: 1,
                slot: slot::HEAD as u8,
                item_id: 600,
                detail: ItemDetail::Equip(Default::default()),
                detail_payload: Some(vec![0; 55]),
                raw_detail_block: None,
            },
        ])));
        assert_eq!(inv.worn(slot::HEAD).unwrap().grade, 3);
        assert!(matches!(inv.worn(slot::HEAD).unwrap().detail, ItemDetail::Equip(_)));
        assert_eq!(inv.worn(slot::HEAD).unwrap().detail_payload, Some(vec![0; 55]));
        // A remove whose slot is stale still finds the item by id.
        assert!(inv.apply(&tasks(1, vec![ItemTask::Remove { container: 1, slot: 9, item_id: 600, template_id: 1 }])));
        assert!(inv.worn(slot::HEAD).is_none());
        // Force remove and growth.
        inv.apply(&tasks(9, vec![ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 2, item: item(1, 9, 1) }]));
        assert!(inv.apply(&Event::ItemsChanged(ItemTasks { force_remove: vec![9], ..ItemTasks::default() })));
        assert!(inv.bag[2].is_none());
        assert!(inv.apply(&Event::InventoryExpanded { container: container::INVENTORY, slots: 30 }));
        assert_eq!((inv.bag_slots, inv.bag.len()), (30, 30));
        assert_eq!(inv.free_bag_slots(), 30);
    }

    #[test]
    fn item_moves_name_the_items_in_both_slots() {
        let mut inv = Inventory::default();
        inv.apply(&selected(10));
        inv.apply(&tasks(9, vec![
            ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 1, item: item(8001, 55, 20) },
            ItemTask::Add { raw_detail_block: None, action: action::CREATE, container: 2, slot: 4, item: item(77, 66, 1) },
        ]));
        assert_eq!(inv.first_free_bag_slot(), Some(0));
        assert_eq!(inv.item_at(container::INVENTORY, 1).map(|i| i.id), Some(55));
        assert_eq!(inv.item_at(container::EQUIPMENT, slot::MAINHAND).map(|i| i.id), Some(900));
        // bag to an empty bag slot, onto another item, from and to nothing
        let m = inv.plan_move((container::INVENTORY, 1), (container::INVENTORY, 7)).unwrap();
        assert_eq!((m.from_item_id, m.to_item_id, m.from_slot, m.to_slot), (55, 0, 1, 7));
        let m = inv.plan_move((container::INVENTORY, 1), (container::INVENTORY, 4)).unwrap();
        assert_eq!((m.from_item_id, m.to_item_id), (55, 66));
        assert!(inv.plan_move((container::INVENTORY, 2), (container::INVENTORY, 3)).is_none());
        assert!(inv.plan_move((container::INVENTORY, 1), (container::INVENTORY, 1)).is_none());
        // wearing the item in bag slot 4 on the empty head slot: from the empty worn slot to the bag item
        let m = inv.plan_equip(4, slot::HEAD).unwrap();
        assert_eq!((m.from_item_id, m.to_item_id, m.from_container, m.from_slot, m.to_container, m.to_slot), (0, 66, 1, 0, 2, 4));
        // on the main hand it swaps with the weapon worn there
        assert_eq!(inv.plan_equip(4, slot::MAINHAND).unwrap().from_item_id, 900);
        // taking the weapon off into a free slot
        let m = inv.plan_unequip(slot::MAINHAND, 7).unwrap();
        assert_eq!((m.from_item_id, m.to_item_id, m.from_container, m.from_slot, m.to_slot), (900, 0, 1, 15, 7));
        assert!(inv.plan_unequip(slot::HEAD, 7).is_none());
        assert!(inv.plan_equip(9, slot::HEAD).is_none());
        // the server's answer to the unequip is the usual swap task
        assert!(inv.apply(&tasks(5, vec![ItemTask::Move {
            from_container: 1, from_slot: 15, from_item_id: 900, to_container: 2, to_slot: 7, to_item_id: 0,
        }])));
        assert!(inv.worn(slot::MAINHAND).is_none());
        assert_eq!(inv.bag[7].as_ref().unwrap().id, 900);
        // and to the equip (the empty worn slot moves to the bag item's slot); the body parts, which have no item id
        // either, stay where they are
        inv.equipment.slots[slot::HAIR] = Item::template_only(5);
        assert!(inv.apply(&tasks(5, vec![ItemTask::Move {
            from_container: 1, from_slot: 0, from_item_id: 0, to_container: 2, to_slot: 4, to_item_id: 66,
        }])));
        assert_eq!(inv.worn(slot::HEAD).unwrap().id, 66);
        assert!(inv.bag[4].is_none());
        assert_eq!(inv.worn(slot::HAIR).map(|i| i.template_id), Some(5));
        assert_eq!(inv.bag.iter().flatten().count(), 2);
    }

    #[test]
    fn money_changes_reach_the_stats() {
        use crate::state::PlayerStats;
        let mut stats = PlayerStats::default();
        stats.apply(&selected(10));
        stats.money = 1_000;
        assert!(stats.apply(&tasks(9, vec![ItemTask::Money(250)])));
        assert!(stats.apply(&tasks(53, vec![ItemTask::Money(-100), ItemTask::BankMoney(40)])));
        assert_eq!((stats.money, stats.bank_money), (1_150, 40));
        assert!(!stats.apply(&tasks(9, vec![ItemTask::AaPoint(1)])));
    }

    #[test]
    fn item_moves_reject_unknown_containers_and_slots_outside_capacity() {
        let mut inv = Inventory::default();
        inv.apply(&selected(10));
        inv.bag[4] = Some(item(77, 66, 1));
        let source = (container::INVENTORY, 4);
        for invalid in [(container::INVENTORY, 10), (container::BANK, 20), (container::EQUIPMENT, slot::COUNT), (99, 0)] {
            assert!(inv.plan_move(source, invalid).is_none(), "target {invalid:?}");
            assert!(inv.plan_move(invalid, source).is_none(), "source {invalid:?}");
        }
        assert!(inv.plan_equip(4, slot::COUNT).is_none());
        assert!(inv.plan_unequip(slot::MAINHAND, 10).is_none());
        assert!(inv.plan_move(source, (container::INVENTORY, 9)).is_some());
        assert!(inv.plan_move(source, (container::BANK, 19)).is_some());
    }

    #[test]
    fn units_follow_equipment_changes() {
        use crate::state::Units;
        let mut units = Units::default();
        units.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(5))));
        assert!(units.apply(&Event::EquipmentChanged { obj_id: 5, items: vec![(15, Some(item(77, 1, 1))), (16, None)] }));
        assert_eq!(units.get(5).unwrap().equipment.slots[15].as_ref().unwrap().template_id, 77);
        assert!(!units.apply(&Event::EquipmentChanged { obj_id: 6, items: vec![] }));
    }

    #[test]
    fn items_move_between_the_bag_and_an_open_coffer() {
        use crate::coffer::{CofferContents, CofferEvent};
        let mut inv = Inventory::default();
        inv.apply(&selected(4));
        inv.bag[1] = Some(item(7, 70, 1));
        // nothing is open: the coffer container is not a place
        assert!(inv.plan_move((container::INVENTORY, 1), (container::TRADE, 0)).is_none());
        let page = |slots: Vec<(u8, Option<Item>)>| Event::Coffer(CofferEvent::Contents(CofferContents { obj_id: 5, container_id: 77, slots }));
        assert!(inv.apply(&page(vec![(0, Some(item(8, 80, 3))), (1, None)])));
        let mv = inv.plan_move((container::INVENTORY, 1), (container::TRADE, 1)).expect("into the empty coffer slot");
        assert_eq!((mv.from_item_id, mv.to_item_id, mv.to_container, mv.to_slot), (70, 0, container::TRADE, 1));
        let task = |from: (u8, u8, u64), to: (u8, u8, u64)| {
            Event::ItemsChanged(ItemTasks {
                reason: 0,
                tasks: vec![ItemTask::Move { from_container: from.0, from_slot: from.1, from_item_id: from.2, to_container: to.0, to_slot: to.1, to_item_id: to.2 }],
                force_remove: vec![],
                truncated: false,
            })
        };
        assert!(inv.apply(&task((container::INVENTORY, 1, 70), (container::TRADE, 1, 0))));
        assert!(inv.bag[1].is_none());
        assert_eq!(inv.coffer.slots[1].as_ref().map(|i| i.id), Some(70));
        // a swap with the item in the coffer's first slot
        assert!(inv.apply(&task((container::TRADE, 0, 80), (container::INVENTORY, 2, 0))));
        assert_eq!(inv.bag[2].as_ref().map(|i| i.id), Some(80));
        assert!(inv.coffer.slots[0].is_none());
        // leaving the world forgets the coffer
        inv.apply(&Event::LeftWorld { target: 0 });
        assert!(!inv.coffer.is_open());
    }

    #[test]
    fn coffer_only_inventory_clears_on_disconnect_and_doodad_despawn() {
        use crate::coffer::{CofferContents, CofferEvent};
        for reset in [Event::Disconnected { message: "lost".into() }, Event::LeftWorld { target: 1 }, Event::DoodadsRemoved(vec![7])] {
            let mut inv = Inventory::default();
            inv.apply(&Event::Coffer(CofferEvent::Contents(CofferContents { obj_id: 7, container_id: 99, slots: vec![(0, Some(item(5, 9, 1)))] })));
            assert!(inv.bag.is_empty());
            assert!(inv.apply(&reset));
            assert!(!inv.coffer.is_open());
            assert!(inv.coffer.slots.is_empty());
            assert!(!inv.has_slot(container::TRADE, 0));
        }
    }
}
