//! Real original Sound Options constructor/Init/Save through physical pointer input.
#[path = "../examples/fake/slider_page.rs"]
mod original;
use aa_ui::{UiRuntime, input, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@slider_page", code).unwrap()
}
fn setup(scale: f32) -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.load_ui_texts(std::path::Path::new(&std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into())))
    .unwrap();
    original::install(&mut ui, scale);
    ui
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn physical_rect(ui: &UiRuntime, name: &str) -> [f32; 4] {
    eval(
        ui,
        &format!(
            "local x,y={name}:GetEffectiveOffset();local w,h={name}:GetEffectiveExtent();return {{x,y,w,h}}"
        ),
    )
}
fn pointer(ui: &UiRuntime, mouse: &mut input::MouseState, p: (f32, f32), down: bool, up: bool) {
    let scale = lua::ui(&ui.lua).ui_scale;
    input::update(
        &ui.lua,
        mouse,
        Some((p.0 / scale, p.1 / scale)),
        down,
        up,
        false,
        0.,
    );
}

#[test]
fn original_sound_quality_track_press_pages_once_and_original_save_keeps_it() {
    let ui = setup(1.);
    let r = physical_rect(&ui, "quality");
    let t = physical_rect(&ui, "quality.thumb");
    let p = (r[0] + r[2] - 0.5, r[1] + r[3] / 2.);
    assert!(p.0 > t[0] + t[2]);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(id(&ui, "quality")));
    let mut mouse = input::MouseState::default();
    pointer(&ui, &mut mouse, p, true, false);
    let pressed: f32 = eval(&ui, "quality:GetValue()");
    run(
        &ui,
        "quality:Save();savedQuality=GetOptionItemValue(OPTION_ITEM_SOUND_QUALITY)",
    );
    let saved: f32 = eval(&ui, "savedQuality");
    let moved = (r[0] + 0.5, p.1);
    pointer(&ui, &mut mouse, moved, false, false);
    let held: f32 = eval(&ui, "quality:GetValue()");
    let calls: usize = eval(&ui, "qualityChanges");
    pointer(&ui, &mut mouse, moved, false, true);
    pointer(&ui, &mut mouse, p, false, false);
    let released: f32 = eval(&ui, "quality:GetValue()");
    assert!(
        pressed == 3. && saved == 3. && held == 3. && released == 3. && calls == 1,
        "original constructor/Init pointer page: pressed={pressed}, originalSave={saved}, held={held}, released={released}, callbacks={calls}; native3/3/3/3,1"
    );
}

fn bare() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        host=UIParent:CreateWidget('emptywidget','pageHost','UIParent')
        host:SetExtent(600,400);host:AddAnchor('TOPLEFT',UIParent,100,200);host:Show(true)
        slider=host:CreateChildWidget('slider','page',0,true)
        slider:SetExtent(300,40);slider:AddAnchor('TOPLEFT',host,0,0);slider:SetOrientation(1)
        thumb=slider:CreateChildWidget('button','thumb',0,true);thumb:SetExtent(20,20)
        slider:SetThumbButtonWidget(thumb);slider:SetFixedThumb(true);slider:SetMinThumbLength(20)
        slider:SetMinMaxValues(0,4);slider:SetValue(2,false);slider:SetPageStep(1)
        changes=0;last=0
        slider:SetHandler('OnSliderChanged',function(self,value) changes=changes+1;last=value end)
    "#,
    );
    ui
}
fn track(ui: &UiRuntime, name: &str, after: bool, vertical: bool) -> (f32, f32) {
    let r = physical_rect(ui, name);
    if vertical {
        (
            r[0] + r[2] / 2.,
            if after { r[1] + r[3] - 0.5 } else { r[1] + 0.5 },
        )
    } else {
        (
            if after { r[0] + r[2] - 0.5 } else { r[0] + 0.5 },
            r[1] + r[3] / 2.,
        )
    }
}

