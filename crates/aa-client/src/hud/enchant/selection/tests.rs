use super::*;
use crate::hud::{api::HudData, data::GameData};
use aa_net::{
    initial_config::InitialConfig,
    items::InvenChunk,
    model::{EquipDetail, ItemDetail},
};
use std::{cell::RefCell, rc::Rc};

fn item(template_id: u32, id: u64) -> Item {
    Item {
        template_id,
        id,
        count: 3,
        ..Default::default()
    }
}
fn config(flags: u8) -> Event {
    let mut feature_bytes = vec![0; 11];
    feature_bytes[5] = flags;
    Event::InitialConfig(InitialConfig {
        host: vec![],
        feature_bytes,
        raw_candidate_count: 0,
        candidate_codes: vec![],
        initial_lp: 0,
        service_flags: [0; 7],
        second_price_type: 0,
        second_password_max_fail_count: 0,
        idle_kick_time: 0,
    })
}
fn seeded() -> State {
    let mut state = State::default();
    state.receive(&config(0));
    let spawn = crate::hud::expansion::fixture::spawn();
    state.receive(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    state.receive(&Event::PlayerSpawn(Box::new(spawn)));
    state.receive(&Event::InventoryInit {
        bag_slots: 50,
        bank_slots: 50,
    });
    let mut items = vec![None; 50];
    let mut target = item(31432, 9_007_199_254_740_993);
    target.grade = 6;
    target.count = 1;
    target.detail = ItemDetail::Equip(EquipDetail::default());
    items[0] = Some(target);
    items[1] = Some(item(28299, 200));
    items[2] = Some(item(31974, 300));
    items[3] = Some(item(31003, 400));
    items[4] = Some(item(327, 500));
    items[5] = Some(item(24632, 600));
    items[6] = Some(item(28299, 700));
    items[6].as_mut().unwrap().count = 4;
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 5,
        start_chunk: 0,
        items,
        truncated: false,
    }));
    state
}
fn tables() -> Tables {
    Tables::load(&rusqlite::Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap())
        .unwrap()
}

#[test]
fn startup_recharge_catalog_is_available_after_hud_lifecycle_rebuild() {
    use aa_net::items::{ItemTask, ItemTasks};

    let first = crate::hud::item_recharge_catalog().expect("readonly static catalog");
    let again = crate::hud::item_recharge_catalog().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    assert_eq!(first.get(&26390), Some(&4874));
    let mut state = seeded();
    for reset in [false, true] {
        if reset {
            state.receive(&Event::LeftWorld { target: 0 });
            let spawn = crate::hud::expansion::fixture::spawn();
            state.receive(&Event::CharacterSelected(Box::new(spawn.state.clone())));
            state.receive(&Event::PlayerSpawn(Box::new(spawn)));
            state.receive(&Event::InventoryInit {
                bag_slots: 10,
                bank_slots: 0,
            });
        }
        let mut charged = item(26390, 123);
        charged.detail = ItemDetail::Equip(EquipDetail {
            charge_time: 102,
            ..Default::default()
        });
        let mut payload = vec![0; 55];
        payload[5] = 0xa5;
        payload[11..19].copy_from_slice(&102i64.to_le_bytes());
        charged.detail_payload = Some(payload);
        let mut items = vec![Some(charged)];
        items.resize(10, None);
        state.receive(&Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 1,
            start_chunk: 0,
            items,
            truncated: false,
        }));
        state.receive(&Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Security {
                container: 2,
                slot: 0,
                item_id: 999,
                bits: 4,
                is_unsecure_excess: 0,
                is_unsecure_set: 0,
                is_unpack: 255,
                unsecure_time: -7,
                unpack_time: -9,
            }],
            ..Default::default()
        }));
        let current = state.records.item(2, 0).expect("unpack keeps qualified native item known");
        assert!(matches!(&current.detail, ItemDetail::Equip(e) if e.charge_time == -9));
        let payload = current.detail_payload.as_ref().unwrap();
        assert_eq!(&payload[11..19], &(-9i64).to_le_bytes());
        assert_eq!(payload[5], 0xa5);
    }
}
fn edit(state: &mut State, slot: u8, change: impl FnOnce(&mut Item)) {
    let start_chunk = slot / 10;
    let mut items: Vec<_> = (start_chunk * 10..start_chunk * 10 + 10)
        .map(|n| state.records.item(2, n).cloned())
        .collect();
    change(items[usize::from(slot % 10)].as_mut().unwrap());
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk,
        items,
        truncated: false,
    }));
}

