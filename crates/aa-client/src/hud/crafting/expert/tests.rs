use super::*;
use crate::hud::{api::HudData, data::GameData};
use aa_net::ActabilityValue;

fn definition(name: &str, limit: i32, capacity: i32) -> super::super::data::Expert {
    super::super::data::Expert {
        name: name.into(),
        up_limit: limit,
        expert_limit: capacity,
        advantage: 5,
        cast_advantage: 2.5,
        color_argb: 0xFF12ABCD,
        gauge_color: 0xFFEEDDCC,
        visible: true,
        up_currency: 0,
        up_price: 0,
        down_currency: 0,
        down_price: 0,
    }
}

fn data() -> HudData {
    let mut game = GameData::default();
    game.crafting.experts.insert(1, definition("Novice", 10000, 0));
    game.crafting.experts.insert(3, definition("Veteran", 40000, 2));
    game.crafting.experts.insert(7, definition("Master", 90000, 1));
    game.crafting.grades = vec![1, 3, 7];
    game.crafting.raw_limits = vec![10000, 20000, 40000, 90000];
    for id in [1, 2, 19, 20, 21, 30, 33] {
        game.crafting.groups.insert(
            id,
            super::super::data::Group {
                name: format!("Group{id}"),
                visible: id != 1,
                skill_page_visible: id != 33,
                ..default()
            },
        );
    }
    game.crafting.expansions.insert(
        1,
        super::super::data::Expansion {
            life_points: 100,
            item: 91,
            count: 2,
        },
    );
    game.crafting.expansions.insert(
        2,
        super::super::data::Expansion {
            life_points: 100,
            item: 91,
            count: 2,
        },
    );
    let mut data = HudData { character_id: 47, ..default() };
    data.game.0.set(game).ok().unwrap();
    data.crafting.actabilities.expanded_expert = Some(0);
    data.crafting.actabilities.apply(&Event::Actability(ActabilityEvent::Snapshot {
        last: true,
        entries: vec![
            ActabilityValue { id: 1, point: 10000, step: 0 },
            ActabilityValue { id: 2, point: 50000, step: 1 },
            ActabilityValue { id: 19, point: 90000, step: 2 },
            ActabilityValue { id: 20, point: 50000, step: 1 },
            ActabilityValue { id: 21, point: 90000, step: 2 },
            ActabilityValue { id: 30, point: 90000, step: 2 },
            ActabilityValue { id: 33, point: 90000, step: 2 },
        ],
    }));
    data
}

fn activation_app(rt: aa_ui::UiRuntime, bridge: Shared) -> App {
    let mut app = App::new();
    app.add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(bridge))
        .add_systems(Update, update);
    app
}

#[test]
fn activation_preserves_stale_refresh_and_real_identical_snapshot_notifications() -> Result<(), Box<dyn std::error::Error>> {
    for stale in 0..=8 {
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
        crate::hud::api::install(&mut rt, &bridge)?;
        rt.lua.load("expertRefreshes=0;expandUpdates=0;actabilityTab={expandBtn={Update=function()expandUpdates=expandUpdates+1 end}};UIParent:SetEventHandler('ACTABILITY_REFRESH_ALL',function(...)assert(select('#',...)==0);expertRefreshes=expertRefreshes+1 end)").exec()?;
        if stale != 8 {
            rt.lua.load("assert(#X2Ability:GetAllMyActabilityInfos()>0)").exec()?;
        }
        match stale {
            1 => { bridge.borrow_mut().crafting.actabilities.apply(&Event::LaborChanged { amount: 0, action: 1, point: 17, step: 0 }); }
            2 => bridge.borrow_mut().tips.attrs.modifiers.push((0, false, 25)),
            3 => bridge.borrow_mut().character_id = 48,
            4..=6 => {
                bridge.borrow_mut().tips.attrs.modifiers.push((0, false, 25));
                if stale == 4 {
                    rt.lua.load("assert(X2Ability:GetMyActabilityInfo(1).modifyPoint==25)").exec()?;
                } else if stale == 6 {
                    rt.lua.load("local found=false;for _,v in ipairs(X2Ability:GetAllMyActabilityInfos()) do if v.type==1 then assert(v.modifyPoint==25);found=true end end;assert(found)").exec()?;
                }
                bridge.borrow_mut().tips.attrs.modifiers.clear();
            }
            7 => bridge.borrow_mut().crafting.actabilities = default(),
            _ => {}
        }
        let mut app = activation_app(rt, bridge.clone());
        app.update();
        let notified = matches!(stale, 1 | 2 | 3 | 4 | 6 | 8);
        assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("expertRefreshes")?, u32::from(notified), "case {stale}");
        assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("expandUpdates")?, u32::from(!notified));
        assert!(bridge.borrow().crafting.expert.constructed.is_none());
        if stale == 0 {
            let entries = bridge.borrow().crafting.actabilities.entries.iter().map(|a| ActabilityValue { id: a.id, point: a.point.unwrap(), step: a.step }).collect();
            app.world_mut().resource_mut::<Messages<NetEvent>>().write(NetEvent(Event::Actability(ActabilityEvent::Snapshot { last: true, entries })));
            app.update();
            assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("expertRefreshes")?, 1, "a real received snapshot must still notify even when its values match");
        }
    }
    Ok(())
}

