//! Explicit native buff receipts for original icons and physical right-button controls.
use super::super::super::{api, panels};
use super::*;
use aa_net::buff::Buff;
use aa_ui::UiRuntime;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(in crate::hud) fn runtime(
    width: f32,
    height: f32,
) -> Result<(UiRuntime, Shared, PathBuf, Buffs, PlayerSpawn), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::super::breath::fixture::runtime(width, height)?;
    api::mark_pushed(&state);
    let mut spawn = super::super::super::expansion::fixture::spawn();
    let received = |index, buff_id, kind| Buff {
        index,
        buff_id,
        kind,
        stacks: 1,
        ability_level: 1,
        duration_ms: 60_000,
        ..default()
    };
    spawn.unit.buffs = vec![received(11, 127, BuffKind::Good), received(12, 1029, BuffKind::Good)];
    let mut buffs = Buffs::default();
    buffs.native.set_catalog(state.borrow().game.get().unwrap().native_buffs.clone());
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn.clone())));
    buffs.apply(&Event::BuffCreated {
        obj_id: 100,
        buff: received(13, 127, BuffKind::Unknown),
    });
    buffs.apply(&Event::BuffCreated {
        obj_id: 100,
        buff: received(14, 1000, BuffKind::Unknown),
    });
    {
        let mut data = state.borrow_mut();
        data.character_id = 47;
        present(&mut data, &buffs, &spawn);
    }
    api::push_changes(&rt, &state);
    aa_ui::lua::tick(&rt.lua, 600.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua.load("BUFF_PLAYER_FRAME=ADDON:GetContent(UIC_PLAYER_UNITFRAME)").exec()?;
    rt.lua.load("assert(X2Unit:UnitBuffCount('player')==3);assert(X2Unit:UnitBuff('player',1).buff_id==127);assert(X2Unit:UnitBuff('player',1).stack==2);assert(X2Unit:UnitBuff('player',2).buff_id==1029)").exec()?;
    Ok((rt, state, path, buffs, spawn))
}
pub(super) fn present(data: &mut HudData, buffs: &Buffs, spawn: &PlayerSpawn) {
    refresh(data, Some(spawn), Some(buffs));
    data.panels.buffs = panels::buff_lists(
        data.game.get(),
        buffs.native.of(spawn.obj_id).unwrap_or_else(|| buffs.of(spawn.obj_id)),
        data.start,
    );
}
pub(super) fn charges(rt: &UiRuntime, state: &Shared, buffs: &mut Buffs, spawn: &PlayerSpawn) -> mlua::Result<()> {
    let mut snapshot = spawn.clone();
    snapshot.unit.buffs = [(21, 1011, 99, -5), (22, 127, 0, 99), (23, 1000, 77, 99)]
        .into_iter()
        .map(|(index, buff_id, stacks, charged)| Buff {
            index,
            buff_id,
            stacks,
            charged,
            kind: BuffKind::Good,
            duration_ms: 60_000,
            ability_level: 1,
            ..default()
        })
        .collect();
    buffs.apply(&Event::PlayerSpawn(Box::new(snapshot)));
    present(&mut state.borrow_mut(), buffs, spawn);
    api::push_changes(rt, state);
    aa_ui::lua::tick(&rt.lua, 100.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua.load("assert(X2Unit:UnitBuff('player',1).stack==5);assert(X2Unit:UnitBuff('player',2).stack==nil);assert(X2Unit:UnitBuff('player',3).stack==1);assert(BUFF_PLAYER_FRAME.buffWindow.button[1].stack:GetText()=='x5');assert(BUFF_PLAYER_FRAME.buffWindow.button[2].stack:GetText()=='');assert(BUFF_PLAYER_FRAME.buffWindow.button[3].stack:GetText()=='')").exec()
}
pub(super) fn right_click(rt: &UiRuntime, expression: &str) -> Result<(), Box<dyn std::error::Error>> {
    let object: Value = rt.lua.load(expression).eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("buff fixture widget missing")?;
    let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(
        aa_ui::input::pick(&rt.lua, point.0, point.1),
        Some(id),
        "{expression} must receive the real mouse hit"
    );
    let mut mouse = aa_ui::input::MouseState::default();
    for buttons in [
        aa_ui::input::MouseButtons {
            right_press: true,
            ..default()
        },
        aa_ui::input::MouseButtons {
            right_release: true,
            ..default()
        },
    ] {
        aa_ui::input::update_buttons(&rt.lua, &mut mouse, Some(point), buttons, 0.);
    }
    Ok(())
}
pub(in crate::hud) fn hover(rt: &UiRuntime) -> Result<aa_ui::input::MouseState, Box<dyn std::error::Error>> {
    let point = tooltip_point(rt)?;
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, false, false, 0.);
    Ok(mouse)
}
fn tooltip_point(rt: &UiRuntime) -> Result<(f32, f32), Box<dyn std::error::Error>> {
    let object = rt.lua.load("BUFF_PLAYER_FRAME.buffWindow.button[1]").eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("buff fixture icon missing")?;
    let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(aa_ui::input::pick(&rt.lua, point.0, point.1), Some(id));
    Ok(point)
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path, mut buffs, spawn) = runtime(width as f32, height as f32).unwrap();
    if std::env::args().any(|arg| arg == "--charges") {
        charges(&rt, &state, &mut buffs, &spawn).unwrap();
    } else if std::env::args().any(|arg| arg == "--remove") {
        right_click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[1]").unwrap();
        assert!(matches!(
            flush(&mut state.borrow_mut()).as_slice(),
            [Command::RemoveBuff {
                character_id: 47,
                obj_id: 100,
                buff_index: 13
            }]
        ));
        // This example has no session: the next explicit fixture receipt is the only removal.
        buffs.apply(&Event::BuffRemoved { obj_id: 100, index: 13 });
        present(&mut state.borrow_mut(), &buffs, &spawn);
        api::push_changes(&rt, &state);
    } else if std::env::args().any(|arg| arg == "--system") {
        right_click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[2]").unwrap();
        assert!(flush(&mut state.borrow_mut()).is_empty());
    }
    aa_ui::lua::tick(&rt.lua, 100.);
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original self-buff controls"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(Startup, physical_pixels)
        .add_systems(
            Update,
            tooltip_preview
                .before(aa_ui::UiFrame)
                .run_if(|| std::env::args().any(|arg| arg == "--tooltip")),
        )
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
fn tooltip_preview(rt: NonSend<UiRuntime>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let point = tooltip_point(&rt).unwrap();
    let scale = aa_ui::lua::ui(&rt.lua).ui_scale;
    pointer.pos = Some(Vec2::new(point.0 * scale, point.1 * scale));
}
fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
