//! Unchanged original character creation button: own fade ignores a physical click, then resumes.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, UiVirtualPointer, lua};
use bevy::prelude::*;

fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::CharacterCreate);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
    lua::tick(&ui.lua, 2500.);
    ui.exec("@prepare_own_button_fade", "characterCreate.bgWindow.nextStepBtn:Show(false)")
        .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec(
        "@start_own_button_fade",
        "characterCreate.bgWindow.nextStepBtn:Show(true,100000)",
    )
    .unwrap();
    lua::tick(&ui.lua, 65000.);
    fake::report(ui, "native_button_fading");
}
fn pointer(
    time: Res<Time>,
    runtime: Option<NonSend<UiRuntime>>,
    mut pointer: ResMut<UiVirtualPointer>,
    mut state: Local<u8>,
) {
    pointer.pos = Some(Vec2::ZERO);
    let Some(ui) = runtime else { return };
    if time.elapsed_secs() < 4. {
        return;
    }
    if *state == 2 {
        assert_eq!(
            ui.lua
                .load("characterCreate.state")
                .eval::<String>()
                .unwrap(),
            "Step1"
        );
        info!("Physical original Next retained its hit and ignored its click during its own fade");
        if std::env::args().any(|a| a == "--ready") {
            lua::tick(&ui.lua, 100000.);
            *state = 3;
        } else {
            *state = 6;
        }
    }
    if *state == 5 {
        assert_eq!(
            ui.lua
                .load("characterCreate.state")
                .eval::<String>()
                .unwrap(),
            "Step2"
        );
        info!("Physical original Next advances to Customize after fade completion");
        *state = 6;
    }
    if matches!(*state, 0 | 1 | 3 | 4) {
        let id = lua::value_id(
            &ui.lua,
            &ui.lua
                .load("characterCreate.bgWindow.nextStepBtn")
                .eval()
                .unwrap(),
        )
        .unwrap();
        let (point, scale) = {
            let u = lua::ui(&ui.lua);
            let r = u.animations.widget_rect(&u.tree, id);
            ((r.x + r.w * 0.5, r.y + r.h * 0.5), u.ui_scale)
        };
        assert_eq!(aa_ui::input::pick(&ui.lua, point.0, point.1), Some(id));
        pointer.pos = Some(Vec2::new(point.0, point.1) * scale);
        if matches!(*state, 0 | 3) {
            pointer.press = true;
        } else {
            pointer.release = true;
        }
        *state += 1;
    }
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original button fade input"),
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
