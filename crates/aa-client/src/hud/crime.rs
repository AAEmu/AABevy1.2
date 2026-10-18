//! Native crime reputation/jury getters and callbacks over received selected-character state.
use super::api::{HudBridge, Shared};
use aa_net::{CrimeNotice, Crimes, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table};
#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct View {
    pub(super) received: Crimes,
    requests: Vec<u32>,
}
fn signed(value: impl Into<i64>) -> f64 {
    f64::from(value.into() as f32)
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let player: Table = lua.globals().get("X2Player")?;
    let s = state.clone();
    player.raw_set(
        "GetCrimeInfo",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let out = lua.create_table()?;
            if let Some(reputation) = data.crime.received.reputation {
                out.raw_set("crimePoint", signed(reputation.crime_points))?;
                out.raw_set("crimeRecord", signed(reputation.infamy_points))?;
                out.raw_set("crimeScore", signed(reputation.crime_state))?;
            }
            // Native assumes a selected header exists. Before that valid domain, keep
            // fields unknown while original constructors can safely inspect the table.
            Ok(out)
        })?,
    )?;
    let s = state.clone();
    player.raw_set(
        "GetJuryPoint",
        lua.create_function(move |_, _: MultiValue| {
            let data = s.borrow();
            let point = data
                .crime
                .received
                .jury_point
                .or_else(|| (data.character_id == 0 || data.disconnected).then_some(0));
            Ok(point.map_or_else(MultiValue::new, |point| MultiValue::from_iter([mlua::Value::Number(signed(point))])))
        })?,
    )?;
    let trial: Table = lua.globals().get("X2Trial")?;
    let s = state.clone();
    trial.raw_set(
        "RequestJuryWaitingNumber",
        lua.create_function(move |_, _: MultiValue| {
            let mut data = s.borrow_mut();
            let id = data.character_id;
            if id != 0 && !data.disconnected && data.crime.received.character_id == Some(id) {
                data.crime.requests.push(id);
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

pub(super) fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let notices = {
        let mut data = state.borrow_mut();
        if matches!(
            event,
            Event::CharacterSelected(_)
                | Event::ReadyToConnect { .. }
                | Event::LoginSucceeded(_)
                | Event::LeftWorld { .. }
                | Event::Disconnected { .. }
                | Event::Kicked { .. }
                | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting)
        ) {
            data.crime.requests.clear();
        }
        data.crime.received.apply_with_notices(event).1
    };
    let Some(rt) = rt else { return };
    for notice in notices {
        match notice {
            CrimeNotice::Reported {
                crime_delta,
                infamy_delta,
                crime_state_delta,
            } => rt.fire_event(
                "CRIME_REPORTED",
                (signed(crime_delta), signed(infamy_delta), signed(crime_state_delta)),
            ),
            CrimeNotice::JuryPoint => rt.fire_event("PLAYER_JURY_POINT", ()),
            CrimeNotice::JuryWaitingNumber(number) => rt.fire_event("JURY_WAITING_NUMBER", signed(number)),
        }
    }
}
fn track(mut events: MessageReader<NetEvent>, state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    for NetEvent(event) in events.read() {
        dispatch(if built.up { rt.as_deref() } else { None }, &state.0, event);
    }
}
fn requests(state: NonSend<HudBridge>, mut out: MessageWriter<NetCommand>) {
    out.write_batch(flush(&mut state.0.borrow_mut()).into_iter().map(NetCommand));
}
fn flush(data: &mut super::api::HudData) -> Vec<aa_net::Command> {
    let requests = std::mem::take(&mut data.crime.requests);
    let mut commands = Vec::new();
    for id in requests {
        if id == data.character_id && !data.disconnected && data.crime.received.character_id == Some(id) {
            commands.push(aa_net::Command::RequestJuryWaitingNumber { character_id: id });
        }
    }
    commands
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track.before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud),
    )
    .add_systems(Update, requests.after(super::sync_hud).after(aa_ui::UiInput));
}
