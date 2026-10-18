//! Original chat scripts with application-driven append, metadata refresh and removal.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, input, lua};
use bevy::prelude::*;
use lua::chat::{ChatTabInfo, append_tab, remove_tab, update_tab};

pub fn install(ui: &mut UiRuntime) -> u32 {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec(
        "@chat_lifecycle_config",
        r#"
        nativeChatNames={[0]='All',[2]='Trade',[5]='Harbor'}
        AAUI.ChatTabs=function() return {{id=0,name='All'},{id=2,name='Trade',filters={9}}} end
        X2Chat.GetChatTabInfoTable=function(self,id) return {name=nativeChatNames[id]} end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let (window, all) = {
        let u = lua::ui(&ui.lua);
        let window = u.tree.of_class("chatwindow")[0];
        (
            window,
            u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].clone(),
        )
    };
    ui.lua
        .globals()
        .set(
            "nativeChat",
            lua::object_table(&ui.lua, window).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@chat_lifecycle_initial",
        r#"
        nativeChat:RemoveAllAnchors();nativeChat:AddAnchor('CENTER',UIParent,0,-20)
        nativeChat:SetExtent(460,220)
        X2Chat:DispatchChatMessage(2,'[Harbor] |kLiora,0;: Meet us at the harbor after sunset.')
        X2Chat:DispatchChatMessage(9,'Trade messages retain their own tab.')
    "#,
    )
    .unwrap();
    if std::env::args().any(|arg| arg == "--before") {
        return window;
    }
    let mut harbor = ChatTabInfo {
        id: 5,
        name: "Friends".into(),
        filters: Some(vec![2, 4]),
        ..Default::default()
    };
    assert!(append_tab(&ui.lua, window, &harbor).unwrap());
    ui.exec("@chat_lifecycle_new",r#"
        X2Chat:DispatchChatMessage(2,'New tab created through the original message and button callbacks.')
        X2Chat:DispatchChatMessage(2,'This older line is recolored by the saved per-tab palette.')
        X2Chat:DispatchChatMessage(2,'|kLiora,0;: Original links and |cFF80BFFFinline colors|r stay intact.')
    "#).unwrap();
    harbor.name = "Harbor".into();
    harbor.font_size = Some(18.);
    harbor.background = Some([0.03, 0.08, 0.12, 0.65]);
    harbor.filter_colors = vec![(2, [0.55, 1., 0.85, 1.]), (4, [0.4, 0.7, 1., 1.])];
    assert!(update_tab(&ui.lua, 5, &harbor).unwrap());
    assert!(remove_tab(&ui.lua, 2).unwrap());
    let c = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .clone()
        .unwrap();
    assert_eq!(c.tabs[0], all);
    assert_eq!(c.tabs[1].id, 5);
    assert_eq!(c.selected, 1);
    let button = c.tabs[1].button;
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + 20., r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    ui.exec("@chat_lifecycle_tooltip", "HideTooltip()").unwrap();
    lua::tick(&ui.lua, 0.);
    window
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original chat lifecycle"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_chat_lifecycle");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .add_systems(
            Update,
            hold_hover.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
        .run();
}
fn hold_hover(ui: Option<NonSend<UiRuntime>>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let Some(ui) = ui else { return };
    let u = lua::ui(&ui.lua);
    let Some(window) = u.tree.of_class("chatwindow").first().copied() else {
        return;
    };
    let Some(c) = u.tree.widget(window).and_then(|w| w.chat.as_ref()) else {
        return;
    };
    let Some(tab) = c.tabs.get(c.selected) else {
        return;
    };
    let r = u.animations.widget_rect(&u.tree, tab.button);
    pointer.pos = Some(Vec2::new(
        (r.x + 20.) * u.ui_scale,
        (r.y + r.h * 0.5) * u.ui_scale,
    ));
}
