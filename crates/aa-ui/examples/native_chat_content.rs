//! Original chat message frames with native default and signed corner offsets.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-ui: native chat content anchors"),aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            auto_clip::install(ui);
            ui.exec("@native_chat_content",r#"
                nativeClipChat:RemoveAllAnchors()
                nativeClipChat:AddAnchor('TOPLEFT',UIParent,100,230)
                nativeClipChat:SetExtent(430,200)
                offsetChat=UIParent:CreateWidget('chatwindow','nativeOffsetChat','UIParent')
                offsetChat:SetChatWindowId(1)
                offsetChat:AddAnchor('TOPLEFT',UIParent,720,230)
                offsetChat:SetExtent(430,200)
                offsetChat:SetContentOffset(12,8,-16,-12)
                offsetChat:Show(true)
                HideTooltip()
            "#).unwrap();
            let windows={let u=aa_ui::lua::ui(&ui.lua);u.tree.of_class("chatwindow")};
            for window in windows {
                let message={let u=aa_ui::lua::ui(&ui.lua);u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].message};
                ui.lua.globals().set("contentMessage",aa_ui::lua::object_table(&ui.lua,message).unwrap().unwrap()).unwrap();
                ui.exec("@original_chat_conversation",r#"
                    contentMessage:AddMessage('[Trade] |kLiora,0;: Fresh lumber is ready at the harbor. Meet us near the ferry.')
                    contentMessage:AddMessage('[Guild] |kAldebaran,0;: The expedition leaves for Crescent Throne after sunset.')
                    contentMessage:AddMessage('Bring the trade packs and a few extra repair hammers.')
                "#).unwrap();
                let r=aa_ui::lua::ui(&ui.lua).tree.rect(message);
                info!("native_chat_content: window={window} message={message} rect={r:?}");
            }
            fake::report(ui,"native_chat_content");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::dump_tree)
        .add_systems(Update,hold_hover.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}

fn hold_hover(ui: Option<NonSend<aa_ui::UiRuntime>>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let Some(ui) = ui else { return };
    let u = aa_ui::lua::ui(&ui.lua);
    let Some(window) = u.tree.of_class("chatwindow").first().copied() else {
        return;
    };
    let Some(tab) = u
        .tree
        .widget(window)
        .and_then(|w| w.chat.as_ref())
        .and_then(|c| c.tabs.first())
    else {
        return;
    };
    let r = u.animations.widget_rect(&u.tree, tab.message);
    pointer.pos = Some(Vec2::new(
        (r.x + r.w * 0.7) * u.ui_scale,
        (r.y + r.h * 0.6) * u.ui_scale,
    ));
}
