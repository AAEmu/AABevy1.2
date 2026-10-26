//! Original skill-upgrade notifier: a new entry while scrolled retains its full range.
#[path = "fake/slider_range.rs"]
mod original;
use aa_ui::UiRuntime;
use bevy::prelude::*;

fn exercise(ui: Option<NonSendMut<UiRuntime>>, time: Res<Time>, mut done: Local<bool>) {
    let Some(ui) = ui else { return };
    if *done || time.elapsed_secs() < 1.0 {
        return;
    }
    let r = original::visible_range_update(&ui);
    *done = true;
    info!(
        "native-slider-range: original SKILL_UPGRADED16064, realwheel old={}, stored_max={}, expected_max={}, last_bottom={}, clip_height={}, reachable=true",
        r[0], r[1], r[2], r[3], r[4]
    );
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original notifier range refit"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(original::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            original::install(ui);
            ui.exec("@range_visual_scale", "UIParent:SetUIScale(.85)")
                .unwrap();
        }))
        .add_systems(Startup, original::physical_pixels)
        .add_systems(Update, exercise)
        .run();
}
