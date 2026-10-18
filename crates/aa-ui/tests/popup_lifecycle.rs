//! Pointer edges and Escape through the unchanged original popup, combo and dialog scripts.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime,
    input::{self, MouseButtons, MouseState},
    lua,
};

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn original() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@popup_owner",
        r#"
        owner=UIParent:CreateWidget('window','nativePopupOwner','UIParent')
        owner:SetExtent(350,200);owner:AddAnchor('TOPLEFT',UIParent,300,200);owner:Show(true)
        outside=UIParent:CreateWidget('button','nativeOutside','UIParent')
        outside:SetExtent(100,30);outside:AddAnchor('TOPLEFT',UIParent,750,200);outside:Show(true)
        outsideClicks=0;outside:SetHandler('OnClick',function() outsideClicks=outsideClicks+1 end)
        combo=CreateComboBox(owner);combo:SetWidth(200);combo:AddAnchor('TOPLEFT',owner,20,30)
        combo.dropdownItem={'First','Second','Third'};combo:Select(1)
        popupActions,popupHides=0,0
        popupInfo=GetDefaultPopupInfoTable()
        popupInfo:AddInfo('Whisper',function() popupActions=popupActions+1 end)
        popupInfo.hideProcedure=function() popupHides=popupHides+1 end
        function openNativePopup()
            ShowPopUpMenu('nativePointerPopup',combo,popupInfo,false,'TOPLEFT','BOTTOMLEFT',0,0)
            popup=combo:GetAttachedWidget();assert(popup and popup:IsVisible())
        end
    "#,
    )
    .unwrap();
    ui
}

fn point(ui: &UiRuntime, widget: &str) -> (f32, f32) {
    eval(
        ui,
        &format!("local x,y={widget}:GetOffset();return x+5,y+5"),
    )
}

fn edge(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32), buttons: MouseButtons) {
    input::update_buttons(&ui.lua, mouse, Some(p), buttons, 0.);
    lua::tick(&ui.lua, 16.);
}

fn click(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32)) {
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
}

#[test]
fn native_mouse_handlers_precede_global_edges_and_clicks_follow_release() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@native_mouse_order",
        r#"
        button=UIParent:CreateWidget('button','edgeButton','UIParent')
        button:SetExtent(100,30);button:Show(true)
        log={}
        button:SetHandler('OnMouseDown',function(self,b) table.insert(log,'down:'..b) end)
        button:SetHandler('OnMouseUp',function(self,b) table.insert(log,'up:'..b) end)
        button:SetHandler('OnClick',function(self,b) table.insert(log,'click:'..b) end)
        events=UIParent:CreateWidget('emptywidget','edgeEvents','UIParent')
        events:RegisterEvent('MOUSE_DOWN');events:RegisterEvent('MOUSE_UP')
        events:SetHandler('OnEvent',function(self,event,id)
            assert(id==button:GetId());assert(button:IsDescendantWidget(id))
            table.insert(log,event)
        end)
    "#,
    )
    .unwrap();
    let mut mouse = MouseState::default();
    click(&ui, &mut mouse, (5., 5.));
    edge(
        &ui,
        &mut mouse,
        (5., 5.),
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
    edge(
        &ui,
        &mut mouse,
        (5., 5.),
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "down:LeftButton,MOUSE_DOWN,up:LeftButton,MOUSE_UP,click:LeftButton,down:RightButton,MOUSE_DOWN,up:RightButton,MOUSE_UP,click:RightButton"
    );
    input::update_buttons(
        &ui.lua,
        &mut mouse,
        None,
        MouseButtons {
            press: true,
            release: true,
            ..Default::default()
        },
        0.,
    );
    assert_eq!(eval::<usize>(&ui, "#log"), 10);
    ui.exec("@empty_space",r#"
        events:SetHandler('OnEvent',function(self,event,id) assert(id==0);table.insert(log,event) end)
        assert(not button:IsDescendantWidget(-1))
        assert(not button:IsDescendantWidget(1.5))
        assert(not button:IsDescendantWidget(1e30))
    "#).unwrap();
    click(&ui, &mut mouse, (1900., 1000.));
    assert_eq!(eval::<usize>(&ui, "#log"), 12);
}

#[test]
fn original_popup_keeps_inside_presses_and_closes_on_left_right_and_empty_space() {
    let ui = original();
    let mut mouse = MouseState::default();
    ui.exec("@open_popup", "openNativePopup()").unwrap();
    let first = lua::value_id(&ui.lua, &eval(&ui, "popup")).unwrap();
    let p = point(&ui, "popup.buttons[1]");
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(&ui, "popup:IsVisible()"));
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(
        &ui,
        "popupActions==1 and not popup:IsVisible() and combo:GetAttachedWidget()==nil"
    ));
    assert!(!lua::ui(&ui.lua).tree.get(first).alive);
    assert!(!lua::ui(&ui.lua).events["MOUSE_DOWN"].contains(&first));
    ui.exec("@reopen_popup", "openNativePopup()").unwrap();
    let p = point(&ui, "outside");
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(
        &ui,
        "not popup:IsVisible() and popupHides==2 and combo:GetAttachedWidget()==nil"
    ));
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(eval::<usize>(&ui, "outsideClicks"), 1);
    ui.exec("@reopen_popup", "openNativePopup()").unwrap();
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(&ui, "not popup:IsVisible() and popupHides==3"));
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
    ui.exec("@reopen_popup", "openNativePopup()").unwrap();
    click(&ui, &mut mouse, (1900., 900.));
    assert!(eval::<bool>(&ui, "not popup:IsVisible() and popupHides==4"));
}

