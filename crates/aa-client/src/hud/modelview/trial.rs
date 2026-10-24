//! The widget's local Item constructor and equipment reducer (392bb980).
//! These ID-zero records are never submitted to the network or inventory.
use aa_anim::db::{GameDb, ModelviewTrialItem};
use aa_net::model::{EquipDetail, Equipment, Item, ItemDetail, slot};
use anyhow::Result;

#[derive(Clone)]
pub(super) struct Trial {
    initial: Equipment,
    pub equipment: Equipment,
    pub selector: u8,
    pub appearance: [u8; 6],
}

impl Trial {
    pub fn new(equipment: Equipment) -> Self {
        Self {
            initial: equipment.clone(),
            equipment,
            // Original Init writes these after both appearance copies and before
            // its create_model branch (392bbfb7..392bbfda; native-init.json).
            selector: 0,
            appearance: [0, 0, 0, 0, 20, 100],
        }
    }

    pub fn reset(&mut self) {
        self.equipment = self.initial.clone();
        self.selector = 0;
        self.appearance = [0, 0, 0, 0, 20, 100];
    }

    pub fn equip(&mut self, db: &GameDb, gender: u32, id: u32, file_time: i64) -> Result<bool> {
        let Some(meta) = db.modelview_trial_item(id)? else {
            return Ok(false);
        };
        let main = if meta.kind == 1 && matches!(meta.slot_type, Some(15 | 20)) {
            self.equipment
                .get(slot::MAINHAND)
                .map(|i| db.modelview_trial_item(i.template_id))
                .transpose()?
                .flatten()
        } else {
            None
        };
        Ok(self.apply(gender, id, &meta, main.as_ref(), file_time))
    }

    fn apply(
        &mut self,
        gender: u32,
        id: u32,
        meta: &ModelviewTrialItem,
        main: Option<&ModelviewTrialItem>,
        file_time: i64,
    ) -> bool {
        // Active widget context, including native zero after Clear. The copied
        // assembly snapshot can still describe a female/male model after that.
        if id == 0 || (meta.gender != 0 && meta.gender != gender) {
            return false;
        }
        let mut selector = self.selector;
        let mut appearance = self.appearance;
        let mut clear = None;
        let target = match meta.kind {
            1 => match meta.slot_type {
                Some(14 | 17) => {
                    selector = 1;
                    slot::MAINHAND
                }
                Some(16) => {
                    selector = 1;
                    clear = Some(slot::OFFHAND);
                    slot::MAINHAND
                }
                Some(15 | 20) => {
                    selector = 1;
                    if main.is_some_and(|m| m.kind == 1 && m.slot_type == Some(16)) {
                        clear = Some(slot::MAINHAND);
                    }
                    slot::OFFHAND
                }
                Some(18) => {
                    selector = 2;
                    slot::RANGED
                }
                Some(21) => {
                    selector = 4;
                    slot::MUSICAL
                }
                _ => return false,
            },
            2 => {
                let Some(raw) = meta.slot_type else {
                    return false;
                };
                // Native retires costume before checking the armor slot mapping.
                if raw != 31 {
                    self.equipment.slots[slot::COSPLAY] = None;
                }
                let Some(target) = mapped_slot(raw) else {
                    return false;
                };
                if raw == 9 {
                    appearance = [0, 0, 0, 0, 100, 0];
                }
                target
            }
            22 => {
                let Some(kind) = meta.backpack_type else {
                    return false;
                };
                appearance = if kind == 2 {
                    [100, 0, 0, 0, 0, 0]
                } else {
                    [0, 0, 0, 0, 0, 100]
                };
                slot::BACKPACK
            }
            _ => return false,
        };
        if let Some(clear) = clear {
            self.equipment.slots[clear] = None;
        }
        self.equipment.slots[target] = Some(construct(id, meta, file_time));
        self.selector = selector;
        self.appearance = appearance;
        true
    }
}

fn mapped_slot(raw: u32) -> Option<usize> {
    Some(match raw {
        1..=10 => raw as usize - 1,
        11 => 11,
        12 => 13,
        13 => 14,
        14 | 16 | 17 => 15,
        15 | 20 => 16,
        18 => 17,
        21 => 18,
        23..=31 => raw as usize - 4,
        _ => return None,
    })
}

