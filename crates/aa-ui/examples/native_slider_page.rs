//! The original Sound Options quality Slider after one positive-page track press.
#[path = "fake/slider_page.rs"]
mod original;
use aa_ui::{UiRuntime, input, lua};
use bevy::prelude::*;

fn page_once(ui: Option<NonSendMut<UiRuntime>>, time: Res<Time>, mut done: Local<bool>) {
    let Some(ui) = ui else { return };
    if *done || time.elapsed_secs() < 1.0 {
        return;
    }
    *done = true;
    let r: [f32; 4] = ui.lua.load("local x,y=quality:GetEffectiveOffset();local w,h=quality:GetEffectiveExtent();return {x,y,w,h}").eval().unwrap();
    let scale = lua::ui(&ui.lua).ui_scale;
    let right = ((r[0] + r[2] - 0.5) / scale, (r[1] + r[3] / 2.0) / scale);
    let left = ((r[0] + 0.5) / scale, right.1);
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(right), true, false, false, 0.0);
    input::update(&ui.lua, &mut mouse, Some(left), false, false, false, 0.0);
    input::update(&ui.lua, &mut mouse, Some(left), false, true, false, 0.0);
    ui.exec("@original_page_save", "quality:Save();assert(quality:GetValue()==3 and GetOptionItemValue(OPTION_ITEM_SOUND_QUALITY)==3 and qualityChanges==1)").unwrap();
    info!(
        "native-slider-page: original Init2 -> track press3, original Save3, callbacks1; held movement did not capture"
    );
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original Sound Options page press"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(original::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| original::install_visual(ui, 0.85)))
        .add_systems(Startup, original::physical_pixels)
        .add_systems(Update, page_once)
        .run();
}