#[test]
fn real_effective_geometry_pages_on_both_axes_and_preserves_frozen_scales() {
    let ui = bare();
    for vertical in [false, true] {
        for (scale, parent) in [(1., 1.), (0.85, 1.), (1.25, 0.75), (0.85, 1.5)] {
            run(
                &ui,
                &format!(
                    "UIParent:SetUIScale({scale});host:SetScale({parent});slider:SetOrientation({});slider:SetExtent({},{});slider:SetValue(2,false);changes=0",
                    if vertical { 0 } else { 1 },
                    if vertical { 40 } else { 300 },
                    if vertical { 300 } else { 40 }
                ),
            );
            let p = track(&ui, "slider", true, vertical);
            let mut mouse = input::MouseState::default();
            pointer(&ui, &mut mouse, p, true, false);
            assert_eq!(
                eval::<f32>(&ui, "slider:GetValue()"),
                3.,
                "axis{vertical} scale{scale} parent{parent}"
            );
            pointer(
                &ui,
                &mut mouse,
                track(&ui, "slider", false, vertical),
                false,
                false,
            );
            assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 3.);
            assert_eq!(eval::<usize>(&ui, "changes"), 1);
            pointer(&ui, &mut mouse, p, false, true);
        }
    }
    run(
        &ui,
        "UIParent:SetUIScale(1);host:SetScale(1);host:ApplyUIScale(false);slider:SetOrientation(1);slider:SetExtent(300,40)",
    );
    let before = physical_rect(&ui, "slider");
    for scale in [0.85, 1.25, 1.] {
        run(
            &ui,
            &format!("UIParent:SetUIScale({scale});slider:SetValue(2,false);changes=0"),
        );
        let now = physical_rect(&ui, "slider");
        assert!((now[2] - before[2]).abs() < 0.001 && (now[3] - before[3]).abs() < 0.001);
        let mut mouse = input::MouseState::default();
        let p = track(&ui, "slider", false, false);
        pointer(&ui, &mut mouse, p, true, false);
        assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 1.);
        pointer(&ui, &mut mouse, p, false, true);
        assert_eq!(eval::<usize>(&ui, "changes"), 1);
    }
}

#[test]
fn direct_receiver_inclusive_thumb_edges_are_not_track_pages() {
    let ui = bare();
    // Force the Slider receiver for this native392c2a80 classifier control only.
    // The actual original ThumbButton path remains unchanged and outside this slice.
    run(&ui, "thumb:Clickable(false)");
    for (scale, parent, frozen) in [
        (1., 1., false),
        (0.85, 1., false),
        (1.25, 0.75, false),
        (0.85, 1.5, false),
        (0.85, 1., true),
    ] {
        run(
            &ui,
            &format!(
                "host:ApplyUIScale(true);UIParent:SetUIScale(1);host:SetScale({parent});host:ApplyUIScale({})",
                !frozen
            ),
        );
        for edge in 0..4 {
            run(
                &ui,
                &format!("UIParent:SetUIScale({scale});slider:SetValue(2,false);changes=0"),
            );
            let r = physical_rect(&ui, "thumb");
            let p = match edge {
                0 => (r[0], r[1] + r[3] / 2.),
                1 => (r[0] + r[2], r[1] + r[3] / 2.),
                2 => (r[0] + r[2] / 2., r[1]),
                _ => (r[0] + r[2] / 2., r[1] + r[3]),
            };
            let mut mouse = input::MouseState::default();
            pointer(&ui, &mut mouse, p, true, false);
            assert_eq!(
                eval::<f32>(&ui, "slider:GetValue()"),
                2.,
                "scale{scale} edge{edge} point{p:?} rect{r:?}"
            );
            assert_eq!(eval::<usize>(&ui, "changes"), 0);
            pointer(&ui, &mut mouse, p, false, true);
        }
    }
}

#[test]
fn page_is_not_step_snapped_and_clamped_unchanged_still_notifies_once() {
    let ui = bare();
    for (command, expected) in [
        (
            "slider:SetMinMaxValues(1,4);slider:SetValue(2.5,false);slider:SetPageStep(.25)",
            2.75,
        ),
        (
            "slider:SetMinMaxValues(1,4);slider:SetValue(3.75,false);slider:SetPageStep(1)",
            4.,
        ),
        (
            "slider:SetMinMaxValues(2,2);slider:SetValue(2,false);slider:SetPageStep(1)",
            2.,
        ),
    ] {
        run(&ui, command);
        run(
            &ui,
            "changes=0;local before=slider:GetValue();slider:SetValue(before,true);assert(changes==1 and last==before and slider:GetValue()==before);changes=0",
        );
        let p = track(&ui, "slider", true, false);
        let mut mouse = input::MouseState::default();
        pointer(&ui, &mut mouse, p, true, false);
        assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), expected);
        assert_eq!(eval::<usize>(&ui, "changes"), 1);
        assert_eq!(eval::<f32>(&ui, "last"), expected);
        pointer(&ui, &mut mouse, p, false, true);
        assert_eq!(eval::<usize>(&ui, "changes"), 1);
    }
}

