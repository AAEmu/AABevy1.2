//! Original ui_management/chat bytecode persists and restores bounds through pointer callbacks.
#[path = "../examples/native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@window_bounds", code).unwrap();
}

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}

fn setup() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    original::install(&mut ui);
    // The application owns storage. Keep the real Lua save/restore handlers and substitute
    // only the X2 storage boundary; no account settings files are read or written.
    run(
        &ui,
        r#"
        boundsStore={};writes=0;completions=0
        X2.GetCharacterUiData=function(self,key) return boundsStore[key] end
        X2.SetCharacterUiData=function(self,key,value)
            boundsStore[key]=value
            if string.sub(key,1,9)=='ui_bound_' then writes=writes+1;lastKey=key end
        end
        nativeAlphaChat.isMoved=nil
        nativeAlphaChat:OnScale()
        nativeAlphaChat:RemoveAllAnchors();nativeAlphaChat:AddAnchor('TOPLEFT',UIParent,100,100)
        nativeAlphaChat:SetExtent(460,220)
        nativeAlphaChat:AllowTabSwitch(true)
        nativeAlphaChat.ProcMovedSize=function() completions=completions+1 end
    "#,
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(eval::<i32>(&ui, "writes"), 0);
    ui
}

fn step(ui: &UiRuntime, mouse: &mut input::MouseState, at: (f32, f32), down: bool, up: bool) {
    input::update(&ui.lua, mouse, Some(at), down, up, false, 0.);
}

fn assert_restored(ui: &UiRuntime, expected: (f32, f32, f32, f32)) -> i32 {
    let before = eval::<i32>(ui, "writes");
    run(
        ui,
        r#"
        savedBound=boundsStore[lastKey].bound
        savedResolution=boundsStore[lastKey].screenResolution
        assert(savedResolution.x==1920 and savedResolution.y==1080)
        local key=string.sub(lastKey,10)
        restored=UIParent:CreateWidget('window','restoredSavedBound','UIParent')
        restored:SetExtent(10,10);restored:AddAnchor('TOPLEFT',UIParent,800,600)
        AddUISaveHandlerByKey(key,restored)
        restored:ApplyLastWindowBound()
    "#,
    );
    assert_eq!(
        eval::<(f32, f32, f32, f32)>(
            ui,
            "return savedBound.x,savedBound.y,savedBound.width,savedBound.height"
        ),
        expected
    );
    assert_eq!(
        eval::<(f32, f32, f32, f32)>(
            ui,
            "local x,y=restored:GetEffectiveOffset();local w,h=restored:GetExtent();return x,y,w,h"
        ),
        expected
    );
    // Native anchor, extent's anchor, then extent's bound notifications save synchronously.
    let after = eval::<i32>(ui, "writes");
    assert_eq!(after, before + 3);
    after
}

#[test]
fn original_chat_tab_click_does_not_save_but_native_drag_saves_and_original_helper_restores() {
    let ui = setup();
    let (button, at) = {
        let u = lua::ui(&ui.lua);
        let w = u.tree.of_class("chatwindow")[0];
        let button = u.tree.widget(w).unwrap().chat.as_ref().unwrap().tabs[0].button;
        let r = u.tree.rect(button);
        (button, (r.x + r.w / 2., r.y + r.h / 2.))
    };
    assert_eq!(input::pick(&ui.lua, at.0, at.1), Some(button));
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, at, true, false);
    step(&ui, &mut mouse, at, false, true);
    assert_eq!(eval::<i32>(&ui, "writes"), 0);
    step(&ui, &mut mouse, at, true, false);
    // Native chat captures when tab dragging starts, after the toolkit's existing threshold.
    step(&ui, &mut mouse, (at.0 + 5., at.1), false, false);
    step(&ui, &mut mouse, (at.0 + 55., at.1 + 75.), false, false);
    assert_eq!(eval::<i32>(&ui, "writes"), 0);
    input::update(&ui.lua, &mut mouse, None, false, true, false, 0.);
    assert_eq!(eval::<i32>(&ui, "writes"), 1);
    assert_eq!(eval::<String>(&ui, "lastKey"), "ui_bound_chatWindow[0]");
    let writes = assert_restored(&ui, (150., 175., 460., 220.));
    step(&ui, &mut mouse, (900., 800.), false, true);
    assert_eq!(eval::<i32>(&ui, "writes"), writes);
    lua::tick(&ui.lua, 0.);
    // No delayed bound save or repeated movement completion follows the synchronous events.
    assert_eq!(eval::<i32>(&ui, "writes"), writes);
    assert_eq!(eval::<i32>(&ui, "completions"), 1);
    assert_eq!(eval::<f32>(&ui, "boundsStore[lastKey].bound.x"), 150.);
}

