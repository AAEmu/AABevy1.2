//! Original chat at its default 1280x720 UI scale, then the original option step.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

#[derive(Resource)]
struct OptionStep {
    value: Option<u32>,
    applied: bool,
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let step = args
        .windows(2)
        .find(|a| a[0] == "--option-step")
        .and_then(|a| a[1].parse().ok());
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native scaled chat clipping"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(OptionStep {
            value: step,
            applied: false,
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            auto_clip::install(ui);
            fake::report(ui, "native_auto_clip_scale");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .add_systems(
            Update,
            hold_hover.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
        .run();
}

fn hold_hover(
    time: Res<Time>,
    mut step: ResMut<OptionStep>,
    ui: Option<NonSend<aa_ui::UiRuntime>>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
) {
    let Some(ui) = ui else { return };
    if !step.applied && time.elapsed_secs() >= 3.0 {
        if let Some(value) = step.value {
            ui.exec(
                "@original_ui_option_step",
                &format!("UIParent:SetUIScale(GetUIScaleValueByOptionWindowValue({value}),true)"),
            )
            .unwrap();
        }
        step.applied = true;
        let u = aa_ui::lua::ui(&ui.lua);
        info!(
            "native_auto_clip_scale: ui_scale={} script_errors={}",
            u.ui_scale,
            ui.errors.len()
        );
    }
    let u = aa_ui::lua::ui(&ui.lua);
    let Some(window) = u.tree.of_class("chatwindow").first().copied() else {
        return;
    };
    let Some(tab) = u
        .tree
        .widget(window)
        .and_then(|w| w.chat.as_ref())
        .and_then(|c| c.tabs.first())
    else {
        return;
    };
    let r = u.animations.widget_rect(&u.tree, tab.button);
    pointer.pos = Some(Vec2::new(
        (r.x + r.w * 0.5) * u.ui_scale,
        (r.y + r.h * 0.5) * u.ui_scale,
    ));
}
