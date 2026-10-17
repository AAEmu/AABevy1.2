//! Loot: which corpses have something for the player, and the loot bag the player opened.
//!
//! The server tells the client when a killed unit has loot for it (`SCLootableState`); the client then offers
//! "loot all" and "open the loot window" (the F and G icons). The bag (`SCLootBagData`) lists the items left
//! in a corpse; the server gives each loot item a fake id `(owner object id << 32) + (owner type << 16) + index`
//! so the item index to take it with, and the owner, are in the item's `id` ([`item_index`], [`item_owner`]).

use crate::model::Item;
use crate::session::Event;

/// `LootOwnerType`: what the loot belongs to.
pub mod owner_type {
    pub const NPC: u16 = 1;
    pub const DOODAD: u16 = 2;
}

/// The index to take a loot item with (`Command::LootItem::item_index`).
pub fn item_index(item: &Item) -> u16 {
    (item.id & 0xffff) as u16
}

/// `(owner type, owner object id)` of a loot item.
pub fn item_owner(item: &Item) -> (u16, u32) {
    (((item.id >> 16) & 0xffff) as u16, (item.id >> 32) as u32)
}

/// The server refused to hand over an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootFailure {
    /// `ErrorMessageType` (bag full, no permission, ...).
    pub error: i32,
    pub item_template_id: u32,
}

resource! {
    /// Loot the player may take.
    pub struct Loot {
        /// Units and doodads with loot for the player, `(owner type, object id)`, oldest first.
        pub lootable: Vec<(u16, u32)>,
        /// The items of the bag the server sent last (what is left in the corpse the player opened).
        pub bag: Vec<Item>,
        /// That bag came with a "loot all" (the window only lists what could not be taken, for a moment).
        pub bag_loot_all: bool,
        /// Counts the bags the server sent: the window comes up again for each one.
        pub bag_seq: u32,
        /// Counts the items taken and the bags emptied: the window closes when its bag is empty.
        pub taken_seq: u32,
        /// The last refusal.
        pub failure: Option<LootFailure>,
    }
}

impl Loot {
    /// Whether the unit or doodad has loot for the player.
    pub fn has_loot(&self, obj_id: u32) -> bool {
        self.lootable.iter().any(|&(_, id)| id == obj_id)
    }

    /// The owner type the server announced for `obj_id`.
    pub fn owner_type_of(&self, obj_id: u32) -> Option<u16> {
        self.lootable.iter().find(|&&(_, id)| id == obj_id).map(|&(t, _)| t)
    }

    /// Returns true when the data changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::Lootable { owner_type, obj_id, has_loot } => {
                let at = self.lootable.iter().position(|&(_, id)| id == *obj_id);
                match (*has_loot, at) {
                    (true, None) => self.lootable.push((*owner_type, *obj_id)),
                    (false, Some(i)) => {
                        self.lootable.remove(i);
                    }
                    _ => return false,
                }
            }
            Event::UnitsRemoved(ids) => {
                let before = self.lootable.len();
                self.lootable.retain(|(_, id)| !ids.contains(id));
                if self.lootable.len() == before {
                    return false;
                }
            }
            Event::LootBag { items, loot_all } => {
                self.bag = items.clone();
                self.bag_loot_all = *loot_all;
                self.bag_seq = self.bag_seq.wrapping_add(1);
            }
            Event::LootItemTook { item_index: index, owner_type, owner_obj_id, .. } => {
                self.bag.retain(|it| !(item_index(it) == *index && item_owner(it) == (*owner_type, *owner_obj_id)));
                self.taken_seq = self.taken_seq.wrapping_add(1);
                self.failure = None;
            }
            Event::LootItemFailed { error, item_template_id, .. } => {
                self.failure = Some(LootFailure { error: *error, item_template_id: *item_template_id });
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.lootable.is_empty() && self.bag.is_empty() && self.failure.is_none() {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(owner: u32, kind: u16, index: u16) -> Item {
        Item { template_id: 500 + u32::from(index), id: (u64::from(owner) << 32) + (u64::from(kind) << 16) + u64::from(index), count: 2, ..Item::default() }
    }

    #[test]
    fn item_ids_carry_the_owner_and_index() {
        let it = item(0x1234, owner_type::NPC, 3);
        assert_eq!(item_index(&it), 3);
        assert_eq!(item_owner(&it), (owner_type::NPC, 0x1234));
    }

    #[test]
    fn corpses_come_and_go() {
        let mut l = Loot::default();
        assert!(l.apply(&Event::Lootable { owner_type: 1, obj_id: 7, has_loot: true }));
        assert!(!l.apply(&Event::Lootable { owner_type: 1, obj_id: 7, has_loot: true }));
        assert!(l.has_loot(7) && !l.has_loot(8) && l.owner_type_of(7) == Some(1));
        assert!(l.apply(&Event::Lootable { owner_type: 1, obj_id: 7, has_loot: false }));
        assert!(!l.has_loot(7));
        l.apply(&Event::Lootable { owner_type: 2, obj_id: 9, has_loot: true });
        assert!(l.apply(&Event::UnitsRemoved(vec![9, 10])));
        assert!(l.lootable.is_empty());
        assert!(!l.apply(&Event::UnitsRemoved(vec![9])));
    }

    #[test]
    fn taking_items_empties_the_bag() {
        let mut l = Loot::default();
        let items = vec![item(7, 1, 1), item(7, 1, 2)];
        assert!(l.apply(&Event::LootBag { items, loot_all: false }));
        assert_eq!((l.bag.len(), l.bag_seq), (2, 1));
        assert!(l.apply(&Event::LootItemTook { template_id: 501, item_index: 1, owner_type: 1, owner_obj_id: 7, count: 2 }));
        assert_eq!(l.bag.len(), 1);
        // another corpse's item with the same index stays
        l.apply(&Event::LootItemTook { template_id: 1, item_index: 2, owner_type: 1, owner_obj_id: 99, count: 1 });
        assert_eq!(l.bag.len(), 1);
        l.apply(&Event::LootItemTook { template_id: 502, item_index: 2, owner_type: 1, owner_obj_id: 7, count: 2 });
        assert!(l.bag.is_empty());
        assert!(l.apply(&Event::LootItemFailed { error: 25, item_index: 2, owner_type: 1, owner_obj_id: 7, item_template_id: 502 }));
        assert_eq!(l.failure, Some(LootFailure { error: 25, item_template_id: 502 }));
        assert!(l.apply(&Event::LeftWorld { target: 0 }));
        assert!(l.lootable.is_empty() && l.failure.is_none() && l.bag.is_empty());
    }
}
