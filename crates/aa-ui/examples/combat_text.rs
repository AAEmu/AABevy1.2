//! Original combat text Lua and native damagedisplay timeline, with projected fixture targets.
//! `cargo run -p aa-ui --example combat_text -- --shot shots/aa-ui/animation/combat-text.png --shot-after 4.25`

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::UiRuntime;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: native combat text"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            ui.exec(
                "@projected_targets",
                r#"
                AAUI.CombatTextPosition=function(source,target)
                    local x=960
                    if target=='basic' then x=600 elseif target=='critical' then x=1250 end
                    return {x=x,y=400}
                end
            "#,
            )
            .unwrap();
            fake::report(ui, "combat_text");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, (burst, fake::scripted_input))
        .run();
}

fn burst(rt: Option<NonSend<UiRuntime>>, time: Res<Time>, mut previous: Local<u32>) {
    let Some(rt) = rt else { return };
    let beat = (time.elapsed_secs() / 2.0) as u32;
    if beat == 0 || beat == *previous {
        return;
    }
    *previous = beat;
    rt.fire_event(
        "COMBAT_TEXT",
        ("source", "basic", -22, "SWING", "HIT", 0, false, 0.0),
    );
    rt.fire_event(
        "COMBAT_TEXT",
        (
            "source", "critical", -128, "SWING", "CRITICAL", 1, false, 0.0,
        ),
    );
    let own: String = rt
        .lua
        .load("return X2Unit:GetUnitId('player')")
        .eval()
        .unwrap();
    rt.fire_event("UNIT_COMBAT_STATE_CHANGED", (true, own));
}
