//! Duels over the received `SCDuel*` packets: the challenge prompt, the start and end events of the
//! original scripts (`STARTED_DUEL` / `ENDED_DUEL`, which refresh the target frame's faction) and the
//! "challenge to duel" action of the target popup (`X2Unit:ChallengeTargetToDuel`).
//!
//! The dialogs are the original's default dialog with its `locale.duel` texts (`duel.lua`); the
//! temporary Red / Blue team factions arrive as `SCUnitFactionChanged` and move the name plates and the
//! target frame through the usual faction relations.
use super::api::{HudBridge, Shared};
use aa_net::state::{Player, Units};
use aa_net::{CharacterNames, DuelNotice, DuelResult, Duels, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table};
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct View {
    pub(super) received: Duels,
    requests: Vec<aa_net::Command>,
    /// The challenger whose prompt was shown (a challenge that arrives before the HUD is up is prompted when it is).
    prompted: Option<u32>,
    /// The results of the duels this session, in order (the script runner reads them).
    pub(super) results: Vec<DuelResult>,
    /// The start countdown that runs (`SCDuelStartCountdown`).
    countdown: Option<Countdown>,
}

/// The fixed length of the client's duel countdown (ms), "Duel will begin in N sec." once a second.
const COUNTDOWN_SECS: u32 = 3;

/// The seconds of the start countdown still to announce.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Countdown {
    elapsed: f32,
    announced: u32,
}

impl Countdown {
    /// Moves the countdown on by `dt` seconds: the remaining seconds to announce now (3 at once, then 2, 1) and
    /// whether it is over.
    pub(super) fn advance(&mut self, dt: f32) -> (Vec<u32>, bool) {
        self.elapsed += dt;
        let due = (self.elapsed.floor() as u32 + 1).min(COUNTDOWN_SECS);
        let said = (self.announced..due).map(|i| COUNTDOWN_SECS - i).collect();
        self.announced = self.announced.max(due);
        (said, self.elapsed >= COUNTDOWN_SECS as f32)
    }
}

/// Who the ids of a packet are, for the texts.
#[derive(Default)]
pub(super) struct Names<'a> {
    pub units: Option<&'a Units>,
    pub names: Option<&'a CharacterNames>,
    pub own_obj: u32,
}

impl Names<'_> {
    fn character(&self, id: u32) -> String {
        let unit = self
            .units
            .and_then(|u| u.iter().find(|u| matches!(u.kind, aa_net::model::UnitKind::Character { char_id } if char_id == id)));
        if let Some(unit) = unit.filter(|u| !u.name.is_empty()) {
            return unit.name.clone();
        }
        self.names.and_then(|n| n.by_id.get(&id)).map_or_else(|| id.to_string(), |n| String::from_utf8_lossy(n).into_owned())
    }
    fn opponent(&self, challenger_obj: u32, challenged_obj: u32) -> String {
        let other = if challenger_obj == self.own_obj { challenged_obj } else { challenger_obj };
        self.units.and_then(|u| u.get(other)).map(|u| u.name.clone()).filter(|n| !n.is_empty()).unwrap_or_default()
    }
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let unit: Table = lua.globals().get("X2Unit")?;
    let s = state.clone();
    unit.raw_set("CanChallengeTargetToDuel", lua.create_function(move |_, _: MultiValue| Ok(target(&s.borrow()).is_some()))?)?;
    let s = state.clone();
    unit.raw_set(
        "ChallengeTargetToDuel",
        lua.create_function(move |_, _: MultiValue| {
            let mut data = s.borrow_mut();
            if let Some(challenged_id) = target(&data) {
                data.duel.requests.push(aa_net::Command::ChallengeDuel { challenged_id });
            }
            Ok(())
        })?,
    )?;
    let hud: Table = lua.globals().get("HUD")?;
    let s = state.clone();
    hud.raw_set(
        "duel_answer",
        lua.create_function(move |_, (challenger_id, accept): (u32, bool)| {
            let mut data = s.borrow_mut();
            if data.duel.received.answered(challenger_id) {
                info!("hud: duel challenge of {challenger_id} {}", if accept { "accepted" } else { "refused" });
                data.duel.requests.push(aa_net::Command::AnswerDuel { challenger_id, accept });
            }
            Ok(())
        })?,
    )?;
    // The Options window's "ignore duel invitations" checkbox is numeric option 78; its native default is off.
    lua.load("X2Option:CreateOptionItemFloat(OPTION_ITEM_IGNORE_DUEL_INVITATION,0)").exec()?;
    lua.load(include_str!("duel.lua")).set_name("@duel.lua").exec()
}

/// The character id of the target when it can be challenged: another player, alive, while neither
/// side is in a duel and no challenge waits for an answer.
fn target(data: &super::api::HudData) -> Option<u32> {
    let target = data.panels.target.as_ref()?;
    let duel = &data.duel.received;
    (!data.disconnected
        && data.character_id != 0
        && target.unit_type == "character"
        && target.character_id != 0
        && target.character_id != data.character_id
        && target.hp > 0
        && duel.active.is_none()
        && duel.challenge.is_none())
    .then_some(target.character_id)
}

