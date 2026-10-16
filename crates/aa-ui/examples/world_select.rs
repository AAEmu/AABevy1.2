//! World (server) select screen from the client's original Lua UI, with three fake servers.
//!
//! ```text
//! cargo run -p aa-ui --example world_select -- --shot shots/aa-ui/world_select.png
//! cargo run -p aa-ui --example world_select -- --worlds ref0 --dump data/extract/aa-ui/ws_tree.json --exit-after 4
//! ```
//!
//! `--worlds ref0|ref1` matches the reference captures (one server); `--dump` writes the tree in
//! the format of `reference/world_select/ui.json`.

#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    let locale = "en_us";
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: world select"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin::default())
        .insert_resource(aa_ui::UiSetup::new(move |ui| {
            fake::install(ui, locale, &fake::worlds_from_args());
            ui.run_stage(aa_ui::Stage::WorldSelect);
            ui.fire_event("ENTERED_WORLD_SELECT", ());
            ui.fire_event("SHOW_SERVER_SELECT_WINDOW", true);
            report(ui);
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, (log_stats, fake::dump_tree))
        .run();
}

fn report(ui: &aa_ui::UiRuntime) {
    let stubs = ui.stub_calls();
    info!("world_select: {} script errors, {} stubbed natives called", ui.errors.len(), stubs.len());
    for (name, n) in stubs.iter().take(60) {
        info!("  stub {name} x{n}");
    }
}

fn log_stats(stats: Res<aa_ui::UiStats>, time: Res<Time>, mut last: Local<f32>) {
    if time.elapsed_secs() - *last > 2.0 {
        *last = time.elapsed_secs();
        info!("aa-ui: quads={} draw_calls={} rebuilds={}", stats.quads, stats.draw_calls, stats.rebuilds);
    }
}
