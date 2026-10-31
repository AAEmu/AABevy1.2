//! The 1.2 linked-item skill rule (`FUN_393925c0` at `0x393925c0`): a skill whose `skills.link_equip_slot_id` is not
//! -1 is cast through the item equipped in that slot, never as the raw skill.
//!
//! The decompile, in `x2game.dll.c`:
//!
//! * 743817-743862: the skill descriptor's `+0xe8` slot is looked up in the actor's equipment; an empty slot
//!   (`DAT_3a177700`) and an item whose `item_backpacks.backpack_type_id` (via `FUN_396a6e40`, field `+8`) differs from
//!   the skill's `+0xe4` both return 0. On a match `FUN_397cf7a0` replaces the skill id with the item's use skill.
//! * 743843-743844 (inside `FUN_393925c0`): `FUN_39044360` builds an **Item caster** (`kind = 2`) from the equipped
//!   item. The item row holds its id at `+0`/`+4`, template at `+8`, grade byte at `+0xc` and `made_unit_id` at `+0x14`;
//!   the caster also carries the owner's object id (`actor+8`). The `CS052` body in `docs/aa-net.md` (line 8181) reads
//!   the same fields after `BC3 owner`.
//! * 748895-748899: `FUN_393989e0` calls it with the caster struct, and on failure raises `0x146` (326
//!   `MUST_EQUIP_PROPER_ITEM`) and returns before the send at 748908.
//!
//! So no glider equipped means the local 326 alert and no packet; a glider equipped means `CSStartSkill` with the glider
//! item's `use_skill_id` and that item as the caster. Skill 14901 is the data's only row linking the back slot (26) to a
//! glider backpack (type 2); 14902 is the same rule for a trade pack (type 3), so the rule is kept generic.

use aa_net::state::Inventory;

use super::data::{Db, ItemBackpack, SkillLink};

/// `item_backpacks.backpack_type_id` of a glider (`item_backpacks` type 2, x2game `DAT_...` comparison in
/// `FUN_393925c0`).
pub const GLIDER_BACKPACK: i64 = 2;

/// What a skill press becomes under `FUN_393925c0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedUse {
    /// No linked equip slot: send the skill as pressed.
    Plain,
    /// The linked slot is empty or holds a backpack of another type: the client raises 326 and sends nothing.
    Refused,
    /// The equipped item's use skill, cast as that item.
    Converted {
        skill: u32,
        item_id: u64,
        template_id: u32,
    },
}

/// The two data lookups `FUN_393925c0` makes (`skills` and `items`/`item_backpacks`), implemented by the in-game
/// [`Db`] and by the HUD's loaded [`GameData`](crate::hud::GameData) so both use this one resolver.
pub trait LinkSource {
    fn skill_link(&self, skill: u32) -> Option<SkillLink>;
    /// `None` when the item template is unknown; a known item has `backpack_type` `None` when it has no
    /// `item_backpacks` row.
    fn item_backpack(&self, item: u32) -> Option<ItemBackpack>;
}

impl LinkSource for Db {
    fn skill_link(&self, skill: u32) -> Option<SkillLink> {
        Db::skill_link(self, skill)
    }

    fn item_backpack(&self, item: u32) -> Option<ItemBackpack> {
        Db::item_backpack(self, item)
    }
}

impl LinkSource for crate::hud::GameData {
    fn skill_link(&self, skill: u32) -> Option<SkillLink> {
        self.skill_links.get(&skill).copied()
    }

    fn item_backpack(&self, item: u32) -> Option<ItemBackpack> {
        Some(ItemBackpack {
            backpack_type: self.item_backpack_types.get(&item).copied(),
            use_skill: self.items.get(&item)?.use_skill,
        })
    }
}

/// Resolve a skill press through the equipped item (`FUN_393925c0`). Without the database the link is unknown, so the
/// press is left alone rather than guessed.
pub fn resolve(skill: u32, inventory: Option<&Inventory>, db: Option<&Db>) -> LinkedUse {
    resolve_with(skill, inventory, db)
}

