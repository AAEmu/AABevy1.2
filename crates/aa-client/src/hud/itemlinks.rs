//! Native item-link records from actual lane items and their retained detail payloads.
use aa_net::model::{Item, ItemDetail};
use aa_ui::item_link::{self, EquipDetail, ItemRecord};
use mlua::{Lua, Table};

use super::api::Shared;
use super::tips::{self, Instance};

pub fn record(item: &Item) -> Option<ItemRecord> {
    let detail = if let Some(payload) = &item.detail_payload {
        item_link::ItemDetail::from_wire(item.detail.kind(), payload)?
    } else {
        match &item.detail {
            ItemDetail::None => item_link::ItemDetail::None,
            ItemDetail::Equip(d) => item_link::ItemDetail::Equip(EquipDetail {
                image_item_template_id: d.image_item_template_id,
                durability: d.durability,
                // Synthetic typed equipment has no raw payload; the lane emits i16(0) here.
                reserved: [0; 2],
                rune_id: d.rune_id,
                charge_time: d.charge_time,
                dye_item_id: d.dye_item_id,
                gem_ids: d.gem_ids,
                temper_physical: d.temper_physical,
                temper_magical: d.temper_magical,
            }),
            ItemDetail::Mate { exp, dead, level } => item_link::ItemDetail::Mate {
                exp: *exp,
                dead: *dead,
                level: *level,
            },
            ItemDetail::Other(_) => return None,
        }
    };
    Some(ItemRecord {
        id: item.id,
        item_type: item.template_id,
        grade: item.grade,
        flags: item.flags,
        count: item.count,
        made_unit_id: item.made_unit_id,
        detail,
        create_time: item.create_time,
        lifespan_mins: item.lifespan_mins,
        world_id: item.world_id,
        unsecure_time: item.unsecure_time,
        unpack_time: item.unpack_time,
    })
}

pub fn link(item: &Item) -> Option<String> {
    Some(item_link::format(&item_link::pack(&record(item)?)))
}

/// Chat supplies only the encoded payload; native loot scripts supply the whole link.
/// Native394dfae0 skips the two comma-separated header fields; the payload owns type/grade.
fn decode(text: &str) -> Option<ItemRecord> {
    let payload = if let Some(body) = text.strip_prefix("|i").and_then(|s| s.strip_suffix(';')) {
        let mut fields = body.splitn(3, ',');
        fields.next()?.parse::<u32>().ok()?;
        fields.next()?.parse::<u8>().ok()?;
        fields.next()?
    } else {
        text
    };
    let record = item_link::unpack(&item_link::decode_payload(payload)?);
    (record.item_type != 0).then_some(record)
}

fn argb(text: &str) -> Option<[f32; 4]> {
    let color = u32::from_str_radix(text, 16).ok()?;
    Some([
        ((color >> 16) & 255) as f32 / 255.0,
        ((color >> 8) & 255) as f32 / 255.0,
        (color & 255) as f32 / 255.0,
        (color >> 24) as f32 / 255.0,
    ])
}

