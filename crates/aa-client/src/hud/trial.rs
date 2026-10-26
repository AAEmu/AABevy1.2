//! Original trial UI over ordered received snapshots, captured requests and explicit local ticks.
use super::api::{HudBridge, HudData, Shared};
use aa_net::{Event, NetCommand, NetEvent, TrialCommand, TrialNotice, Trials};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use web_time::{SystemTime, UNIX_EPOCH};
mod bounty;
#[allow(dead_code)]
pub(super) mod fixture;
mod getters;
mod tasks;
#[cfg(test)]
mod tests;

enum UiEffect {
    Bounty(aa_net::BountyNotice, aa_net::Bounties),
    Notice(TrialNotice, Trials),
    ForcedMethod(i32, Trials),
}

#[derive(Default)]
pub(super) struct View {
    received: Trials,
    callback: Option<Trials>,
    owner: Option<u32>,
    requests: Vec<(u32, TrialCommand)>,
    tasks: tasks::Tasks,
    login_started: bool,
    main_world: Option<bool>,
    ignore_jury: bool,
    pending: Vec<UiEffect>,
    bounty: bounty::View,
}
impl View {
    fn current(&self) -> &Trials {
        self.callback.as_ref().unwrap_or(&self.received)
    }
}
fn signed(value: i32) -> f64 {
    f64::from(value as f32)
}
fn unsigned(value: u32) -> f64 {
    f64::from(value as f32)
}
fn text(lua: &Lua, bytes: &[u8]) -> mlua::Result<mlua::LuaString> {
    lua.create_string(bytes.split(|&byte| byte == 0).next().unwrap_or_default())
}
fn integer(value: Option<&Value>) -> Option<i32> {
    let value = match value? {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    Some(if value.is_finite() && (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    })
}
fn now() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| i64::try_from(time.as_secs()).ok())
}
fn queue(data: &mut HudData, command: TrialCommand) {
    let owner = data.character_id;
    if !data.disconnected {
        queue_captured(data, owner, command);
    }
}
fn queue_captured(data: &mut HudData, owner: u32, command: TrialCommand) {
    if owner != 0 && data.trial.owner == Some(owner) && command.validate_received(data.trial.current()).is_ok() {
        data.trial.requests.push((owner, command));
    }
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Trial")?;
    let locale = aa_ui::lua::ui(lua).textures.locale.clone();
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into());
    let texts = aa_ui::uitext::load(std::path::Path::new(&path), &locale).unwrap_or_else(|error| {
        warn!("trial: UI texts unavailable: {error}");
        Default::default()
    });
    let s = state.clone();
    api.raw_set(
        "GetTrialType",
        lua.create_function(move |_, _: MultiValue| {
            Ok(s.borrow()
                .trial
                .current()
                .records_raw_unknown
                .map_or_else(MultiValue::new, |kind| MultiValue::from_iter([Value::Number(signed(kind as i32))])))
        })?,
    )?;
    let s = state.clone();
    let t = texts.clone();
    api.raw_set(
        "GetCrimeData",
        lua.create_function(move |lua, _: MultiValue| getters::crime_data(lua, &s.borrow(), &t))?,
    )?;
    let s = state.clone();
    let record_texts = texts.clone();
    api.raw_set(
        "GetCrimeRecordsByPage",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(page) = integer(args.get(1)) else {
                return Ok(MultiValue::new());
            };
            Ok(MultiValue::from_iter([Value::Table(getters::records(
                lua,
                &s.borrow(),
                &record_texts,
                page,
            )?)]))
        })?,
    )?;
    for method in ["ConfirmCrimeRecords", "CancelTrial", "ChooseVerdict"] {
        let s = state.clone();
        api.raw_set(
            method,
            lua.create_function(move |_, args: MultiValue| {
                let mut data = s.borrow_mut();
                let received = data.trial.current();
                let command = match method {
                    "ConfirmCrimeRecords" => received.jury_number.map(|jury_number| TrialCommand::ConfirmCrimeRecords {
                        trial_id: received.trial_id,
                        jury_number,
                    }),
                    "CancelTrial" => Some(TrialCommand::CancelTrial {
                        trial_id: received.trial_id,
                    }),
                    _ => integer(args.get(1)).filter(|choice| (1..=6).contains(choice)).and_then(|choice| {
                        received.jury_number.map(|jury_number| TrialCommand::ChooseVerdict {
                            trial_id: received.trial_id,
                            jury_number,
                            choice: choice as u8,
                        })
                    }),
                };
                if let Some(command) = command {
                    queue(&mut data, command);
                }
                Ok(())
            })?,
        )?;
    }
    tasks::install(lua, state)?;
    bounty::install(lua, state, texts)
}

