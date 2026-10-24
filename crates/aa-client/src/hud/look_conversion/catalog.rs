//! Native 397d0500 /397d0460 /397ce340 static conversion rules.
use aa_net::model::{Item, ItemDetail};
use rusqlite::Connection;
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct ItemKind {
    kind: u32,
    gender: u32,
}

#[derive(Clone, Copy)]
struct Equipment {
    asset: u32,
    class: u32,
    skin: u32,
    stat: bool,
    armor_kind: u32,
}

#[derive(Clone, Debug)]
pub(super) struct Required {
    pub item: u32,
    pub count: i32,
}

#[derive(Clone, Debug)]
pub(super) struct Conversion {
    pub id: u32,
    pub gold: i32,
    pub required: Vec<Required>,
    holdables: Vec<u32>,
    wearables: Vec<u32>,
}

#[derive(Clone, Default)]
pub(in crate::hud) struct Tables {
    items: HashMap<u32, ItemKind>,
    weapons: HashMap<u32, Equipment>,
    armor: HashMap<u32, Equipment>,
    backpacks: HashMap<u32, Equipment>,
    wearable_slots: HashMap<u32, u32>,
    // Native traversal chooses the first matching group. Slot 9 occurs twice.
    groups: Vec<Conversion>,
}

impl Tables {
    pub fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut result = Self::default();
        let mut query = db.prepare("SELECT id, impl_id, char_gender_id FROM items")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                ItemKind {
                    kind: r.get::<_, Option<u32>>(1)?.unwrap_or(0),
                    gender: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                },
            ))
        })? {
            let (id, item) = row?;
            result.items.insert(id, item);
        }
        for (name, sql) in [
            (
                1,
                "SELECT item_id, asset_id, holdable_id, skin_kind_id, useAsStat, 0 FROM item_weapons",
            ),
            (
                2,
                "SELECT item_id, item_id, slot_type_id, skin_kind_id, useAsStat, type_id FROM item_armors",
            ),
            (
                22,
                "SELECT item_id, asset_id, backpack_type_id, skin_kind_id, use_as_stat, 0 FROM item_backpacks",
            ),
        ] {
            let mut query = db.prepare(sql)?;
            for row in query.query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    Equipment {
                        asset: r.get::<_, Option<u32>>(1)?.unwrap_or(0),
                        class: r.get::<_, Option<u32>>(2)?.unwrap_or(0),
                        skin: r.get::<_, Option<u32>>(3)?.unwrap_or(0),
                        stat: matches!(r.get::<_, Option<String>>(4)?.as_deref(), Some("t")),
                        armor_kind: r.get::<_, Option<u32>>(5)?.unwrap_or(0),
                    },
                ))
            })? {
                let (id, equipment) = row?;
                match name {
                    1 => &mut result.weapons,
                    2 => &mut result.armor,
                    _ => &mut result.backpacks,
                }
                .entry(id)
                .or_insert(equipment);
            }
        }
        let mut query = db.prepare("SELECT id, slot_type_id FROM wearable_slots")?;
        for row in query.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (id, slot) = row?;
            result.wearable_slots.insert(id, slot);
        }
        let mut query = db.prepare("SELECT id, gold FROM item_look_converts")?;
        for row in query.query_map([], |r| {
            Ok(Conversion {
                id: r.get(0)?,
                gold: r.get(1)?,
                required: Vec::new(),
                holdables: Vec::new(),
                wearables: Vec::new(),
            })
        })? {
            result.groups.push(row?);
        }
        let mut query = db.prepare("SELECT item_look_convert_id, item_id, item_count FROM item_look_convert_required_items")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                Required {
                    item: r.get(1)?,
                    count: r.get(2)?,
                },
            ))
        })? {
            let (id, required) = row?;
            if let Some(group) = result.groups.iter_mut().find(|g| g.id == id) {
                group.required.push(required);
            }
        }
        for (weapon, sql) in [
            (
                true,
                "SELECT item_look_convert_id, holdable_id FROM item_look_convert_holdables",
            ),
            (
                false,
                "SELECT item_look_convert_id, wearable_slot_id FROM item_look_convert_wearables",
            ),
        ] {
            let mut query = db.prepare(sql)?;
            for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
                let (id, value) = row?;
                if let Some(group) = result.groups.iter_mut().find(|g| g.id == id) {
                    if weapon {
                        group.holdables.push(value);
                    } else {
                        group.wearables.push(value);
                    }
                }
            }
        }
        Ok(result)
    }

    fn equipment(&self, id: u32) -> Option<(u32, Equipment)> {
        let kind = self.items.get(&id)?.kind;
        let map = match kind {
            1 => &self.weapons,
            2 => &self.armor,
            22 => &self.backpacks,
            _ => return None,
        };
        Some((kind, *map.get(&id)?))
    }

    fn valid(kind: u32, item: Equipment) -> bool {
        match kind {
            1 => item.asset != 0,
            2 => item.asset != 0 && item.armor_kind != 4,
            22 => item.class == 2,
            _ => false,
        }
    }

    fn group(&self, kind: u32, class: u32) -> Option<u32> {
        if kind == 22 {
            return Some(27);
        }
        let slot = if kind == 2 {
            *self.wearable_slots.get(&class)?
        } else {
            class
        };
        self.groups
            .iter()
            .find(|g| {
                if kind == 1 {
                    g.holdables.contains(&slot)
                } else {
                    g.wearables.contains(&slot)
                }
            })
            .map(|g| g.id)
    }

    pub fn base_group(&self, item: &Item) -> Option<u32> {
        let (kind, equipment) = self.equipment(item.template_id)?;
        //39046c50 first-slot setter only checks the stat/group descriptor;
        // stricter asset/type exclusions belong to pair validation.
        (equipment.stat && (kind != 22 || equipment.class == 2))
            .then(|| self.group(kind, equipment.class))
            .flatten()
    }
    pub fn base_eligible(&self, item: &Item) -> bool {
        self.base_group(item).is_some()
            && self
                .equipment(item.template_id)
                .is_some_and(|(kind, data)| kind != 2 || data.asset != 0 && data.armor_kind != 4)
    }
    pub fn image_group(&self, item: &Item) -> Option<u32> {
        let (kind, data) = self.equipment(item.template_id)?;
        //39046e90 has only weapon/armor branches when the base key is empty.
        // Backpacks reach the special pair branch after selecting their base.
        (matches!(kind, 1 | 2) && (data.skin == 2 || data.skin == 3 && item.flags & 8 != 0))
            .then(|| self.group(kind, data.class))
            .flatten()
    }

    pub fn pair(&self, base: &Item, image: &Item) -> Result<u32, i16> {
        let a = self.items.get(&base.template_id).ok_or(0xb5i16)?;
        let b = self.items.get(&image.template_id).ok_or(0xb5i16)?;
        if a.kind != b.kind {
            return Err(0x2e1);
        }
        if a.gender != b.gender {
            return Err(0x2f1);
        }
        if !matches!(a.kind, 1 | 2 | 22) {
            return Err(0x2e3);
        }
        let (kind, stat) = self.equipment(base.template_id).ok_or(0xb5i16)?;
        let (_, look) = self.equipment(image.template_id).ok_or(0xb5i16)?;
        if !stat.stat || !Self::valid(kind, stat) {
            return Err(0x2e2);
        }
        if !Self::valid(kind, look) || !(look.skin == 2 || look.skin == 3 && image.flags & 8 != 0) {
            return Err(0x2e3);
        }
        let group = self.group(kind, stat.class).ok_or(0x2e1i16)?;
        if self.group(kind, look.class) != Some(group) {
            return Err(0x2e1);
        }
        Ok(group)
    }

    pub fn skinizable(&self, item: &Item) -> bool {
        item.flags & 8 == 0
            && self
                .equipment(item.template_id)
                .is_some_and(|(kind, data)| data.skin == 3 && Self::valid(kind, data))
    }

    pub(super) fn conversion(&self, id: u32) -> Option<&Conversion> {
        self.groups.iter().find(|g| g.id == id)
    }
}