pub(super) fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event, names: &Names) {
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
            data.duel.requests.clear();
        }
        let notices = data.duel.received.apply_with_notices(event).1;
        for notice in &notices {
            match notice {
                DuelNotice::Ended(result) => data.duel.results.push(*result),
                DuelNotice::Countdown => data.duel.countdown = Some(Countdown::default()),
                _ => {}
            }
        }
        // the start, the end and leaving the world cut the countdown short
        if !data.duel.received.countdown {
            data.duel.countdown = None;
        }
        notices
    };
    let Some(rt) = rt else { return };
    let opponent = match event {
        Event::Duel(aa_net::DuelEvent::Started { challenger_obj_id, challenged_obj_id }) => names.opponent(*challenger_obj_id, *challenged_obj_id),
        _ => String::new(),
    };
    prompt(rt, state, names);
    for notice in notices {
        let result = match notice {
            DuelNotice::Challenged { .. } => Ok(()),
            // The server sends the countdown packet together with the start: nothing to wait for.
            DuelNotice::Countdown => Ok(()),
            DuelNotice::Started => {
                rt.fire_event("STARTED_DUEL", ());
                rt.lua
                    .load("HUD.duel_close_challenge(); return HUD.duel_start_text(...)")
                    .call::<String>(opponent.clone())
                    .map(|text| rt.fire_event("SYSMSG", text))
            }
            DuelNotice::Ended(result) => {
                rt.fire_event("ENDED_DUEL", ());
                rt.lua.load("HUD.duel_close_challenge(); HUD.duel_show_result(...)").call::<()>(result.key())
            }
        };
        if let Err(error) = result {
            warn!("hud: duel {notice:?}: {error}");
        }
    }
}

/// Opens the prompt of an unanswered challenge once.
fn prompt(rt: &UiRuntime, state: &Shared, names: &Names) {
    let challenger_id = {
        let mut data = state.borrow_mut();
        let view = &mut data.duel;
        let current = view.received.challenge;
        if current == view.prompted {
            return;
        }
        view.prompted = current;
        match current {
            Some(id) => id,
            None => return,
        }
    };
    // Ignored invitations are refused without a prompt (`CSStartDuel` 507, the answer of the Cancel button).
    let ignored = rt
        .lua
        .load("return X2Option:GetOptionItemValue(OPTION_ITEM_IGNORE_DUEL_INVITATION)==1")
        .eval::<bool>()
        .unwrap_or(false);
    if ignored {
        let mut data = state.borrow_mut();
        if data.duel.received.answered(challenger_id) {
            info!("hud: duel challenge of {challenger_id} ignored (option)");
            data.duel.requests.push(aa_net::Command::AnswerDuel { challenger_id, accept: false });
        }
        return;
    }
    if let Err(error) = rt.lua.load("HUD.duel_show_challenge(...)").call::<()>((challenger_id, names.character(challenger_id))) {
        warn!("hud: duel prompt: {error}");
    }
}

fn track(
    mut events: MessageReader<NetEvent>,
    state: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<super::HudBuilt>,
    units: Option<Res<Units>>,
    names: Option<Res<CharacterNames>>,
    player: Option<Res<Player>>,
) {
    let own_obj = player.and_then(|p| p.spawn.as_ref().map(|s| s.obj_id)).unwrap_or(0);
    let names = Names { units: units.as_deref(), names: names.as_deref(), own_obj };
    for NetEvent(event) in events.read() {
        dispatch(if built.up { rt.as_deref() } else { None }, &state.0, event, &names);
    }
    if let (true, Some(rt)) = (built.up, rt.as_deref()) {
        prompt(rt, &state.0, &names);
    }
}

/// The start countdown: "Duel will begin in N sec." at 3, 2 and 1 seconds, as system messages.
fn countdown(time: Res<Time>, state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    let said = {
        let mut data = state.0.borrow_mut();
        let Some(running) = data.duel.countdown.as_mut() else { return };
        let (said, over) = running.advance(time.delta_secs());
        if over {
            data.duel.countdown = None;
        }
        said
    };
    let (true, Some(rt)) = (built.up, rt.as_deref()) else { return };
    for seconds in said {
        match rt.lua.load("return HUD.duel_countdown_text(...)").call::<String>(seconds) {
            Ok(text) => rt.fire_event("SYSMSG", text),
            Err(error) => warn!("hud: duel countdown: {error}"),
        }
    }
}

fn requests(state: NonSend<HudBridge>, mut out: MessageWriter<NetCommand>) {
    let commands = std::mem::take(&mut state.0.borrow_mut().duel.requests);
    out.write_batch(commands.into_iter().map(NetCommand));
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track.before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud),
    )
    .add_systems(Update, countdown.after(track))
    .add_systems(Update, requests.after(super::sync_hud).after(aa_ui::UiInput));
}