fn emit(rt: Option<&UiRuntime>, state: &Shared, notice: TrialNotice) -> mlua::Result<()> {
    match notice {
        TrialNotice::SkipFinalStatementTask { .. } | TrialNotice::CloseSkipFinalStatementTask | TrialNotice::CloseTrialTasks => {
            return tasks::notice(rt.map(|rt| &rt.lua), state, notice);
        }
        _ => {}
    }
    let Some(rt) = rt else { return Ok(()) };
    let number = |value: f64| Value::Number(value);
    let (event, args) = match notice {
        TrialNotice::BeautyshopStatus => ("UPDATE_INGAME_BEAUTYSHOP_STATUS", vec![]),
        TrialNotice::Status { phase, current_jury } => ("TRIAL_STATUS", vec![number(unsigned(phase.into())), number(signed(current_jury))]),
        TrialNotice::ShowCrimeRecords { phase } => {
            if !getters::ready(state.borrow().trial.current()) {
                // Native reads uninitialized fields outside a coherent receipt sequence. Refuse that domain.
                warn!("trial: crime-record UI refused incomplete received history/details/type");
                return Ok(());
            }
            ("SHOW_CRIME_RECORDS", vec![number(unsigned(phase.into()))])
        }
        TrialNotice::Verdicts { minutes } => ("SHOW_VERDICTS", minutes.into_iter().map(|value| number(signed(value))).collect()),
        TrialNotice::Waiting { order, sentence_time_ms } => (
            "SHOW_DEPENDANT_WAIT_TRIAL",
            vec![number(signed(order as i32)), number(unsigned(sentence_time_ms as u32))],
        ),
        TrialNotice::JuryWaiting {
            count,
            total,
            sentence_time_ms,
        } => (
            "SHOW_DEPENDANT_WAIT_JURY",
            vec![
                number(signed(count)),
                number(signed(total)),
                number(unsigned(sentence_time_ms as u32)),
            ],
        ),
        TrialNotice::JuryOkCount { count, total } => ("JURY_OK_COUNT", vec![number(signed(count)), number(signed(total))]),
        TrialNotice::VerdictCount { count, total } => ("VERDICT_COUNT", vec![number(signed(count)), number(signed(total))]),
        TrialNotice::RulingStatus(status) => {
            let data = state.borrow();
            if data.trial.current().defendant.is_none() || data.trial.current().crime_kind_counts.is_none() {
                warn!("trial: ruling UI refused unknown received defendant/counts");
                return Ok(());
            }
            (
                "RULING_STATUS",
                vec![
                    number(signed(status.count)),
                    number(signed(status.total)),
                    number(unsigned(status.sentence_type.into())),
                    number(unsigned(status.sentence_time_ms as u32)),
                ],
            )
        }
        TrialNotice::AudienceJoined { name } => ("AUDIENCE_JOINED", vec![Value::String(text(&rt.lua, &name)?)]),
        TrialNotice::AudienceLeft { name } => ("AUDIENCE_LEFT", vec![Value::String(text(&rt.lua, &name)?)]),
        TrialNotice::RulingClosed => ("RULING_CLOSED", vec![]),
        TrialNotice::Closed => ("TRIAL_CLOSED", vec![]),
        TrialNotice::Canceled => ("TRIAL_CANCELED", vec![]),
        TrialNotice::Timer { phase, remaining_seconds } => (
            "TRIAL_TIMER",
            vec![
                number(unsigned(phase.into())),
                Value::Table(super::period::table(&rt.lua, remaining_seconds as u64)?),
            ],
        ),
        _ => unreachable!("task notices handled above"),
    };
    aa_ui::lua::fire_ui_event(&rt.lua, event, args);
    Ok(())
}

fn apply_ui(rt: &UiRuntime, state: &Shared, effect: UiEffect) {
    let (notice, forced, snapshot) = match effect {
        UiEffect::Bounty(notice, snapshot) => {
            bounty::apply_ui(rt, state, notice, snapshot);
            return;
        }
        UiEffect::Notice(notice, snapshot) => (Some(notice), None, snapshot),
        UiEffect::ForcedMethod(method, snapshot) => (None, Some(method), snapshot),
    };
    state.borrow_mut().trial.callback = Some(snapshot);
    let result = if let Some(method) = forced {
        aa_ui::lua::chat_input::set_forced_method(&rt.lua, method);
        Ok(())
    } else {
        emit(Some(rt), state, notice.unwrap())
    };
    state.borrow_mut().trial.callback = None;
    if let Err(error) = result {
        warn!("trial: callback: {error}");
    }
}
fn deliver(rt: Option<&UiRuntime>, state: &Shared, effect: UiEffect) {
    if let Some(rt) = rt {
        apply_ui(rt, state, effect);
    } else {
        state.borrow_mut().trial.pending.push(effect);
    }
}
fn replay(rt: &UiRuntime, state: &Shared) {
    let pending = std::mem::take(&mut state.borrow_mut().trial.pending);
    for effect in pending {
        apply_ui(rt, state, effect);
    }
}

