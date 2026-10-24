use super::*;

#[test]
fn original_weapon_comparison_after_unequip_preserves_empty_text_contract() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path, mut inventory) = crate::hud::expansion::fixture::runtime(1280., 720.)?;
    inventory.bag[9] = Some(aa_net::model::Item {
        id: 16777797, template_id: 26118, count: 1, grade: 0, flags: 1,
        detail: aa_net::model::ItemDetail::Equip(Default::default()), ..Default::default()
    });
    inventory.equipment.slots[aa_net::model::slot::OFFHAND] = Some(aa_net::model::Item {
        id: 16777781, template_id: 6152, count: 1,
        detail: aa_net::model::ItemDetail::Equip(Default::default()), ..Default::default()
    });
    let mut world = World::new();
    world.insert_resource(inventory);
    {
        let mut data = state.borrow_mut();
        data.panels = crate::hud::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    crate::hud::api::push_changes(&rt, &state);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    rt.lua.load(r#"
      local tip=HUD.item_tip(26118,0,1,HUD.bag[10]);assert(tip.itemType==26118)
      for i=1,4 do ShowTooltip(tip,newbag.slots.btns[10],1,false) end
      assert(select('#',X2Equipment:GetEquippedItemTooltipInfo(ES_MAINHAND))==0)
      assert(X2Equipment:GetEquippedItemTooltipInfo(ES_OFFHAND).itemType==6152)
      assert(X2Equipment:GetEquippedItemTooltipText('player',ES_MAINHAND).itemType==0)
      local item,grade=X2Equipment:ItemIdentifier(ES_MAINHAND);assert(item==0 and grade==0)
    "#).exec()?;
    if path.exists() { std::fs::remove_file(path)?; }
    Ok(())
}

fn setup() -> (Shared, UiRuntime) {
    let state = super::super::tests::state();
    {
        let mut data = state.borrow_mut();
        data.snapshot.hp = 100;
        data.snapshot.mp = 1000;
        data.snapshot.level = 30;
    }
    let rt = super::super::tests::runtime(&state);
    (state, rt)
}
fn source(state: &Shared, skill: u32) -> Target {
    let data = state.borrow();
    Target {
        owner: 47,
        actor: 100,
        skill,
        source: CapturedBagItem {
            slot: 12,
            item: enchant::records(&data).item(2, 12).unwrap().clone(),
        },
    }
}
fn decorate(rt: &UiRuntime) {
    rt.lua.load(r#"
      X2DialogManager:SetHandler(DLG_TASK_DEFAULT,function(w,info) end)
      local request=X2DialogManager.RequestTaskDialog
      X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
        local wnd=request(self,task,info,owner); TASK_WINDOW=wnd;TASK_INFO=info;TASK_KIND=task;return wnd
      end
    "#).exec().unwrap();
}
#[test]
fn target_is_separate_from_held_and_532_observes_old_mode_before_converter_enter() {
    let (state, rt) = setup();
    rt.lua.load(r#"
      states={};local w=UIParent:CreateWidget('emptywidget','skinObserver','UIParent')
      for _,e in ipairs({'ENTER_ITEM_SKINIZE_MODE','LEAVE_ITEM_SKINIZE_MODE','ENTER_ITEM_LOOK_CONVERT_MODE'})do w:RegisterEvent(e)end
      w:SetHandler('OnEvent',function(_,e)states[#states+1]={e,X2Item:IsInItemSkinizeMode()}end)
    "#).exec().unwrap();
    enter(&rt.lua, &state, source(&state, 23741));
    rt.lua.load("assert(X2Cursor:GetCursorInfo()==nil and X2Item:IsInItemSkinizeMode());X2Cursor:ClearCursor();assert(X2Item:IsInItemSkinizeMode())").exec().unwrap();
    enter(&rt.lua, &state, source(&state, 23741));
    rt.lua.load("X2ItemLookConverter:EnterItemConvertMode();assert(not X2Item:IsInItemSkinizeMode());assert(#states==5 and states[1][2] and states[2][2] and states[3][2] and states[4][2] and not states[5][2])").exec().unwrap();
    assert!(state.borrow().requests.is_empty());
}
#[test]
fn task_cancel_unlocks_before_callback_and_preserves_new_target_mode() {
    let (state, rt) = setup();
    decorate(&rt);
    enter(&rt.lua, &state, source(&state, 23741));
    choose(&rt.lua, &state, 11).unwrap();
    assert!(state.borrow().look_conversion.target.is_none());
    rt.lua.load("assert(TASK_KIND==19 and TASK_INFO.itemInfo.itemType==26118 and TASK_INFO.scrollType==29880 and TASK_INFO.scrollGrade==0 and X2Bag:IsLocked(12) and not X2Bag:IsLocked(13))").exec().unwrap();
    enter(&rt.lua, &state, source(&state, 23741));
    rt.lua.load("X2Cursor:SetHeld({kind='bag',slot=7,count=1});TASK_WINDOW:CancelProc();assert(not X2Bag:IsLocked(12) and X2Item:IsInItemSkinizeMode() and X2Cursor:GetHeld()==nil)").exec().unwrap();
    assert!(state.borrow().requests.is_empty());
}
#[test]
fn task_replacement_discards_old_callback_and_ok_only_queues_current_full_captures() {
    let (state, rt) = setup();
    decorate(&rt);
    enter(&rt.lua, &state, source(&state, 23741));
    choose(&rt.lua, &state, 11).unwrap();
    rt.lua.load("OLD_TASK=TASK_WINDOW").exec().unwrap();
    enter(&rt.lua, &state, source(&state, 23741));
    choose(&rt.lua, &state, 6).unwrap();
    let token = state.borrow().look_conversion.task.as_ref().unwrap().token;
    rt.lua.load("assert(not X2Bag:IsLocked(12) and X2Bag:IsLocked(7));OLD_TASK:OkProc();assert(X2Bag:IsLocked(7))").exec().unwrap();
    assert_eq!(
        state.borrow().look_conversion.task.as_ref().unwrap().token,
        token
    );
    assert!(state.borrow().requests.is_empty());
    rt.lua
        .load("TASK_WINDOW:OkProc();assert(not X2Bag:IsLocked(7))")
        .exec()
        .unwrap();
    let data = state.borrow();
    let Request::ItemLook { request, .. } = &data.requests[0] else {
        panic!()
    };
    assert_eq!(request.operation(), ItemLookOperation::Skinize);
    assert_eq!(request.target().item.id, 9_007_199_254_740_993);
    assert_eq!(request.source().item.id, 9_007_199_254_740_997);
    assert_eq!(enchant::records(&data).item(2, 6).unwrap().flags, 0);
}
#[test]
fn missing_visual_decorator_keeps_task_and_lock_until_the_visual_can_be_built() {
    let (state, rt) = setup();
    enter(&rt.lua, &state, source(&state, 23741));
    choose(&rt.lua, &state, 11).unwrap();
    assert!(
        state
            .borrow()
            .look_conversion
            .task
            .as_ref()
            .is_some_and(|task| !task.shown)
    );
    assert!(enchant::bag_pending(&state.borrow(), 11));
    decorate(&rt);
    show_pending(&rt.lua, &state).unwrap();
    assert!(state.borrow().look_conversion.task.as_ref().unwrap().shown);
    rt.lua.load("TASK_WINDOW:CancelProc()").exec().unwrap();
    assert!(!enchant::bag_pending(&state.borrow(), 11));
}
#[test]
fn admission_needs_actual_rules_actor_and_capture_then_honors_ordinary_resources() {
    let (state, _) = setup();
    let target = source(&state, 23741);
    let skills = Skills {
        obj_id: 100,
        ..Default::default()
    };
    let inventory = Inventory::default();
    let flight = InFlight::default();
    let data = state.borrow();
    let env = Env::of(&data, Some(&skills), &[], Some(&inventory));
    assert!(matches!(
        admission(&data, &env, &flight, &target.source, target.skill),
        Admission::Ready
    ));
    let missing = Env::of(&data, None, &[], Some(&inventory));
    assert!(matches!(
        admission(&data, &missing, &flight, &target.source, target.skill),
        Admission::Unsupported
    ));
    drop(data);
    state.borrow_mut().snapshot.mp = 0;
    let data = state.borrow();
    let env = Env::of(&data, Some(&skills), &[], Some(&inventory));
    assert!(matches!(
        admission(&data, &env, &flight, &target.source, target.skill),
        Admission::Failed(_)
    ));
}

#[test]
fn cleanup_resolves_moved_source_and_queues_inline_without_task_or_item_mutation() {
    use aa_net::{
        Event,
        items::{ItemTask, ItemTasks},
    };
    let (state, rt) = setup();
    // A genuine source receipt, then its physical move while targeting.
    let id = 9_007_199_254_740_997;
    enchant::dispatch(
        Some(&rt),
        &state,
        &Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Add {
                action: 5,
                container: 2,
                slot: 12,
                item: aa_net::model::Item {
                    template_id: 29704,
                    id,
                    count: 1,
                    detail_payload: Some(Vec::new()),
                    ..Default::default()
                },
                raw_detail_block: Some(vec![0; 128]),
            }],
            ..Default::default()
        }),
        None,
    );
    enter(&rt.lua, &state, source(&state, 23457));
    enchant::dispatch(
        Some(&rt),
        &state,
        &Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Move {
                from_container: 2,
                from_slot: 12,
                from_item_id: id,
                to_container: 2,
                to_slot: 13,
                to_item_id: 0,
            }],
            ..Default::default()
        }),
        None,
    );
    let before = enchant::records(&state.borrow()).clone();
    assert!(choose(&rt.lua, &state, 6).unwrap());
    let data = state.borrow();
    assert!(data.look_conversion.target.is_none() && data.look_conversion.task.is_none());
    let Request::ItemLook { request, .. } = &data.requests[0] else {
        panic!()
    };
    assert_eq!(request.operation(), ItemLookOperation::Cleanup);
    assert_eq!((request.source().slot, request.source().item.id), (13, id));
    assert_eq!(request.target().item.id, 9_007_199_254_740_993);
    assert_eq!(enchant::records(&data), &before);
    assert!(!enchant::bag_pending(&data, 6));
}