#[test]
fn actual_four_modes_select_target_source_in_either_order_and_support_follows_native_grade_order() {
    let tables = tables();
    let skills = Skills::default();
    for (mode, source) in [(Mode::Grade, 1), (Mode::Socket, 3), (Mode::Unsocket, 4), (Mode::Gem, 5)] {
        for source_first in [false, true] {
            let mut state = seeded();
            if mode == Mode::Unsocket {
                edit(&mut state, 0, |item| {
                    let ItemDetail::Equip(e) = &mut item.detail else { unreachable!() };
                    e.gem_ids = [31003, 0, 31003, 0, 0, 0, 0];
                });
            }
            let order = if source_first {
                [(1, source), (0, 0)]
            } else {
                [(0, 0), (1, source)]
            };
            for (index, slot) in order {
                assert!(state.enchantable(&tables, &skills, mode, slot));
                assert_eq!(state.set(&tables, &skills, mode, index, slot), Ok(()), "{mode:?}/{index}");
            }
            assert!(state.presence());
            assert!(!state.enchantable(&tables, &skills, mode, source));
            if mode == Mode::Grade {
                assert_eq!(state.set(&tables, &skills, mode, 2, 2), Ok(()));
                assert!(!state.enchantable(&tables, &skills, mode, 6));
            } else {
                assert!(state.set(&tables, &skills, mode, 2, 2).is_err());
            }
            let before = state.slots;
            assert!(state.set(&tables, &skills, mode, 0, 6).is_err());
            assert_eq!(state.slots, before);
        }
    }
}

#[test]
fn received_snapshot_feature_pending_and_loss_guards_do_not_invent_readiness() {
    let tables = tables();
    let skills = Skills::default();
    let mut state = seeded();
    let items: Vec<_> = (0..50).map(|slot| state.records.item(2, slot).cloned()).collect();
    state.receive(&Event::InventoryInit {
        bag_slots: 50,
        bank_slots: 50,
    });
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 4,
        start_chunk: 0,
        items: items[..40].to_vec(),
        truncated: false,
    }));
    assert_eq!(state.target(&tables, &skills, Mode::Grade, 0), Err(0xb5));
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 4,
        items: items[40..].to_vec(),
        truncated: false,
    }));
    edit(&mut state, 0, |item| item.flags = 4);
    state.config.received = None;
    assert_eq!(state.target(&tables, &skills, Mode::Grade, 0), Err(1));
    state.receive(&config(0x20));
    assert_eq!(state.target(&tables, &skills, Mode::Grade, 0), Err(0x298));
    state.receive(&config(0));
    assert_eq!(state.target(&tables, &skills, Mode::Grade, 0), Ok(()));
    let mut pending = Inventory::default();
    pending.mark_item_slot_pending(2, 0);
    state.sync_pending(&pending);
    assert_eq!(state.target(&tables, &skills, Mode::Grade, 0), Err(0xb5));
    pending.clear_item_slot_pending(2, 0);
    state.sync_pending(&pending);
    state.set(&tables, &skills, Mode::Grade, 0, 0).unwrap();
    state.receive(&Event::UnhandledGamePacket {
        opcode: 0xffff,
        level: 3,
        len: 1,
    });
    assert!(state.selected(0).is_some());
    state.receive(&Event::UnhandledGamePacket {
        opcode: 0xffff,
        level: 4,
        len: 1,
    });
    assert!(state.selected(0).is_none());
    assert_eq!(state.slots, [None; 3]);
    state.receive(&Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())));
    assert!(!state.complete(), "same-actor spawn cannot reconstruct lost inventory");
}