/// Native39271e10: task type, level difference and the native low-level band.
fn quest_color(task: bool, level: u8, player: u8) -> [f32; 4] {
    if task {
        [0.72, 0.97, 0.99, 1.0]
    } else if i32::from(level) - i32::from(player) >= 6 {
        [0.73, 0.22, 0.22, 1.0]
    } else if i32::from(level) - i32::from(player) >= 3 {
        [0.82, 0.55, 0.04, 1.0]
    } else if i32::from(level) < i32::from(player) - 5.max((i32::from(player) + 1) / 5) {
        [0.56, 0.56, 0.56, 1.0]
    } else {
        [1.0; 4]
    }
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let aaui: Table = lua.globals().get("AAUI")?;
    let b = bridge.clone();
    aaui.set(
        "ChatLink",
        lua.create_function(move |lua, (kind, id, grade): (String, u32, Option<u8>)| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok(None) };
            let (name, color) = match kind.as_str() {
                "item" => {
                    let Some(info) = game.items.get(&id) else { return Ok(None) };
                    let grade = i64::from(super::store::ui_grade(grade.unwrap_or(0)));
                    (info.named.name.clone(), game.grades.get(&grade).and_then(|g| argb(&g.1)).unwrap_or([1.0; 4]))
                }
                "quest" => {
                    let Some(q) = game.quests.def(id) else { return Ok(None) };
                    let texts = super::quests::Texts {
                        game,
                        player: &d.snapshot.name,
                        race: d.snapshot.race,
                    };
                    (texts.expand(&q.name), quest_color(q.detail == 6, q.level, d.snapshot.level))
                }
                _ => return Ok(None),
            };
            if name.is_empty() {
                return Ok(None);
            }
            let out = lua.create_table()?;
            out.set("name", name)?;
            out.set("color", lua.create_sequence_from(color)?)?;
            Ok(Some(out))
        })?,
    )?;
    let item: Table = lua.globals().get("X2Item")?;
    let b = bridge.clone();
    item.set(
        "InfoFromLink",
        lua.create_function(move |lua, (_, text): (Table, String)| {
            let Some(record) = decode(&text) else { return Ok(None) };
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok(None) };
            let music_note = match &record.detail {
                item_link::ItemDetail::Opaque { kind: 8, payload } => Some(u64::from_le_bytes(payload[..8].try_into().unwrap())),
                _ => None,
            };
            let instance = match record.detail {
                item_link::ItemDetail::Equip(e) => Instance {
                    durability: Some(e.durability),
                    temper: (e.temper_physical, e.temper_magical),
                    flags: Some(record.flags),
                },
                _ => Instance {
                    flags: Some(record.flags),
                    ..Default::default()
                },
            };
            tips::item_tip_with(
                lua,
                game,
                &d.tips,
                record.item_type,
                Some(i64::from(super::store::ui_grade(record.grade))),
                Some(i64::from(record.count)),
                instance,
                &tips::ItemExtra { music_note, ..Default::default() },
            )
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_item_fields_survive_native_record_and_chat_normalization() {
        let mut item = Item {
            id: 9_007_199_254_740_993,
            template_id: 24488,
            grade: 4,
            flags: 3,
            count: 17,
            detail: ItemDetail::Equip(aa_net::model::EquipDetail {
                image_item_template_id: 123,
                durability: 39,
                rune_id: 456,
                charge_time: 789,
                dye_item_id: 321,
                gem_ids: [1, 2, 3, 4, 5, 6, 7],
                temper_physical: 91,
                temper_magical: 92,
            }),
            create_time: 1_792_276_001,
            lifespan_mins: 43,
            made_unit_id: 44,
            world_id: 45,
            unsecure_time: 1_792_276_002,
            unpack_time: 1_792_276_003,
            detail_payload: None,
        };
        let original = record(&item).unwrap();
        let text = link(&item).unwrap();
        let decoded = decode(&text).unwrap();
        let mut expected = original.clone();
        expected.id = 0;
        expected.count = 1;
        assert_eq!(decoded, expected);
        let payload = text.split(',').nth(2).unwrap().trim_end_matches(';');
        assert_eq!(decode(payload), Some(expected.clone()));
        assert_eq!(decode(&format!("|i1,0,{payload};")), Some(expected));
        assert_eq!(item.id, original.id);
        assert_eq!(item.count, 17);
        item.detail = ItemDetail::Mate { exp: 1234, dead: 1, level: 28 };
        assert_eq!(
            decode(&link(&item).unwrap()).unwrap().detail,
            item_link::ItemDetail::Mate { exp: 1234, dead: 1, level: 28 }
        );
        item.detail = ItemDetail::None;
        assert_eq!(decode(&link(&item).unwrap()).unwrap().detail, item_link::ItemDetail::None);
        item.detail = ItemDetail::Other(9);
        assert!(link(&item).is_none());
    }

    #[test]
    fn malformed_or_legacy_links_never_create_item_tooltip_records() {
        for text in [
            "",
            "|i1,0,bad;",
            "|i1,nope,000;",
            "|i1,0,000",
            &"0".repeat(235),
            &"=".repeat(256),
            &"0".repeat(256),
        ] {
            assert!(decode(text).is_none());
        }
    }

    #[test]
    fn retained_payload_keeps_opaque_details_and_equipment_reserved_bytes() {
        let mut item = Item {
            template_id: 24488,
            id: 1,
            detail: ItemDetail::Other(8),
            detail_payload: Some(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            ..Default::default()
        };
        let packed = item_link::pack(&record(&item).unwrap());
        assert_eq!(&packed[24..33], &[8, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(packed[33..152].iter().all(|b| *b == 0));
        let mut payload = vec![0; 55];
        payload[5..7].copy_from_slice(&[0xaa, 0xbb]);
        item.detail = ItemDetail::Equip(Default::default());
        item.detail_payload = Some(payload);
        let packed = item_link::pack(&record(&item).unwrap());
        assert_eq!(&packed[30..32], &[0xaa, 0xbb]);
        item.detail = ItemDetail::Other(2);
        item.detail_payload = Some(vec![0; 33]);
        assert!(link(&item).is_none()); // Native tag2 is29 bytes; never truncate the lane's33.
    }

    #[test]
    fn link_colors_keep_native_alpha_grade_and_quest_level_boundaries() {
        assert_eq!(argb("80402010"), Some([64.0 / 255.0, 32.0 / 255.0, 16.0 / 255.0, 128.0 / 255.0]));
        assert_eq!(quest_color(false, 56, 50), [0.73, 0.22, 0.22, 1.0]);
        assert_eq!(quest_color(false, 55, 50), [0.82, 0.55, 0.04, 1.0]);
        assert_eq!(quest_color(false, 53, 50), [0.82, 0.55, 0.04, 1.0]);
        assert_eq!(quest_color(false, 52, 50), [1.0; 4]);
        assert_eq!(quest_color(false, 40, 50), [1.0; 4]);
        assert_eq!(quest_color(false, 39, 50), [0.56, 0.56, 0.56, 1.0]);
        assert_eq!(quest_color(true, 56, 50), [0.72, 0.97, 0.99, 1.0]);
    }
}
