//! Original ranking HUD over received boards, independent stream names and native local frame policy.
use super::api::{HudBridge, HudData, Shared};
use aa_net::{Event, NetCommand, NetEvent, RankingCommand, RankingEvent, RankingNotice, Rankings};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use std::collections::BTreeMap;
mod catalog;
#[allow(dead_code)]
pub(super) mod fixture;
mod query;
pub(super) use catalog::Tables;
#[cfg(test)]
mod tests;

#[derive(Clone, Default)]
struct Providers {
    names: aa_net::CharacterNames,
    /// Object ID -> actual persistent identity and name; NPCs never enter this map.
    actors: BTreeMap<u32, (u32, String)>,
    worlds: BTreeMap<u8, String>,
}
impl Providers {
    fn apply(&mut self, event: &Event) {
        self.names.apply(event);
        match event {
            Event::WorldList(list) => self.worlds = list.worlds.iter().map(|world| (world.id, world.name.clone())).collect(),
            Event::PlayerSpawn(spawn) => self.actor(&spawn.unit),
            Event::UnitSpawned(unit) => self.actor(unit),
            Event::UnitNameChanged { obj_id, name } => {
                if let Some((_, current)) = self.actors.get_mut(obj_id) {
                    *current = name.clone();
                }
            }
            Event::UnitsRemoved(ids) => self.actors.retain(|obj_id, _| !ids.contains(obj_id)),
            _ if lifecycle(event) => self.actors.clear(),
            _ => {}
        }
    }
    fn actor(&mut self, unit: &aa_net::model::Unit) {
        self.actors.remove(&unit.obj_id);
        if let aa_net::model::UnitKind::Character { char_id } = unit.kind {
            self.actors.insert(unit.obj_id, (char_id, unit.name.clone()));
        }
    }
    fn lookup(&self, id: u32) -> (Option<Vec<u8>>, bool) {
        if id == 0 || self.names.pending.contains(&id) {
            return (None, false);
        }
        let mut actors = self.actors.values().filter(|(character, _)| *character == id);
        if let Some((_, name)) = actors.next() {
            // The native persistent-ID lookup is unique. Refuse ambiguous app actor snapshots.
            if actors.next().is_some() {
                return (None, false);
            }
            return (Some(name.as_bytes().to_vec()), false);
        }
        self.names.by_id.get(&id).cloned().map_or((None, true), |name| (Some(name), false))
    }
}

#[derive(Clone, Default)]
struct Projection {
    received: Rankings,
    providers: Providers,
    tabs: Vec<(bool, u32)>,
    default_tab: i32,
    selected_tab: i32,
    selected_rank: u32,
    ready: bool,
    lock_seconds: f32,
    next_update: BTreeMap<u32, u64>,
}
impl Projection {
    fn personal(&mut self, tables: &Tables) {
        self.tabs = tables
            .rows
            .iter()
            .take(16)
            .map(|entry| (self.received.by_id.contains_key(&entry.id), entry.id))
            .collect();
        self.default_tab = tables
            .rows
            .iter()
            .position(|entry| self.received.by_id.contains_key(&entry.id))
            .map_or(0, |i| i as i32 + 1);
        let selected = self
            .selected_tab
            .checked_sub(1)
            .filter(|index| *index >= 0)
            .and_then(|index| self.tabs.get(index as usize));
        if !selected.is_some_and(|&(present, id)| present && id != 0) {
            self.selected_tab = self.default_tab;
        }
        self.selected_rank = self
            .selected_tab
            .checked_sub(1)
            .filter(|index| *index >= 0)
            .and_then(|index| self.tabs.get(index as usize))
            .map_or(0, |&(_, id)| id);
    }
}

