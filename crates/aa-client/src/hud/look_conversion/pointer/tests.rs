use super::*;
use aa_net::items::ActionSlot;

fn setup() -> (Shared, UiRuntime) {
    let (state, rt) = crate::hud::look_conversion::test_runtime();
    rt.lua.load("DELEGATED=0;AAUI.UseSlot=function(...) DELEGATED=DELEGATED+1;return 'kept',select('#',...) end").exec().unwrap();
    install(&rt.lua, &state).unwrap();
    let mut data = state.borrow_mut();
    let mut inventory = Inventory::default();
    inventory.bag = (0..20)
        .map(|slot| enchant::records(&data).item(2, slot).cloned())
        .collect();
    let bar = ActionBar {
        slots: vec![
            ActionSlot::default(),
            ActionSlot {
                kind: ActionKind::ItemType,
                id: 29880,
            },
            ActionSlot {
                kind: ActionKind::ItemId,
                id: 9_007_199_254_740_997,
            },
            ActionSlot {
                kind: ActionKind::Spell,
                id: 23741,
            },
        ],
        page: 0,
    };
    let cache = &mut data.look_conversion.pointer;
    cache.update(
        47,
        &Skills {
            obj_id: 100,
            ..default()
        },
        &[],
        &inventory,
        &bar,
        &default(),
        [true; 4],
    );
    cache.interval = true;
    drop(data);
    (state, rt)
}

