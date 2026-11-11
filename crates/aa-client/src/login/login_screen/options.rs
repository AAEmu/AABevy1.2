//! The original Options window over the native login form.
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::{GOLD, GOLD_DIM, host};
use crate::login::{LoginBridge, gameapi};

#[derive(Component)]
pub(super) struct Button;

pub(super) fn spawn(parent: &mut ChildSpawnerCommands, scale: f32) {
    parent
        .spawn((
            Name::new("Settings"),
            Button,
            Node {
                position_type: PositionType::Absolute,
                right: px(143.0 * scale),
                bottom: px(36.0 * scale),
                width: px(46.0 * scale),
                height: px(46.0 * scale),
                border: UiRect::all(px(2.0 * scale)),
                border_radius: BorderRadius::all(px(6.0 * scale)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.06, 0.08, 0.9)),
            BorderColor::all(GOLD_DIM),
        ))
        .with_children(|button| {
            for step in 0..4 {
                button.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(4.0 * scale),
                        height: px(24.0 * scale),
                        ..default()
                    },
                    bevy::ui::UiTransform::from_rotation(Rot2::radians(
                        step as f32 * std::f32::consts::FRAC_PI_4,
                    )),
                    BackgroundColor(GOLD),
                ));
            }
            button
                .spawn((
                    Node {
                        width: px(18.0 * scale),
                        height: px(18.0 * scale),
                        border_radius: BorderRadius::MAX,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(GOLD),
                ))
                .with_child((
                    Node {
                        width: px(8.0 * scale),
                        height: px(8.0 * scale),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.02, 0.06, 0.08)),
                ));
        });
}

pub(super) fn visible(runtime: Option<&UiRuntime>) -> bool {
    runtime.is_some_and(|rt| {
        use mlua::ObjectLike;
        rt.lua
            .globals()
            .get::<mlua::Table>("optionFrame")
            .is_ok_and(|frame| frame.call_method("IsVisible", ()).unwrap_or(false))
    })
}

pub(super) fn open(world: &mut World) {
    let loaded = world.get_non_send::<UiRuntime>().is_some_and(|rt| {
        rt.lua
            .globals()
            .contains_key("ToggleOptionFrame")
            .unwrap_or(false)
    });
    if !loaded {
        let shared = world.non_send::<LoginBridge>().0.clone();
        if !host::switch_stage(world, |rt| {
            if let Err(error) = load(rt, &shared) {
                warn!("login settings: {error}");
            }
        }) {
            return;
        }
    }
    if let Err(error) = world
        .non_send::<UiRuntime>()
        .exec("login settings", "ToggleOptionFrame(true)")
    {
        warn!("login settings: {error}");
    }
}

fn load(rt: &mut UiRuntime, shared: &gameapi::Shared) -> mlua::Result<()> {
    gameapi::install(rt, shared)?;
    crate::settings::install_bindings(&rt.lua)?;
    rt.load_base_libs();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_loads_the_original_options_without_a_session() -> Result<(), Box<dyn std::error::Error>> {
        let pak = std::env::var_os("AA_PAK").unwrap_or_else(|| aa_pak::DEFAULT_PAK.into());
        if !std::path::Path::new(&pak).is_file() {
            return Ok(());
        }
        if !aa_pak::Pak::global().ready(["scriptsbin/x2ui/option/toc.g"]) {
            return Ok(());
        }
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        load(&mut rt, &gameapi::new_shared())?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        assert!(!visible(Some(&rt)));
        rt.exec("open login settings", "ToggleOptionFrame(true)")?;
        assert!(visible(Some(&rt)));
        rt.exec(
            "cancel login settings",
            "optionFrame:Cancel(); ToggleOptionFrame(false)",
        )?;
        assert!(!visible(Some(&rt)));
        Ok(())
    }
}
