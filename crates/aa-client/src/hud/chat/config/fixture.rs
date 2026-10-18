//! Original chat configuration controls over explicit local-only native records.
use super::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(in crate::hud) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::super::breath::fixture::runtime(width, height)?;
    state.borrow_mut().chat_config.identity(Some(17), 12345);
    ensure(&rt.lua, &state)?;
    rt.lua
        .load(
            r#"
        assert(X2Chat:AddNewChatTabByUser(0,'Harbor'))
        local tab=X2Chat:GetChatTabInfoTable(3)
        tab.fontSize=17;tab.filters[2]=true
        tab.bgColor={r=.04,g=.06,b=.08,a=.8}
        tab.filterColors[2]={r=.78,g=.92,b=1,a=1}
        assert(X2Chat:UpdateChatTabInfo(3,tab))
    "#,
        )
        .exec()?;
    for (who, text) in [
        ("Sailor", "The harbor is quiet tonight."),
        ("Quartermaster", "Supplies are ready for departure."),
        ("Sailor", "This tab keeps its own filters, colours and font size."),
    ] {
        rt.fire_event("CHAT_MESSAGE", (0, "200", false, who, text, false, None::<String>));
    }
    Ok((rt, state, path))
}

pub(in crate::hud) fn show_options(rt: &UiRuntime) -> mlua::Result<()> {
    rt.lua.load("ShowChatOptionWindow(3)").exec()?;
    aa_ui::lua::flush(&rt.lua);
    // Finish opening and retire the previous hidden instance before resolving its reused name.
    aa_ui::lua::tick(&rt.lua, 600.);
    let widget = ui(&rt.lua).tree.find("chatOptionWindow").expect("original chat options");
    rt.lua
        .globals()
        .set("CHAT_OPTIONS_FIXTURE", object_table(&rt.lua, widget)?.unwrap())?;
    Ok(())
}

pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    if !std::env::args().any(|a| a == "--chat") {
        show_options(&rt).unwrap();
    }
    if std::env::args().any(|a| a == "--style") {
        rt.lua
            .load("CHAT_OPTIONS_FIXTURE.menuListWindow.menu[4]:OnClick('LeftButton')")
            .exec()
            .unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original chat settings"), aa_dev::DevPlugin))
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
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
