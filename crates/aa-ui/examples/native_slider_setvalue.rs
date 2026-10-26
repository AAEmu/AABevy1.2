//! Original full Sound page smoke plus strict same-value provider-application assertions.
#[path = "fake/slider_setvalue.rs"]
mod original;
use aa_ui::UiRuntime;
use bevy::prelude::*;

fn exercise(ui: Option<NonSendMut<UiRuntime>>, time: Res<Time>, mut done: Local<bool>) {
    let Some(ui) = ui else { return };
    if *done || time.elapsed_secs() < 1.0 {
        return;
    }
    original::init(&ui);
    let records = original::application_report(&ui);
    *done = true;
    info!(
        "native-slider-setvalue: complete original Sound.Init at .85, unchanged values10/options1, actual_provider_records={records:?}"
    );
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original Sound value notification"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(original::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            original::install(ui);
            original::show_original_sound(ui);
        }))
        .add_systems(Startup, original::physical_pixels)
        .add_systems(Update, exercise)
        .run();
}