fn lifecycle(event: &Event) -> bool {
    matches!(
        event,
        Event::CharacterSelected(_)
            | Event::ReadyToConnect { .. }
            | Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting)
    )
}
fn dispatch_at(rt: Option<&UiRuntime>, state: &Shared, event: &Event, epoch: Option<i64>) {
    if let Some(rt) = rt {
        state.borrow_mut().trial.ignore_jury = tasks::ignored(&rt.lua);
    }
    let forced = match event {
        Event::Trial(aa_net::TrialEvent::SummonJury { .. } | aa_net::TrialEvent::SummonDefendant { .. }) => Some(11),
        Event::Trial(aa_net::TrialEvent::RulingClosed) => Some(-6),
        Event::Trial(aa_net::TrialEvent::AudienceLeft { obj_id, name })
            if name.len() <= 128 && *obj_id == state.borrow().trial.received.local_obj_id.unwrap_or(0) =>
        {
            Some(-6)
        }
        _ => None,
    };
    if let Some(method) = forced {
        let snapshot = state.borrow().trial.received.clone();
        deliver(rt, state, UiEffect::ForcedMethod(method, snapshot));
    }
    let reset = lifecycle(event);
    let notices = {
        let mut data = state.borrow_mut();
        let view = &mut data.trial;
        if reset {
            view.owner = None;
            view.requests.clear();
            view.pending.clear();
            tasks::clear(&mut view.tasks);
        }
        if let Event::CharacterSelected(selected) = event {
            view.owner = Some(selected.character.id);
        }
        if let Event::PlayerSpawn(spawn) = event {
            view.owner = Some(spawn.character().id);
        }
        view.received.apply_with_dispatch_at(event, epoch).1
    };
    if reset && let Some(rt) = rt {
        let _ = rt.exec("@trial_stale_tasks", "HUD.trial_close_tasks()");
    }
    for (notice, snapshot) in notices {
        if matches!(
            notice,
            TrialNotice::SkipFinalStatementTask { .. } | TrialNotice::CloseSkipFinalStatementTask | TrialNotice::CloseTrialTasks
        ) {
            state.borrow_mut().trial.callback = Some(snapshot);
            if let Err(error) = emit(rt, state, notice) {
                warn!("trial: task: {error}");
            }
            state.borrow_mut().trial.callback = None;
        } else {
            deliver(rt, state, UiEffect::Notice(notice, snapshot));
        }
    }
    tasks::received(rt.map(|rt| &rt.lua), state, event);
    bounty::dispatch(rt, state, event);
}
fn flush(data: &mut HudData) -> Vec<aa_net::Command> {
    let mut commands: Vec<_> = std::mem::take(&mut data.trial.requests)
        .into_iter()
        .filter_map(|(owner, command)| {
            (data.trial.owner == Some(owner) && command.validate_received(&data.trial.received).is_ok()).then_some(aa_net::Command::Trial {
                character_id: owner,
                command,
            })
        })
        .collect();
    commands.extend(bounty::flush(data));
    commands
}
fn context(
    state: NonSend<HudBridge>,
    started: Option<Res<crate::login::NativeLoginStarted>>,
    screen: Res<State<crate::Screen>>,
    world: Option<Res<aa_world::stream::WorldInfo>>,
    config: Option<Res<aa_world::WorldConfig>>,
) {
    let mut data = state.0.borrow_mut();
    data.trial.login_started = started.is_some_and(|started| started.0);
    // The loaded main-world metadata exists during login too. Only the actual
    // in-game layer supplies the Seamless current-layer predicate to this HUD.
    data.trial.main_world = match (world, config) {
        (Some(world), Some(config)) if *screen.get() == crate::Screen::InGame && world.0.name == config.world => {
            Some(world.0.name == "main_world")
        }
        _ => None,
    };
}
fn track(mut events: MessageReader<NetEvent>, state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    if let Some(rt) = rt.as_deref() {
        state.0.borrow_mut().trial.ignore_jury = tasks::ignored(&rt.lua);
    }
    let rt = if built.up { rt.as_deref() } else { None };
    if let Some(rt) = rt {
        replay(rt, &state.0);
    }
    for NetEvent(event) in events.read() {
        dispatch_at(rt, &state.0, event, now());
    }
    let timer = now().and_then(|now| state.0.borrow_mut().trial.received.advance_time(now));
    if let Some(timer) = timer
        && let Err(error) = emit(rt, &state.0, timer)
    {
        warn!("trial: timer: {error}");
    }
}
fn update(state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>, mut out: MessageWriter<NetCommand>) {
    if built.up
        && let Some(rt) = rt
    {
        tasks::show(&rt.lua, &state.0);
    }
    out.write_batch(flush(&mut state.0.borrow_mut()).into_iter().map(NetCommand));
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, context.before(track).before(aa_ui::UiInput))
        .add_systems(
            Update,
            track
                .before(aa_ui::UiInput)
                .before(super::sync_hud)
                .before(super::prebuild_hud)
                .before(super::build_hud),
        )
        .add_systems(Update, update.after(super::sync_hud).after(aa_ui::UiFrame));
}
