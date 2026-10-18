//! Native bounty tables, ordered receipts and weak captured controls; no settlement optimism.
use super::*;
use aa_net::{Bounties, BountyCommand, BountyEvent, BountyNotice};
use aa_ui::uitext::UiTexts;
use std::sync::Arc;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct View {
    received: Bounties,
    callback: Option<Bounties>,
    // Fold actual actor receipts in order so a later removal cannot erase an earlier callback.
    actors: aa_net::state::Units,
    requests: Vec<(u32, BountyCommand)>,
}
impl View {
    fn current(&self) -> &Bounties {
        self.callback.as_ref().unwrap_or(&self.received)
    }
}
fn number(value: Option<&Value>) -> Option<f32> {
    match value? {
        Value::Integer(value) => Some(*value as f32),
        Value::Number(value) => Some(*value as f32),
        _ => None,
    }
}
fn queue(data: &mut HudData, command: BountyCommand) {
    let owner = data.character_id;
    if !data.disconnected && owner != 0 && data.trial.owner == Some(owner) && command.validate().is_ok() {
        data.trial.bounty.requests.push((owner, command));
    }
}
pub(super) fn install(lua: &Lua, state: &Shared, texts: Arc<UiTexts>) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Trial")?;
    let s = state.clone();
    api.raw_set(
        "GetBountyData",
        lua.create_function(move |lua, _: MultiValue| {
            let data = s.borrow();
            let out = lua.create_table()?;
            for row in data.trial.bounty.current().by_id.values() {
                let entry = lua.create_table()?;
                entry.raw_set(1, unsigned(row.player_id))?;
                entry.raw_set(2, text(lua, &row.name)?)?;
                entry.raw_set(3, unsigned(row.level.into()))?;
                entry.raw_set(4, signed(row.zone_id))?;
                entry.raw_set(5, signed(row.money as i32))?;
                entry.raw_set(6, signed(row.crime_points.into()))?;
                // The native current-world spatial registry is still an explicit adapter boundary.
                entry.raw_set(7, "")?;
                entry.raw_set(8, getters::race(lua, &texts, row.race)?)?;
                entry.raw_set(9, getters::faction(lua, &data, row.faction_id)?)?;
                for (index, value) in [
                    row.arrest_count,
                    row.accept_guilty_count,
                    row.accept_trial_count,
                    row.first_verdict_count,
                    row.second_verdict_count,
                    row.seconds,
                ]
                .into_iter()
                .enumerate()
                {
                    entry.raw_set(index + 10, signed(value))?;
                }
                out.raw_push(entry)?;
            }
            Ok((out, unsigned(data.trial.bounty.current().by_id.len() as u32)))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "RequestSetBountyMoney",
        lua.create_function(move |_, args: MultiValue| {
            if let Some(Value::String(name)) = args.get(1)
                && let Ok(command) = BountyCommand::request_set_money(&name.as_bytes())
            {
                queue(&mut s.borrow_mut(), command);
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "SendBountyUpdate",
        lua.create_function(move |_, args: MultiValue| {
            if let (Some(kind), Some(Value::String(name)), Some(money)) = (number(args.get(1)), args.get(2), number(args.get(3)))
                && let Ok(command) = BountyCommand::update_from_lua(kind, &name.as_bytes(), money)
            {
                queue(&mut s.borrow_mut(), command);
            }
            Ok(())
        })?,
    )
}
pub(super) fn apply_ui(rt: &UiRuntime, state: &Shared, notice: BountyNotice, snapshot: Bounties) {
    state.borrow_mut().trial.bounty.callback = Some(snapshot);
    if let Err(error) = emit(rt, notice) {
        warn!("trial bounty: callback: {error}");
    }
    state.borrow_mut().trial.bounty.callback = None;
}
fn emit(rt: &UiRuntime, notice: BountyNotice) -> mlua::Result<()> {
    let number = |value: f64| Value::Number(value);
    let money = |value: i64| number(signed(value as i32));
    let (event, args) = match notice {
        BountyNotice::ToggleBulletin => ("TOGGLE_BOUNTY_BULLETIN", vec![]),
        BountyNotice::Set { kind, name, money: amount } => (
            "SET_BOUNTY",
            vec![number(unsigned(kind)), Value::String(text(&rt.lua, &name)?), money(amount)],
        ),
        BountyNotice::Done { kind, name, money: amount } => (
            "SET_BOUNTY_DONE",
            vec![
                number(unsigned(kind)),
                Value::String(text(&rt.lua, &name)?),
                money(amount),
                number(1.),
            ],
        ),
        BountyNotice::Paid {
            hunter,
            arrested,
            money: amount,
        } => (
            "BOUNTY_PAID",
            vec![
                Value::String(text(&rt.lua, &hunter)?),
                Value::String(text(&rt.lua, &arrested)?),
                money(amount),
                number(1.),
            ],
        ),
        BountyNotice::UnitMoney {
            obj_id,
            name,
            money: amount,
            is_local,
        } => (
            "SET_BOUNTY_DONE",
            vec![
                number(unsigned(obj_id)),
                Value::String(text(&rt.lua, name.as_bytes())?),
                money(amount),
                number(if is_local { 1. } else { 0. }),
            ],
        ),
        BountyNotice::Error(code) => {
            if let Ok(code) = i16::try_from(code) {
                tasks::show_error(&rt.lua, code)?;
            }
            return Ok(());
        }
    };
    aa_ui::lua::fire_ui_event(&rt.lua, event, args);
    Ok(())
}
pub(super) fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let (notices, snapshot) = {
        let mut data = state.borrow_mut();
        let local_obj_id = data.trial.received.local_obj_id;
        let view = &mut data.trial.bounty;
        if lifecycle(event) {
            view.requests.clear();
            view.actors = Default::default();
        }
        view.actors.apply(event);
        if let Event::PlayerSpawn(spawn) = event {
            view.actors.by_id.insert(spawn.obj_id, spawn.unit.clone());
        }
        let mut notices = view.received.apply_with_notices(event).1;
        if let Event::Bounty(receipt @ BountyEvent::UnitMoney { obj_id, .. }) = event
            && let Some(unit) = view.actors.get(*obj_id)
            && let Some(notice) = Bounties::unit_money_notice(receipt, unit, local_obj_id)
        {
            notices.push(notice);
        }
        let snapshot = (!notices.is_empty()).then(|| view.received.clone());
        (notices, snapshot)
    };
    if let Some(snapshot) = snapshot {
        for notice in notices {
            deliver(rt, state, UiEffect::Bounty(notice, snapshot.clone()));
        }
    }
}
pub(super) fn flush(data: &mut HudData) -> Vec<aa_net::Command> {
    std::mem::take(&mut data.trial.bounty.requests)
        .into_iter()
        .filter_map(|(owner, command)| {
            (data.trial.owner == Some(owner) && command.validate().is_ok()).then_some(aa_net::Command::Bounty {
                character_id: owner,
                command,
            })
        })
        .collect()
}