#[test]
fn received_native_bag_header_floors_low_byte_and_contents_cannot_expand_it() {
    let mut state = seeded();
    // Genuine Init routes through3961bcc0: both native header fields floor10.
    for (slots, expected) in [(0, 0), (9, 0), (10, 10), (53, 50), (255, 250), (256, 0), (306, 50)] {
        state.receive(&Event::InventoryInit {
            bag_slots: slots,
            bank_slots: 50,
        });
        assert_eq!(state.records.bag_extent(), Some(expected));
        state.receive(&Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 5,
            start_chunk: 0,
            items: vec![None; 50],
            truncated: false,
        }));
        assert_eq!(state.records.bag_extent(), Some(expected), "contents do not modify native headers");
        assert_eq!(state.complete(), expected > 0 && expected <= 50);
    }
    state.receive(&Event::InventoryExpanded { container: 2, slots: 63 });
    assert_eq!(state.records.bag_extent(), Some(60));
    assert!(!state.complete(), "new slots need actual contents before selection");
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 5,
        items: vec![None; 10],
        truncated: false,
    }));
    assert!(state.complete());
    state.receive(&Event::InventoryExpanded { container: 3, slots: 90 });
    assert_eq!(state.records.bag_extent(), Some(60), "bank expansion cannot alter BAG header");
    state.receive(&Event::InventoryExpanded { container: 2, slots: 140 });
    assert!(!state.complete(), "whole-BAG quotes cannot scan beyond native130-row storage");
    let mut state = seeded();
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 6,
        start_chunk: 0,
        items: vec![None; 60],
        truncated: false,
    }));
    assert!(!state.complete(), "native SC4c packet storage holds only five blocks");
    let mut state = seeded();
    state.receive(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 13,
        items: vec![None; 10],
        truncated: false,
    }));
    assert!(
        !state.complete(),
        "a chunk outside native static storage cannot establish certainty"
    );
}

#[test]
fn selected_quotes_keep_native_zero_stacks_then_apply_id_removal_in_receipt_order() {
    use aa_net::items::{ItemTask, ItemTasks};
    let tables = tables();
    let mut state = seeded();
    state.set(&tables, &Skills::default(), Mode::Grade, 0, 0).unwrap();
    state.set(&tables, &Skills::default(), Mode::Grade, 1, 1).unwrap();
    state.receive(&Event::ItemsChanged(ItemTasks {
        reason: 40,
        tasks: vec![ItemTask::Count {
            container: 2,
            slot: 1,
            item_id: 999,
            template_id: 999,
            delta: -3,
        }],
        ..Default::default()
    }));
    assert_eq!(state.selected(1).unwrap().count, 0, "legacy reducer would erase this slot");
    let total = state
        .records
        .bag_items()
        .filter(|(_, item)| item.template_id == 28299)
        .map(|(_, item)| item.count)
        .sum::<i32>();
    assert_eq!(total, 4);
    state.receive(&Event::ItemsChanged(ItemTasks {
        reason: 40,
        tasks: vec![
            ItemTask::Count {
                container: 2,
                slot: 1,
                item_id: 999,
                template_id: 999,
                delta: -1,
            },
            ItemTask::Remove {
                container: 2,
                slot: 1,
                item_id: 200,
                template_id: 999,
            },
        ],
        ..Default::default()
    }));
    assert!(state.selected(1).is_none());
    assert_eq!(state.selected(0).unwrap().template_id, 31432);
    assert!(state.complete());
}