#[test]
fn first_activation_notifies_real_receipts_without_reapplying_copied_deltas() -> Result<(), Box<dyn std::error::Error>> {
    for changed in [false, true] {
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
        crate::hud::api::install(&mut rt, &bridge)?;
        rt.lua.load("expertRefreshes=0;UIParent:SetEventHandler('ACTABILITY_REFRESH_ALL',function(...)assert(select('#',...)==0);expertRefreshes=expertRefreshes+1 end);assert(#X2Ability:GetAllMyActabilityInfos()>0)").exec()?;
        if changed {
            bridge.borrow_mut().crafting.actabilities.apply(&Event::LaborChanged { amount: 0, action: 1, point: 17, step: 0 });
        }
        let entries = bridge.borrow().crafting.actabilities.entries.iter().map(|a| ActabilityValue { id: a.id, point: a.point.unwrap(), step: a.step }).collect();
        let snapshot = Event::Actability(ActabilityEvent::Snapshot { last: true, entries });
        let mut app = activation_app(rt, bridge);
        app.world_mut().resource_mut::<Messages<NetEvent>>().write(NetEvent(snapshot));
        app.update();
        assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("expertRefreshes")?, 1, "first-frame real receipt must notify exactly once, changed={changed}");
    }

    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua.load("pointReceipts=0;UIParent:SetEventHandler('ACTABILITY_EXPERT_CHANGED',function(id,name,delta,total)assert(id==1 and delta==17 and total==10017);pointReceipts=pointReceipts+1 end);assert(#X2Ability:GetAllMyActabilityInfos()>0)").exec()?;
    let delta = Event::LaborChanged { amount: 0, action: 1, point: 17, step: 0 };
    // sync_hud has copied the authoritative resource before the first update.
    bridge.borrow_mut().crafting.actabilities.apply(&delta);
    let mut app = activation_app(rt, bridge.clone());
    app.world_mut().resource_mut::<Messages<NetEvent>>().write(NetEvent(delta));
    app.update();
    assert_eq!(bridge.borrow().crafting.actabilities.get(1).unwrap().point, Some(10017));
    assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("pointReceipts")?, 1);

    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua.load("downgradeReceipts=0;UIParent:SetEventHandler('ACTABILITY_EXPERT_GRADE_CHANGED',function(id,upgrade)local info=X2Ability:GetMyActabilityInfo(id);assert(id==2 and not upgrade and info.point==10000 and info.grade==0);downgradeReceipts=downgradeReceipts+1 end);assert(#X2Ability:GetAllMyActabilityInfos()>0)").exec()?;
    let downgrade = Event::Actability(ActabilityEvent::TierChanged { is_upgrade: false, id: 2, step: 0 });
    bridge.borrow_mut().crafting.actabilities.apply(&downgrade);
    assert_eq!(bridge.borrow().crafting.actabilities.get(2).unwrap().point, None);
    let mut app = activation_app(rt, bridge.clone());
    app.world_mut().resource_mut::<Messages<NetEvent>>().write(NetEvent(downgrade));
    app.update();
    assert_eq!(bridge.borrow().crafting.actabilities.get(2).unwrap().point, Some(10000));
    assert_eq!(app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.globals().get::<u32>("downgradeReceipts")?, 1);
    Ok(())
}

