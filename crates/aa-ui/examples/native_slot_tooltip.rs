//! Original action-slot hover with static snapshot (--snapshot) or lazy tooltip data.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua, tree::Id};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime, snapshot: bool) -> Id {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.lua.globals().set("SCORE_SNAPSHOT", snapshot).unwrap();
    ui.exec(
        "@slot_tooltip_fixture",
        include_str!("fixtures/slot_tooltip.lua"),
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    lua::value_id(&ui.lua, &ui.lua.globals().get("SCORE_SLOT").unwrap()).unwrap()
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original slot tooltip provider"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui, std::env::args().any(|a| a == "--snapshot"));
            fake::report(ui, "native_slot_tooltip");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(
            Update,
            sequence.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
        .run();
}

fn sequence(
    ui: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
    mut phase: Local<u8>,
) {
    let Some(ui) = ui else { return };
    let id = lua::value_id(&ui.lua, &ui.lua.globals().get("SCORE_SLOT").unwrap()).unwrap();
    let (p, away) = {
        let u = lua::ui(&ui.lua);
        let r = u.tree.rect(id);
        (
            Vec2::new((r.x + r.w / 2.) * u.ui_scale, (r.y + r.h / 2.) * u.ui_scale),
            Vec2::new(1100., 100.),
        )
    };
    let t = time.elapsed_secs();
    pointer.pos = Some(if t < 0.6 || (4.0..4.3).contains(&t) {
        away
    } else {
        p
    });
    if *phase == 0 && t >= 2.5 {
        ui.exec("@score_arrival", "SCORE_RECEIVED=true").unwrap();
        *phase = 1;
        info!("slot tooltip fixture: score available; stationary tooltip retained");
    }
    if *phase == 1 && t >= 5.8 {
        let calls: i32 = ui.lua.globals().get("SCORE_TOOLTIP_CALLS").unwrap();
        info!("slot tooltip fixture: rehover complete, queries={calls}");
        *phase = 2;
    }
}