#[test]
fn gui_powder_bag_and_action_sources_admit_synchronously_without_duplicate_requests() {
    let (state, rt) = setup();
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    assert!(state.borrow().look_conversion.cancel_aim_pending);
    assert!(state.borrow().requests.is_empty());
    target::cancel_target(&rt.lua, &state);
    rt.lua.load("AAUI.UseSlot(ISLOT_ACTION,1,'LeftButton');assert(X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    target::cancel_target(&rt.lua, &state);
    rt.lua.load("AAUI.UseSlot(ISLOT_ACTION,2,'LeftButton');assert(X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    let expected = 9_007_199_254_740_997;
    let actual = state
        .borrow()
        .look_conversion
        .target
        .as_ref()
        .map(|_| enchant::records(&state.borrow()).item(2, 12).unwrap().id);
    assert_eq!(actual, Some(expected));
    target::cancel_target(&rt.lua, &state);
    rt.lua.load("local a,b=AAUI.UseSlot(ISLOT_ACTION,3,'LeftButton',123);assert(a=='kept' and b==4 and DELEGATED==1)").exec().unwrap();
    state.borrow_mut().look_conversion.pointer.interval = false;
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(not X2Item:IsInItemSkinizeMode() and DELEGATED==2)").exec().unwrap();
}

#[test]
fn direct_admission_uses_live_records_resources_and_releases_borrows_before_events() {
    let (state, rt) = setup();
    let s = state.clone();
    rt.lua
        .globals()
        .set(
            "check_entry",
            rt.lua
                .create_function(move |_, ()| {
                    let data = s.borrow_mut();
                    assert!(data.look_conversion.target.is_some());
                    assert!(data.look_conversion.fresh && data.look_conversion.cancel_aim_pending);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    rt.lua.load("local w=UIParent:CreateWidget('emptywidget','borrowProbe','UIParent');w:RegisterEvent('ENTER_ITEM_SKINIZE_MODE');w:SetHandler('OnEvent',check_entry)").exec().unwrap();
    // Failed and pending attempts are consumed, preserving the ordinary route.
    state.borrow_mut().snapshot.mp = 0;
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(not X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    assert_eq!(state.borrow().look_conversion.pointer.failures[0].0, 23741);
    state.borrow_mut().snapshot.mp = 1000;
    enchant::lock_bag(&mut state.borrow_mut(), 12, true);
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(not X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    enchant::lock_bag(&mut state.borrow_mut(), 12, false);
    state.borrow_mut().look_conversion.pointer.ready = false;
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(not X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    state.borrow_mut().look_conversion.pointer.ready = true;
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(X2Item:IsInItemSkinizeMode() and DELEGATED==0)").exec().unwrap();
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn release_completion_preserves_fresh_entry_then_retires_before_next_release_and_tick() {
    let (state, rt) = setup();
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');OBS={};local w=UIParent:CreateWidget('emptywidget','releaseObserver','UIParent');w:Show(true);w:RegisterEvent('LEAVE_ITEM_SKINIZE_MODE');w:SetHandler('OnEvent',function() OBS[#OBS+1]=X2Item:IsInItemSkinizeMode() end);w:SetHandler('OnUpdate',function() TICK_MODE=X2Item:IsInItemSkinizeMode() end)").exec().unwrap();
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        None,
        aa_ui::input::MouseButtons {
            right_release: true,
            ..default()
        },
        0.0,
    );
    assert!(
        state.borrow().look_conversion.target.is_some(),
        "entry survives its release"
    );
    state.borrow_mut().look_conversion.fresh = false;
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        None,
        aa_ui::input::MouseButtons {
            release: true,
            right_release: true,
            ..default()
        },
        0.0,
    );
    assert!(state.borrow().look_conversion.target.is_none());
    let data = state.borrow();
    // Right already had a claim from the prior delivered callback; a new
    // frame resets claims, independently of the fresh byte and held cursor.
    assert!(data.look_conversion.pointer.claimed[0]);
    drop(data);
    aa_ui::lua::tick(&rt.lua, 16.0);
    rt.lua.load("assert(#OBS==1,'event count '..#OBS);assert(OBS[1],'532 must observe old mode');assert(TICK_MODE==false,'tick mode '..tostring(TICK_MODE))").exec().unwrap();
    state.borrow_mut().look_conversion.pointer.claimed = [false; 2];
    rt.lua
        .load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton')")
        .exec()
        .unwrap();
    state.borrow_mut().look_conversion.fresh = false;
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        None,
        aa_ui::input::MouseButtons {
            release: true,
            right_release: true,
            ..default()
        },
        0.0,
    );
    assert_eq!(
        state.borrow().look_conversion.pointer.claimed,
        [true, false]
    );
}

#[test]
fn held_world_delivery_is_stack_bag_only_once_and_never_destroys_the_cursor() {
    let (state, rt) = setup();
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');X2Cursor:SetHeld({kind='bag',cursorInfo='Item_Id',slot=7,count=1})").exec().unwrap();
    assert!(world_drop(&rt.lua, &state, false).unwrap());
    assert!(state.borrow().look_conversion.target.is_some());
    assert!(state.borrow().look_conversion.task.is_none());
    state.borrow_mut().look_conversion.pointer.delivered_left = false;
    rt.lua
        .load("X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=7,count=1})")
        .exec()
        .unwrap();
    assert!(world_drop(&rt.lua, &state, false).unwrap());
    assert!(state.borrow().look_conversion.target.is_none());
    assert!(state.borrow().look_conversion.task.is_some());
    assert!(
        world_drop(&rt.lua, &state, false).unwrap(),
        "retained ownership consumes later fallback"
    );
    rt.lua
        .load("assert(X2Cursor:GetHeld()~=nil)")
        .exec()
        .unwrap();
    assert!(state.borrow().requests.is_empty());
}

#[test]
fn pointer_cache_copies_only_changed_resources_and_refreshes_native_clock_scalar() {
    let mut cache = PointerUse::default();
    let mut skills = Skills {
        obj_id: 100,
        ..default()
    };
    let inventory = Inventory::default();
    let bar = ActionBar::default();
    cache.update(47, &skills, &[], &inventory, &bar, &default(), [false; 4]);
    for i in 1..=100 {
        skills.native_last_frame_ms = i;
        cache.update(47, &skills, &[], &inventory, &bar, &default(), [false; 4]);
    }
    assert_eq!(cache.copies, [1; 4]);
    assert_eq!(cache.skills.native_last_frame_ms, 100);
    cache.update(
        47,
        &skills,
        &[],
        &inventory,
        &bar,
        &default(),
        [true, true, false, false],
    );
    assert_eq!(cache.copies, [2, 2, 1, 1]);
    cache.update(
        47,
        &skills,
        &[],
        &inventory,
        &bar,
        &default(),
        [false, false, true, false],
    );
    assert_eq!(cache.copies, [2, 2, 2, 1]);
    cache.update(48, &skills, &[], &inventory, &bar, &default(), [false; 4]);
    assert_eq!(cache.copies, [3, 3, 3, 2]);
}

#[test]
fn retained_gui_claim_protects_later_drop_fallback_and_reentrant_entry_cancel() {
    let (state, rt) = setup();
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=7,count=1});assert(HUD.look_target(7,'LeftButton'));assert(HUD.look_world_drop(false));assert(X2Cursor:GetHeld()~=nil)").exec().unwrap();
    assert!(!state.borrow().look_conversion.pointer.delivered_left);
    assert!(state.borrow().look_conversion.pointer.claimed[0]);
    assert!(state.borrow().look_conversion.task.is_some());
    state.borrow_mut().look_conversion.pointer.claimed = [false; 2];
    rt.lua.load("local w=UIParent:CreateWidget('emptywidget','reenterCancel','UIParent');w:RegisterEvent('ENTER_ITEM_SKINIZE_MODE');w:SetHandler('OnEvent',function() X2ItemLookConverter:EnterItemConvertMode() end);AAUI.UseSlot(ISLOT_BAG,13,'RightButton');assert(not X2Item:IsInItemSkinizeMode())").exec().unwrap();
    let data = state.borrow();
    assert!(
        !data.look_conversion.pointer.claimed[1],
        "synthetic UseSlot creates no release claim by itself"
    );
    assert!(data.look_conversion.cancel_aim_pending);
}

#[test]
fn held_world_delivery_uses_actual_gui_pick_after_layout_change() {
    let (state, rt) = setup();
    state.borrow_mut().look_conversion.pointer.cursor_present = true;
    rt.lua.load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton');X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=7,count=1});COVER=UIParent:CreateWidget('button','scaledCover','UIParent');COVER:SetExtent(100,100);COVER:AddAnchor('TOPLEFT','UIParent',1000,1000);COVER:Show(true)").exec().unwrap();
    assert!(aa_ui::input::pick(&rt.lua, 25., 25.).is_none());
    // UiFrame's OnScale can move the hit target after root preparation.
    rt.lua
        .load("COVER:RemoveAllAnchors();COVER:AddAnchor('TOPLEFT','UIParent',10,10)")
        .exec()
        .unwrap();
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        Some((25., 25.)),
        aa_ui::input::MouseButtons {
            release: true,
            ..default()
        },
        0.0,
    );
    assert!(aa_ui::lua::ui(&rt.lua).hover.is_some());
    assert!(state.borrow().look_conversion.target.is_some());
    assert!(state.borrow().look_conversion.task.is_none());
}

#[test]
fn skipped_pointer_frames_do_not_age_fresh_target() {
    let (state, rt) = setup();
    rt.lua
        .load("AAUI.UseSlot(ISLOT_BAG,13,'RightButton')")
        .exec()
        .unwrap();
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(Skills {
            obj_id: 100,
            ..default()
        })
        .init_resource::<Buffs>()
        .init_resource::<Inventory>()
        .init_resource::<ActionBar>()
        .init_resource::<InFlight>()
        .init_resource::<aa_ui::UiVirtualPointer>()
        .add_message::<crate::hud::UseFailed>()
        .add_systems(Update, (prepare, finish).chain());
    app.update();
    assert!(state.borrow().look_conversion.fresh, "no primary window");
    let first = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    let second = app
        .world_mut()
        .spawn((Window::default(), PrimaryWindow))
        .id();
    app.update();
    assert!(
        state.borrow().look_conversion.fresh,
        "ambiguous primary window"
    );
    app.world_mut().despawn(second);
    app.update();
    assert!(
        !state.borrow().look_conversion.fresh,
        "actual eligible UI frame ages entry"
    );
    state.borrow_mut().look_conversion.fresh = true;
    app.world_mut().remove_non_send::<UiRuntime>();
    app.update();
    assert!(state.borrow().look_conversion.fresh, "missing runtime");
    app.world_mut().despawn(first);
}

#[test]
fn original_bag_right_release_admits_before_completion_without_ordinary_use_queue()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::hud::{api, expansion};
    let (rt, state, path, _) = expansion::fixture::runtime(1280., 720.)?;
    super::super::tests::seed(&state);
    let mut inventory = Inventory::default();
    let mut items: Vec<_> = (0..20)
        .map(|slot| enchant::records(&state.borrow()).item(2, slot).cloned())
        .collect();
    items[11].as_mut().unwrap().flags = 0;
    for event in [
        aa_net::Event::InventoryInit {
            bag_slots: 20,
            bank_slots: 0,
        },
        aa_net::Event::InventoryChunk(aa_net::items::InvenChunk {
            container: 2,
            chunks: 2,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        inventory.apply(&event);
        enchant::dispatch(Some(&rt), &state, &event, None);
    }
    {
        let mut data = state.borrow_mut();
        data.snapshot.hp = 100;
        data.snapshot.mp = 1000;
        data.snapshot.level = 30;
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        data.panels = crate::hud::panels::refresh(
            &world,
            data.game.get(),
            0,
            100,
            data.start,
            &data.snapshot,
            Vec3::ZERO,
        )
        .0;
        data.panels.actions_known = true;
        data.look_conversion.pointer.update(
            47,
            &Skills {
                obj_id: 100,
                ..default()
            },
            &[],
            &inventory,
            &default(),
            &default(),
            [true; 4],
        );
        data.look_conversion.pointer.interval = true;
    }
    api::push_changes(&rt, &state);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    let object: Value = rt.lua.load("return newbag.slots.btns[13]").eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("missing original powder slot")?;
    let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(aa_ui::input::pick(&rt.lua, point.0, point.1), Some(id));
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        Some(point),
        aa_ui::input::MouseButtons {
            right_press: true,
            ..default()
        },
        0.,
    );
    aa_ui::input::update_buttons(
        &rt.lua,
        &mut mouse,
        Some(point),
        aa_ui::input::MouseButtons {
            right_release: true,
            ..default()
        },
        0.,
    );
    rt.lua.load("assert(X2Item:IsInItemSkinizeMode())").exec()?;
    assert!(
        state.borrow().look_conversion.fresh,
        "entry is visible to its own release checker"
    );
    assert!(
        state.borrow().requests.is_empty(),
        "no queued UseBag/UseItem duplicate"
    );
    state.borrow_mut().look_conversion.fresh = false;
    expansion::fixture::click(&rt, "newbag.slots.btns[12]")?;
    assert!(state.borrow().look_conversion.target.is_none());
    assert!(state.borrow().look_conversion.task.is_some());
    assert!(enchant::bag_pending(&state.borrow(), 11));
    assert!(state.borrow().requests.is_empty());
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