#[test]
fn original_language_constructor_already_contains_the_received_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
    {
        let mut d = bridge.borrow_mut();
        d.character_id = 47;
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Language construction fixture".into();
        d.snapshot.level = 30;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.snapshot.mp = 100;
        d.snapshot.max_mp = 100;
        d.feature_limits = (Some(55), Some(50));
        d.panels.actions_known = true;
        d.panels.bag_slots = 50;
        d.game.0.set(GameData::load(aa_anim::db::DEFAULT_DB)?).ok().unwrap();
        d.crafting.actabilities.apply(&Event::Actability(ActabilityEvent::Snapshot { last: true, entries: vec![ActabilityValue { id: 21, point: 1337, step: 0 }] }));
        let cap = *d.game.get().unwrap().crafting.expansions.keys().max().ok_or("missing expert expansion catalog")?;
        d.crafting.actabilities.expanded_expert = Some(u8::try_from(cap)?);
    }
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua.load("assert(X2Ability:GetMyActabilityInfo(21).point==1337,'received input missing before construction')").exec()?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    let id = u32::try_from(aa_ui::lua::ui(&rt.lua).tree.objects.iter().enumerate().find(|(_, o)| o.alive && o.name == "language_emptyWidget").ok_or("original language page missing")?.0)?;
    rt.lua.globals().set("LANGUAGE_CONSTRUCTION", aa_ui::lua::object_table(&rt.lua, id)?.ok_or("original language table missing")?)?;
    rt.lua.load("assert(X2Ability:GetMyActabilityInfo(21).point==1337,'received input changed during construction');local found=false;for _,row in ipairs(LANGUAGE_CONSTRUCTION.actabilityItem) do if row.itemInfo.type==21 then assert(row.itemInfo.point==1337 and row.actabilityPoint:GetText()=='1,337','constructor point='..tostring(row.itemInfo.point)..' text='..tostring(row.actabilityPoint:GetText()));found=true end end;assert(found,'language21 missing, row count='..tostring(#LANGUAGE_CONSTRUCTION.actabilityItem));assert(not X2Ability:CanExpandExpert());expertRefreshes=0;UIParent:SetEventHandler('ACTABILITY_REFRESH_ALL',function()expertRefreshes=expertRefreshes+1 end)").exec()?;
    let mut app = activation_app(rt, bridge.clone());
    app.update();
    app.world().get_non_send::<aa_ui::UiRuntime>().unwrap().lua.load("assert(expertRefreshes==0);assert(not actabilityTab.expandBtn:IsEnabled(),'capped expansion button must be initialized');for _,row in ipairs(LANGUAGE_CONSTRUCTION.actabilityItem) do if row.itemInfo.type==21 then assert(row.itemInfo.point==1337 and row.actabilityPoint:GetText()=='1,337') end end").exec()?;
    assert!(bridge.borrow().crafting.expert.constructed.is_none());
    if path.exists() { std::fs::remove_file(path)?; }
    Ok(())
}

#[test]
fn upgrade_capacity_counts_higher_grades_and_native_special_exclusions() {
    let mut d = data();
    assert_eq!(count(&d, 1, false), 2);
    assert_eq!(count(&d, 2, false), 0);
    assert!(!upgrade(&d, 1));
    d.crafting.actabilities.expanded_expert = Some(1);
    assert!(upgrade(&d, 1));
    assert!(!downgrade(&d, 1));
    assert!(downgrade(&d, 2));
    assert!(!downgrade(&d, 999));
    d.crafting.actabilities.entries.iter_mut().find(|a| a.id == 1).unwrap().point = Some(9999);
    assert!(!upgrade(&d, 1));
    assert!(grade(d.game.get().map(|g| &g.crafting).unwrap(), 255).is_none());
    d.crafting.actabilities.entries.iter_mut().find(|a| a.id == 1).unwrap().point = Some(10000);
    for entry in d.crafting.actabilities.entries.iter_mut().filter(|a| matches!(a.id, 2 | 20)) {
        entry.step = 2;
    }
    d.crafting.actabilities.expanded_expert = Some(0);
    assert_eq!(count(&d, 1, false), 0);
    assert!(!upgrade(&d, 1), "capacity counts higher grades too");
    let game = std::sync::Arc::get_mut(&mut d.game.0).unwrap().get_mut().unwrap();
    game.crafting.experts.get_mut(&3).unwrap().expert_limit = 0;
    d.crafting.actabilities.expanded_expert = None;
    assert!(upgrade(&d, 1), "base capacity zero is unlimited without expansion state");
}

