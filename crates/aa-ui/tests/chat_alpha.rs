//! Executed native alpha-state contracts, render factors and actual original chat controls.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, input, lua, render, tree::Id};
use lua::chat::{
    ChatTabInfo, ChatWindowInfo, append_tab, hydrate_window, set_alpha_options, update_tab,
};

fn run(ui: &UiRuntime, text: &str) {
    ui.exec("@chat_alpha", text).unwrap();
}
fn id(ui: &UiRuntime, text: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(text).eval().unwrap()).unwrap()
}
fn setup() -> (UiRuntime, Id, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        function OnCreateChatMessage(m)
            m.bg=m:CreateColorDrawable(0.2,0.3,0.4,0.8,'background')
            m.bg:AddAnchor('TOPLEFT',m,0,0);m.bg:AddAnchor('BOTTOMRIGHT',m,0,0)
            m:AddMessage('Full opacity text')
        end
        window=UIParent:CreateWidget('chatwindow','alphaChat','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
    "#,
    );
    let window = id(&ui, "window");
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            window_id: 0,
            tabs: vec![ChatTabInfo {
                id: 0,
                name: "All".into(),
                background: Some([0.2, 0.3, 0.4, 0.8]),
                ..Default::default()
            }],
            selected: 0,
            method: 0,
        },
    )
    .unwrap();
    let message = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs[0]
        .message;
    lua::tick(&ui.lua, 0.);
    (ui, window, message)
}
fn current(ui: &UiRuntime, id: Id) -> [f32; 4] {
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .dynamic_drawable
        .current
}
fn background(ui: &UiRuntime) -> [f32; 4] {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .find(|q| q.color[..3] == [0.2, 0.3, 0.4])
        .unwrap()
        .color
}

#[test]
fn native_global_pair_uses_opaque_metadata_tint_once_and_preserves_text() {
    let (ui, window, message) = setup();
    assert_eq!(current(&ui, message)[..3], [0.5; 3]);
    assert!((background(&ui)[3] - 0.5).abs() < 1e-6);
    {
        let mut u = lua::ui(&ui.lua);
        u.cursor = (200., 180.);
    }
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, message)[..3], [0.7; 3]);
    assert!((background(&ui)[3] - 0.7).abs() < 1e-6);
    let quads = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(
        quads.iter().any(|q| q.color == [1.; 4]),
        "message glyphs retain full opacity"
    );
    let raw = lua::ui(&ui.lua).tree.widget(message).unwrap().drawables[0];
    assert_eq!(
        lua::ui(&ui.lua).tree.get(raw).drawable().unwrap().color,
        [0.2, 0.3, 0.4, 1.]
    );
    assert_eq!(lua::ui(&ui.lua).tree.get(window).alpha, 1.);
    assert_eq!(lua::ui(&ui.lua).tree.get(message).alpha, 1.);
}

#[test]
fn global_refresh_keeps_raw_values_and_resets_idle_even_while_pointer_is_inside() {
    let (ui, _window, message) = setup();
    lua::ui(&ui.lua).cursor = (200., 180.);
    lua::tick(&ui.lua, 0.);
    set_alpha_options(&ui.lua, 1.5, -0.25);
    assert_eq!(current(&ui, message)[..3], [-0.25; 3]);
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, message)[..3], [1.5; 3]);
    set_alpha_options(&ui.lua, 1.5, -0.25);
    assert_eq!(current(&ui, message)[..3], [-0.25; 3]);
    run(
        &ui,
        "second=UIParent:CreateWidget('chatwindow','secondAlpha','UIParent')",
    );
    assert_eq!(current(&ui, id(&ui, "second"))[..3], [-0.25; 3]);
}