#[test]
fn off_axis_track_point_within_thumb_axis_does_not_page_or_capture() {
    let ui = bare();
    let r = physical_rect(&ui, "slider");
    let t = physical_rect(&ui, "thumb");
    let p = (t[0] + t[2] / 2., r[1] + 0.5);
    let mut mouse = input::MouseState::default();
    assert!(p.1 < t[1]);
    pointer(&ui, &mut mouse, p, true, false);
    pointer(
        &ui,
        &mut mouse,
        track(&ui, "slider", true, false),
        false,
        false,
    );
    assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 2.);
    assert_eq!(eval::<usize>(&ui, "changes"), 0);
}

#[test]
fn admission_preserves_clipping_disable_and_mouse_down_liveness() {
    let ui = bare();
    for command in [
        "slider:Enable(false)",
        "thumb:Enable(false)",
        "host:EnableScroll(true);host:SetWidth(100)",
        "slider:SetHandler('OnMouseDown',function(self) self:Show(false) end)",
    ] {
        run(
            &ui,
            "slider:Enable(true);thumb:Enable(true);slider:Show(true);host:EnableScroll(false);host:SetWidth(600);slider:ReleaseHandler('OnMouseDown');slider:SetValue(2,false);changes=0",
        );
        run(&ui, command);
        let p = track(&ui, "slider", true, false);
        let mut mouse = input::MouseState::default();
        pointer(&ui, &mut mouse, p, true, false);
        pointer(
            &ui,
            &mut mouse,
            track(&ui, "slider", false, false),
            false,
            false,
        );
        assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 2., "{command}");
        assert_eq!(eval::<usize>(&ui, "changes"), 0);
        pointer(&ui, &mut mouse, p, false, true);
    }
}

#[test]
fn callbacks_disable_hide_retire_and_reenter_without_stale_capture() {
    use std::{cell::Cell, rc::Rc};
    for action in [
        "self:Enable(false)",
        "self:Show(false,50)",
        "self:EnableHidingIsRemove(true);self:Show(false)",
        "thumb:EnableHidingIsRemove(true);thumb:Show(false)",
        "self:SetValue(1,false)",
        "self:SetMinMaxValues(0,1)",
    ] {
        let ui = bare();
        let releases = Rc::new(Cell::new(0));
        let seen = Rc::clone(&releases);
        input::set_release_callback(&ui.lua, move |_, _| {
            seen.set(seen.get() + 1);
            Ok(())
        })
        .unwrap();
        let slider = id(&ui, "slider");
        let thumb = id(&ui, "thumb");
        run(
            &ui,
            &format!(
                "callbackRecords={{}};slider:SetHandler('OnSliderChanged',function(self,value) changes=changes+1;local lo,hi=self:GetMinMaxValues();callbackRecords[changes]={{value,lo,hi,self:GetValue()}};if changes==1 then last=value;seenValue=self:GetValue() end;{action} end)"
            ),
        );
        let p = track(&ui, "slider", true, false);
        let mut mouse = input::MouseState::default();
        pointer(&ui, &mut mouse, p, true, false);
        assert_eq!(eval::<f32>(&ui, "seenValue"), 3.);
        assert_eq!(eval::<f32>(&ui, "last"), 3.);
        lua::tick(&ui.lua, 100.);
        pointer(&ui, &mut mouse, (1., 1.), false, false);
        pointer(&ui, &mut mouse, (1., 1.), false, true);
        // Native range clamping emits its own callback after the first page callback.
        let expected_calls = if action == "self:SetMinMaxValues(0,1)" { 2 } else { 1 };
        assert_eq!(eval::<usize>(&ui, "changes"), expected_calls, "{action}");
        assert_eq!(eval::<usize>(&ui, "#callbackRecords"), expected_calls, "{action}");
        assert_eq!(eval::<[f32; 4]>(&ui, "callbackRecords[1]"), [3., 0., 4., 3.]);
        if action == "self:SetMinMaxValues(0,1)" {
            assert_eq!(eval::<[f32; 4]>(&ui, "callbackRecords[2]"), [1., 0., 1., 1.]);
        }
        assert_eq!(releases.get(), 1);
        if action.starts_with("self:EnableHiding") {
            assert!(!lua::ui(&ui.lua).tree.get(slider).alive);
        } else if action.starts_with("thumb:") {
            assert!(!lua::ui(&ui.lua).tree.get(thumb).alive);
        } else {
            assert_eq!(
                eval::<f32>(&ui, "slider:GetValue()"),
                if action.contains("SetValue") || action.contains("SetMinMaxValues") {
                    1.
                } else {
                    3.
                }
            );
        }
    }
}

