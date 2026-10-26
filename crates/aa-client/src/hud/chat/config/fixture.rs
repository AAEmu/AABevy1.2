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
    show_options_for(rt, 3)
}
pub(super) fn show_options_for(rt: &UiRuntime, raw_id: i32) -> mlua::Result<()> {
    rt.lua.globals().get::<mlua::Function>("ShowChatOptionWindow")?.call::<()>(raw_id)?;
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
    let locked = aa_dev::args().into_iter().any(|a| a == "--locked");
    let reset = aa_dev::args().into_iter().any(|a| a == "--reset");
    if locked {
        context_action(&rt).unwrap();
    }
    if reset {
        show_options_for(&rt, 0).unwrap();
        crate::hud::expansion::fixture::click(&rt, "CHAT_OPTIONS_FIXTURE.chatResetButton").unwrap();
        aa_ui::lua::tick(&rt.lua, 600.);
        for (who, text) in [
            ("Sailor", "Chat reset restored the original All, Party and Trade tabs."),
            ("Quartermaster", "Previous history, custom styles and window locks were cleared."),
        ] {
            rt.fire_event("CHAT_MESSAGE", (0, "200", false, who, text, false, None::<String>));
        }
    } else if !locked && !aa_dev::args().into_iter().any(|a| a == "--chat") {
        show_options(&rt).unwrap();
    }
    if !reset && aa_dev::args().into_iter().any(|a| a == "--style") {
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
        .add_systems(Update, lock_preview.before(aa_ui::UiFrame).run_if(move || locked))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
pub(super) fn context_action(rt: &UiRuntime) -> Result<(), Box<dyn std::error::Error>> {
    let window = live_window(&rt.lua, 0).unwrap();
    let (button, point) = {
        let u = ui(&rt.lua);
        let button = u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].button;
        let r = u.tree.rect(button);
        (button, (r.x + r.w * 0.5, r.y + r.h * 0.5))
    };
    assert_eq!(aa_ui::input::pick(&rt.lua, point.0, point.1), Some(button));
    rt.lua.globals().set("CHAT_LOCK_TAB", object_table(&rt.lua, button)?.unwrap())?;
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, true, 0.);
    crate::hud::expansion::fixture::click(rt, "CHAT_LOCK_TAB:GetAttachedWidget().buttons[1]")
}
pub(super) fn drag_attempt(rt: &UiRuntime) {
    let window = live_window(&rt.lua, 0).unwrap();
    let p = {
        let u = ui(&rt.lua);
        let button = u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].button;
        let r = u.tree.rect(button);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    let mut mouse = aa_ui::input::MouseState::default();
    for (point, down, up) in [
        (p, true, false),
        ((p.0 + 5., p.1), false, false),
        ((p.0 + 30., p.1 + 15.), false, false),
        ((p.0 + 30., p.1 + 15.), false, true),
    ] {
        aa_ui::input::update(&rt.lua, &mut mouse, Some(point), down, up, false, 0.);
    }
}
fn lock_preview(rt: NonSend<UiRuntime>, time: Res<Time>, mut last: Local<f32>) {
    if time.elapsed_secs() - *last >= 2. {
        *last = time.elapsed_secs();
        drag_attempt(&rt);
    }
}
fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
