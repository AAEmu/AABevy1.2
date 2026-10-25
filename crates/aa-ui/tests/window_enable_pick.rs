//! Window+2e8 subtree admission and the original physical-Shift roadmap consumer.
//! CPU only. Original event handlers are retained; all game data is local.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/radar.rs"]
mod radar_fixture;

use aa_ui::{UiRuntime, input, lua, tree::Id};
use bevy::input::keyboard::KeyCode;

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@window_enable_pick", code).unwrap();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(code).eval().unwrap()).unwrap()
}

fn original_map() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    radar_fixture::install(&mut ui);
    run(
        &ui,
        r#"
        ADDON:ShowContent(UIC_WORLDMAP,false)
        AAUI.RoadMap=function()return {
          level=2,zone=5,player={u=.846,v=.265},markers_version=101,
          markers={{u=.2,v=.2,kind='quest_zone_gives',tooltip='Original Shift marker'}}
        }end
        pickEvents={};roadTips={};hideTips=0
        UIParent:SetEventHandler('SET_ROADMAP_PICKABLE',function(on)
          table.insert(pickEvents,on)
        end)
        UIParent:SetEventHandler('SHOW_ROADMAP_TOOLTIP',function(rows,count)
          assert(count==1);table.insert(roadTips,rows[1])
        end)
        UIParent:SetEventHandler('HIDE_ROADMAP_TOOLTIP',function()hideTips=hideTips+1 end)
    "#,
    );
    lua::tick(&ui.lua, 500.);
    ui
}

fn marker_point(ui: &UiRuntime) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let map = u
        .tree
        .widget(id(ui, "roadmap"))
        .unwrap()
        .roadmap
        .as_ref()
        .unwrap();
    let rect = u.tree.rect(map.marker_drawables[0]);
    (rect.x + rect.w / 2., rect.y + rect.h / 2.)
}

#[test]
fn original_shift_producer_reaches_real_roadmap_tooltip_and_releases_once() {
    let ui = original_map();
    let point = marker_point(&ui);
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(point), false, false, false, 0.);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#pickEvents==0 and #roadTips==0)");
    ui.key_event(KeyCode::ShiftLeft, true, None);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        r#"
        assert(#pickEvents==1 and pickEvents[1]==true,'original Shift transition missing')
        assert(#roadTips==1 and roadTips[1].text=='Original Shift marker','original tooltip missing')
        assert(mapTooltip:IsVisible(),'original map tooltip must be visible')
    "#,
    );
    ui.key_event(KeyCode::ShiftRight, true, None);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#pickEvents==1 and #roadTips==1)");
    ui.key_event(KeyCode::ShiftRight, false, None);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#pickEvents==2 and pickEvents[2]==false and not mapTooltip:IsVisible())",
    );
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#pickEvents==2)");
}

#[test]
fn original_window_enablepick_false_survives_clickable_true_and_windowpick_true() {
    let ui = original_map();
    run(
        &ui,
        r#"
        roadmapWindow:EnableWindowPick(true)
        X2Input:IsShiftKeyDown()
    "#,
    );
    lua::fire_ui_event(
        &ui.lua,
        "SET_ROADMAP_PICKABLE",
        vec![mlua::Value::Boolean(false)],
    );
    run(&ui, "roadmapWindow:Clickable(true)");
    let window = id(&ui, "roadmapWindow");
    let rect = lua::ui(&ui.lua).tree.rect(window);
    assert_ne!(
        input::pick(&ui.lua, rect.x + 2., rect.y + 2.),
        Some(window),
        "Clickable must not undo the original Window subtree veto"
    );
}