#[test]
fn original_chat_border_resize_saves_final_extent_once_for_automatic_and_explicit_stop() {
    for explicit in [false, true] {
        let ui = setup();
        run(
            &ui,
            "nativeAlphaChat:SetMinResizingExtent(100,50);nativeAlphaChat:SetMaxResizingExtent(1000,1000);nativeAlphaChat:UseResizing(true)",
        );
        let mut mouse = input::MouseState::default();
        let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
        // The authored top border is exposed; the message child overlaps the side/bottom strips.
        assert_eq!(input::pick(&ui.lua, 330., 102.), Some(window));
        step(&ui, &mut mouse, (330., 102.), true, false);
        step(&ui, &mut mouse, (330., 52.), false, false);
        assert_eq!(eval::<i32>(&ui, "writes"), 0);
        if explicit {
            run(&ui, "nativeAlphaChat:StopMovingOrSizing()");
            assert_eq!(eval::<i32>(&ui, "writes"), 1);
        }
        step(&ui, &mut mouse, (330., 52.), false, true);
        assert_eq!(eval::<i32>(&ui, "writes"), 1);
        let writes = assert_restored(&ui, (100., 50., 460., 270.));
        step(&ui, &mut mouse, (330., 22.), false, false);
        assert_eq!(eval::<f32>(&ui, "nativeAlphaChat:GetHeight()"), 270.);
        assert_eq!(eval::<i32>(&ui, "writes"), writes);
        lua::tick(&ui.lua, 0.);
        assert_eq!(eval::<i32>(&ui, "completions"), 1);
        assert_eq!(eval::<f32>(&ui, "boundsStore[lastKey].bound.height"), 270.);
    }
}

#[test]
fn original_chat_saves_programmatic_anchor_and_size_setters_before_layout_and_restores() {
    let ui = setup();
    run(
        &ui,
        r#"
        anchorChanges=0;boundChanges=0
        nativeAlphaChat.ProcChangedAnchor=function(self,...)
            assert(self==nativeAlphaChat and select('#',...)==0)
            anchorChanges=anchorChanges+1
        end
        nativeAlphaChat.ProcBoundChanged=function() boundChanges=boundChanges+1 end
        -- The original anchor handler does not mark an untouched window as user-moved.
        nativeAlphaChat:AddAnchor('TOPLEFT',UIParent,125,150)
        assert(anchorChanges==1 and writes==0 and not nativeAlphaChat.isMoved)
        nativeAlphaChat.isMoved=true
        nativeAlphaChat:RemoveAllAnchors()
        assert(anchorChanges==1 and writes==0)
        nativeAlphaChat:AddAnchor('TOPLEFT',UIParent,200,250)
        assert(anchorChanges==2 and writes==1 and boundChanges==0 and completions==0)
        assert(lastKey=='ui_bound_chatWindow[0]')
        local bound=boundsStore[lastKey].bound
        assert(bound.x==200 and bound.y==250 and bound.width==460 and bound.height==220)
        -- Native does not suppress a valid identical anchor or unchanged extent.
        nativeAlphaChat:AddAnchor('TOPLEFT',UIParent,200,250)
        nativeAlphaChat:SetExtent(460,220)
        assert(anchorChanges==4 and writes==4 and boundChanges==1 and completions==0)
        nativeAlphaChat:SetExtent(500,240)
        assert(anchorChanges==5 and writes==6 and boundChanges==2)
    "#,
    );
    assert_restored(&ui, (200., 250., 500., 240.));
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "assert(anchorChanges==5 and boundChanges==2 and completions==0)",
    );
}