pub(super) fn effective_look(item: &Item) -> u32 {
    match &item.detail {
        ItemDetail::Equip(detail) if detail.image_item_template_id != 0 => {
            detail.image_item_template_id
        }
        ItemDetail::Other(9) => item
            .detail_payload
            .as_deref()
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_le_bytes)
            .filter(|&id| id != 0)
            .unwrap_or(item.template_id),
        _ => item.template_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Tables {
        let db = Connection::open_with_flags(
            aa_anim::db::DEFAULT_DB,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        Tables::load(&db).unwrap()
    }
    fn item(template: u32, flags: u8) -> Item {
        Item {
            template_id: template,
            id: u64::from(template),
            count: 1,
            flags,
            ..Default::default()
        }
    }

    #[test]
    fn original_real_weapon_pair_and_recipe_retain_native_groups_and_flags() {
        let mut data = tables();
        let base = item(26060, 0);
        let image = item(26118, 8);
        assert_eq!(data.pair(&base, &image), Ok(1));
        assert_eq!(data.pair(&base, &item(26118, 0)), Err(739));
        let recipe = data.conversion(1).unwrap();
        assert_eq!(recipe.gold, 0);
        assert_eq!(
            (recipe.required[0].item, recipe.required[0].count),
            (29880, 9)
        );
        assert!(data.skinizable(&item(26118, 0)));
        assert!(!data.skinizable(&image));

        // Same controlled descriptor mutations as the executed original397d0500
        // goldens: base validity, gender, group mismatch and donor skin-kind law.
        data.weapons.get_mut(&26060).unwrap().stat = false;
        assert_eq!(data.pair(&base, &image), Err(738));
        data.weapons.get_mut(&26060).unwrap().stat = true;
        data.items.get_mut(&26118).unwrap().gender = 99;
        assert_eq!(data.pair(&base, &image), Err(753));
        data.items.get_mut(&26118).unwrap().gender = data.items[&26060].gender;
        let class = data.weapons[&26118].class;
        data.weapons.get_mut(&26118).unwrap().class = 3;
        assert_eq!(data.pair(&base, &image), Err(737));
        data.weapons.get_mut(&26118).unwrap().class = class;
        for (skin, expected) in [(1, Err(739)), (2, Ok(1)), (3, Ok(1)), (4, Err(739))] {
            data.weapons.get_mut(&26118).unwrap().skin = skin;
            assert_eq!(data.pair(&base, &image), expected);
        }
    }

    #[test]
    fn duplicate_wearable_groups_keep_first_native_enumeration_match() {
        let data = tables();
        let matches: Vec<_> = data
            .groups
            .iter()
            .filter(|g| g.wearables.contains(&9))
            .map(|g| g.id)
            .collect();
        assert_eq!(matches, [25, 27]);
        let slot = data
            .wearable_slots
            .iter()
            .find_map(|(id, class)| (*class == 9).then_some(*id))
            .unwrap();
        assert_eq!(data.group(2, slot), Some(25));
        assert_eq!(data.group(22, 2), Some(27));
    }

    #[test]
    fn effective_appearance_uses_both_native_detail_kinds_without_changing_base_identity() {
        let mut gear = item(26060, 0);
        assert_eq!(effective_look(&gear), 26060);
        gear.detail = ItemDetail::Equip(aa_net::model::EquipDetail {
            image_item_template_id: 26118,
            ..Default::default()
        });
        assert_eq!(effective_look(&gear), 26118);
        gear.detail = ItemDetail::Other(9);
        gear.detail_payload = Some(26118u32.to_le_bytes().to_vec());
        assert_eq!(effective_look(&gear), 26118);
        gear.detail_payload = Some(0u32.to_le_bytes().to_vec());
        assert_eq!(effective_look(&gear), 26060);
        assert_eq!(gear.template_id, 26060);
    }
}