#[test]
fn subtree_gate_is_strict_independent_and_does_not_change_nonwindow_enablepick() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        floor=UIParent:CreateWidget('button','floor','UIParent')
        floor:SetExtent(400,300);floor:Show(true)
        root=UIParent:CreateWidget('window','gateRoot','UIParent')
        root:SetExtent(200,100);root:AddAnchor('TOPLEFT',UIParent,20,20);root:Show(true)
        child=root:CreateChildWidget('window','child',0,true)
        child:SetExtent(200,100);child:AddAnchor('TOPLEFT',root,0,0)
        button=child:CreateChildWidget('button','button',0,true)
        button:SetExtent(25,25);button:AddAnchor('TOPLEFT',child,210,0)
        root:EnablePick(false);root:Clickable(true);root:EnableWindowPick(true)
        child:EnablePick(true);child:Clickable(true);child:EnableWindowPick(true)
        for _,v in ipairs({0,1,'true','false',{}})do assert(select('#',root:EnablePick(v))==0)end
        assert(select('#',root:EnablePick())==0 and select('#',root:EnablePick(nil))==0)
    "#,
    );
    assert_eq!(input::pick(&ui.lua, 25., 25.), Some(id(&ui, "floor")));
    assert_eq!(input::pick(&ui.lua, 235., 25.), Some(id(&ui, "floor")));
    run(
        &ui,
        "root:EnablePick(true);root:Clickable(false);root:EnableWindowPick(false)",
    );
    assert_eq!(input::pick(&ui.lua, 25., 25.), Some(id(&ui, "floor")));
    assert_eq!(input::pick(&ui.lua, 235., 25.), Some(id(&ui, "button")));
    run(
        &ui,
        r#"
        root:EnableWindowPick(true);child:Clickable(false)
        generic=UIParent:CreateWidget('gametooltip','generic','UIParent')
        generic:SetExtent(50,50);generic:AddAnchor('TOPLEFT',UIParent,300,200);generic:Show(true)
        generic:EnablePick(false)
        genericButton=generic:CreateChildWidget('button','genericButton',0,true)
        genericButton:SetExtent(30,30);genericButton:AddAnchor('TOPLEFT',generic,0,0)
    "#,
    );
    assert_eq!(input::pick(&ui.lua, 25., 25.), Some(id(&ui, "floor")));
    assert_eq!(
        input::pick(&ui.lua, 305., 205.),
        Some(id(&ui, "genericButton"))
    );
    run(&ui, "root:Clickable(true);root:EnablePick(true,false)");
    assert_eq!(input::pick(&ui.lua, 25., 25.), Some(id(&ui, "root")));
}

#[test]
fn physical_shift_controls_both_original_tooltip_paths_independently_of_window_flags() {
    let ui = original_map();
    let mut mouse = input::MouseState::default();
    input::update(
        &ui.lua,
        &mut mouse,
        Some(marker_point(&ui)),
        false,
        false,
        false,
        0.,
    );
    for key in [KeyCode::AltLeft, KeyCode::ControlRight] {
        ui.key_event(key, true, None);
        lua::tick(&ui.lua, 16.);
    }
    run(&ui, "assert(#roadTips==0 and #pickEvents==0)");
    ui.key_event(KeyCode::ShiftRight, true, None);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        r#"
        assert(#roadTips==1 and roadTips[1].tooltipType=='normal')
        roadmapWindow:Clickable(false);roadmapWindow:EnablePick(false)
        roadmapWindow:EnableWindowPick(false)
    "#,
    );
    let map = lua::ui(&ui.lua).tree.rect(id(&ui, "roadmap"));
    input::update(
        &ui.lua,
        &mut mouse,
        Some((map.x + map.w * 0.32, map.y + map.h * 0.44)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        r#"
        assert(#pickEvents==1,'held Shift must not overwrite a manual Window gate')
        local last=roadTips[#roadTips]
        assert(last.tooltipType=='slave' and last.id==201 and last.hp==720 and last.maxHp==1000)
        assert(mapTooltip:IsVisible() and mapTooltip.targetId==201)
    "#,
    );
    assert!(
        !lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "roadmapWindow"))
            .unwrap()
            .pick_enabled
    );
    ui.key_event(KeyCode::ShiftRight, false, None);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#pickEvents==2 and pickEvents[2]==false and not mapTooltip:IsVisible())",
    );
}

