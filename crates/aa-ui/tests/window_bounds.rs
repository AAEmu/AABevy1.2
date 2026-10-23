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
    // Native anchor + extent notifications run the original SaveBound synchronously.
    let after = eval::<i32>(ui, "writes");
    assert_eq!(after, before + 2);
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
    // Layout's separate OnBoundChanged may save again; it must not repeat movement completion.
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
        assert(anchorChanges==4 and writes==3 and boundChanges==0 and completions==0)
        nativeAlphaChat:SetExtent(500,240)
        assert(anchorChanges==5 and writes==4 and boundChanges==0)
    "#,
    );
    assert_restored(&ui, (200., 250., 500., 240.));
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "assert(anchorChanges==5 and boundChanges==1 and completions==0)",
    );
}