#[test]
fn original_combo_preserves_owner_and_dropdown_descendants_and_selects_before_closing() {
    let ui = original();
    let mut mouse = MouseState::default();
    let arrow = point(&ui, "combo.button");
    click(&ui, &mut mouse, arrow);
    assert!(eval::<bool>(
        &ui,
        "commonDropdown:IsVisible() and combo:GetAttachedWidget()==commonDropdown"
    ));
    edge(
        &ui,
        &mut mouse,
        arrow,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(&ui, "commonDropdown:IsVisible()"));
    edge(
        &ui,
        &mut mouse,
        arrow,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert!(!eval::<bool>(&ui, "commonDropdown:IsVisible()"));
    click(&ui, &mut mouse, arrow);
    let row = eval(
        &ui,
        "local x,y=commonDropdown.list:GetOffset();local l,t=commonDropdown.list:GetInset();return x+l+10,y+t+commonDropdown.list:GetHeight()*1.5",
    );
    edge(
        &ui,
        &mut mouse,
        row,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(&ui, "commonDropdown:IsVisible()"));
    edge(
        &ui,
        &mut mouse,
        row,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert!(eval::<bool>(
        &ui,
        "combo:GetSelectedIndex()==2 and not commonDropdown:IsVisible()"
    ));
    click(&ui, &mut mouse, arrow);
    edge(
        &ui,
        &mut mouse,
        point(&ui, "outside"),
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
    assert!(!eval::<bool>(&ui, "commonDropdown:IsVisible()"));
}

#[test]
fn original_confirmation_escape_runs_cancel_and_notice_escape_runs_ok_before_hide() {
    let ui = original();
    ui.exec(
        "@confirmation",
        r#"
        dialogLog={}
        dialog=X2DialogManager:RequestDefaultDialog(function(wnd)
            wnd:SetTitle('Confirm');wnd:SetContent('Original dialog lifecycle')
            wnd.CancelProc=function(self)
                assert(self:IsVisible());table.insert(dialogLog,'cancel')
            end
            wnd:SetHandler('OnHide',function() table.insert(dialogLog,'hide') end)
        end,owner:GetId())
    "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &eval(&ui, "dialog")).unwrap();
    assert_eq!(ui.escape().unwrap(), aa_ui::content::Escaped::Closed(id));
    assert_eq!(
        eval::<String>(&ui, "table.concat(dialogLog,',')"),
        "cancel,hide"
    );
    ui.exec(
        "@notice",
        r#"
        notice=X2DialogManager:RequestNoticeDialog(function(wnd)
            wnd:SetTitle('Notice');wnd:SetContent('Original notice lifecycle')
            wnd.OkProc=function(self)
                assert(self:IsVisible());table.insert(dialogLog,'ok')
            end
            wnd.CancelProc=function() error('notice must call OkProc') end
            wnd:SetHandler('OnHide',function() table.insert(dialogLog,'hide') end)
        end,owner:GetId())
    "#,
    )
    .unwrap();
    ui.escape().unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(dialogLog,',')"),
        "cancel,hide,ok,hide"
    );
    // Manager cleanup must not call a second hide for the dialogs already closed by Escape.
    ui.exec(
        "@owner_cleanup",
        "X2DialogManager:DeleteByOwnerWindow(owner:GetId())",
    )
    .unwrap();
    assert_eq!(eval::<usize>(&ui, "#dialogLog"), 4);
}

#[test]
fn removal_waits_for_a_shown_to_hidden_transition_and_fade_completion() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@hidden_removal",
        r#"
        window=UIParent:CreateWidget('window','removeWindow','UIParent')
        window:Show(false);window:EnableHidingIsRemove(true)
        edit=window:CreateChildWidget('editbox','edit',0,true)
        edit:SetExtent(100,30);edit:Show(true)
        events,hides=0,0
        edit:RegisterEvent('REMOVAL_PROBE')
        edit:SetHandler('OnEvent',function() events=events+1 end)
        window:SetHandler('OnHide',function(self)
            assert(self:GetId()~=nil and edit:GetId()~=nil);hides=hides+1
        end)
    "#,
    )
    .unwrap();
    let window = lua::value_id(&ui.lua, &eval(&ui, "window")).unwrap();
    let edit = lua::value_id(&ui.lua, &eval(&ui, "edit")).unwrap();
    lua::tick(&ui.lua, 16.);
    assert!(lua::ui(&ui.lua).tree.get(window).alive);
    ui.exec(
        "@show_then_hide",
        "window:Show(true);edit:SetFocus();edit:EnableKeyboard(true)",
    )
    .unwrap();
    assert!(ui.key_event(bevy::prelude::KeyCode::F4, true, None));
    ui.exec("@fade_remove", "window:Show(false,100)").unwrap();
    lua::tick(&ui.lua, 50.);
    assert!(lua::ui(&ui.lua).tree.get(window).alive);
    assert!(lua::ui(&ui.lua).tree.visible(window));
    ui.fire_event("REMOVAL_PROBE", ());
    assert_eq!(eval::<usize>(&ui, "events"), 1);
    lua::tick(&ui.lua, 60.);
    assert!(!lua::ui(&ui.lua).tree.get(window).alive);
    assert!(!lua::ui(&ui.lua).tree.get(edit).alive);
    assert!(!lua::ui(&ui.lua).events["REMOVAL_PROBE"].contains(&edit));
    assert!(lua::object_table(&ui.lua, edit).unwrap().is_none());
    assert!(ui.focus().is_none());
    assert_eq!(eval::<usize>(&ui, "hides"), 1);
    ui.fire_event("REMOVAL_PROBE", ());
    assert_eq!(eval::<usize>(&ui, "events"), 1);
    // A key pressed while the removed edit owned capture stays consumed until released.
    assert!(ui.key_event(bevy::prelude::KeyCode::F4, false, None));
    lua::keyboard::begin_frame(&ui.lua);
    assert!(!ui.keyboard_captured());
}

#[test]
fn showing_again_or_disabling_removal_cancels_pending_retirement() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@cancel_remove",
        r#"
        window=UIParent:CreateWidget('window','keepWindow','UIParent')
        window:Show(true);window:EnableHidingIsRemove(true)
        window:Show(false);window:EnableHidingIsRemove(false)
    "#,
    )
    .unwrap();
    let window = lua::value_id(&ui.lua, &eval(&ui, "window")).unwrap();
    lua::tick(&ui.lua, 16.);
    assert!(lua::ui(&ui.lua).tree.get(window).alive);
    ui.exec("@reshow", "window:Show(true);window:EnableHidingIsRemove(true);window:Show(false,100);window:Show(true)").unwrap();
    lua::tick(&ui.lua, 150.);
    assert!(lua::ui(&ui.lua).tree.visible(window));
    ui.exec("@actual_remove", "window:Show(false)").unwrap();
    lua::tick(&ui.lua, 16.);
    assert!(!lua::ui(&ui.lua).tree.get(window).alive);
}
