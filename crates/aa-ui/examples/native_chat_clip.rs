//! Original fixed chat tabs crop at the internal area, preserving the full tooltip/body.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native chat tab clipping area"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            auto_clip::install(ui);
            ui.exec(
                "@native_chat_clip",
                r#"
                nativeClipChat:UseAutoResizingTabButtonMode(false)
                nativeClipChat:SetTabWidth(220);nativeClipChat:SetExtent(345,180)
                nativeClipChat:SetLeftLineOffset(-8);nativeClipChat:SetRightLineOffset(12)
                HideTooltip()
            "#,
            )
            .unwrap();
            let (button, message) = {
                let u = aa_ui::lua::ui(&ui.lua);
                let window = u.tree.of_class("chatwindow")[0];
                let tab = &u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[1];
                (tab.button, tab.message)
            };
            let r = {
                let u = aa_ui::lua::ui(&ui.lua);
                u.animations.widget_rect(&u.tree, button)
            };
            let p = (r.x + 20., r.y + r.h * 0.5);
            let mut mouse = Default::default();
            assert_eq!(aa_ui::input::pick(&ui.lua, p.0, p.1), Some(button));
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
            ui.lua
                .globals()
                .set(
                    "clipMessage",
                    aa_ui::lua::object_table(&ui.lua, message).unwrap().unwrap(),
                )
                .unwrap();
            ui.exec(
                "@native_chat_clip_body",
                r#"
                clipMessage:AddMessage('[Guild] |kLiora,0;: Meet us at the harbor after sunset.')
                clipMessage:AddMessage('The expedition is ready to sail.')
            "#,
            )
            .unwrap();
            fake::report(ui, "native_chat_clip");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .add_systems(
            Update,
            hold_hover.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
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
    let r = u.animations.widget_rect(&u.tree, tab.button);
    pointer.pos = Some(Vec2::new(
        (r.x + 20.) * u.ui_scale,
        (r.y + r.h * 0.5) * u.ui_scale,
    ));
}