#[test]
fn typed_preparation_recaptures_all_four_modes_and_received_auto_setting_without_mutation() {
    let tables = tables();
    let skills = Skills::default();
    for (mode, slot, skill, ready) in [
        (Mode::Grade, 1, 22522, false),
        (Mode::Socket, 3, 23728, false),
        (Mode::Unsocket, 4, 23729, true),
        (Mode::Gem, 5, 11285, false),
    ] {
        let mut state = seeded();
        if mode == Mode::Unsocket {
            edit(&mut state, 0, |item| {
                let ItemDetail::Equip(detail) = &mut item.detail else {
                    unreachable!()
                };
                detail.gem_ids[0] = 31003;
            });
        }
        state.set(&tables, &skills, mode, 0, 0).unwrap();
        state.set(&tables, &skills, mode, 1, slot).unwrap();
        if mode == Mode::Grade {
            state.set(&tables, &skills, mode, 2, 2).unwrap();
        }
        let before: Vec<_> = state.records.bag_items().map(|(slot, item)| (slot, item.clone())).collect();
        let prepared = state.prepare(&tables, &skills, mode).unwrap();
        assert_eq!(prepared.character_id, 47);
        assert_eq!(prepared.request.skill_id(), skill);
        assert_eq!(prepared.input_ready, ready);
        assert_eq!(prepared.request.target().item.id, 9_007_199_254_740_993);
        assert_eq!(prepared.request.source().slot, slot);
        assert_eq!(prepared.request.support().is_some(), mode == Mode::Grade);
        assert_eq!(
            before,
            state
                .records
                .bag_items()
                .map(|(slot, item)| (slot, item.clone()))
                .collect::<Vec<_>>()
        );
        if mode == Mode::Grade {
            for raw in [0, 1, 2, 255] {
                state.receive(&Event::ItemsChanged(aa_net::items::ItemTasks {
                    tasks: vec![aa_net::items::ItemTask::AutoUseAaPoint(raw)],
                    ..Default::default()
                }));
                assert_eq!(
                    state.prepare(&tables, &skills, mode).unwrap().request.mode(),
                    ItemEffectMode::Grade {
                        auto_use_aa_point: raw == 1
                    }
                );
            }
        }
    }
}

#[test]
fn typed_preparation_revalidates_changed_items_external_locks_and_nonpositive_records() {
    let tables = tables();
    let skills = Skills::default();
    let mut state = seeded();
    state.set(&tables, &skills, Mode::Grade, 0, 0).unwrap();
    state.set(&tables, &skills, Mode::Grade, 1, 1).unwrap();
    let mut pending = Inventory::default();
    pending.mark_item_slot_pending(2, 1);
    state.sync_pending(&pending);
    assert!(state.prepare(&tables, &skills, Mode::Grade).is_err());
    pending.clear_item_slot_pending(2, 1);
    state.sync_pending(&pending);
    for count in [0, -1, i32::MIN] {
        edit(&mut state, 1, |item| item.count = count);
        assert!(state.prepare(&tables, &skills, Mode::Grade).is_err());
    }
    edit(&mut state, 1, |item| {
        item.count = 3;
        item.template_id = 31003;
    });
    assert!(state.prepare(&tables, &skills, Mode::Grade).is_err());
    assert_eq!(
        state.slots,
        [Some(0), Some(1), None],
        "preparation failure does not clear the selected physical keys"
    );
}

#[test]
fn completion_restores_current_physical_slots_and_uses_fallback_only_for_missing_old_source() {
    let tables = tables();
    let skills = Skills::default();
    let mut state = seeded();
    state.set(&tables, &skills, Mode::Grade, 0, 0).unwrap();
    state.set(&tables, &skills, Mode::Grade, 1, 1).unwrap();
    state.set(&tables, &skills, Mode::Grade, 2, 2).unwrap();
    let request = state.prepare(&tables, &skills, Mode::Grade).unwrap().request;
    edit(&mut state, 0, |item| item.id += 2);
    assert!(state.restore(&tables, &skills, Mode::Grade, &request, true));
    assert_eq!(state.slots, [Some(0), Some(1), Some(2)]);
    assert_eq!(
        state.selected(0).unwrap().id,
        9_007_199_254_740_995,
        "target is restored by old physical slot, not old ID"
    );
    edit(&mut state, 1, |item| item.template_id = 31003);
    assert!(!state.restore(&tables, &skills, Mode::Grade, &request, true));
    assert_eq!(
        state.slots,
        [Some(0), None, None],
        "valid wrong-class old source fails without trying duplicate slot6"
    );
    edit(&mut state, 1, |item| *item = Item::default());
    assert!(state.restore(&tables, &skills, Mode::Grade, &request, true));
    assert_eq!(state.slots, [Some(0), Some(6), Some(2)]);
    edit(&mut state, 0, |item| *item = Item::default());
    assert!(!state.restore(&tables, &skills, Mode::Grade, &request, true));
    assert_eq!(state.slots, [None; 3], "missing target stops before restoring sources");
}