fn sizing_setup() -> UiRuntime {
    let ui = setup();
    run(&ui, r#"
        nativeAlphaChat:SetMinResizingExtent(100,50)
        nativeAlphaChat:SetMaxResizingExtent(1000,1000)
        nativeAlphaChat:UseResizing(true)
    "#);
    let message = {
        let u = lua::ui(&ui.lua);
        let window = u.tree.of_class("chatwindow")[0];
        u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].message
    };
    ui.lua.globals().set("message", lua::object_table(&ui.lua, message).unwrap().unwrap()).unwrap();
    ui
}

fn bounds(ui: &UiRuntime) -> (f32, f32, f32, f32) {
    eval(ui, "local x,y=nativeAlphaChat:GetEffectiveOffset();local w,h=nativeAlphaChat:GetExtent();return x,y,w,h")
}

#[test]
fn original_message_and_base_child_covered_eight_edges_resize_save_and_restore() {
    for (from, to, expected) in [
        ((102., 180.), (82., 180.), (80., 100., 480., 220.)),
        ((558., 180.), (588., 180.), (100., 100., 490., 220.)),
        ((330., 318.), (330., 348.), (100., 100., 460., 250.)),
        ((102., 318.), (82., 348.), (80., 100., 480., 250.)),
        ((558., 318.), (588., 348.), (100., 100., 490., 250.)),
        ((102., 102.), (82., 82.), (80., 80., 480., 240.)),
        ((330., 102.), (330., 82.), (100., 80., 460., 240.)),
        ((558., 102.), (588., 82.), (100., 80., 490., 240.)),
    ] {
        let ui = sizing_setup();
        if from.1 < 120. {
            // The original message covers five strips/corners; cover its exposed
            // top with a base child to exercise the same native fallback there.
            run(&ui, r#"
                cover=nativeAlphaChat:CreateChildWidget('emptywidget','topCover',0,true)
                cover:SetExtent(460,15);cover:AddAnchor('TOPLEFT',UIParent,100,100)
                cover:Show(true);cover:SetHandler('OnMouseDown',function() end)
            "#);
        } else { run(&ui, "cover=message"); }
        let cover = lua::value_id(&ui.lua, &eval(&ui, "cover")).unwrap();
        assert_eq!(input::pick(&ui.lua, from.0, from.1), Some(cover));
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, from, true, false);
        step(&ui, &mut mouse, to, false, false);
        assert_eq!(bounds(&ui), expected);
        assert_eq!(eval::<i32>(&ui, "writes"), 0);
        input::update(&ui.lua, &mut mouse, None, false, true, false, 0.);
        assert_eq!(eval::<i32>(&ui, "writes"), 1);
        assert_eq!(eval::<i32>(&ui, "completions"), 1);
        let writes = assert_restored(&ui, expected);
        step(&ui, &mut mouse, (from.0 + 100., from.1 + 100.), false, true);
        assert_eq!(bounds(&ui), expected);
        assert_eq!(eval::<i32>(&ui, "writes"), writes);
    }
}

#[test]
fn original_locked_chat_interior_outside_and_right_press_do_not_capture_or_move() {
    for (locked, at) in [(true, (102., 180.)), (true, (558., 318.)),
                          (false, (330., 180.)), (false, (98., 180.))] {
        let ui = sizing_setup();
        if locked { run(&ui, "nativeAlphaChat:UseResizing(false);nativeAlphaChat:AllowTabSwitch(false)"); }
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, at, true, false);
        step(&ui, &mut mouse, (at.0 + 30., at.1 + 30.), false, false);
        step(&ui, &mut mouse, (at.0 + 30., at.1 + 30.), false, true);
        assert_eq!(bounds(&ui), (100., 100., 460., 220.));
        assert_eq!(eval::<i32>(&ui, "writes"), 0);
    }
    let ui = sizing_setup();
    let mut mouse = input::MouseState::default();
    input::update_buttons(&ui.lua, &mut mouse, Some((102., 180.)), input::MouseButtons {
        right_press: true, ..Default::default()
    }, 0.);
    step(&ui, &mut mouse, (82., 180.), false, false);
    input::update_buttons(&ui.lua, &mut mouse, Some((82., 180.)), input::MouseButtons {
        right_release: true, ..Default::default()
    }, 0.);
    assert_eq!(bounds(&ui), (100., 100., 460., 220.));
    assert_eq!(eval::<i32>(&ui, "writes"), 0);
}

