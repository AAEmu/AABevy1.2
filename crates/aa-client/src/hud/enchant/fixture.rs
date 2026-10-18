//! Physical controls of the original empty enchant window; no network session.
use super::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path, _) = super::super::expansion::fixture::runtime(width, height)?;
    // The original bag keeps this child private. Obtain its actual widget
    // identity, then send the physical hit/press/release through aa-ui.
    let button = {
        let ui = aa_ui::lua::ui(&rt.lua);
        let found = ui
            .tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.name.ends_with("gradeEnchantBtn"))
            .map(|(id, _)| id as aa_ui::tree::Id)
            .collect::<Vec<_>>();
        assert_eq!(found.len(), 1, "original bag enchant button identity");
        found[0]
    };
    rt.lua.globals().set(
        "ENCHANT_BAG_BUTTON",
        aa_ui::lua::object_table(&rt.lua, button)?.ok_or("bag enchant button missing")?,
    )?;
    super::super::expansion::fixture::click(&rt, "ENCHANT_BAG_BUTTON")?;
    // The click creates the window lazily; the next normal frame lays out
    // its newly skinned tab buttons before another physical click can occur.
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua
        .load("assert(GetEnchantWindow():IsVisible() and X2Item:IsInEnchantMode())")
        .exec()?;
    Ok((rt, state, path))
}
pub(super) fn select(rt: &UiRuntime, mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let page = match mode {
        "socket" | "unsocket" => 2,
        "gem" => 3,
        _ => 1,
    };
    let selected: i64 = rt.lua.load("GetEnchantWindow().tab:GetSelectedTab()").eval()?;
    if selected != page {
        super::super::expansion::fixture::click(rt, &format!("GetEnchantWindow().tab.unselectedButton[{page}]"))?;
        aa_ui::lua::tick(&rt.lua, 16.);
        aa_ui::lua::flush(&rt.lua);
    }
    if mode == "unsocket" {
        super::super::expansion::fixture::click(rt, "GetEnchantWindow().tab.window[2].tabSubMenu.subMenuButtons[2]")?;
    }
    Ok(())
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    let mode = std::env::args()
        .find_map(|arg| ["socket", "unsocket", "gem"].into_iter().find(|mode| arg == format!("--{mode}")))
        .unwrap_or("grade");
    // Grade is selected by the real bag button already.
    if mode != "grade" {
        select(&rt, mode).unwrap();
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original enchant mode controls"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.));
        window.resolution.set_physical_resolution(width, height);
    }
}