fn standalone_maps() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('window','mapHost','UIParent')
        host:SetExtent(300,200);host:Show(true)
        a=host:CreateChildWidget('roadmap','a',0,true);a:SetExtent(100,100)
        events={};UIParent:SetEventHandler('SET_ROADMAP_PICKABLE',function(on)table.insert(events,on)end)
    "#,
    );
    ui
}

#[test]
fn per_map_identity_handles_new_hidden_and_retired_maps_without_a_data_hook() {
    let ui = standalone_maps();
    run(
        &ui,
        "b=host:CreateChildWidget('roadmap','b',0,true);b:Show(false)",
    );
    ui.key_event(KeyCode::ShiftLeft, true, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==1 and events[1]==true)");
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==1)");
    run(&ui, "b:Show(true)");
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==2 and events[2]==true);b:Show(false)");
    ui.key_event(KeyCode::ShiftLeft, false, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==3 and events[3]==false);b:Show(true)");
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==4 and events[4]==false)");
    let old = id(&ui, "a");
    run(&ui, "a:EnableHidingIsRemove(true);a:Show(false)");
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(old).alive);
    ui.key_event(KeyCode::ShiftLeft, true, None);
    run(&ui, "a=host:CreateChildWidget('roadmap','a',0,true)");
    assert_ne!(id(&ui, "a"), old);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#events==6 and events[5]==true and events[6]==true)",
    );
}

#[test]
fn derived_pass_follows_children_and_uses_existing_hidden_fade_admission() {
    let ui = standalone_maps();
    run(
        &ui,
        r#"
        order={}
        a:SetHandler('OnUpdate',function()table.insert(order,'map')end)
        child=a:CreateChildWidget('emptywidget','child',0,true)
        child:SetHandler('OnUpdate',function()table.insert(order,'child')end)
        UIParent:SetEventHandler('SET_ROADMAP_PICKABLE',function()table.insert(order,'transition')end)
        host:Show(false,100)
    "#,
    );
    ui.key_event(KeyCode::ShiftLeft, true, None);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(table.concat(order,',')=='map,child,transition');host:Show(true);a:Show(false,100);order={}",
    );
    ui.key_event(KeyCode::ShiftLeft, false, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#order==0 and #events==1);a:Show(true)");
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(table.concat(order,',')=='map,child,transition' and #events==2)",
    );
}

#[test]
fn transition_commits_before_reentrant_delivery_and_retirement() {
    let ui = standalone_maps();
    ui.lua
        .globals()
        .set(
            "nestedTick",
            ui.lua
                .create_function(|lua, ()| {
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        nested=false
        UIParent:SetEventHandler('SET_ROADMAP_PICKABLE',function(on)
          if not nested then
            nested=true;nestedTick()
            a:EnableHidingIsRemove(true);a:Show(false);nestedTick()
          end
        end)
    "#,
    );
    ui.key_event(KeyCode::ShiftLeft, true, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==1 and events[1]==true and nested)");
    assert!(!lua::ui(&ui.lua).tree.get(id(&ui, "a")).alive);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==1)");
}

#[test]
fn original_tooltip_precedes_transition_and_retirement_stops_the_host_continuation() {
    let ui = original_map();
    ui.lua
        .globals()
        .set(
            "nestedTick",
            ui.lua
                .create_function(|lua, ()| {
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        UIParent:SetEventHandler('SHOW_ROADMAP_TOOLTIP',function()
          assert(#pickEvents==0,'tooltip must precede transition')
          roadmap:EnableHidingIsRemove(true);roadmap:Show(false);nestedTick()
        end)
    "#,
    );
    input::update(
        &ui.lua,
        &mut input::MouseState::default(),
        Some(marker_point(&ui)),
        false,
        false,
        false,
        0.,
    );
    ui.key_event(KeyCode::ShiftLeft, true, None);
    lua::tick(&ui.lua, 16.);
    assert!(!lua::ui(&ui.lua).tree.get(id(&ui, "roadmap")).alive);
    run(&ui, "assert(#roadTips==1 and #pickEvents==0)");
}