#[test]
fn original_chat_child_handlers_and_drag_registration_arbitrate_before_capture() {
    for handler in ["return true", "return false", "return nil", "return 0", "return -2", "return 'true'",
                    "error('expected resize handler failure')",
                    "nativeAlphaChat:UseResizing(false)", "nativeAlphaChat:Show(false)",
                    "message:SetHandler('OnDragStart',function() end)"] {
        let ui = sizing_setup();
        run(&ui, &format!(r#"
            childDowns=0;ownerDowns=0
            nativeAlphaChat:SetHandler('OnMouseDown',function() ownerDowns=ownerDowns+1 end)
            message:SetHandler('OnMouseDown',function(self,button,...)
                assert(self==message and button=='LeftButton' and select('#',...)==0)
                childDowns=childDowns+1; {handler}
            end)
        "#));
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, (102., 180.), true, false);
        step(&ui, &mut mouse, (82., 180.), false, true);
        let admitted = matches!(handler, "return false" | "return nil" | "return 0" | "return 'true'"
            | "error('expected resize handler failure')");
        assert_eq!(bounds(&ui), if admitted { (80., 100., 480., 220.) } else { (100., 100., 460., 220.) });
        assert_eq!(eval::<i32>(&ui, "childDowns"), 1);
        // Automatic border sizing consumes before the owner's script mouse-down.
        assert_eq!(eval::<i32>(&ui, "ownerDowns"), 0);
        assert_eq!(eval::<i32>(&ui, "completions"), i32::from(admitted));
    }
}

#[test]
fn interactive_children_keep_native_priority_over_chat_border() {
    for class in ["button", "editbox", "slider", "textbox"] {
        let ui = sizing_setup();
        run(&ui, &format!(r#"
            control=message:CreateChildWidget('{class}','edgeControl',0,true)
            control:SetExtent(24,80);control:AddAnchor('TOPLEFT',UIParent,100,140);control:Show(true);control:Clickable(true)
            controlDowns=0;controlClicks=0
            control:SetHandler('OnMouseDown',function() controlDowns=controlDowns+1;return false end)
            control:SetHandler('OnClick',function() controlClicks=controlClicks+1 end)
        "#));
        if class == "editbox" { run(&ui, "control:SetText('Keep selecting this text')"); }
        if class == "slider" { run(&ui, "control:SetMinMaxValues(0,100);control:SetValue(50)"); }
        let control = lua::value_id(&ui.lua, &eval(&ui, "control")).unwrap();
        assert_eq!(input::pick(&ui.lua, 102., 180.), Some(control), "{class}: {:?}", lua::ui(&ui.lua).tree.rect(control));
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, (102., 180.), true, false);
        step(&ui, &mut mouse, (118., 180.), false, false);
        step(&ui, &mut mouse, (118., 180.), false, true);
        assert_eq!(bounds(&ui), (100., 100., 460., 220.));
        assert_eq!(eval::<i32>(&ui, "controlDowns"), 1, "{class}");
        assert_eq!(eval::<i32>(&ui, "completions"), 0);
        if class == "editbox" {
            let u = lua::ui(&ui.lua);
            assert_eq!(u.focus, Some(control));
            let edit = u.tree.widget(control).unwrap().edit.as_ref().unwrap();
            assert_ne!(edit.selection_anchor, Some(edit.cursor));
        }
    }
}

#[test]
fn original_scroll_controls_and_tab_buttons_keep_border_input() {
    for getter in ["GetUpButton", "GetDownButton", "GetDownToBottomButton", "GetSlider"] {
        let ui = sizing_setup();
        run(&ui, &format!(r#"
            control=message:{getter}();control:RemoveAllAnchors()
            control:SetExtent(20,60);control:AddAnchor('TOPLEFT',UIParent,100,150)
            control:Show(true);controlDowns=0
            control:SetHandler('OnMouseDown',function() controlDowns=controlDowns+1 end)
        "#));
        if getter == "GetSlider" { run(&ui, "control:GetThumbButtonWidget():Clickable(false)"); }
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, (102., 180.), true, false);
        step(&ui, &mut mouse, (112., 180.), false, true);
        assert_eq!(bounds(&ui), (100., 100., 460., 220.));
        assert_eq!(eval::<i32>(&ui, "controlDowns"), 1, "{getter}: {:?}", lua::ui(&ui.lua).hover);
        assert_eq!(eval::<i32>(&ui, "completions"), 0);
    }
    // Hidden native slider still rejects border admission through its rectangle (+118).
    let ui = sizing_setup();
    run(&ui, r#"
        local slider=message:GetSlider();slider:RemoveAllAnchors()
        slider:SetExtent(20,60);slider:AddAnchor('TOPLEFT',UIParent,100,150);slider:Show(false)
    "#);
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (102., 180.), true, false);
    step(&ui, &mut mouse, (82., 180.), false, true);
    assert_eq!(bounds(&ui), (100., 100., 460., 220.));
    assert_eq!(eval::<i32>(&ui, "completions"), 0);
    // Lock drag policy independently, leaving resizing enabled, so a tab at the
    // left border must keep its own native press/drag rather than start sizing.
    let ui = sizing_setup();
    run(&ui, "nativeAlphaChat:AllowTabSwitch(false)");
    let (button, at) = {
        let u = lua::ui(&ui.lua);
        let window = u.tree.of_class("chatwindow")[0];
        let button = u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].button;
        let r = u.tree.rect(button);
        (button, (r.x + 1., r.y + r.h / 2.))
    };
    assert!(at.0 > 100. && at.0 < 105., "{at:?}");
    assert_eq!(input::pick(&ui.lua, at.0, at.1), Some(button));
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, at, true, false);
    step(&ui, &mut mouse, (at.0 - 10., at.1), false, false);
    step(&ui, &mut mouse, (at.0 - 30., at.1), false, true);
    assert_eq!(bounds(&ui), (100., 100., 460., 220.));
    assert_eq!(eval::<i32>(&ui, "completions"), 0);
}

#[test]
fn covered_chat_capture_explicit_stop_disable_hide_and_retirement_remain_distinct() {
    for action in ["nativeAlphaChat:StopMovingOrSizing()", "nativeAlphaChat:UseResizing(false)",
                   "nativeAlphaChat:Show(false)", "retire"] {
        let ui = sizing_setup();
        run(&ui, "childClicks=0;message:SetHandler('OnClick',function() childClicks=childClicks+1 end)");
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, (102., 180.), true, false);
        step(&ui, &mut mouse, (82., 180.), false, false);
        if action == "retire" { assert_eq!(lua::chat::reset_windows(&ui.lua), 1); }
        else { run(&ui, action); }
        step(&ui, &mut mouse, (72., 180.), false, true);
        let completions = eval::<i32>(&ui, "completions");
        assert_eq!(completions, i32::from(action != "retire"));
        // Native manager press identity is the message, while capture is the owner.
        // Its click query can still admit the message click, including after explicit stop.
        assert_eq!(eval::<i32>(&ui, "childClicks"), i32::from(action.contains("UseResizing(false)")), "{action}");
        let writes = eval::<i32>(&ui, "writes");
        step(&ui, &mut mouse, (42., 180.), false, true);
        assert_eq!(eval::<i32>(&ui, "writes"), writes);
        assert_eq!(eval::<i32>(&ui, "completions"), completions);
        if action != "retire" {
            assert_eq!(bounds(&ui), if action.contains("UseResizing(false)") {
                (70., 100., 490., 220.)
            } else { (80., 100., 480., 220.) });
        }
    }
}

