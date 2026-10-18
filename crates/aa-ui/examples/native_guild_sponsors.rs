//! Original guild creation's six Nuia/five Haranya sponsor controls.
#[path = "fake/mod.rs"]
mod fake;
#[path = "fake/guild_sponsors.rs"]
mod guild_sponsors;

use bevy::prelude::*;

fn main() {
    let alliance = if std::env::args().any(|a| a == "--harihara") {
        149
    } else {
        148
    };
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original guild sponsors"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(move |ui| {
            guild_sponsors::install(ui, alliance);
            // Select the last active original button through the physical pointer path.
            let button = ui
                .lua
                .load("return sponsorTab.unselectedButton[sponsorTab:GetActivateTabCount()]")
                .eval::<mlua::Value>()
                .unwrap();
            let button = aa_ui::lua::value_id(&ui.lua, &button).unwrap();
            let r = {
                let u = aa_ui::lua::ui(&ui.lua);
                u.animations.widget_rect(&u.tree, button)
            };
            let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
            assert_eq!(aa_ui::input::pick(&ui.lua, p.0, p.1), Some(button));
            let mut mouse = Default::default();
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
            ui.exec(
                "@original_guild_sponsor_selection",
                "assert(sponsorTab:GetSelectedTab()==sponsorTab:GetActivateTabCount())",
            )
            .unwrap();
            fake::report(ui, "native_guild_sponsors");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .run();
}
