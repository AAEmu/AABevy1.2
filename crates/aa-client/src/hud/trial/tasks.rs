//! Received native Tasks6/7/8, with their distinct duplicate and false-callback rules.
use super::*;
use mlua::ObjectLike;
use std::collections::BTreeMap;

#[derive(Clone)]
enum Info {
    Jury(Vec<u8>),
    Imprison { crime: i32, minutes: i32 },
    Skip,
}
#[derive(Clone)]
struct Quote {
    token: u32,
    owner: u32,
    command: TrialCommand,
    info: Info,
    shown: bool,
}
#[derive(Default)]
pub(super) struct Tasks {
    next: u32,
    quotes: BTreeMap<u8, Quote>,
}
pub(super) fn clear(tasks: &mut Tasks) {
    tasks.quotes.clear();
}

fn insert(data: &mut HudData, task: u8, command: TrialCommand, info: Info) {
    let Some(owner) = data.trial.owner.filter(|id| *id != 0) else {
        return;
    };
    let tasks = &mut data.trial.tasks;
    tasks.next = tasks.next.wrapping_add(1).max(1);
    tasks.quotes.insert(
        task,
        Quote {
            token: tasks.next,
            owner,
            command,
            info,
            shown: false,
        },
    );
}
pub(super) fn ignored(lua: &Lua) -> bool {
    lua.load("return X2Option:GetOptionItemValue(OPTION_ITEM_IGNORE_JURY_INVITATION)==1")
        .eval()
        .unwrap_or(false)
}
fn choose(state: &Shared, token: u32, accepted: bool) -> Option<i16> {
    let mut data = state.borrow_mut();
    let (&task, quote) = data.trial.tasks.quotes.iter().find(|(_, quote)| quote.token == token)?;
    if data.trial.owner != Some(quote.owner) {
        return None;
    }
    let quote = data.trial.tasks.quotes.remove(&task)?;
    let mut refused_world = false;
    let command = match quote.command {
        TrialCommand::ReplyImprisonOrTrial { .. } => Some(TrialCommand::ReplyImprisonOrTrial { request_trial: accepted }),
        TrialCommand::ReplyJuryInvitation { trial_id, .. } => {
            let accept = accepted && data.trial.login_started && data.trial.main_world == Some(true);
            refused_world = accepted && !accept;
            Some(TrialCommand::ReplyJuryInvitation { accept, trial_id })
        }
        command @ TrialCommand::SkipFinalStatement { .. } => accepted.then_some(command),
        _ => None,
    };
    if let Some(command) = command {
        queue_captured(&mut data, quote.owner, command);
    }
    refused_world.then_some(544)
}
fn show_error(lua: &Lua, code: i16) -> mlua::Result<()> {
    let Some(key) = super::super::alerts::error_name(code) else {
        return Ok(());
    };
    let locale: Table = lua.globals().get("X2Locale")?;
    let group: Value = lua.globals().get("ERROR_MSG")?;
    let message: Value = locale.call_method("LocalizeUiText", (group, key))?;
    aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![message]);
    Ok(())
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    lua.load("X2Option:CreateOptionItemFloat(OPTION_ITEM_IGNORE_JURY_INVITATION,0)")
        .exec()?;
    let hud: Table = lua.globals().get("HUD")?;
    let s = state.clone();
    hud.raw_set(
        "trial_choose",
        lua.create_function(move |lua, (token, accepted): (u32, bool)| {
            if let Some(code) = choose(&s, token, accepted) {
                show_error(lua, code)?;
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    hud.raw_set(
        "trial_visual_failed",
        lua.create_function(move |_, token: u32| {
            if let Some(quote) = s.borrow_mut().trial.tasks.quotes.values_mut().find(|quote| quote.token == token) {
                quote.shown = false;
            }
            Ok(())
        })?,
    )?;
    lua.load(include_str!("tasks.lua")).set_name("@trial_tasks.lua").exec()
}
pub(super) fn received(lua: Option<&Lua>, state: &Shared, event: &Event) {
    let mut data = state.borrow_mut();
    let ignore = data.trial.ignore_jury;
    match event {
        Event::Trial(aa_net::TrialEvent::ImprisonOrTrialOffered { crime_value, jail_minutes }) => {
            //171 has an earlier explicit duplicate7 return, without a false choice.
            if !data.trial.tasks.quotes.contains_key(&7) {
                insert(
                    &mut data,
                    7,
                    TrialCommand::ReplyImprisonOrTrial { request_trial: false },
                    Info::Imprison {
                        crime: *crime_value,
                        minutes: *jail_minutes,
                    },
                );
            }
        }
        Event::Trial(aa_net::TrialEvent::JuryInvited { defendant_name, trial_id }) => {
            let command = TrialCommand::ReplyJuryInvitation {
                accept: false,
                trial_id: *trial_id,
            };
            // Native CanStart rejection calls the INCOMING captured task's false callback.
            if ignore || data.trial.tasks.quotes.contains_key(&6) {
                if let Some(owner) = data.trial.owner {
                    queue_captured(&mut data, owner, command);
                }
            } else {
                insert(&mut data, 6, command, Info::Jury(defendant_name.clone()));
            }
        }
        _ => {}
    }
    drop(data);
    if let Some(lua) = lua {
        show(lua, state);
    }
}
fn close(lua: Option<&Lua>, state: &Shared, task: u8) -> mlua::Result<()> {
    let quote = state.borrow().trial.tasks.quotes.get(&task).cloned();
    let Some(quote) = quote else { return Ok(()) };
    if quote.shown
        && let Some(lua) = lua
    {
        let hud: Table = lua.globals().get("HUD")?;
        let close: mlua::Function = hud.raw_get("trial_close_task")?;
        close.call::<()>(task)?;
        return Ok(());
    }
    // Actual native cef0 runs false before removal even if the app's HUD is still loading.
    if let Some(code) = choose(state, quote.token, false)
        && let Some(lua) = lua
    {
        show_error(lua, code)?;
    }
    Ok(())
}
pub(super) fn notice(lua: Option<&Lua>, state: &Shared, notice: TrialNotice) -> mlua::Result<()> {
    match notice {
        TrialNotice::SkipFinalStatementTask { trial_id } => {
            let mut data = state.borrow_mut();
            // Task8 CanStart also retains the old queued task on duplicate; new false sends nothing.
            if !data.trial.tasks.quotes.contains_key(&8) {
                insert(&mut data, 8, TrialCommand::SkipFinalStatement { trial_id }, Info::Skip);
            }
        }
        TrialNotice::CloseSkipFinalStatementTask => close(lua, state, 8)?,
        TrialNotice::CloseTrialTasks => {
            close(lua, state, 6)?;
            close(lua, state, 7)?;
        }
        _ => {}
    }
    if let Some(lua) = lua {
        show(lua, state);
    }
    Ok(())
}
pub(super) fn show(lua: &Lua, state: &Shared) {
    let quotes = {
        let mut data = state.borrow_mut();
        let owner = data.trial.owner;
        data.trial
            .tasks
            .quotes
            .iter_mut()
            .filter_map(|(&task, quote)| {
                if quote.shown || owner != Some(quote.owner) {
                    return None;
                }
                quote.shown = true;
                Some((task, quote.clone()))
            })
            .collect::<Vec<_>>()
    };
    for (task, quote) in quotes {
        let result = (|| -> mlua::Result<()> {
            let info = lua.create_table()?;
            info.raw_set("buttonType", 0)?;
            info.raw_set("isModal", task != 6)?;
            match quote.info {
                Info::Jury(name) => info.raw_set("defendantName", text(lua, &name)?)?,
                Info::Imprison { crime, minutes } => {
                    info.raw_set("crimeValue", signed(crime))?;
                    info.raw_set("jailMinutes", signed(minutes))?;
                }
                Info::Skip => {
                    let locale: Table = lua.globals().get("X2Locale")?;
                    info.raw_set(
                        "title",
                        locale.call_method::<String>("LocalizeUiText", (28, "skip_final_statement"))?,
                    )?;
                    info.raw_set(
                        "content",
                        locale.call_method::<String>("LocalizeUiText", (29, "skip_final_statement"))?,
                    )?;
                }
            }
            let hud: Table = lua.globals().get("HUD")?;
            hud.raw_get::<mlua::Function>("trial_show_task")?.call((task, quote.token, info))
        })();
        if let Err(error) = result {
            if let Some(pending) = state
                .borrow_mut()
                .trial
                .tasks
                .quotes
                .get_mut(&task)
                .filter(|pending| pending.token == quote.token)
            {
                pending.shown = false;
            }
            warn!("trial: task: {error}");
        }
    }
}