#[test]
fn change_callback_can_reenter_real_input_on_another_slider() {
    let ui = bare();
    run(
        &ui,
        r#"
        other=UIParent:CreateWidget('slider','otherPage','UIParent');other:SetExtent(300,40);other:Show(true);other:AddAnchor('TOPLEFT',UIParent,600,100);other:SetOrientation(1)
        otherThumb=other:CreateChildWidget('button','thumb',0,true);otherThumb:SetExtent(20,20)
        other:SetThumbButtonWidget(otherThumb);other:SetFixedThumb(true);other:SetMinMaxValues(0,4);other:SetValue(2,false);other:SetPageStep(1)
        otherCalls=0;other:SetHandler('OnSliderChanged',function() otherCalls=otherCalls+1 end)
    "#,
    );
    let p = track(&ui, "other", true, false);
    let scale = lua::ui(&ui.lua).ui_scale;
    ui.lua
        .globals()
        .set(
            "nestedPage",
            ui.lua
                .create_function(move |lua, ()| {
                    let mut nested = input::MouseState::default();
                    input::update(
                        lua,
                        &mut nested,
                        Some((p.0 / scale, p.1 / scale)),
                        true,
                        false,
                        false,
                        0.,
                    );
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "slider:SetHandler('OnSliderChanged',function(self,value) changes=changes+1;last=value;nestedPage() end)",
    );
    let mut mouse = input::MouseState::default();
    pointer(
        &ui,
        &mut mouse,
        track(&ui, "slider", true, false),
        true,
        false,
    );
    assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 3.);
    assert_eq!(eval::<f32>(&ui, "other:GetValue()"), 3.);
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    assert_eq!(eval::<usize>(&ui, "otherCalls"), 1);
}

#[test]
fn original_scroll_window_pages_one_viewport_through_its_existing_bridge() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::load(&mut ui, 1.);
    run(
        &ui,
        r#"
        host=UIParent:CreateWidget('emptywidget','scrollPageHost','UIParent');host:SetExtent(300,200);host:Show(true);host:AddAnchor('TOPLEFT',UIParent,50,50)
        scroll=CreateScrollWindow(host,'scroll',0);scroll:SetExtent(300,200);scroll:AddAnchor('TOPLEFT',host,0,0)
        marker=scroll.content:CreateChildWidget('label','marker',0,true);marker:SetExtent(30,20);marker:AddAnchor('TOPLEFT',scroll.content,0,350)
        calls=0;scroll.SliderChangedProc=function(self,value) calls=calls+1;last=value end
        scroll:SetMinMaxValues('vert',0,600);scroll:SetValue(0);calls=0
        slider=scroll.scroll.vs
    "#,
    );
    let before: [f32; 2] = eval(&ui, "local x,y=marker:GetOffset();return {x,y}");
    let p = track(&ui, "slider", true, true);
    let mut mouse = input::MouseState::default();
    pointer(&ui, &mut mouse, p, true, false);
    assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 200.);
    assert_eq!(eval::<usize>(&ui, "calls"), 1);
    let after: [f32; 2] = eval(&ui, "local x,y=marker:GetOffset();return {x,y}");
    assert!((before[1] - after[1] - 200.).abs() < 0.001);
    pointer(
        &ui,
        &mut mouse,
        track(&ui, "slider", false, true),
        false,
        false,
    );
    assert_eq!(eval::<f32>(&ui, "slider:GetValue()"), 200.);
}

#[test]
fn visual_fixture_keeps_the_complete_original_sound_constructor_and_save() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.load_ui_texts(std::path::Path::new(&std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into())))
    .unwrap();
    original::install_visual(&mut ui, 0.85);
    let mut mouse = input::MouseState::default();
    let p = track(&ui, "quality", true, false);
    pointer(&ui, &mut mouse, p, true, false);
    pointer(
        &ui,
        &mut mouse,
        track(&ui, "quality", false, false),
        false,
        false,
    );
    pointer(&ui, &mut mouse, p, false, true);
    run(
        &ui,
        "quality:Save();assert(quality:GetValue()==3 and GetOptionItemValue(OPTION_ITEM_SOUND_QUALITY)==3 and qualityChanges==1)",
    );
}
