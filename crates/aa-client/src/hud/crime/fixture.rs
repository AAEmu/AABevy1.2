//! Original character crime/jury labels, driven only by explicit selected-header/receipt fixtures.
use super::*;
use aa_ui::lua::{object_table, ui};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn header() -> aa_net::model::CharacterState {
    let mut state = aa_net::model::CharacterState::default();
    state.character.id = 47;
    state.character.crime_point = 6;
    state.character.infamy_point = 12;
    state.character.crime_state = 1;
    state.jury_point = 8;
    state
}
pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(width, height)?;
    state.borrow_mut().character_id = 47;
    dispatch(Some(&rt), &state, &Event::CharacterSelected(Box::new(header())));
    dispatch(
        Some(&rt),
        &state,
        &Event::Crime(aa_net::CrimeEvent::Changed {
            points: 99,
            crime_points: 60,
            infamy_points: 1234,
            crime_state: 2,
        }),
    );
    dispatch(
        Some(&rt),
        &state,
        &Event::Crime(aa_net::CrimeEvent::JuryPointChanged { jury_point: 31 }),
    );
    rt.lua
        .load("ADDON:ShowContent(UIC_BAG,false);ADDON:ShowContent(UIC_CHARACTER_INFO,true)")
        .exec()?;
    aa_ui::lua::flush(&rt.lua);
    aa_ui::lua::tick(&rt.lua, 600.);
    for (name, alias) in [
        ("character.window.crimePoint", "CRIME_FIXTURE"),
        ("character.window.juryPoint", "JURY_FIXTURE"),
    ] {
        let widget = ui(&rt.lua).tree.find(name).ok_or("original character label missing")?;
        rt.lua
            .globals()
            .set(alias, object_table(&rt.lua, widget)?.ok_or("original label object missing")?)?;
    }
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok((rt, state, path))
}

pub(super) fn hover(rt: &UiRuntime) -> Result<aa_ui::input::MouseState, Box<dyn std::error::Error>> {
    let value = rt.lua.load("JURY_FIXTURE.iconWnd").eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &value).ok_or("jury icon missing")?;
    let rect = ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(
        aa_ui::input::pick(&rt.lua, point.0, point.1),
        Some(id),
        "actual original jury icon must receive the hit"
    );
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
    Ok(mouse)
}

pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    let waiting = std::env::args().any(|arg| arg == "--waiting");
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original crime and jury"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, waiting_preview.before(aa_ui::UiFrame).run_if(move || waiting))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
// The toolkit's virtual pointer drives its normal input/render path. Reply to
// the previous frame's actual icon-enter request with an explicit fixture receipt.
fn waiting_preview(rt: NonSend<UiRuntime>, state: NonSend<HudBridge>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let value = rt.lua.load("JURY_FIXTURE.iconWnd").eval().unwrap();
    let id = aa_ui::lua::value_id(&rt.lua, &value).unwrap();
    let rect = ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    let scale = ui(&rt.lua).ui_scale;
    pointer.pos = Some(Vec2::new(point.0 * scale, point.1 * scale));
    let commands = flush(&mut state.0.borrow_mut());
    if !commands.is_empty() {
        assert!(matches!(
            commands.as_slice(),
            [aa_net::Command::RequestJuryWaitingNumber { character_id: 47 }]
        ));
        dispatch(
            Some(&rt),
            &state.0,
            &Event::Crime(aa_net::CrimeEvent::JuryWaitingNumber { waiting_number: 17 }),
        );
    }
}
fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.));
        window.resolution.set_physical_resolution(width, height);
    }
}