#[test]
fn popup_disable_holds_current_state_reenable_snaps_and_callbacks_precede_changes() {
    let (ui, window, message) = setup();
    run(
        &ui,
        r#"
        observed={}
        window:SetHandler('OnEnter',function(self) observed[#observed+1]=observeAlpha() end)
        window:SetHandler('OnLeave',function(self) observed[#observed+1]=observeAlpha() end)
    "#,
    );
    ui.lua
        .globals()
        .set(
            "observeAlpha",
            ui.lua
                .create_function(move |lua, (): ()| {
                    Ok(lua::ui(lua)
                        .tree
                        .widget(message)
                        .unwrap()
                        .dynamic_drawable
                        .current[0])
                })
                .unwrap(),
        )
        .unwrap();
    let mut mouse = Default::default();
    input::update(
        &ui.lua,
        &mut mouse,
        Some((200., 180.)),
        false,
        false,
        false,
        0.,
    );
    assert_eq!(ui.lua.load("observed[1]").eval::<f32>().unwrap(), 0.5);
    assert_eq!(current(&ui, message)[0], 0.7);
    run(
        &ui,
        "for _,layer in ipairs({'background','artwork','overlay'}) do window:UseDynamicDrawableState(layer,false) end",
    );
    input::update(
        &ui.lua,
        &mut mouse,
        Some((900., 500.)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(ui.lua.load("observed[2]").eval::<f32>().unwrap(), 0.7);
    assert_eq!(current(&ui, message)[..3], [0.7; 3]);
    run(&ui, "window:UseDynamicDrawableState('background',true)");
    assert_eq!(current(&ui, message)[..3], [0.5, 0.7, 0.7]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .dynamic_drawable
            .enabled,
        [true, false, false, false]
    );
    run(
        &ui,
        "window:UseDynamicDrawableState('invalid',true);window:UseDynamicDrawableState('artwork');window:UseDynamicDrawableState('artwork',1)",
    );
    assert_eq!(current(&ui, message)[..3], [0.5, 0.7, 0.7]);
}

#[test]
fn late_children_inherit_pairs_without_enabling_state_and_old_history_tint_survives() {
    let (ui, window, message) = setup();
    lua::ui(&ui.lua).cursor = (200., 180.);
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "late=window:CreateChildWidget('emptywidget','late',0,true)",
    );
    let late = id(&ui, "late");
    assert_eq!(current(&ui, late)[..3], [0.5; 3]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(late)
            .unwrap()
            .dynamic_drawable
            .enabled,
        [false; 4]
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, late)[..3], [0.7; 3]);
    let original = lua::ui(&ui.lua)
        .tree
        .widget(message)
        .unwrap()
        .message
        .as_ref()
        .unwrap()
        .lines
        .clone();
    update_tab(
        &ui.lua,
        0,
        &ChatTabInfo {
            id: 0,
            name: "All".into(),
            background: Some([0.2, 0.3, 0.4, 0.8]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines,
        original
    );
    assert!((background(&ui)[3] - 0.7).abs() < 1e-6);
    append_tab(
        &ui.lua,
        window,
        &ChatTabInfo {
            id: 2,
            name: "Trade".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let newest = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs[1]
        .message;
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, newest)[..3], [0.7; 3]);
    let c = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .clone()
        .unwrap();
    assert_eq!(current(&ui, c.edit)[..3], [1.; 3]);
}

#[test]
fn original_chat_background_hover_and_message_control_overrides() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let window = original::install(&mut ui);
    let tab = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs[0]
        .clone();
    let r = lua::ui(&ui.lua).tree.rect(tab.message);
    let mut mouse = Default::default();
    input::update(
        &ui.lua,
        &mut mouse,
        Some((r.x + r.w / 2., r.y + r.h / 2.)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, tab.message)[..3], [0.7; 3]);
    let message = lua::object_table(&ui.lua, tab.message).unwrap().unwrap();
    ui.lua.globals().set("alphaMessage", message).unwrap();
    let up = id(&ui, "alphaMessage:GetUpButton()");
    assert_eq!(current(&ui, up)[..3], [1.; 3]);
    input::update(&ui.lua, &mut mouse, Some((0., 0.)), false, false, false, 0.);
    lua::tick(&ui.lua, 0.);
    assert_eq!(current(&ui, tab.message)[..3], [0.5; 3]);
    assert_eq!(current(&ui, up)[..3], [0.; 3]);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn original_fresh_runtime_four_tab_restore_can_refresh_alpha_inside_the_app_provider() {
    for _ in 0..2 {
        let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        fake::install(&mut ui, "en_us", &fake::default_worlds());
        fake::install_ingame(&mut ui);
        ui.lua
            .globals()
            .set(
                "restoreChatAlpha",
                ui.lua
                    .create_function(|lua, (): ()| {
                        set_alpha_options(lua, 0.7, 0.5);
                        Ok(())
                    })
                    .unwrap(),
            )
            .unwrap();
        run(
            &ui,
            r#"
            AAUI.ChatTabs=function()
                restoreChatAlpha()
                return {{id=0,name='All'}, {id=1,name='Crew',filters={}},
                    {id=2,name='Trade',filters={9}},
                    {id=3,name='X',fontSize=19,filters={2},filterColors={[2]={r=.25,g=.5,b=.75,a=.8}}},
                    selected=3,method=0}
            end
            X2Chat.GetChatTabInfoTable=function(self,id) return {name=({'All','Crew','Trade','X'})[id+1]} end
        "#,
        );
        ui.run_stage(aa_ui::Stage::World);
        assert!(ui.errors.is_empty(), "{:?}", ui.errors);
        let u = lua::ui(&ui.lua);
        let windows = u.tree.of_class("chatwindow");
        assert!(!windows.is_empty());
        for id in windows {
            let c = u.tree.widget(id).unwrap().chat.as_ref().unwrap();
            assert_eq!(c.tabs.len(), 4);
            assert_eq!(c.selected, 3);
        }
    }
}
