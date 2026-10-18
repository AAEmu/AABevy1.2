//! Original World-stage map widgets, populated by explicit published radar fixture data.
//! cargo run -p aa-ui --example native_radar -- --shot shots/aa-ui/native-radar.png

#[path = "fake/mod.rs"]
mod fake;
#[path = "fake/radar.rs"]
mod radar_fixture;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original radar map markers"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            radar_fixture::install(ui);
            fake::report(ui, "native_radar");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(
            Update,
            (point_at_ship.before(aa_ui::UiInput), fake::dump_tree),
        )
        .run();
}

fn point_at_ship(
    ui: Option<NonSend<aa_ui::UiRuntime>>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
) {
    let Some(ui) = ui else { return };
    let Ok(value) = ui.lua.globals().raw_get::<mlua::Value>("worldmap") else {
        return;
    };
    let Some(id) = aa_ui::lua::value_id(&ui.lua, &value) else {
        return;
    };
    let r = aa_ui::lua::ui(&ui.lua).tree.rect(id);
    pointer.pos = Some(Vec2::new(r.x + r.w * 0.32, r.y + r.h * 0.44));
}
