//! Native constant cooldown callbacks, including the original equipment handler.

use aa_ui::{UiRuntime, lua};

#[path = "../examples/fake/constant_cooldown.rs"]
mod constant_cooldown;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

fn run(ui: &UiRuntime, source: &str) {
    ui.exec("@constant_cooldown_test", source).unwrap();
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        button=UIParent:CreateWidget('cooldownconstantbutton','constant','UIParent')
        button:SetExtent(42,42);button:Show(true)
        events={};known=true;remaining=0;total=0;queries=0
        AAUI.SkillCooldown=function(id)
            queries=queries+1;queriedId=id
            if known then return remaining,total end
        end
        button:SetHandler('OnContentUpdated',function(self,kind,remain,duration)
            table.insert(events,{kind,remain,duration})
        end)
        button:SetSkill(123)
    "#,
    );
    ui
}

#[test]
fn ready_is_once_active_is_each_frame_and_rebinding_resets_the_flag() {
    let ui = setup();
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#events==1 and events[1][1]=='skill_cooldown' and events[1][2]==0)",
    );
    run(&ui, "remaining=8000;total=12000");
    lua::tick(&ui.lua, 16.);
    run(&ui, "remaining=7984");
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#events==3 and events[2][2]==8000 and events[3][2]==7984 and events[3][3]==12000)",
    );
    run(&ui, "remaining=0");
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#events==4 and events[4][2]==0 and events[4][3]==12000);button:SetSkill(123);assert(#events==4)",
    );
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==5);button:SetSkill(0);known=false");
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(#events==7 and events[7][2]==0 and events[7][3]==0)",
    );
}

#[test]
fn missing_descriptor_stays_unready_and_hidden_widgets_do_not_query() {
    let ui = setup();
    run(&ui, "known=false");
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==2);button:Show(false)");
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(queries==2 and #events==2);button:Show(true);known=true",
    );
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==3 and queries==4)");
    // Once ready, losing the descriptor does not clear the native ready flag.
    run(&ui, "known=false");
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==3);button:SetSkill(123)");
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(#events==4)");
}

#[test]
fn raw_identity_keeps_u32_precision_and_callback_matches_native_f32_numbers() {
    let ui = setup();
    run(
        &ui,
        "button:SetSkill(16777217);remaining=16777217;total=4294967295",
    );
    lua::tick(&ui.lua, 16.);
    run(
        &ui,
        "assert(queriedId==16777217 and events[1][2]==16777216 and events[1][3]==4294967296)",
    );
}

#[test]
fn original_equipment_cooldown_handler_preserves_icon_and_click_handler() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    constant_cooldown::install(&mut ui);
    run(
        &ui,
        r#"
        button=nativeCooldownButtons[1]
        clicks=0;AAUI.UseSkill=function() error('cooldown identity is not an action accessor') end
        button.OnClickProc=function() clicks=clicks+1 end
        button:SetSkill(1)
        assert(button.bgs and #button.bgs==4)
    "#,
    );
    let id = lua::value_id(&ui.lua, &ui.lua.load("button").eval().unwrap()).unwrap();
    let icon = lua::value_id(&ui.lua, &ui.lua.load("button.bgs[1]").eval().unwrap()).unwrap();
    let icon_before = lua::ui(&ui.lua).tree.get(icon).drawable().unwrap().clone();
    assert!(
        icon_before
            .texture
            .as_ref()
            .is_some_and(|p| p.contains("icon_item_teleport"))
    );
    let before = lua::ui(&ui.lua).tree.widget(id).unwrap().clone();
    lua::tick(&ui.lua, 16.);
    {
        let u = lua::ui(&ui.lua);
        let after = u.tree.widget(id).unwrap();
        assert_eq!(after.slot, before.slot);
        assert_eq!(
            after.button.as_ref().unwrap().bg,
            before.button.as_ref().unwrap().bg
        );
        let cooldown = after
            .cooldown
            .as_ref()
            .expect("original handler started the sweep");
        assert_eq!(cooldown.total_ms, 12000.);
        assert_eq!(cooldown.end_ms - u.time_ms, 8000.);
        assert!(after.slot.is_none());
        assert_eq!(u.tree.get(icon).drawable().unwrap(), &icon_before);
    }
    lua::call_handler(&ui.lua, id, "OnClick", vec![]);
    run(&ui, "assert(clicks==1);nativeCooldownSamples[1][1]=0");
    lua::tick(&ui.lua, 16.);
    let u = lua::ui(&ui.lua);
    let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
    assert_eq!((cd.sampled_ms, cd.total_ms), (Some(0.), 0.));
}

