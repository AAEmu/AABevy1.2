//! Original mode-bar Lua over explicit piano and memorized-score receipts, offline.
use bevy::prelude::*;

pub(in crate::hud) fn run() {
    let (rt, state, path) = crate::hud::user_music::fixture::runtime().unwrap();
    let before = std::env::args().any(|a| a == "--before");
    if before {
        // Explicit control of the former unimplemented getter; both runs fold
        // the same real catalog buff data. This is not an old-binary capture.
        rt.lua.load("X2Unit.GetModeActionsCount=function() return 0 end;HUD.mode_slot=function() return nil end").exec().unwrap();
    }
    for (id, index) in [(5068, 9), (6010, 10)] {
        crate::hud::enchant::dispatch(
            Some(&rt),
            &state,
            &aa_net::Event::BuffCreated {
                obj_id: 100,
                buff: aa_net::buff::Buff {
                    buff_id: id,
                    index,
                    ..default()
                },
            },
            None,
        );
    }
    assert_eq!(super::actions(&state.borrow()).len(), 11);
    assert_eq!(
        rt.lua
            .load("return X2Unit:GetModeActionsCount()")
            .eval::<usize>()
            .unwrap(),
        if before { 0 } else { 11 }
    );
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original received mode actions"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(state))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
