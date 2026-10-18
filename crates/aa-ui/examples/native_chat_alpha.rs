//! Original chat scripts with native global alpha and per-tab raw tint.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> u32 {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    lua::chat::set_alpha_options(&ui.lua, 0.7, 0.5);
    ui.exec(
        "@alpha_chat_config",
        r#"
        AAUI.ChatTabs=function() return {{id=0,name='All',bgColor={r=.08,g=.15,b=.2,a=.8}}} end
        X2Chat.GetChatTabInfoTable=function() return {name='All'} end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    ui.lua
        .globals()
        .set(
            "nativeAlphaChat",
            lua::object_table(&ui.lua, window).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec("@alpha_chat_history",r#"
        nativeAlphaChat:RemoveAllAnchors();nativeAlphaChat:AddAnchor('CENTER',UIParent,0,0)
        nativeAlphaChat:SetExtent(460,220)
        X2Chat:DispatchChatMessage(2,'[Harbor] |kLiora,0;: Meet us at the harbor after sunset.')
        X2Chat:DispatchChatMessage(2,'Message text keeps its full opacity while the background changes.')
        X2Chat:DispatchChatMessage(2,'|cFF80BFFFInline colors|r and original chat links remain intact.')
    "#).unwrap();
    lua::tick(&ui.lua, 600.);
    window
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native chat alpha"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_chat_alpha");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, point.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .add_systems(Update, context.after(point).before(aa_ui::UiFrame))
        .run();
}

fn context(ui: Option<NonSend<UiRuntime>>, mut step: Local<u8>, mut mouse: Local<aa_ui::input::MouseState>) {
    if !std::env::args().any(|arg| arg == "--context") || *step >= 2 { return; }
    let Some(ui) = ui else { return };
    let p = {
        let u = lua::ui(&ui.lua);
        let Some(window) = u.tree.of_class("chatwindow").first().copied() else { return };
        let button = u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].button;
        let r = u.animations.widget_rect(&u.tree, button);
        (r.x + r.w / 2., r.y + r.h / 2.)
    };
    aa_ui::input::update_buttons(&ui.lua, &mut mouse, Some(p), aa_ui::input::MouseButtons {
        right_press: *step == 0, right_release: *step == 1, ..Default::default()
    }, 0.);
    *step += 1;
}

fn point(ui: Option<NonSend<UiRuntime>>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let Some(ui) = ui else { return };
    let u = lua::ui(&ui.lua);
    let Some(window) = u.tree.of_class("chatwindow").first().copied() else {
        return;
    };
    let r = u.tree.rect(window);
    pointer.pos = Some(if std::env::args().any(|arg| arg == "--hover") {
        Vec2::new((r.x + r.w / 2.) * u.ui_scale, (r.y + r.h / 2.) * u.ui_scale)
    } else {
        Vec2::new(u.tree.screen_w * u.ui_scale - 10., 10.)
    });
}