#[test]
fn covered_chat_explicit_stop_retains_child_click_and_unregistered_button_uses_base_fallback() {
    for button in [false, true] {
        let ui = sizing_setup();
        run(&ui, if button { r#"
            child=message:CreateChildWidget('button','unregisteredEdge',0,true)
            child:SetExtent(24,80);child:AddAnchor('TOPLEFT',UIParent,100,140);child:Show(true)
            child:RegisterForClicks('LeftButton',false)
        "# } else { "child=message" });
        run(&ui, "childClicks=0;child:SetHandler('OnClick',function() childClicks=childClicks+1 end)");
        let mut mouse = input::MouseState::default();
        step(&ui, &mut mouse, (102., 180.), true, false);
        step(&ui, &mut mouse, (82., 180.), false, false);
        assert_eq!(bounds(&ui), (80., 100., 480., 220.));
        run(&ui, "nativeAlphaChat:StopMovingOrSizing()");
        step(&ui, &mut mouse, (82., 180.), false, true);
        assert_eq!(eval::<i32>(&ui, "completions"), 1);
        if !button { assert_eq!(eval::<i32>(&ui, "childClicks"), 1); }
        step(&ui, &mut mouse, (62., 180.), false, false);
        assert_eq!(bounds(&ui), (80., 100., 480., 220.));
    }
}

#[test]
fn covered_chat_stop_on_another_receiver_and_completion_reentry_preserve_capture_identity() {
    let ui = sizing_setup();
    run(&ui, r#"
        other=UIParent:CreateWidget('window','otherCapture','UIParent')
        other:SetExtent(40,40);other:AddAnchor('TOPLEFT',UIParent,800,800);other:Show(true)
        otherStops=0;other:SetHandler('OnMovedPosition',function() otherStops=otherStops+1 end)
    "#);
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (102., 180.), true, false);
    step(&ui, &mut mouse, (82., 180.), false, false);
    run(&ui, "other:StopMovingOrSizing()");
    step(&ui, &mut mouse, (72., 180.), false, true);
    assert_eq!(bounds(&ui), (80., 100., 480., 220.));
    assert_eq!(eval::<(i32, i32, i32)>(&ui, "return completions,otherStops,writes"), (0, 1, 0));
    run(&ui, "nativeAlphaChat.ProcMovedSize=function() completions=completions+1;other:StartMoving() end");
    step(&ui, &mut mouse, (82., 180.), true, false);
    step(&ui, &mut mouse, (62., 180.), false, true);
    assert_eq!(eval::<(i32, i32)>(&ui, "return completions,writes"), (1, 1));
    step(&ui, &mut mouse, (72., 190.), false, false);
    assert_eq!(eval::<(f32, f32)>(&ui, "return other:GetEffectiveOffset()"), (810., 810.));
    assert_eq!(bounds(&ui), (60., 100., 500., 220.));
    run(&ui, "other:StopMovingOrSizing()");
    assert_eq!(eval::<i32>(&ui, "otherStops"), 2);
}

#[test]
fn original_child_callback_can_retire_its_owner_before_resize_capture() {
    let ui = sizing_setup();
    ui.lua.globals().set("retireChats", ui.lua.create_function(|lua, ()| {
        Ok(lua::chat::reset_windows(lua))
    }).unwrap()).unwrap();
    run(&ui, "message:SetHandler('OnMouseDown',function() assert(retireChats()==1) end)");
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (102., 180.), true, false);
    step(&ui, &mut mouse, (82., 180.), false, true);
    assert!(lua::ui(&ui.lua).tree.of_class("chatwindow").is_empty());
    assert_eq!(eval::<(i32, i32)>(&ui, "return completions,writes"), (0, 0));
}

#[test]
fn stopped_covered_resize_does_not_click_a_different_interactive_descendant_on_release() {
    let ui = sizing_setup();
    let mut mouse = input::MouseState::default();
    step(&ui, &mut mouse, (102., 180.), true, false);
    step(&ui, &mut mouse, (82., 180.), false, false);
    run(&ui, r#"
        nativeAlphaChat:StopMovingOrSizing()
        control=message:CreateChildWidget('button','releaseTarget',0,true)
        control:SetExtent(24,80);control:AddAnchor('TOPLEFT',UIParent,80,140);control:Show(true)
        wrongClicks=0;control:SetHandler('OnClick',function() wrongClicks=wrongClicks+1 end)
    "#);
    let control = lua::value_id(&ui.lua, &eval(&ui, "control")).unwrap();
    assert_eq!(input::pick(&ui.lua, 82., 180.), Some(control));
    step(&ui, &mut mouse, (82., 180.), false, true);
    assert_eq!(eval::<(i32, i32, i32)>(&ui, "return wrongClicks,completions,writes"), (0, 1, 1));
    assert_eq!(bounds(&ui), (80., 100., 480., 220.));
}