#[test]
fn pending_target_keeps_cursor_but_changed_confirmation_capture_only_unlocks() {
    use aa_net::{
        Event,
        items::{ItemTask, ItemTasks},
    };
    let (state, rt) = setup();
    decorate(&rt);
    enter(&rt.lua, &state, source(&state, 23741));
    enchant::lock_bag(&mut state.borrow_mut(), 6, true);
    assert!(choose(&rt.lua, &state, 6).unwrap());
    assert!(state.borrow().look_conversion.target.is_some());
    assert!(state.borrow().look_conversion.task.is_none());
    enchant::lock_bag(&mut state.borrow_mut(), 6, false);
    choose(&rt.lua, &state, 6).unwrap();
    enchant::dispatch(
        Some(&rt),
        &state,
        &Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Grade {
                container: 2,
                slot: 6,
                item_id: 9_007_199_254_740_993,
                grade: 5,
            }],
            ..Default::default()
        }),
        None,
    );
    rt.lua
        .load("TASK_WINDOW:OkProc();assert(not X2Bag:IsLocked(7))")
        .exec()
        .unwrap();
    assert!(state.borrow().requests.is_empty());
    assert!(state.borrow().look_conversion.task.is_none());
    assert_eq!(
        enchant::records(&state.borrow()).item(2, 6).unwrap().grade,
        5
    );
}

#[test]
fn source_entry_uses_live_admission_without_starting_a_cast() {
    let (state, rt) = setup();
    let skills = Skills {
        obj_id: 100,
        ..Default::default()
    };
    let buffs = Buffs::default();
    let inventory = Inventory::default();
    let flight = InFlight::default();
    let message = ItemLookSource {
        item_id: 9_007_199_254_740_997,
        template: 29880,
        skill: 23741,
    };
    assert!(
        source_use(
            &rt,
            &state,
            message,
            (&skills, &buffs, &inventory, &flight),
            None
        )
        .is_none()
    );
    assert!(state.borrow().look_conversion.target.is_some());
    assert!(state.borrow().requests.is_empty());
    assert!(!flight.pending(&skills, std::time::Instant::now()));
    cancel_target(&rt.lua, &state);
    state.borrow_mut().snapshot.mp = 0;
    assert!(
        source_use(
            &rt,
            &state,
            message,
            (&skills, &buffs, &inventory, &flight),
            None
        )
        .is_some()
    );
    assert!(state.borrow().look_conversion.target.is_none());
}