#[test]
fn original_getters_use_visible_indices_price_strings_and_consecutive_expansion_rows() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua.load(r#"
        assert(X2Ability:GetMaxGrade()==2)
        local grade=X2Ability:GetGradeInfo(1)
        assert(grade.grade==1 and grade.name=='Veteran' and grade.upLimit==40000 and grade.expertLimit==2)
        assert(grade.colorString=='FF12ABCD' and grade.gaugeColorString=='FFEEDDCC')
        assert(type(grade.upPrice)=='string' and grade.upPrice=='0' and type(grade.upCurrency)=='number')
        assert(X2Ability:GetMinActabilityPoint(2)==10000 and X2Ability:GetMaxActabilityPoint(2)==40000)
        assert(X2Ability:IsLanguageActability(21) and X2Ability:IsLanguageActability(30) and not X2Ability:IsLanguageActability(20))
        local all=X2Ability:GetAllMyActabilityInfos();assert(#all==6)
        local found=false;for _,v in ipairs(all) do if v.type==1 then found=true end;assert(v.type~=33) end;assert(found)
        local groups=X2Craft:EnumActabilityGroupTypes();for _,id in ipairs(groups) do assert(id~=20) end
        local info=X2Ability:GetExpandExpertInfo();assert(info.requireLP==100 and info.requireItemType==91 and info.requireItemCount==2 and info.remainExpandCnt==1)
        assert(X2Ability:CanExpandExpert())
        assert(X2Ability:GetGradeInfo(99).upLimit==10000)
        assert(X2Ability:GetGradeInfo(-1).upLimit==10000)
    "#).exec()?;
    bridge.borrow_mut().crafting.actabilities.expanded_expert = Some(1);
    rt.lua
        .load("assert(X2Ability:GetGradeInfo(0).expertLimit==1);assert(X2Ability:GetExpandExpertInfo().remainExpandCnt==0)")
        .exec()?;
    bridge.borrow_mut().crafting.actabilities.expanded_expert = Some(2);
    rt.lua
        .load("assert(not X2Ability:CanExpandExpert() and X2Ability:GetExpandExpertInfo()==nil)")
        .exec()?;
    Ok(())
}

#[test]
fn received_grade_events_expose_updated_state_and_lane_downgrade_points_before_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua
        .load(
            r#"
        seen={}
        UIParent:SetEventHandler('ACTABILITY_EXPERT_GRADE_CHANGED',function(id,up,name,grade)
            assert(type(id)=='number' and type(up)=='boolean' and name=='Group1')
            local info=X2Ability:GetMyActabilityInfo(id)
            table.insert(seen,{info.grade,info.point,grade})
        end)
        UIParent:SetEventHandler('ACTABILITY_EXPERT_EXPANDED',function(...) assert(select('#',...)==0); expanded=X2Ability:GetGradeInfo(0).expertLimit end)
    "#,
        )
        .exec()?;
    for (up, step) in [(true, 2), (false, 1)] {
        dispatch(&rt, &bridge, &Event::Actability(ActabilityEvent::TierChanged { id: 1, is_upgrade: up, step }));
    }
    dispatch(&rt, &bridge, &Event::Actability(ActabilityEvent::Expanded { count: 2 }));
    rt.lua.load("assert(#seen==2 and seen[1][1]==2 and seen[1][2]==10000 and seen[1][3]=='Master');assert(seen[2][1]==1 and seen[2][2]==20000 and seen[2][3]=='Veteran');assert(expanded==2)").exec()?;
    Ok(())
}

