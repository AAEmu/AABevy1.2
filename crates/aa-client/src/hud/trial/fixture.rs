//! Explicit trial receipts drive the unchanged original user-trial scripts.
use super::*;
use aa_net::{TrialCrimeRecord, TrialDefendantData, TrialEvent as E, TrialHistory, TrialRecords};
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn history() -> TrialHistory {
    TrialHistory {
        player_id: 99,
        crime_point: 60,
        arrest: 4,
        accept_guilty: 1,
        accept_trial: 3,
        not_guilty: 1,
        guilty: 2,
        total_seconds: 5400,
        current_seconds: 0,
        crime_record_count: 5,
        bot_report: 2,
    }
}
pub(super) fn row(id: u32, kind: u8) -> TrialCrimeRecord {
    TrialCrimeRecord {
        id,
        victim_id: 48,
        victim_name: b"Witness\0unused".to_vec(),
        reporter_id: 49,
        reporter_name: b"Reporter\0unused".to_vec(),
        victim_faction_id: 148,
        victim_expedition_id: 0,
        crime_kind: kind,
        doodad_template_id: 0,
        used_skill_id: 0,
        position_x: 0,
        position_y: 0,
        position_z_bits: 0,
        message: b"A received crime record".to_vec(),
        report_time: 1_700_000_000,
    }
}
pub(super) fn send(rt: Option<&UiRuntime>, state: &Shared, event: E) {
    dispatch_at(rt, state, &Event::Trial(event), Some(1_700_000_000));
}
pub(super) fn selected(rt: Option<&UiRuntime>, state: &Shared) {
    state.borrow_mut().character_id = 47;
    let mut header = aa_net::model::CharacterState::default();
    header.character.id = 47;
    dispatch_at(rt, state, &Event::CharacterSelected(Box::new(header)), Some(1_700_000_000));
    dispatch_at(
        rt,
        state,
        &Event::PlayerSpawn(Box::new(super::super::expansion::fixture::spawn())),
        Some(1_700_000_000),
    );
    state.borrow_mut().trial.login_started = true;
    state.borrow_mut().trial.main_world = Some(true);
}
pub(super) fn records(rt: Option<&UiRuntime>, state: &Shared) {
    send(
        rt,
        state,
        E::SummonJury {
            trial_id: 7,
            court: 4,
            jury_number: 2,
        },
    );
    send(rt, state, E::History(history()));
    send(
        rt,
        state,
        E::Records(TrialRecords {
            trial_id: 7,
            raw_unknown: 0,
            total_count: 5,
            count: 5,
            records: (0..5).map(|id| row(id + 1, [3, 2, 4, 1, 3][id as usize])).collect(),
        }),
    );
    send(
        rt,
        state,
        E::DefendantData(TrialDefendantData {
            defendant_id: 50,
            name: b"Received defendant\0unused".to_vec(),
            race: 1,
            faction_id: 148,
            trial_id: 999,
            sentence_time_ms: 1_800_000,
            judge_obj_id: 101,
        }),
    );
    send(
        rt,
        state,
        E::PhaseChanged {
            trial_id: 7,
            phase: 3,
            current_jury: 5,
            remaining_time_ms: 60_000,
        },
    );
}
pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(width, height)?;
    selected(Some(&rt), &state);
    rt.lua.load("ADDON:ShowContent(UIC_BAG,false)").exec()?;
    let args: Vec<String> = aa_dev::args();
    if args.iter().any(|arg| arg == "--bounty-empty") {
        dispatch_at(
            Some(&rt),
            &state,
            &Event::Bounty(aa_net::BountyEvent::List {
                total: 0,
                declared_count: 0,
                rows: vec![],
            }),
            Some(1_700_000_000),
        );
    } else if args.iter().any(|arg| arg == "--bounty-setup") {
        dispatch_at(
            Some(&rt),
            &state,
            &Event::Bounty(aa_net::BountyEvent::SetPermitted {
                kind: 50,
                name: b"Received defendant".to_vec(),
                money: 12345,
                raw_allowed: 1,
            }),
            Some(1_700_000_000),
        );
        // Explicit standalone original function probe: SET_BOUNTY has no original listener.
        rt.lua.load("ShowSetupBountyWindow(50,'Received defendant',12345)").exec()?;
    } else if args.iter().any(|arg| arg == "--waiting") {
        send(Some(&rt), &state, E::SummonDefendant { trial_id: 9 });
        send(
            Some(&rt),
            &state,
            E::Waiting {
                order: 17,
                sentence_time_ms: 1_800_000,
            },
        );
        send(
            Some(&rt),
            &state,
            E::JuryWaiting {
                count: 0,
                total: 5,
                sentence_time_ms: 1_800_000,
            },
        );
    } else if args.iter().any(|arg| arg == "--invitation") {
        send(
            Some(&rt),
            &state,
            E::JuryInvited {
                defendant_name: b"Received defendant".to_vec(),
                trial_id: 7,
            },
        );
    } else {
        records(Some(&rt), &state);
        if args.iter().any(|arg| arg == "--verdict") {
            send(
                Some(&rt),
                &state,
                E::PhaseChanged {
                    trial_id: 7,
                    phase: 5,
                    current_jury: 5,
                    remaining_time_ms: 60_000,
                },
            );
        }
    }
    aa_ui::lua::flush(&rt.lua);
    aa_ui::lua::tick(&rt.lua, 600.);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok((rt, state, path))
}
pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = runtime(width as f32, height as f32).unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original trial controls"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, tick.before(aa_ui::UiFrame))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
fn tick(time: Res<Time>, rt: NonSend<UiRuntime>, state: NonSend<HudBridge>) {
    let epoch = 1_700_000_000 + time.elapsed_secs_f64() as i64;
    let timer = state.0.borrow_mut().trial.received.advance_time(epoch);
    if let Some(timer) = timer {
        emit(Some(&rt), &state.0, timer).unwrap();
    }
}
fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