/// [`resolve`] over any [`LinkSource`] (the HUD keeps the same tables in memory).
pub fn resolve_with<S: LinkSource>(
    skill: u32,
    inventory: Option<&Inventory>,
    source: Option<&S>,
) -> LinkedUse {
    let (Some(inventory), Some(source)) = (inventory, source) else {
        return LinkedUse::Plain;
    };
    let Some(link) = source.skill_link(skill) else {
        return LinkedUse::Plain;
    };
    let Some(item) = inventory.equipment.get(link.slot) else {
        return LinkedUse::Refused;
    };
    let Some(backpack) = source.item_backpack(item.template_id) else {
        return LinkedUse::Plain;
    };
    // FUN_396a6e40(template) == 0 (no item_backpacks row) accepts the item; only a row of another
    // backpack type refuses (x2game 743840-743842).
    if backpack
        .backpack_type
        .is_some_and(|kind| kind != link.backpack_type)
    {
        return LinkedUse::Refused;
    }
    // FUN_397cf7a0 returns the item's static use skill; 0 is passed on, not turned into a 326 (the
    // grade-specific reagent selection it can do is not modeled).
    LinkedUse::Converted {
        skill: backpack.use_skill,
        item_id: item.id,
        template_id: item.template_id,
    }
}

/// The use skill of `template` when its `item_backpacks.backpack_type_id` is `backpack_type` (the shared lookup of the
/// double-jump deploy path and [`resolve`]).
pub fn backpack_use_skill(db: &Db, template: u32, backpack_type: i64) -> Option<u32> {
    let item = db.item_backpack(template)?;
    (item.backpack_type == Some(backpack_type) && item.use_skill != 0).then_some(item.use_skill)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::model::{Item, slot};

    fn item(id: u64, template_id: u32) -> Item {
        Item {
            id,
            template_id,
            count: 0,
            grade: 0,
            ..Default::default()
        }
    }

    fn inventory_with(slot_index: usize, item: Option<Item>) -> Inventory {
        let mut inventory = Inventory::default();
        inventory.equipment.slots[slot_index] = item;
        inventory
    }

    /// (a) no glider -> refused, (c) a type-3 trade pack in the back slot -> refused.
    #[test]
    fn the_back_slot_must_hold_a_glider() {
        let Some(db) = Db::skill_link_fixture() else {
            return;
        };
        let empty = inventory_with(slot::BACKPACK, None);
        assert_eq!(resolve(14901, Some(&empty), Some(&db)), LinkedUse::Refused);
        let pack = inventory_with(slot::BACKPACK, Some(item(0x0100_0001, 23605)));
        assert_eq!(resolve(14901, Some(&pack), Some(&db)), LinkedUse::Refused);
        // F4: a matching row with use_skill 0 converts to skill 0; it is not a 326.
        assert_eq!(
            resolve(14902, Some(&pack), Some(&db)),
            LinkedUse::Converted {
                skill: 0,
                item_id: 0x0100_0001,
                template_id: 23605
            }
        );
    }

    /// F1: no `item_backpacks` row accepts the item (Play Instrument 14900 with a lute 20480/19742).
    #[test]
    fn an_item_without_a_backpack_row_converts() {
        let Some(db) = Db::skill_link_fixture() else {
            return;
        };
        let lute = inventory_with(aa_net::model::slot::MUSICAL, Some(item(0x0100_0002, 20480)));
        assert_eq!(
            resolve(14900, Some(&lute), Some(&db)),
            LinkedUse::Converted {
                skill: 11951,
                item_id: 0x0100_0002,
                template_id: 20480
            }
        );
        let harp = inventory_with(aa_net::model::slot::MUSICAL, Some(item(0x0100_0003, 19742)));
        assert_eq!(
            resolve(14900, Some(&harp), Some(&db)),
            LinkedUse::Converted {
                skill: 19903,
                item_id: 0x0100_0003,
                template_id: 19742
            }
        );
    }

    /// (b) glider 14677 -> 12373 as an Item cast; 14902's type-3 rule also converts.
    #[test]
    fn an_equipped_glider_becomes_its_use_skill_on_the_item() {
        let Some(db) = Db::skill_link_fixture() else {
            return;
        };
        let inventory = inventory_with(slot::BACKPACK, Some(item(0x0100_002B, 14677)));
        assert_eq!(
            resolve(14901, Some(&inventory), Some(&db)),
            LinkedUse::Converted {
                skill: 12373,
                item_id: 0x0100_002B,
                template_id: 14677
            }
        );
        assert_eq!(backpack_use_skill(&db, 14677, 2), Some(12373));
        assert_eq!(backpack_use_skill(&db, 14677, 3), None);
    }

    /// (d) a skill without a linked equip slot is untouched, and without the database nothing is guessed.
    #[test]
    fn plain_and_unresolved_skills_stay_plain() {
        let Some(db) = Db::skill_link_fixture() else {
            return;
        };
        let inventory = inventory_with(slot::BACKPACK, None);
        assert_eq!(
            resolve(18132, Some(&inventory), Some(&db)),
            LinkedUse::Plain
        );
        assert_eq!(resolve(14901, Some(&inventory), None), LinkedUse::Plain);
        assert_eq!(resolve(14901, None, Some(&db)), LinkedUse::Plain);
    }
}
