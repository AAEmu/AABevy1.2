//! Original chat dividers after physical tab selection and signed line offsets.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-ui: native selected chat dividers"),aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin{config:Some(fake::config())})
        .insert_resource(aa_ui::UiSetup::new(|ui|{
            auto_clip::install(ui);
            ui.exec("@native_chat_lines",r#"
                nativeClipChat:SetExtent(430,180)
                nativeClipChat:SetLeftLineOffset(-8)
                nativeClipChat:SetRightLineOffset(12)
            "#).unwrap();
            let (button,message)={let u=aa_ui::lua::ui(&ui.lua);let window=u.tree.of_class("chatwindow")[0];let tab=&u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[1];(tab.button,tab.message)};
            let r={let u=aa_ui::lua::ui(&ui.lua);u.animations.widget_rect(&u.tree,button)};
            let p=(r.x+r.w*0.5,r.y+r.h*0.5);
            let mut mouse=Default::default();
            aa_ui::input::update(&ui.lua,&mut mouse,Some(p),true,false,false,0.);
            aa_ui::input::update(&ui.lua,&mut mouse,Some(p),false,true,false,0.);
            ui.lua.globals().set("lineMessage",aa_ui::lua::object_table(&ui.lua,message).unwrap().unwrap()).unwrap();
            ui.exec("@native_chat_lines_conversation",r#"
                lineMessage:AddMessage('[Guild] |kLiora,0;: Meet us at Crescent Throne after sunset.')
                lineMessage:AddMessage('The expedition is ready to sail.')
                HideTooltip()
            "#).unwrap();
            fake::report(ui,"native_chat_lines");
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
    let Some(c) = u.tree.widget(window).and_then(|w| w.chat.as_ref()) else {
        return;
    };
    let Some(tab) = c.tabs.get(c.selected) else {
        return;
    };
    let r = u.animations.widget_rect(&u.tree, tab.message);
    pointer.pos = Some(Vec2::new(
        (r.x + r.w * 0.7) * u.ui_scale,
        (r.y + r.h * 0.6) * u.ui_scale,
    ));
}
