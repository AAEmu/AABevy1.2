//! Explicit received fixtures for the unchanged original ranking and reward windows.
use super::*;
use aa_net::{RankCharacterRow, RankSnapshot, RankSnapshotEntry, RankSnapshotScope};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(width, height)?;
    state.borrow_mut().character_id = 47;
    let mut selected = aa_net::model::CharacterState::default();
    selected.character.id = 47;
    dispatch(Some(&rt.lua), &state, &Event::CharacterSelected(Box::new(selected)));
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.unit.name = "Breath fixture".into();
    dispatch(Some(&rt.lua), &state, &Event::PlayerSpawn(Box::new(spawn)));
    dispatch(
        Some(&rt.lua),
        &state,
        &Event::WorldList(aa_net::login::WorldList {
            worlds: vec![aa_net::login::World {
                id: 1,
                name: "Fixture world".into(),
                online: true,
                congestion: 0,
                race_congestion: [0; 9],
            }],
            characters: vec![],
        }),
    );
    rt.lua
        .load("RANK_FIXTURE=ADDON:GetContent(UIC_RANK);ADDON:ShowContent(UIC_BAG,false);ADDON:ShowContent(UIC_RANK,true)")
        .exec()?;
    let rows = || {
        (1..=3)
            .map(|rank_id| RankCharacterRow {
                account_id: 1,
                account: b"fixture".to_vec(),
                character_id: 47,
                rank_id,
                world_id: 1,
                v1: 123,
                raw_sum_v1: 0,
                v2: 9,
                raw_sum_v2: 0,
                recorded_raw: 1_700_000_000,
            })
            .collect::<Vec<_>>()
    };
    let personal = || RankingEvent::Personal {
        type_id: 0,
        error: 0,
        ack_time_raw: 1_700_000_000,
        count: 3,
        rows: rows(),
    };
    dispatch(Some(&rt.lua), &state, &Event::Ranking(personal()));
    let seconds = state.borrow().start.elapsed().as_secs_f32();
    frame(Some(&rt.lua), &state, seconds + 4.);
    aa_ui::lua::flush(&rt.lua);
    aa_ui::lua::tick(&rt.lua, 600.);
    let args: Vec<String> = std::env::args().collect();
    let rank_id = if args.iter().any(|arg| arg == "--fish-length") {
        super::super::expansion::fixture::click(&rt, "RANK_FIXTURE.tab.unselectedButton[2]")?;
        dispatch(Some(&rt.lua), &state, &Event::Ranking(personal()));
        2
    } else {
        1
    };
    for (id, name) in [(42, b"Received rival".to_vec()), (43, b"Received runner".to_vec())] {
        dispatch(
            Some(&rt.lua),
            &state,
            &Event::CharacterName(aa_net::CharacterName { character_id: id, name }),
        );
    }
    let entries = [(47, 1, 444), (42, 2, 333), (43, 3, 222)]
        .into_iter()
        .map(|(character_id, ranking, v1)| RankSnapshotEntry {
            character_id,
            rank_id,
            world_id: 1,
            v1,
            v2: 9,
            updated_raw: 1_700_000_000,
            ranking,
        })
        .collect::<Vec<_>>();
    let scopes = (1..=10)
        .map(|scope_id| RankSnapshotScope {
            scope_id,
            v1: (11 - scope_id) * 100,
            v2: 0,
        })
        .collect::<Vec<_>>();
    dispatch(
        Some(&rt.lua),
        &state,
        &Event::Ranking(RankingEvent::Snapshot {
            rank_id,
            error: 0,
            board: RankSnapshot {
                board_id: 77,
                updated_raw: 1_700_000_000,
                reset_raw: 1_699_999_000,
                entry_count: entries.len() as i32,
                entries,
                scope_count: scopes.len() as i32,
                scopes,
            },
        }),
    );
    frame(Some(&rt.lua), &state, seconds + 5.);
    aa_ui::lua::flush(&rt.lua);
    aa_ui::lua::tick(&rt.lua, 600.);
    if args.iter().any(|arg| arg == "--rewards") {
        super::super::expansion::fixture::click(&rt, "RANK_FIXTURE.content.upperFrame.rewardButton")?;
        aa_ui::lua::flush(&rt.lua);
        aa_ui::lua::tick(&rt.lua, 600.);
    }
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok((rt, state, path))
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original ranking controls"), aa_dev::DevPlugin))
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
