//! Original Options controls with isolated local settings and no server connection.
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::Table;

pub(super) fn prepare(rt: &UiRuntime) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!("aa-bindings-fixture-{}-{unique}.json", std::process::id()));
    let store = Arc::new(Mutex::new(super::super::Store::open(path.clone())));
    let net: Table = rt.lua.globals().get("NET")?;
    super::super::install_store(&rt.lua, &net, store.clone())?;
    super::install(&rt.lua, store)?;
    open(rt)?;
    Ok(path)
}

pub(super) fn open(rt: &UiRuntime) -> mlua::Result<()> {
    select_page(rt, 8)?;
    rt.lua
        .load(
            r#"
        BINDING_PAGE=optionFrame.contentWindow.pageWindow.subFrames[8]
        BINDING_PRIMARY=nil
        for _,row in ipairs(BINDING_PAGE.content.optionFrames) do
          if row.primaryKeyEdit and row.primaryKeyEdit.action=='moveforward' then BINDING_PRIMARY=row.primaryKeyEdit; break end
        end
        assert(BINDING_PRIMARY)
    "#,
        )
        .set_name("@original_bindings_fixture")
        .exec()
}

fn select_page(rt: &UiRuntime, page: i64) -> mlua::Result<()> {
    rt.lua
        .load(
            r#"
        local page=...
        ToggleOptionFrame(true)
        local clicked=false
        for _,category in ipairs(optionFrame.contentWindow.menuListFrame.categoryFrames) do
          for _,button in ipairs(category.buttons) do
            if button.subFrameIndex==page then button:OnClick('LeftButton'); clicked=true; break end
          end
        end
        assert(clicked)
    "#,
        )
        .set_name("@original_bindings_page")
        .call(page)
}

pub(crate) fn run(rt: UiRuntime) {
    let path = prepare(&rt).unwrap();
    // The checked-in real-client reference shows Game Keybinds (page10).
    select_page(&rt, 10).unwrap();
    let bridge = Rc::new(RefCell::new(crate::hud::api::HudData::default()));
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original key bindings"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(bridge))
        .init_resource::<crate::settings::Actions>()
        .init_resource::<crate::hud::HudPointerOverUi>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, crate::settings::update_input.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
