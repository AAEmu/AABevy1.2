//! Executed native eight-layer manager and per-widget frame-order controls.
use aa_ui::{UiRuntime, lua};

#[path = "../examples/native_widget_frame.rs"]
#[allow(dead_code)]
mod original;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup(code: &str) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@frame_setup",
        r#"
        log={}
        a=UIParent:CreateWidget('window','frameA','UIParent');a:Show(true)
        b=UIParent:CreateWidget('window','frameB','UIParent')
        c=UIParent:CreateWidget('window','frameC','UIParent')
        d=UIParent:CreateWidget('window','frameD','UIParent')
        function record(w,name)
            w:SetHandler('OnUpdate',function() table.insert(log,name) end)
        end
        function scale(w)
            w:SetScaleAnimation(1.5,1,.1,0,'CENTER');w:SetStartAnimation(false,true)
        end
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@frame_control", code).unwrap();
    ui
}

fn own_scale(ui: &UiRuntime, name: &str) -> f32 {
    let id = lua::value_id(&ui.lua, &eval(ui, name)).unwrap();
    lua::ui(&ui.lua).animations.widget_sample(id).scale
}

#[test]
fn update_then_effect_start_channels_before_shown_children_receive_the_frame() {
    let ui = setup(
        r#"
        ch=a:CreateChildWidget('button','child',0,true)
        a:SetHandler('OnUpdate',function(self,dt)
            assert(dt==20);table.insert(log,'aU');self:SetAlphaAnimation(.2,1,.1,0)
            self:SetStartAnimation(true,false)
        end)
        a:SetHandler('OnEffect',function(self,dt)
            assert(dt==20);assert(self:GetAlpha()==.2 or math.abs(self:GetAlpha()-.2)<.000001)
            table.insert(log,'aE')
        end)
        ch:SetHandler('OnUpdate',function(self)
            table.insert(log,'childU');parentAlpha=a:GetAlpha();scale(self)
        end)
        ch:SetHandler('OnEffect',function() table.insert(log,'childE') end)
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(
        eval::<Vec<String>>(&ui, "log"),
        ["aU", "aE", "childU", "childE"]
    );
    assert_eq!(eval::<f32>(&ui, "parentAlpha").to_bits(), 0.36f32.to_bits());
    assert_eq!(own_scale(&ui, "ch").to_bits(), 1.4f32.to_bits());
}

#[test]
fn effect_handler_is_resolved_after_update_and_release_takes_effect_immediately() {
    let ui = setup(
        r#"
        a:SetHandler('OnEffect',function() error('stale handler') end)
        a:SetHandler('OnUpdate',function(self)
            self:SetHandler('OnEffect',function() table.insert(log,'new') end)
            self:ReleaseHandler('OnUpdate')
        end)
    "#,
    );
    lua::tick(&ui.lua, 20.);
    ui.exec(
        "@release_before_effect",
        r#"
        a:SetHandler('OnUpdate',function(self) self:ReleaseHandler('OnEffect') end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<Vec<String>>(&ui, "log"), ["new"]);
}

#[test]
fn layer_snapshots_admit_new_later_roots_but_defer_current_layer_additions() {
    let ui = setup(
        r#"
        b:SetUILayer('background');b:Show(true);record(b,'background')
        d:SetUILayer('system');record(d,'system');record(c,'normalNew')
        a:SetHandler('OnUpdate',function(self)
            table.insert(log,'normal');c:Show(true);d:Show(true)
        end)
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(
        eval::<Vec<String>>(&ui, "log"),
        ["background", "normal", "system"]
    );
    ui.exec("@clear_log", "log={}").unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(
        eval::<Vec<String>>(&ui, "log"),
        ["background", "normal", "normalNew", "system"]
    );
}

#[test]
fn admitted_roots_finish_even_when_hidden_and_shown_children_do_not_recheck_ancestor() {
    let ui = setup(
        r#"
        b:Show(true);scale(a);scale(b)
        ch=a:CreateChildWidget('button','child',0,true);scale(ch);record(ch,'child')
        a:SetHandler('OnUpdate',function(self)
            table.insert(log,'a');self:Show(false);b:Show(false)
        end)
        a:SetHandler('OnEffect',function() table.insert(log,'aEffect') end)
        record(b,'b')
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(
        eval::<Vec<String>>(&ui, "log"),
        ["a", "aEffect", "child", "b"]
    );
    for name in ["a", "b", "ch"] {
        assert_eq!(own_scale(&ui, name), 1.4);
    }
    ui.exec("@clear_hidden_log", "log={}").unwrap();
    lua::tick(&ui.lua, 20.);
    assert!(eval::<Vec<String>>(&ui, "log").is_empty());
    for name in ["a", "b", "ch"] {
        assert_eq!(own_scale(&ui, name), 1.4);
    }
}

#[test]
fn pending_root_fade_keeps_clock_but_hidden_child_fade_does_not() {
    let ui = setup(
        r#"
        ch=a:CreateChildWidget('button','hiddenChild',0,true)
        visible=a:CreateChildWidget('button','shownChild',0,true)
        scale(a);scale(ch);scale(visible)
        record(a,'a');record(ch,'hiddenChild');record(visible,'shownChild')
        ch:Show(false,100);a:Show(false,100)
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<Vec<String>>(&ui, "log"), ["a", "shownChild"]);
    assert_eq!(own_scale(&ui, "ch"), 1.5);
    assert_eq!(own_scale(&ui, "visible"), 1.4);
    lua::tick(&ui.lua, 80.);
    assert_eq!(eval::<Vec<String>>(&ui, "log"), ["a", "shownChild"]);
    assert_eq!(own_scale(&ui, "visible"), 1.4);
}

#[test]
fn alpha_zero_and_offscreen_clipped_geometry_do_not_suppress_widget_updates() {
    let ui = setup(
        r#"
        a:SetExtent(40,40);a:AddAnchor('TOPLEFT',UIParent,-20000,-20000)
        a:EnableScroll(true);a:SetAlpha(0);record(a,'root')
        ch=a:CreateChildWidget('button','outsideChild',0,true)
        ch:SetExtent(20,20);ch:AddAnchor('TOPLEFT',a,200,200);scale(ch);record(ch,'child')
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<Vec<String>>(&ui, "log"), ["root", "child"]);
    assert_eq!(own_scale(&ui, "ch"), 1.4);
}

#[test]
fn live_child_vector_visits_appended_children_and_checks_each_current_shown_flag() {
    let ui = setup(
        r#"
        one=a:CreateChildWidget('button','one',0,true)
        two=a:CreateChildWidget('button','two',0,true);two:Show(false)
        a:SetHandler('OnUpdate',function()
            table.insert(log,'a');two:Show(true)
            three=a:CreateChildWidget('button','three',0,true);record(three,'three')
        end)
        one:SetHandler('OnUpdate',function()
            table.insert(log,'one');two:Show(false)
            four=a:CreateChildWidget('button','four',0,true);record(four,'four')
        end)
        record(two,'two')
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(
        eval::<Vec<String>>(&ui, "log"),
        ["a", "one", "three", "four"]
    );
}

#[test]
fn moving_a_root_to_a_later_layer_can_admit_it_again_in_that_native_frame() {
    let ui = setup(
        r#"
        a:SetHandler('OnUpdate',function(self)
            table.insert(log,'a');self:SetUILayer('system')
        end)
    "#,
    );
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<Vec<String>>(&ui, "log"), ["a", "a"]);
}

#[test]
fn retiring_subtree_clears_paused_and_running_channel_samples_and_handlers() {
    let ui = setup(
        r#"
        ch=a:CreateChildWidget('button','retiredChild',0,true)
        a:EnableHidingIsRemove(true);scale(a);scale(ch);record(a,'staleRoot');record(ch,'staleChild')
        a:SetScaleAnimation(1.5,1,.1,0,'BOTTOMRIGHT')
        ch:SetStartAnimation(false,false)
        a:Show(false)
    "#,
    );
    let ids: Vec<_> = ["a", "ch"]
        .map(|name| lua::value_id(&ui.lua, &eval(&ui, name)).unwrap())
        .into();
    lua::tick(&ui.lua, 0.);
    let u = lua::ui(&ui.lua);
    for id in ids {
        assert!(!u.tree.get(id).alive);
        assert_eq!(u.animations.widget_sample(id), Default::default());
        assert!(!u.frame_handlers.iter().any(|(owner, _)| *owner == id));
    }
    assert!(eval::<Vec<String>>(&ui, "log").is_empty());
}

#[test]
fn original_battlefield_hold_callback_starts_and_steps_fade_in_the_same_frame() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    original::demonstrate(&ui);
    lua::tick(&ui.lua, 480.);
    assert_eq!(eval::<i32>(&ui, "scoreboardRequests"), 1);
    assert!(!eval::<bool>(&ui, "alarm:IsVisible()"));
    lua::tick(&ui.lua, 0.);
    let alarm = lua::value_id(&ui.lua, &eval(&ui, "alarm")).unwrap();
    assert!(!lua::ui(&ui.lua).tree.fading_paint_order().contains(&alarm));
}