enum Notice {
    Event(&'static str),
    Error(i32),
    Alarm(i32, String),
}
struct Effect(Notice, Projection);

#[derive(Default)]
pub(super) struct View {
    live: Projection,
    callback: Option<Projection>,
    owner: Option<u32>,
    requests: Vec<aa_net::Command>,
    pending: Vec<Effect>,
    /// Native ranking assumes its static database initializer has finished.
    deferred: Vec<Event>,
}
impl View {
    fn current(&self) -> &Projection {
        self.callback.as_ref().unwrap_or(&self.live)
    }
}

fn signed(value: i32) -> f64 {
    f64::from(value as f32)
}
fn unsigned(value: u32) -> f64 {
    f64::from(value as f32)
}
fn integer(value: Option<&Value>) -> Option<i32> {
    let number = match value? {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    Some(if number.is_finite() && (-2147483648.0..2147483648.0).contains(&number) {
        number as i32
    } else {
        i32::MIN
    })
}
fn owner(data: &HudData) -> Option<u32> {
    (!data.disconnected && data.character_id != 0 && data.ranking.owner == Some(data.character_id)).then_some(data.character_id)
}
fn name(data: &mut HudData, id: u32) -> Option<Vec<u8>> {
    let (name, missing) = data.ranking.current().providers.lookup(id);
    if missing && let Some(character_id) = owner(data) {
        data.ranking.live.providers.names.pending.insert(id);
        if let Some(callback) = &mut data.ranking.callback {
            callback.providers.names.pending.insert(id);
        }
        data.ranking.requests.push(aa_net::Command::QueryCharacterName {
            character_id,
            requested_id: id,
        });
    }
    name
}
fn apply_ui(lua: &Lua, state: &Shared, Effect(notice, snapshot): Effect) -> mlua::Result<()> {
    let previous = state.borrow_mut().ranking.callback.replace(snapshot);
    let result = (|| {
        match notice {
            Notice::Event(event) => aa_ui::lua::fire_ui_event(lua, event, vec![]),
            Notice::Alarm(kind, message) => aa_ui::lua::fire_ui_event(
                lua,
                "RANK_ALARM_MSG",
                vec![
                    Value::Number(signed(kind)),
                    Value::String(lua.create_string(message.split('\0').next().unwrap_or_default())?),
                ],
            ),
            Notice::Error(code) => {
                if let Ok(code) = i16::try_from(code)
                    && let Some(key) = super::alerts::error_name(code)
                {
                    let locale: Table = lua.globals().get("X2Locale")?;
                    let group: Value = lua.globals().get("ERROR_MSG")?;
                    let message: Value = locale.call_method("LocalizeUiText", (group, key))?;
                    aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![message]);
                }
            }
        }
        Ok(())
    })();
    state.borrow_mut().ranking.callback = previous;
    result
}
fn deliver(lua: Option<&Lua>, state: &Shared, notice: Notice, snapshot: Projection) {
    if let Some(lua) = lua {
        if let Err(error) = apply_ui(lua, state, Effect(notice, snapshot)) {
            warn!("ranking: callback: {error}");
        }
    } else {
        state.borrow_mut().ranking.pending.push(Effect(notice, snapshot));
    }
}
fn request_records(lua: &Lua, state: &Shared, tab: i32) {
    let snapshot = {
        let mut data = state.borrow_mut();
        let Some(character_id) = owner(&data) else { return };
        let Some(tables) = data.game.get().map(|game| &game.ranking) else {
            return;
        };
        //39073cc0 admits zero-based indices, while the later personal callback is ONE-based.
        if tab < 0 || tab as usize >= tables.rows.len() {
            return;
        }
        data.ranking.live.selected_tab = tab;
        data.ranking.requests.push(aa_net::Command::Ranking {
            character_id,
            command: RankingCommand::RequestPlayerRecords,
        });
        data.ranking.live.clone()
    };
    deliver(Some(lua), state, Notice::Event("RANK_LOCK"), snapshot);
    let mut data = state.borrow_mut();
    data.ranking.live.lock_seconds = data.start.elapsed().as_secs_f32();
}
fn request_snapshots(state: &Shared) {
    let mut data = state.borrow_mut();
    let Some(character_id) = owner(&data) else { return };
    let key = data.ranking.current().selected_rank;
    if !data.ranking.current().received.by_id.contains_key(&key) {
        return;
    }
    data.ranking.live.ready = true;
    data.ranking.requests.push(aa_net::Command::Ranking {
        character_id,
        command: RankingCommand::RequestSnapshots { rank_id: key },
    });
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    query::install(lua, state)?;
    let api: Table = lua.globals().get("X2Rank")?;
    api.raw_set("MAX_RANK_SNAPSHOT", 100.)?;
    let s = state.clone();
    api.raw_set(
        "RequestPlayerRecords",
        lua.create_function(move |lua, args: MultiValue| {
            if let Some(tab) = integer(args.get(1)) {
                request_records(lua, &s, tab);
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "RequestRankSnapshots",
        lua.create_function(move |_, _: MultiValue| {
            request_snapshots(&s);
            Ok(())
        })?,
    )
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
fn dispatch(lua: Option<&Lua>, state: &Shared, event: &Event) {
    {
        let mut data = state.borrow_mut();
        let view = &mut data.ranking;
        let replace = match event {
            Event::CharacterSelected(selected) => view.live.received.character_id != Some(selected.character.id),
            Event::PlayerSpawn(spawn) => view.live.received.character_id != Some(spawn.character().id),
            Event::LoginSucceeded(_)
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting) => true,
            _ => false,
        };
        if lifecycle(event) || replace {
            view.owner = None;
            for command in std::mem::take(&mut view.requests) {
                if let aa_net::Command::QueryCharacterName { requested_id, .. } = command {
                    view.live.providers.names.pending.remove(&requested_id);
                }
            }
            view.pending.clear();
            view.deferred.clear();
            view.live.lock_seconds = 0.;
            view.live.ready = true;
        }
        if replace {
            let providers = std::mem::take(&mut view.live.providers);
            view.live = Projection {
                providers,
                ..Default::default()
            };
        }
        let new_owner = match event {
            Event::CharacterSelected(selected) => Some(selected.character.id),
            Event::PlayerSpawn(spawn) => Some(spawn.character().id),
            _ => None,
        };
        if let Some(id) = new_owner {
            view.owner = Some(id);
        }
        view.live.providers.apply(event);
        if let Event::Ranking(_) = event
            && data.game.get().is_none()
        {
            data.ranking.deferred.push(event.clone());
            return;
        }
    }
    if !matches!(
        event,
        Event::Ranking(_)
            | Event::CharacterSelected(_)
            | Event::PlayerSpawn(_)
            | Event::LoginSucceeded(_)
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting)
    ) {
        return;
    }
    let effects = {
        let mut data = state.borrow_mut();
        let mut received = data.ranking.live.received.clone();
        let notices = received.apply_with_dispatch(event).1;
        let mut effects = Vec::new();
        for (notice, cache) in notices {
            data.ranking.live.received = cache;
            match notice {
                RankingNotice::Error(code) => effects.push(Effect(Notice::Error(code), data.ranking.live.clone())),
                RankingNotice::PlayerRecords => {
                    if let Some(game) = data.game.get() {
                        let mut projection = data.ranking.live.clone();
                        projection.personal(&game.ranking);
                        data.ranking.live = projection;
                    }
                    effects.push(Effect(Notice::Event("RANK_PLAYER_RECORDS"), data.ranking.live.clone()));
                }
            }
        }
        data.ranking.live.received = received;
        if let Event::Ranking(RankingEvent::Snapshot { rank_id, error: 0, board }) = event
            && let Some(current) = data.ranking.live.received.by_id.get(rank_id)
            && current.snapshot.as_ref() == Some(board)
        {
            data.ranking.live.ready = false;
            if board.updated_raw > 0
                && let Some(entry) = data.game.get().and_then(|game| game.ranking.get(*rank_id))
            {
                let max = entry
                    .scope_ids
                    .iter()
                    .filter_map(|id| data.game.get()?.ranking.scopes.get(id))
                    .map(|scope| scope.to)
                    .max()
                    .unwrap_or(0)
                    .max(0);
                data.ranking
                    .live
                    .next_update
                    .insert(*rank_id, board.updated_raw.wrapping_add(if max < 1001 { 1800 } else { 3600 }));
            }
            for id in board.entries.iter().map(|entry| entry.character_id) {
                name(&mut data, id);
            }
        }
        if let Event::Ranking(RankingEvent::Alarm { rank_id, raw_kind }) = event
            && let Some(entry) = data.game.get().and_then(|game| game.ranking.get(*rank_id))
        {
            effects.push(Effect(
                Notice::Alarm(
                    entry.kind,
                    if *raw_kind == 1 {
                        entry.start_alarm.clone()
                    } else {
                        entry.end_alarm.clone()
                    },
                ),
                data.ranking.live.clone(),
            ));
        }
        effects
    };
    for Effect(notice, snapshot) in effects {
        deliver(lua, state, notice, snapshot);
    }
}
fn frame(lua: Option<&Lua>, state: &Shared, seconds: f32) {
    let unlock = {
        let data = state.borrow();
        let view = &data.ranking.live;
        (view.lock_seconds > 0. && seconds > view.lock_seconds + 3.).then(|| view.clone())
    };
    if let Some(snapshot) = unlock {
        deliver(lua, state, Notice::Event("RANK_UNLOCK"), snapshot);
        state.borrow_mut().ranking.live.lock_seconds = 0.;
    }
    let snapshot = {
        let mut data = state.borrow_mut();
        let projection = &data.ranking.live;
        let ids = (owner(&data).is_some() && !projection.ready)
            .then(|| projection.received.by_id.get(&projection.selected_rank))
            .flatten()
            .map(|board| {
                board.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
                    snapshot.entries.iter().map(|entry| entry.character_id).collect::<Vec<_>>()
                })
            });
        if let Some(ids) = ids {
            let mut available = true;
            for id in ids {
                if name(&mut data, id).is_none() {
                    available = false;
                    break;
                }
            }
            if available {
                data.ranking.live.ready = true;
                Some(data.ranking.live.clone())
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(snapshot) = snapshot {
        deliver(lua, state, Notice::Event("RANK_SNAPSHOTS"), snapshot);
    }
}
fn track(mut events: MessageReader<NetEvent>, state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    let lua = if built.up { rt.as_deref().map(|rt| &rt.lua) } else { None };
    if let Some(lua) = lua {
        let pending = std::mem::take(&mut state.0.borrow_mut().ranking.pending);
        for effect in pending {
            if let Err(error) = apply_ui(lua, &state.0, effect) {
                warn!("ranking: replay: {error}");
            }
        }
    }
    let deferred = {
        let mut data = state.0.borrow_mut();
        if data.game.get().is_some() {
            std::mem::take(&mut data.ranking.deferred)
        } else {
            Vec::new()
        }
    };
    for event in deferred {
        dispatch(lua, &state.0, &event);
    }
    for NetEvent(event) in events.read() {
        dispatch(lua, &state.0, event);
    }
    let seconds = state.0.borrow().start.elapsed().as_secs_f32();
    frame(lua, &state.0, seconds);
}
fn flush(data: &mut HudData) -> Vec<aa_net::Command> {
    let owner = owner(data);
    std::mem::take(&mut data.ranking.requests)
        .into_iter()
        .filter(|command| match command {
            aa_net::Command::Ranking { character_id, .. } | aa_net::Command::QueryCharacterName { character_id, .. } => {
                Some(*character_id) == owner
            }
            _ => false,
        })
        .collect()
}
fn update(state: NonSend<HudBridge>, mut out: MessageWriter<NetCommand>) {
    out.write_batch(flush(&mut state.0.borrow_mut()).into_iter().map(NetCommand));
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track
            .before(aa_ui::UiInput)
            .before(super::sync_hud)
            .before(super::prebuild_hud)
            .before(super::build_hud),
    )
    .add_systems(Update, update.after(aa_ui::UiFrame).after(super::sync_hud));
}
