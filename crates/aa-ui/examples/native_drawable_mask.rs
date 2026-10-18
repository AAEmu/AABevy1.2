//! Unchanged original item buttons: physical hover and independent cooldown masks.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_button_colors.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, UiVirtualPointer, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    original::install(ui);
    ui.exec("@original_drawable_mask",r#"
        panel.heading:SetText('Original item hover and independent cooldown masks')
        panel.columnTitle[1]:SetText('Normal + cooldown')
        panel.columnTitle[2]:SetText('Dimmed hover + cooldown')
        panel.columnTitle[3]:SetText('Highlighted icon')
        panel.columnValue[1]:SetText('Own cooldown mask')
        panel.columnValue[2]:SetText('Hover mask + own sweep')
        panel.columnValue[3]:SetText('Native slot_over.dds')
        panel.note:SetText('Original 42 px icons. Hover masks apply to each image layer; cooldown masks stay independent.')
        for i=1,2 do
            colorButtons[i]:SetCoolDownMask('ui/icon/cooldown_mask.dds')
            colorButtons[i]:SetCoolDown(8000,12000)
        end
        colorButtons[3]:SetButtonState('HIGHLIGHTED')
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
}

fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
) {
    pointer.pos = Some(Vec2::ZERO);
    if time.elapsed_secs() >= 6.
        && let Some(runtime) = runtime
    {
        let id = lua::value_id(
            &runtime.lua,
            &runtime.lua.load("colorButtons[2]").eval().unwrap(),
        )
        .unwrap();
        let u = lua::ui(&runtime.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        pointer.pos = Some(Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5) * u.ui_scale);
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native drawable masks"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, pointer.before(aa_ui::UiFrame))
        .run();
}