#[test]
fn constructor_observes_invalid_identity_until_set_skill() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        calls=0;events=0
        AAUI.SkillCooldown=function(id) assert(id==0);calls=calls+1 end
        button=UIParent:CreateWidget('cooldownconstantbutton','unbound','UIParent')
        button:Show(true)
        button:SetHandler('OnContentUpdated',function(self,kind,remaining,total)
            assert(kind=='skill_cooldown' and remaining==0 and total==0)
            events=events+1
        end)
    "#,
    );
    lua::tick(&ui.lua, 16.);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(calls==2 and events==2)");
}

#[test]
fn generic_set_cooldown_is_a_snapshot_not_an_independent_clock() {
    for class in [
        "cooldownbutton",
        "cooldownactionbutton",
        "cooldownconstantbutton",
        "cooldowninventorybutton",
    ] {
        let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
        ui.lua.globals().set("class", class).unwrap();
        run(
            &ui,
            r#"
            button=UIParent:CreateWidget(class,'snapshot','UIParent')
            button:SetExtent(42,42);button:Show(true);button:SetCoolDown(8000,12000)
            local image=button:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
            image:AddAnchor('TOPLEFT',button,1,1);image:AddAnchor('BOTTOMRIGHT',button,-1,-1)
            button:SetNormalBackground(image)
            button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        "#,
        );
        let id = lua::value_id(&ui.lua, &ui.lua.load("button").eval().unwrap()).unwrap();
        let mask = |ui: &UiRuntime| {
            aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
                .into_iter()
                .filter_map(|q| q.corners)
                .collect::<Vec<_>>()
        };
        let before = mask(&ui);
        assert!(!before.is_empty());
        lua::tick(&ui.lua, 9000.);
        assert_eq!(mask(&ui), before, "{class}");
        assert_eq!(
            lua::ui(&ui.lua)
                .tree
                .widget(id)
                .unwrap()
                .cooldown
                .as_ref()
                .unwrap()
                .sampled_ms,
            Some(8000.)
        );
        run(&ui, "button:SetCoolDown(0,0)");
        assert!(mask(&ui).is_empty());
    }
}

#[test]
fn original_consumer_alone_replaces_the_generic_draw_snapshot() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    constant_cooldown::install(&mut ui);
    run(
        &ui,
        r#"
        button=nativeCooldownButtons[1]
        button:SetCoolDown(8000,12000)
        button:SetHandler('OnContentUpdated',nil)
        nativeCooldownSamples[1]={500,1000}
    "#,
    );
    let id = lua::value_id(&ui.lua, &ui.lua.load("button").eval().unwrap()).unwrap();
    lua::tick(&ui.lua, 9000.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id)
            .unwrap()
            .cooldown
            .as_ref()
            .unwrap()
            .sampled_ms,
        Some(8000.)
    );
    run(&ui, "AddOnUpdateConstantCooldown(button,'action')");
    lua::tick(&ui.lua, 16.);
    {
        let u = lua::ui(&ui.lua);
        let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
        assert_eq!(cd.sampled_ms, Some(500.));
        assert_eq!(cd.total_ms, 1000.);
    }
    run(&ui, "nativeCooldownSamples[1][1]=0");
    lua::tick(&ui.lua, 16.);
    let u = lua::ui(&ui.lua);
    let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
    assert_eq!((cd.sampled_ms, cd.total_ms), (Some(0.), 0.));
}