#[test]
fn point_changes_have_four_native_arguments_and_incomplete_snapshots_do_not_refresh_ui() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua
        .load(
            r#"
        points={};refreshes=0
        UIParent:SetEventHandler('ACTABILITY_EXPERT_CHANGED',function(id,name,delta,total)
            assert(id==1 and name=='Group1' and X2Ability:GetMyActabilityInfo(id).point==total)
            table.insert(points,{delta,total})
        end)
        UIParent:SetEventHandler('ACTABILITY_REFRESH_ALL',function(...)assert(select('#',...)==0);refreshes=refreshes+1 end)
    "#,
        )
        .exec()?;
    for (action, point) in [(1, 150), (1, -50), (1, 0), (33, 10), (0, 0)] {
        dispatch(
            &rt,
            &bridge,
            &Event::LaborChanged {
                amount: 0,
                action,
                point,
                step: 0,
            },
        );
    }
    rt.lua
        .load("assert(#points==2 and points[1][1]==150 and points[1][2]==10150 and points[2][1]==-50 and points[2][2]==10100)")
        .exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Actability(ActabilityEvent::Snapshot {
            last: false,
            entries: vec![aa_net::ActabilityValue { id: 1, point: 30000, step: 1 }],
        }),
    );
    rt.lua.load("assert(refreshes==0)").exec()?;
    dispatch(&rt, &bridge, &Event::Actability(ActabilityEvent::Snapshot { last: true, entries: vec![] }));
    rt.lua.load("assert(refreshes==1 and X2Ability:GetMyActabilityInfo(1).point==30000)").exec()?;
    Ok(())
}

#[test]
fn actions_recheck_points_certificates_and_preserve_received_auto_aa_setting() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data()));
    {
        let mut d = bridge.borrow_mut();
        d.crafting.actabilities.expanded_expert = Some(1);
        let mut inventory = aa_net::state::Inventory::default();
        inventory.auto_use_aa_point_raw = 1;
        d.crafting.inventory = Some(inventory);
    }
    crate::hud::api::install(&mut rt, &bridge)?;
    rt.lua
        .load("assert(select('#',X2Ability:UpgradeExpert(1))==0);X2Ability:ExpandExpert()")
        .exec()?;
    let mut d = bridge.borrow_mut();
    let commands = std::mem::take(&mut d.crafting.expert.requests);
    assert_eq!(
        commands[0],
        (
            47,
            Command::Upgrade {
                id: 1,
                auto_use_aa_point: true
            }
        )
    );
    assert!(can_send(&d, commands[0].1));
    assert!(!can_send(&d, Command::Expand));
    d.snapshot.vocation = 100;
    let inventory = d.crafting.inventory.as_mut().unwrap();
    inventory.bag_slots = 2;
    inventory.bag = vec![Some(aa_net::model::Item {
        template_id: 91,
        id: 1,
        count: 2,
        ..default()
    })];
    assert!(can_send(&d, Command::Expand));
    d.crafting.inventory.as_mut().unwrap().bag[0].as_mut().unwrap().flags = 4;
    assert!(can_send(&d, Command::Expand), "native certificate count has no secure-item flag exclusion");
    let inventory = d.crafting.inventory.as_mut().unwrap();
    inventory.mark_item_slot_pending(2, 0);
    inventory.bag_slots = 1;
    inventory.bag.push(None);
    inventory.bag.push(Some(aa_net::model::Item {
        template_id: 91,
        id: 2,
        count: 100,
        ..default()
    }));
    inventory.bank = vec![Some(aa_net::model::Item {
        template_id: 91,
        id: 3,
        count: 100,
        ..default()
    })];
    assert!(
        !can_send(&d, Command::Expand),
        "pending bag slots, bank and slots outside the received bag are excluded"
    );
    d.crafting.actabilities.entries.iter_mut().find(|a| a.id == 1).unwrap().point = Some(0);
    assert!(!can_send(&d, commands[0].1));
    drop(d);
    rt.lua
        .load("assert(X2Bag:CountBagItemByItemType(91)==2 and X2Bag:CountBagItemByItemType(0)==0)")
        .exec()?;
    Ok(())
}