#[test]
fn magical_receipt_latch_stops_after_target_restore_even_when_raw_result_was_failure() {
    let tables = tables();
    let skills = Skills::default();
    let mut state = seeded();
    state.set(&tables, &skills, Mode::Gem, 0, 0).unwrap();
    state.set(&tables, &skills, Mode::Gem, 1, 5).unwrap();
    let request = state.prepare(&tables, &skills, Mode::Gem).unwrap().request;
    assert!(!state.restore(&tables, &skills, Mode::Gem, &request, true));
    assert_eq!(state.slots, [Some(0), None, None]);
    assert!(
        state.restore(&tables, &skills, Mode::Gem, &request, false),
        "native stop without receipt latch takes ordinary source restore path"
    );
    assert_eq!(state.slots, [Some(0), Some(5), None]);
}

#[test]
fn lua_selection_numeric_arities_cursor_order_reentrant_updates_quotes_and_clear_contracts() -> Result<(), Box<dyn std::error::Error>> {
    let rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    rt.lua
        .load("X2Item={};X2ItemEnchant={};X2Bag={IsLocked=function(_,slot)return slot==40 end};X2Locale={LocalizeUiText=function(_,_,key)return key end};ERROR_MSG=1")
        .exec()?;
    let shared = Rc::new(RefCell::new(HudData::default()));
    {
        let mut data = shared.borrow_mut();
        assert!(data.game.0.set(GameData::load(aa_anim::db::DEFAULT_DB)?).is_ok());
        data.enchant.selection = seeded();
    }
    crate::hud::enchant::install(&rt.lua, &shared)?;
    rt.lua.load(r#"
      updates={};messages={};errors={};local w=UIParent:CreateWidget('emptywidget','selectionObserver','UIParent')
      w:RegisterEvent('UPDATE_ENCHANT_ITEM_MODE');w:RegisterEvent('SYSMSG');w:RegisterEvent('UI_ERROR_MESSAGE')
      w:SetHandler('OnEvent',function(_,e,...)
        if e=='SYSMSG' then messages[#messages+1]=...;return end
        if e=='UI_ERROR_MESSAGE' then errors[#errors+1]=...;return end
        updates[#updates+1]={n=select('#',...),...}
        assert(select('#',X2ItemEnchant:GetTargetItemInfo())<=1)
        -- A same-mode native call is safe during the synchronous UPDATE.
        X2ItemEnchant:SwitchItemEnchantGradeMode()
      end)
      X2ItemEnchant:EnterItemEnchantMode()
      assert(select('#',X2ItemEnchant:Execute())==0 and errors[1]=='invalid item slot index')
      assert(select('#',X2ItemEnchant:GetTargetItemInfo())==0 and select('#',X2ItemEnchant:GetRatioInfos())==0)
      for _,v in ipairs({'1',false,{}}) do assert(select('#',X2ItemEnchant:SetItemSlotFromBag(v))==0);assert(select('#',X2Item:IsEnchantable(v))==0) end
      assert(select('#',X2ItemEnchant:SetItemSlotFromBag())==0)
      assert(not X2Item:IsEnchantable(0) and not X2Item:IsEnchantable(-255))
      assert(#errors==3 and errors[2]=='invalid item slot index' and errors[3]=='invalid item slot index')
      assert(not X2Item:IsEnchantable(2147483648) and #errors==3) -- CVTT_MIN minus1 wraps positive
      assert(X2ItemEnchant:SetItemSlotFromBag(257)) -- wrapping low-byte key0
      assert(X2Bag:IsLocked(1) and X2Bag:IsLocked(40) and not X2Bag:IsLocked(2))
      assert(#updates==1 and updates[1].n==2 and updates[1][1]==false and updates[1][2]==false)
      local target=X2ItemEnchant:GetTargetItemInfo();assert(target.itemType==31432 and target.itemGrade==6)
      assert(target.total==1 and target.grade_enchant_cost==506607 and target.grade_enchant_currency==0)
      assert(not X2ItemEnchant:SetItemSlotFromBag(50) and #updates==1 and #messages==0)
      assert(X2ItemEnchant:SetItemSlotFromBag(2));assert(updates[2][1]==true and updates[2][2]==false)
      local source=X2ItemEnchant:GetEnchantItemInfo();assert(source.itemType==28299 and source.total==7)
      local ratio=X2ItemEnchant:GetRatioInfos();assert(ratio.success==true and ratio['break']==false and ratio.downgrade==true and ratio.success_var_ratio==nil)
      assert(X2ItemEnchant:SetItemSlotFromBag(3));ratio=X2ItemEnchant:GetRatioInfos();assert(ratio.success_var_ratio==50 and ratio.downgrade_var_ratio==0)
      local before=#updates;assert(select('#',X2ItemEnchant:Execute())==0);assert(#updates==before+1 and updates[#updates][1]==false and updates[#updates][2]==true)
      assert(select('#',X2ItemEnchant:ClearSupportItemSlot())==0 and updates[#updates][1]==true)
      assert(select('#',X2ItemEnchant:ClearEnchantItemSlot())==0 and updates[#updates][1]==false and X2ItemEnchant:GetTargetItemInfo()~=nil)
      X2Cursor:SetHeld({kind='bag',slot=2});assert(X2ItemEnchant:SetEnchantItemSlotFromPick());assert(X2Cursor:GetHeld()==nil)
      X2Cursor:SetHeld({kind='equip',slot=1});assert(X2ItemEnchant:SetSupportItemSlotFromPick()==false);assert(X2Cursor:GetHeld()==nil and messages[#messages]=='BAG_INVALID_ITEM')
      for _,held in ipairs({{kind='bank',slot=1},{kind='item',id=31432},{kind='action',what='item',slot=1}})do X2Cursor:SetHeld(held);assert(X2ItemEnchant:SetSupportItemSlotFromPick()==false and X2Cursor:GetHeld()==nil)end
      X2Cursor:SetHeld({kind='skill',id=1});assert(select('#',X2ItemEnchant:SetTargetItemSlotFromPick())==0 and X2Cursor:GetHeld()~=nil)
      X2ItemEnchant:ClearTargetItemSlot();assert(updates[#updates][1]==false and select('#',X2ItemEnchant:GetTargetItemInfo())==0)
      assert(not X2Bag:IsLocked(1) and X2Bag:IsLocked(40))
      local count=#updates;X2ItemEnchant:ClearTargetItemSlot();assert(#updates==count+1)
      assert(X2ItemEnchant:SetItemSlotFromBag(-255)) -- same low-byte key0; query above rejected signed index
      X2ItemEnchant:SwitchItemEnchantSocketingMode();assert(select('#',X2ItemEnchant:GetTargetItemInfo())==0)
    "#).exec()?;
    assert_eq!(shared.borrow().enchant.selection.slots, [None; 3]);
    let data = shared.borrow();
    assert!(
        matches!(&data.requests[..], [crate::hud::api::Request::ItemEffect { character_id: 47, request }]
        if request.skill_id() == 22522 && request.target().item.id == 9_007_199_254_740_993
        && request.source().slot == 1 && request.support().is_some_and(|item| item.slot == 2))
    );
    Ok(())
}
