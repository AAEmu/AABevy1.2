//! Original player HP loss, captured during its native 0.3-second after-image animation.

#[path = "fake/mod.rs"]
mod fake;
#[path = "fake/healthbar.rs"]
mod healthbar;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native HP animation"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            healthbar::install(ui);
            ui.exec(
                "@native_healthbar_position",
                r#"
                nativeHealthFrame:RemoveAllAnchors()
                nativeHealthFrame:AddAnchor('CENTER',UIParent,0,0)
            "#,
            )
            .unwrap();
            fake::report(ui, "native_healthbar_animation");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, damage.before(aa_ui::UiFrame))
        .add_systems(Update, fake::dump_tree)
        .run();
}

fn damage(time: Res<Time>, runtime: Option<NonSend<aa_ui::UiRuntime>>, mut done: Local<bool>) {
    // aa-dev's default shot is at 8s: show the authentic after-image near its midpoint.
    if !*done
        && time.elapsed_secs() >= 7.85
        && let Some(runtime) = runtime
    {
        healthbar::lose_health(&runtime);
        *done = true;
    }
}