#[test]
#[ignore = "loads original world-stage bytecode and exercises physical confirmation controls"]
fn original_proficiency_upgrade_cancel_confirm_and_received_downgrade() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = fixture::runtime(1920., 1080.)?;
    let click = crate::hud::expansion::fixture::click;
    click(&rt, "EXPERT_ROW.upgradeButton")?;
    rt.lua.load("assert(EXPERT_DIALOG and EXPERT_DIALOG:IsVisible())").exec()?;
    assert!(bridge.borrow().crafting.expert.requests.is_empty());
    click(&rt, "EXPERT_DIALOG.btnCancel")?;
    assert!(bridge.borrow().crafting.expert.requests.is_empty());
    click(&rt, "EXPERT_ROW.upgradeButton")?;
    click(&rt, "EXPERT_DIALOG.btnOk")?;
    assert_eq!(bridge.borrow().crafting.actabilities.get(1).unwrap().step, 0);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(bridge.clone()))
        .add_systems(Update, update);
    let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|m| m.0).collect::<Vec<_>>();
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [aa_net::Command::Actability(Command::Upgrade {
            id: 1,
            auto_use_aa_point: false
        })]
    ));
    assert_eq!(bridge.borrow().crafting.actabilities.get(1).unwrap().step, 0);
    for event in [
        ActabilityEvent::Requested(Command::Upgrade {
            id: 1,
            auto_use_aa_point: false,
        }),
        ActabilityEvent::TierChanged {
            is_upgrade: true,
            id: 1,
            step: 1,
        },
    ] {
        app.world_mut().write_message(NetEvent(Event::Actability(event)));
    }
    app.update();
    {
        let rt = app.world().non_send::<aa_ui::UiRuntime>();
        rt.lua
            .load("assert(EXPERT_ROW.itemInfo.grade==1 and EXPERT_ROW.downgradeButton:IsVisible() and EXPERT_ROW.downgradeButton:IsEnabled())")
            .exec()?;
        click(rt, "EXPERT_ROW.downgradeButton")?;
        click(rt, "EXPERT_DIALOG.btnOk")?;
    }
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [aa_net::Command::Actability(Command::Downgrade {
            id: 1,
            auto_use_aa_point: false
        })]
    ));
    assert_eq!(bridge.borrow().crafting.actabilities.get(1).unwrap().step, 1);
    for event in [
        ActabilityEvent::Requested(Command::Downgrade {
            id: 1,
            auto_use_aa_point: false,
        }),
        ActabilityEvent::TierChanged {
            is_upgrade: false,
            id: 1,
            step: 0,
        },
    ] {
        app.world_mut().write_message(NetEvent(Event::Actability(event)));
    }
    app.update();
    {
        let rt = app.world().non_send::<aa_ui::UiRuntime>();
        rt.lua
            .load("assert(EXPERT_ROW.itemInfo.grade==0 and EXPERT_ROW.itemInfo.point==10000)")
            .exec()?;
        click(rt, "actabilityTab.expandBtn")?;
        rt.lua.load("assert(actabilityTab.dialog and actabilityTab.dialog:IsVisible(),'expansion dialog missing; certificate count='..tostring(X2Bag:CountBagItemByItemType(29656)))").exec()?;
        click(rt, "actabilityTab.dialog.cancelButton")?;
        assert!(bridge.borrow().crafting.expert.requests.is_empty());
        click(rt, "actabilityTab.expandBtn")?;
        click(rt, "actabilityTab.dialog.okButton")?;
    }
    app.update();
    assert!(matches!(drain(&mut app).as_slice(), [aa_net::Command::Actability(Command::Expand)]));
    assert_eq!(bridge.borrow().crafting.actabilities.expanded_expert, Some(0));
    assert_eq!(bridge.borrow().crafting.inventory.as_ref().unwrap().bag[0].as_ref().unwrap().count, 1);
    for event in [ActabilityEvent::Requested(Command::Expand), ActabilityEvent::Expanded { count: 1 }] {
        app.world_mut().write_message(NetEvent(Event::Actability(event)));
    }
    app.update();
    assert_eq!(bridge.borrow().crafting.actabilities.expanded_expert, Some(1));
    assert_eq!(
        bridge.borrow().crafting.inventory.as_ref().unwrap().bag[0].as_ref().unwrap().count,
        1,
        "expansion ACK does not invent item consumption"
    );
    app.world().non_send::<aa_ui::UiRuntime>().lua.load("X2Ability:UpgradeExpert(1)").exec()?;
    bridge.borrow_mut().character_id = 48;
    app.update();
    assert!(drain(&mut app).is_empty(), "an old character's confirmation cannot submit for the replacement");
    drop(app);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