fn construct(id: u32, meta: &ModelviewTrialItem, file_time: i64) -> Item {
    let charged = meta.recharge_buff_id != 0;
    let detail = EquipDetail {
        durability: meta.durability,
        dye_item_id: if meta.kind == 2 {
            meta.default_dyeing_item_id
        } else {
            0
        },
        charge_time: if charged { file_time } else { 0 },
        ..Default::default()
    };
    // Exact interpreted 55-byte prefix after tag1; the native 128-byte union's
    // remaining zero padding has no public Item representation or render consumer.
    let mut payload = vec![0; 55];
    payload[4] = detail.durability;
    if charged {
        payload[5..7].copy_from_slice(&meta.charge_count.to_le_bytes());
    }
    payload[11..19].copy_from_slice(&detail.charge_time.to_le_bytes());
    payload[19..23].copy_from_slice(&detail.dye_item_id.to_le_bytes());
    Item {
        template_id: id,
        grade: 0,
        flags: u8::from(matches!(meta.bind_id, 2 | 3 | 5)),
        count: 1,
        detail: ItemDetail::Equip(detail),
        detail_payload: Some(payload),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_35_original_slot_mapping_outputs_match() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("trial/native.json")).unwrap();
        for case in data["slots"].as_array().unwrap() {
            let raw = case["raw_slot_type"].as_u64().unwrap() as u32;
            let expected = case["equipment_slot"].as_u64().unwrap() as u32;
            assert_eq!(
                mapped_slot(raw).map_or(u32::MAX, |v| v as u32),
                expected,
                "raw slot {raw}"
            );
        }
    }
    #[test]
    #[ignore = "requires supplied read-only compact catalog"]
    fn ten_actual_constructor_prefixes_match_executed_native_records() {
        let db = crate::ingame::data::Db::open().unwrap();
        let db = db.game.lock().unwrap();
        let data: serde_json::Value =
            serde_json::from_str(include_str!("trial/native.json")).unwrap();
        for case in data["constructors"].as_object().unwrap().values() {
            let id = case["template"].as_u64().unwrap() as u32;
            let meta = db.modelview_trial_item(id).unwrap().unwrap();
            let item = construct(id, &meta, 0x123456789abcdef);
            assert_eq!(item.id, case["id"].as_u64().unwrap());
            assert_eq!(u64::from(item.grade), case["grade"].as_u64().unwrap());
            assert_eq!(u64::from(item.flags), case["flags"].as_u64().unwrap());
            assert_eq!(item.count, case["count"].as_i64().unwrap() as i32);
            let ItemDetail::Equip(detail) = item.detail else {
                panic!("tag1")
            };
            for (name, actual) in [
                ("image", u64::from(detail.image_item_template_id)),
                ("durability", u64::from(detail.durability)),
                ("dye", u64::from(detail.dye_item_id)),
                ("charge_time", detail.charge_time as u64),
            ] {
                assert_eq!(actual, case[name].as_u64().unwrap(), "{id} {name}");
            }
            let payload = item.detail_payload.unwrap();
            assert_eq!(
                u64::from(u16::from_le_bytes([payload[5], payload[6]])),
                case["charge_count"].as_u64().unwrap()
            );
        }
    }
    fn meta(kind: u32, raw: u32) -> ModelviewTrialItem {
        ModelviewTrialItem {
            kind,
            gender: 0,
            bind_id: 0,
            slot_type: Some(raw),
            backpack_type: None,
            default_dyeing_item_id: 0,
            durability: 0,
            charge_count: 0,
            recharge_buff_id: 0,
        }
    }
    #[test]
    fn conflicts_use_holdable_slot_including_staff_and_reset_keeps_received_details() {
        let mut equipment = Equipment::default();
        equipment.slots[slot::MAINHAND] = Some(Item {
            template_id: 6102,
            id: 47,
            grade: 6,
            count: 1,
            detail: ItemDetail::Equip(EquipDetail {
                image_item_template_id: 5660,
                dye_item_id: 42,
                ..Default::default()
            }),
            ..Default::default()
        });
        equipment.slots[slot::RANGED] = Item::template_only(6127);
        let mut trial = Trial::new(equipment.clone());
        assert_eq!(trial.selector, 0);
        assert_eq!(trial.appearance, [0, 0, 0, 0, 20, 100]);
        assert!(trial.apply(2, 6152, &meta(1, 20), Some(&meta(1, 16)), 0));
        assert!(trial.equipment.get(slot::MAINHAND).is_none());
        assert_eq!(trial.equipment.get(slot::RANGED).unwrap().template_id, 6127);
        assert!(trial.apply(2, 5660, &meta(1, 16), None, 0));
        assert!(trial.equipment.get(slot::OFFHAND).is_none());
        trial.reset();
        assert_eq!(trial.equipment, equipment);
        assert_eq!(trial.selector, 0);
        assert_eq!(trial.appearance, [0, 0, 0, 0, 20, 100]);
    }
    #[test]
    fn constructor_keeps_grade_zero_and_raw_charge_and_dye() {
        let mut m = meta(2, 7);
        m.bind_id = 3;
        m.durability = 40;
        m.default_dyeing_item_id = 29621;
        m.charge_count = 0x1234;
        m.recharge_buff_id = 42;
        let item = construct(24583, &m, 0x123456789abcdef);
        assert_eq!((item.id, item.grade, item.flags, item.count), (0, 0, 1, 1));
        let payload = item.detail_payload.as_ref().unwrap();
        assert_eq!(&payload[5..7], &[0x34, 0x12]);
        let mut reader = aa_net::wire::Reader::new(payload);
        assert_eq!(ItemDetail::read(&mut reader, 1).unwrap(), item.detail);
    }
    #[test]
    fn armor_costume_backpack_and_gender_follow_distinct_branches() {
        let mut trial = Trial::new(Equipment::default());
        assert!(trial.apply(1, 8000028, &meta(2, 31), None, 0));
        assert!(trial.apply(1, 18180, &meta(2, 7), None, 0));
        assert!(trial.equipment.get(slot::COSPLAY).is_none());
        assert!(trial.apply(1, 24088, &meta(2, 9), None, 0));
        assert_eq!(trial.appearance, [0, 0, 0, 0, 100, 0]);
        let mut pack = meta(22, 30);
        pack.backpack_type = Some(2);
        assert!(trial.apply(1, 23612, &pack, None, 0));
        assert_eq!(trial.appearance, [100, 0, 0, 0, 0, 0]);
        let before = trial.equipment.clone();
        pack.gender = 2;
        assert!(!trial.apply(1, 23612, &pack, None, 0));
        assert!(!trial.apply(0, 23612, &pack, None, 0));
        assert_eq!(trial.equipment, before);
    }
}
