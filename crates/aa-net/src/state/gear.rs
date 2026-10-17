//! The hotbar, the bag and bank, and the equipment of the local player.

use crate::items::{container, ActionSlot, ItemTask, ACTION_SLOTS};
use crate::model::{Equipment, Item, slot};
use crate::session::Event;

resource! {
    /// The hotbar: 85 slots, index = slot number (the UI's slot 1 is index 0).
    pub struct ActionBar {
        pub slots: Vec<ActionSlot>,
    }
}

impl ActionBar {
    pub fn get(&self, slot: usize) -> Option<&ActionSlot> {
        self.slots.get(slot).filter(|s| !s.is_empty())
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
                if self.slots.is_empty() {
                    return false;
                }
                self.slots.clear();
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
        pub bag: Vec<Option<Item>>,
        pub bank: Vec<Option<Item>>,
        /// Worn items by `model::slot`.
        pub equipment: Equipment,
        /// The player's object id (set by the spawn), to pick its equipment changes.
        own_obj_id: u32,
    }
}

impl Inventory {
    /// The bag's items with their slot index.
    pub fn bag_items(&self) -> impl Iterator<Item = (usize, &Item)> {
        self.bag.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|it| (i, it)))
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
                item.flags = *bits;
            }
            ItemTask::Update { container, slot, item_id, detail } => {
                let Some(Some(item)) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                item.detail = detail.clone();
            }
            ItemTask::Remove { container, slot, item_id, .. } | ItemTask::RemoveSlot { container, slot, item_id } => {
                let Some(s) = self.locate(*container, usize::from(*slot), *item_id) else { return false };
                *s = None;
            }
            ItemTask::RemoveCrafting { item_id } => return self.remove_by_id(*item_id),
            ItemTask::Move { from_container, from_slot, from_item_id, to_container, to_slot, to_item_id } => {
                // Take both ends first, then put them back crossed: handles empty targets,
                // swaps and moves between the bag, bank and equipment alike.
                let from = self
                    .locate(*from_container, usize::from(*from_slot), *from_item_id)
                    .and_then(Option::take);
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
            // Purse and points are the player stats'.
            ItemTask::Money(_)
            | ItemTask::BankMoney(_)
            | ItemTask::GamePoint { .. }
            | ItemTask::AaPoint(_)
            | ItemTask::BankAaPoint(_)
            | ItemTask::AutoUseAaPoint(_) => return false,
        }
        true
    }

    fn remove_by_id(&mut self, id: u64) -> bool {
        let mut removed = false;
        for list in [&mut self.bag[..], &mut self.bank[..], &mut self.equipment.slots[..]] {
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
                    bag: vec![None; usize::from(cs.num_inventory_slots)],
                    bank: vec![None; cs.num_bank_slots.max(0) as usize],
                    equipment: cs.character.equipment.clone(),
                    own_obj_id: 0,
                };
            }
            Event::PlayerSpawn(s) => self.own_obj_id = s.obj_id,
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
                for task in &changes.tasks {
                    changed |= self.apply_task(task);
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
                if self.bag.is_empty() && self.bank.is_empty() && self.bag_slots == 0 {
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

    fn tasks(reason: u8, tasks: Vec<ItemTask>) -> Event {
        Event::ItemsChanged(ItemTasks { reason, tasks, force_remove: vec![], truncated: false })
    }

    #[test]
    fn the_action_bar_follows_the_server_and_local_changes() {
        let mut bar = ActionBar::default();
        let mut slots = vec![ActionSlot::default(); ACTION_SLOTS];
        slots[0] = ActionSlot { kind: ActionKind::Spell, id: 18132 };
        slots[10] = ActionSlot { kind: ActionKind::ItemType, id: 8001 };
        assert!(bar.apply(&Event::ActionSlots(slots)));
        assert_eq!(bar.get(0).unwrap().skill_id(), Some(18132));
        assert!(bar.get(1).is_none());
        assert!(bar.apply(&Event::ActionSlotChanged {
            slot: 1,
            action: ActionSlot { kind: ActionKind::ItemId, id: 77 }
        }));
        assert_eq!(bar.get(1).unwrap().item_id(), Some(77));
        assert!(!bar.apply(&Event::ActionSlotChanged { slot: 200, action: ActionSlot::default() }));
        // The item slots show counts from the bag.
        let mut inv = Inventory::default();
        inv.apply(&selected(10));
        inv.apply(&tasks(9, vec![
            ItemTask::Add { action: action::CREATE, container: 2, slot: 3, item: item(8001, 77, 12) },
            ItemTask::Add { action: action::CREATE, container: 2, slot: 5, item: item(8001, 78, 8) },
        ]));
        assert_eq!(bar.item_count(10, &inv), Some(20));
        assert_eq!(bar.item_count(1, &inv), Some(12));
        assert_eq!(bar.item_count(0, &inv), None);
        assert!(bar.apply(&Event::LeftWorld { target: 1 }));
        assert!(bar.slots.is_empty());
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
        assert!(inv.apply(&tasks(9, vec![ItemTask::Add { action: action::CREATE, container: 2, slot: 4, item: item(8001, 600, 5) }])));
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
            },
        ])));
        assert_eq!(inv.worn(slot::HEAD).unwrap().grade, 3);
        assert!(matches!(inv.worn(slot::HEAD).unwrap().detail, ItemDetail::Equip(_)));
        // A remove whose slot is stale still finds the item by id.
        assert!(inv.apply(&tasks(1, vec![ItemTask::Remove { container: 1, slot: 9, item_id: 600, template_id: 1 }])));
        assert!(inv.worn(slot::HEAD).is_none());
        // Force remove and growth.
        inv.apply(&tasks(9, vec![ItemTask::Add { action: action::CREATE, container: 2, slot: 2, item: item(1, 9, 1) }]));
        assert!(inv.apply(&Event::ItemsChanged(ItemTasks { force_remove: vec![9], ..ItemTasks::default() })));
        assert!(inv.bag[2].is_none());
        assert!(inv.apply(&Event::InventoryExpanded { container: container::INVENTORY, slots: 30 }));
        assert_eq!((inv.bag_slots, inv.bag.len()), (30, 30));
        assert_eq!(inv.free_bag_slots(), 30);
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
    fn units_follow_equipment_changes() {
        use crate::state::Units;
        let mut units = Units::default();
        units.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(5))));
        assert!(units.apply(&Event::EquipmentChanged { obj_id: 5, items: vec![(15, Some(item(77, 1, 1))), (16, None)] }));
        assert_eq!(units.get(5).unwrap().equipment.slots[15].as_ref().unwrap().template_id, 77);
        assert!(!units.apply(&Event::EquipmentChanged { obj_id: 6, items: vec![] }));
    }
}
