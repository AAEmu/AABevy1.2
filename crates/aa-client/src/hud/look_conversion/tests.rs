use super::super::{
    api::{HudData, Request, Shared},
    data::{GameData, SharedGameData},
    enchant,
};
use super::{controller as c, *};
use aa_net::{
    Event,
    items::{InvenChunk, ItemTask, ItemTasks},
    model::{EquipDetail, Item, ItemDetail},
};
use std::{cell::RefCell, rc::Rc, sync::OnceLock};

fn game() -> SharedGameData {
    static GAME: OnceLock<SharedGameData> = OnceLock::new();
    GAME.get_or_init(|| {
        let data = SharedGameData::default();
        assert!(
            data.0
                .set(GameData::load(aa_anim::db::DEFAULT_DB).unwrap())
                .is_ok()
        );
        data
    })
    .clone()
}
fn equipment(template_id: u32, id: u64, grade: u8, flags: u8) -> Item {
    Item {
        template_id,
        id,
        grade,
        flags,
        count: 1,
        detail: ItemDetail::Equip(EquipDetail::default()),
        ..Default::default()
    }
}
pub(super) fn seed(shared: &Shared) {
    let spawn = super::super::expansion::fixture::spawn();
    let mut items = vec![None; 20];
    items[6] = Some(equipment(26060, 9_007_199_254_740_993, 4, 0));
    items[11] = Some(equipment(26118, 9_007_199_254_740_995, 1, 8));
    items[12] = Some(Item {
        template_id: 29880,
        id: 9_007_199_254_740_997,
        count: 10,
        ..Default::default()
    });
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 20,
            bank_slots: 0,
        },
        Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 2,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        enchant::dispatch(None, shared, &event, None);
    }
}
pub(super) fn state() -> Shared {
    let state = Rc::new(RefCell::new(HudData {
        game: game(),
        character_id: 47,
        ..Default::default()
    }));
    seed(&state);
    state
}
fn selected(state: &Shared) {
    c::select(&mut state.borrow_mut(), 0, 6).unwrap();
    c::select(&mut state.borrow_mut(), 1, 11).unwrap();
}
pub(super) fn runtime(state: &Shared) -> aa_ui::UiRuntime {
    let rt = aa_ui::UiRuntime::new("en_us", 1280., 720.).unwrap();
    rt.lua.load("HUD={};X2Item={};X2ItemEnchant={};X2ItemLookConverter={};X2Bag={IsLocked=function(_,s)return s==40 end};X2Locale={LocalizeUiText=function(_,_,key)return key end};ERROR_MSG=1").exec().unwrap();
    enchant::install(&rt.lua, state).unwrap();
    install(&rt.lua, state).unwrap();
    rt
}

#[test]
fn common_pending_is_one_set_and_explicit_unlock_is_not_undone_by_selected_keys() {
    let state = state();
    selected(&state);
    let rt = runtime(&state);
    rt.lua
        .load("X2Store={};X2Interaction={};HUD.bag={}")
        .exec()
        .unwrap();
    rt.lua.load(include_str!("../store.lua")).exec().unwrap();
    rt.lua.load("assert(X2Bag:IsLocked(7));X2Bag:LockSlot(7);X2Bag:LockSlot(7);X2Bag:UnlockSlot(7);assert(not X2Bag:IsLocked(7));assert(X2ItemLookConverter:GetConvertBaseItemInfo()~=nil)").exec().unwrap();
    // Inventory polling with the same observed pending snapshot cannot create
    // a second ownership/refcount for an explicitly unlocked physical key.
    let mut app = App::new();
    app.insert_non_send(super::super::api::HudBridge(state.clone()))
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<super::super::HudBuilt>()
        .add_message::<aa_net::NetEvent>()
        .add_systems(Update, enchant::track);
    app.update();
    rt.lua
        .load("assert(not X2Bag:IsLocked(7) and X2Bag:IsLocked(12))")
        .exec()
        .unwrap();
}
#[test]
fn requests_capture_full_ids_and_never_mutate_inventory_or_selection() {
    let state = state();
    selected(&state);
    let before = enchant::records(&state.borrow()).clone();
    let rt = runtime(&state);
    c::submit(&rt.lua, &state).unwrap();
    let data = state.borrow();
    let Request::ItemLook {
        character_id,
        request,
    } = &data.requests[0]
    else {
        panic!()
    };
    assert_eq!(*character_id, 47);
    assert_eq!(request.target().item.id, 9_007_199_254_740_993);
    assert_eq!(request.source().item.id, 9_007_199_254_740_995);
    assert_eq!(request.recheck_native(enchant::records(&data)), Ok(()));
    assert_eq!(enchant::records(&data), &before);
    assert_eq!(data.look_conversion.slots, [Some(6), Some(11)]);
}
#[test]
fn native_quote_counts_and_received_count_changes_gate_submission() {
    let state = state();
    selected(&state);
    let quote = c::quote(&state.borrow()).unwrap();
    assert_eq!(quote.items, [(29880, 9, 10)]);
    assert!(quote.money && quote.enough_items());
    enchant::dispatch(
        None,
        &state,
        &Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Count {
                container: 2,
                slot: 12,
                item_id: 9_007_199_254_740_997,
                delta: -2,
                template_id: 29880,
            }],
            ..Default::default()
        }),
        None,
    );
    let quote = c::quote(&state.borrow()).unwrap();
    assert_eq!(quote.items, [(29880, 9, 8)]);
    assert!(!quote.enough_items());
    assert_eq!(c::prepare(&state.borrow()).unwrap_err(), 138);
    assert_eq!(state.borrow().look_conversion.slots, [Some(6), Some(11)]);
}
#[test]
fn loss_never_quotes_or_submits_from_legacy_inventory_and_reseed_restores() {
    let state = state();
    selected(&state);
    enchant::dispatch(
        None,
        &state,
        &Event::GameDecodeFailed {
            message: "fixture gap".into(),
        },
        None,
    );
    assert!(c::quote(&state.borrow()).is_none());
    assert!(c::prepare(&state.borrow()).is_err());
    seed(&state);
    selected(&state);
    assert!(c::prepare(&state.borrow()).is_ok());
}
#[test]
fn lua_methods_preserve_zero_returns_raw_grade_numeric_wrapping_and_reentry() {
    let state = state();
    let rt = runtime(&state);
    rt.lua.load(r#"
      seen={};local w=UIParent:CreateWidget('emptywidget','converterObserver','UIParent')
      for _,e in ipairs({'ENTER_ITEM_LOOK_CONVERT_MODE','UPDATE_ITEM_LOOK_CONVERT_MODE','LEAVE_ITEM_LOOK_CONVERT_MODE'}) do w:RegisterEvent(e) end
      w:SetHandler('OnEvent',function(_,e,...)
        assert(select('#',...)==0)
        seen[#seen+1]={e,X2Item:IsInItemLookConvertMode(),X2ItemLookConverter:GetConvertBaseItemInfo()}
        if e=='LEAVE_ITEM_LOOK_CONVERT_MODE' then assert(not X2Bag:IsLocked(7));X2ItemLookConverter:LeaveItemConvertMode() end
      end)
      assert(select('#',X2ItemLookConverter:GetConvertBaseItemInfo())==0)
      for _,v in ipairs({'7',false,{}}) do assert(select('#',X2ItemLookConverter:SetConvertBaseItemSlotFromBag(v))==0) end
      assert(select('#',X2ItemLookConverter:EnterItemConvertMode())==0)
      X2ItemLookConverter:EnterItemConvertMode();assert(#seen==2)
      assert(X2ItemLookConverter:SetConvertBaseItemSlotFromBag(263))
      assert(seen[3][3].itemType==26060 and seen[3][3].itemGrade==4)
      assert(X2Bag:IsLocked(7) and X2Bag:IsLocked(40))
      assert(not X2Item:IsLookConvertableItem(7) and X2Item:IsLookConvertableItem(12))
      assert(X2ItemLookConverter:SetConvertLookItemSlotFromBag(12))
      local result=X2ItemLookConverter:GetConvertResultItemInfo()
      assert(result.itemType==26060 and result.lookType==26118 and result.itemGrade==4 and result.lookChanged and result.useAsSkin==false)
      local plan=X2ItemLookConverter:GetItemLookConvertInfo()
      assert(plan.money=='0' and plan.enoughMoney and plan.enoughItems and plan.items[1].has==10 and plan.items[1].need==9)
      X2ItemLookConverter:ClearConvertBaseItemSlot();X2ItemLookConverter:ClearConvertBaseItemSlot()
      assert(seen[5][3]==nil and seen[6][3]==nil)
      X2ItemLookConverter:LeaveItemConvertMode();assert(#seen==7 and not seen[7][2])
      X2ItemLookConverter:LeaveItemConvertMode();assert(#seen==7)
    "#).exec().unwrap();
}
#[test]
fn from_pick_checks_count_and_clears_before_success_update() {
    let state = state();
    let rt = runtime(&state);
    rt.lua.load(r#"
      X2Cursor:SetHeld({kind='bag',slot=7,id=26060,count=2})
      assert(select('#',X2ItemLookConverter:SetConvertBaseItemSlotFromPick())==0 and X2Cursor:GetHeld()~=nil)
      local w=UIParent:CreateWidget('emptywidget','pickObserver','UIParent');w:RegisterEvent('UPDATE_ITEM_LOOK_CONVERT_MODE')
      w:SetHandler('OnEvent',function()assert(X2Cursor:GetHeld()==nil)end)
      X2Cursor:SetHeld({kind='bag',slot=7,id=777,count=1})
      assert(X2ItemLookConverter:SetConvertBaseItemSlotFromPick() and X2Cursor:GetHeld()==nil)
      X2Cursor:SetHeld({kind='bag',slot=13,id=29880,count=10})
      assert(not X2ItemLookConverter:SetConvertLookItemSlotFromPick() and X2Cursor:GetHeld()==nil)
      X2Cursor:SetHeld({kind='skill',id=1})
      assert(select('#',X2ItemLookConverter:SetConvertBaseItemSlotFromPick())==0 and X2Cursor:GetHeld()~=nil)
    "#).exec().unwrap();
}

fn update_image(image: u32) -> ItemTask {
    let mut raw = vec![0; 128];
    raw[0] = 1;
    raw[1..5].copy_from_slice(&image.to_le_bytes());
    ItemTask::Update {
        container: 2,
        slot: 6,
        item_id: 9_007_199_254_740_993,
        detail: ItemDetail::Equip(EquipDetail {
            image_item_template_id: image,
            ..Default::default()
        }),
        detail_payload: Some(raw[1..56].to_vec()),
        raw_detail_block: Some(raw),
    }
}
#[test]
fn converted_callbacks_are_post_row_and_precede_later_rows_in_the_same_packet() {
    let state = state();
    selected(&state);
    let rt = runtime(&state);
    let s = state.clone();
    rt.lua
        .globals()
        .set(
            "currentLook",
            rt.lua
                .create_function(move |_, ()| {
                    Ok(c::item(&s.borrow(), 6).map(super::catalog::effective_look))
                })
                .unwrap(),
        )
        .unwrap();
    rt.lua.load(r#"
      seen={};local w=UIParent:CreateWidget('emptywidget','lookResultObserver','UIParent')
      w:RegisterEvent('ITEM_LOOK_CONVERTED');w:RegisterEvent('ITEM_LOOK_CONVERTED_EFFECT')
      w:SetHandler('OnEvent',function(_,e,payload)
        if e=='ITEM_LOOK_CONVERTED' then
          assert(type(payload)=='string');seen[#seen+1]={e,currentLook()}
          X2ItemLookConverter:ClearConvertBaseItemSlot()
        else seen[#seen+1]={e,payload.lookType,payload.itemGrade};assert(payload.itemType==26060) end
      end)
    "#).exec().unwrap();
    enchant::dispatch(
        Some(&rt),
        &state,
        &Event::ItemsChanged(ItemTasks {
            reason: 111,
            tasks: vec![update_image(26118), update_image(26060)],
            ..Default::default()
        }),
        None,
    );
    rt.lua.load("assert(#seen==4 and seen[1][2]==26118 and seen[2][2]==26118 and seen[2][3]==4 and seen[3][2]==26060 and seen[4][2]==26060)").exec().unwrap();
    assert_eq!(
        c::item(&state.borrow(), 6).unwrap().id,
        9_007_199_254_740_993
    );
    assert_eq!(state.borrow().look_conversion.slots, [None, Some(11)]);
}
#[test]
fn skinized_flags_and_type_only_info_follow_base_descriptor_not_image() {
    let state = state();
    let rt = runtime(&state);
    for (template, kind) in [(26060, 1), (23612, 9)] {
        for (flags, image, expected) in [
            (0, 0u32, false),
            (8, 0, true),
            (0, 26118, false),
            (8, 26118, true),
        ] {
            let mut item = equipment(template, 123, 4, flags);
            if kind == 9 {
                item.detail = ItemDetail::Other(9);
                item.detail_payload = Some(image.to_le_bytes().to_vec());
            } else {
                item.detail = ItemDetail::Equip(EquipDetail {
                    image_item_template_id: image,
                    ..Default::default()
                });
            }
            let data = state.borrow();
            let tip = c::tooltip(&rt.lua, &data, &item).unwrap().unwrap();
            assert_eq!(tip.raw_get::<bool>("useAsSkin").unwrap(), expected);
            assert_eq!(
                tip.raw_get::<u32>("lookType").unwrap(),
                if image == 0 { template } else { image }
            );
        }
    }
    let mut item = equipment(26060, 123, 4, 8);
    item.detail = ItemDetail::None;
    assert!(
        !c::tooltip(&rt.lua, &state.borrow(), &item)
            .unwrap()
            .unwrap()
            .raw_get::<bool>("useAsSkin")
            .unwrap(),
        "an unsupported detail keeps static type-only skin status"
    );
}
