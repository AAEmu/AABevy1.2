//! Original composition window. Staging, the item cast, and the received fired callback
//! are separate steps; a successful socket write never means that a sheet was saved.
pub(super) mod cache;
mod data;
pub(super) use cache::tooltip as fill_tip;
#[allow(dead_code)]
pub(super) mod fixture;
mod playback;
#[cfg(test)]
mod tests;
pub(super) use data::Tables;

use super::api::{HudBridge, HudData, Shared};
use aa_net::{Command, Event, MusicCommand, MusicComposeRequest, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct MusicItemUsed(pub u64);

#[derive(Default)]
pub(super) struct View {
    cache: cache::Shared,
    actabilities: aa_net::Actabilities,
    buffs: aa_net::state::NativeBuffs,
    buff_actor: u32,
    buff_startup: Vec<Event>,
    buff_startup_lost: bool,
    actors: std::collections::BTreeMap<u32, Vec3>,
    received: Vec<Received>,
    commands: Vec<Command>,
    generation: u64,
    preview: Option<Vec<u8>>,
    staged: Option<Staged>,
    cast: Option<Staged>,
    requests: Vec<(u32, MusicComposeRequest)>,
    errors: Vec<i16>,
}

enum Received {
    Play {
        actor: u32,
        midi: Vec<u8>,
        position: Option<Vec3>,
    },
    Pause(u32),
    Reset,
}

#[derive(Clone)]
struct Staged {
    owner: u32,
    actor: u32,
    request: MusicComposeRequest,
    title: String,
    notes: String,
    timeline: Option<u16>,
    sent: bool,
}

// GameData loads asynchronously; keep the real own-buff receipts until its
// native stack rules exist. One authoritative snapshot plus bounded deltas.
const MAX_BUFF_STARTUP: usize = 1024;
fn ready_buff_catalog(data: &mut HudData) {
    if data.user_music.buffs.catalog_loaded() {
        return;
    }
    let Some(game) = data.game.get() else {
        return;
    };
    let rules = game.native_buffs.clone();
    data.user_music.buffs.set_catalog(rules);
    for event in std::mem::take(&mut data.user_music.buff_startup) {
        let removed = super::mode_actions::removal(data, &event);
        data.user_music.buffs.apply(&event);
        super::mode_actions::reduce(data, &event, removed);
    }
}

pub(super) fn refresh_catalog(state: &Shared) {
    ready_buff_catalog(&mut state.borrow_mut());
}

fn buff_receipt(data: &mut HudData, event: &Event) {
    if matches!(
        event,
        Event::CharacterSelected(_)
            | Event::ReadyToConnect { .. }
            | Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
    ) || matches!(event,Event::UnitsRemoved(ids) if ids.contains(&data.user_music.buff_actor))
    {
        data.input_modes.clear();
        data.user_music.buff_actor = 0;
        data.user_music.buffs = default();
        data.user_music.buff_startup.clear();
        data.user_music.buff_startup_lost = false;
        return;
    }
    let snapshot = match event {
        Event::PlayerSpawn(spawn) => Some(spawn.obj_id),
        Event::UnitSpawned(unit) if unit.obj_id == data.user_music.buff_actor => Some(unit.obj_id),
        Event::OwnUnitSnapshot(unit) if unit.obj_id == data.user_music.buff_actor => {
            Some(unit.obj_id)
        }
        _ => None,
    };
    if let Some(actor) = snapshot {
        data.user_music.buff_actor = actor;
        if matches!(event, Event::PlayerSpawn(_)) {
            data.user_music.buffs = default();
        }
        // Retain the received own identity even before the catalog is ready.
        // NativeBuffs snapshots need no stack rules; deltas still wait for replay.
        data.user_music.buffs.apply(event);
        data.user_music.buff_startup.clear();
        data.user_music.buff_startup_lost = false;
    }
    ready_buff_catalog(data);
    let relevant = snapshot.is_some()
        || matches!(event, Event::GameDecodeFailed { .. })
        || matches!(event, Event::UnhandledGamePacket { level, len, .. }
            if *level == aa_net::game::LEVEL_COMPRESSED && *len > 0)
        || matches!(event,
        Event::BuffCreated {obj_id,..} | Event::BuffRemoved {obj_id,..} | Event::BuffUpdated {obj_id,..}
        if *obj_id==data.user_music.buff_actor && *obj_id!=0);
    if !relevant || data.user_music.buff_startup_lost {
        return;
    }
    if data.user_music.buffs.catalog_loaded() {
        data.user_music.buffs.apply(event);
    } else if data.user_music.buff_startup.len() < MAX_BUFF_STARTUP {
        data.user_music.buff_startup.push(event.clone());
    } else {
        // Never invent a known-empty list after losing startup history.
        data.user_music.buff_startup.clear();
        data.user_music.buff_startup_lost = true;
        warn!("user music: startup buff history overflow; awaiting own snapshot");
    }
}

fn instrument(data: &HudData) -> Option<aa_midi::mml::Voice> {
    let item = super::enchant::equipment(data, aa_net::model::slot::MUSICAL as u8)?;
    // The wrapper first requires a valid equipped instrument. The compiler's
    // initializer then checks raw buff template IDs across all three lists.
    let base = data.game.get()?.user_music.voice(item.template_id)?;
    let buffs = own_buffs(data)?;
    Some(if buffs.iter().any(|entry| entry.buff.buff_id == 5068) {
        aa_midi::mml::Voice::melodic(0)
    } else if buffs.iter().any(|entry| entry.buff.buff_id == 8079) {
        aa_midi::mml::Voice::PERCUSSION
    } else {
        base
    })
}

pub(super) fn own_buffs(data: &HudData) -> Option<&[aa_net::state::BuffEntry]> {
    if !data.user_music.buffs.catalog_loaded() || data.user_music.buff_startup_lost {
        return None;
    }
    let (_, actor) = data.user_music.cache.borrow().context()?;
    data.user_music.buffs.of(actor)
}

pub(super) fn own_identity(data: &HudData) -> Option<(u32, u32)> {
    data.user_music.cache.borrow().context()
}

// Original394eaac0/394eb0e2 checks these raw IDs before opening blank paper.
// Sitting at a piano/drum is allowed; a written sheet bypasses this guard.
fn composer_allowed(data: &HudData) -> Option<bool> {
    Some(
        !own_buffs(data)?
            .iter()
            .any(|entry| matches!(entry.buff.buff_id, 6010 | 6176 | 6177 | 7011 | 8216)),
    )
}

fn quality(data: &HudData, song_grade: u8) -> Option<f32> {
    quality_at(data, super::crafting::proficiency(data, 33)?, song_grade)
}

fn quality_at(data: &HudData, (points, grade): (i32, u8), song_grade: u8) -> Option<f32> {
    if grade < song_grade {
        return None;
    }
    if grade > song_grade {
        return Some(1.0);
    }
    // Original39083a8a always calls397a8d40(0), including nonzero grades.
    let upper = super::crafting::music_upper_limit(data, 0)? as f32;
    if upper <= 0.0 {
        return None;
    }
    let lower = grade
        .checked_sub(1)
        .and_then(|g| super::crafting::music_upper_limit(data, usize::from(g)))
        .unwrap_or(0);
    Some((points.wrapping_sub(lower) as f32 + upper * 9.0) / (upper * 10.0))
}

// XlMultiToWide3301a100: optional initial BOM, 3000 u32 codepoints plus NUL.
// Rust accepts valid UTF-8; malformed native byte sequences are not reproduced.
fn compiler_input(notes: &str) -> &str {
    let notes = notes.strip_prefix('\u{feff}').unwrap_or(notes);
    let end = notes
        .char_indices()
        .nth(3000)
        .map_or(notes.len(), |(at, _)| at);
    &notes[..end]
}

fn started_commands(data: &mut HudData, started: &aa_net::skill::SkillStarted) -> Vec<Command> {
    ready_buff_catalog(data);
    use aa_net::skill::SkillCasterData;
    let actor = match &started.raw_caster {
        SkillCasterData::Unit(id)
        | SkillCasterData::Item { obj_id: id, .. }
        | SkillCasterData::Mount { obj_id: id, .. } => *id,
        _ => return Vec::new(),
    };
    let Some((owner, own_actor)) = data.user_music.cache.borrow().context() else {
        return Vec::new();
    };
    if started.result != 0
        || started.raw_result.code != 0
        || started.tl == 0
        || actor == 0
        || own_actor != actor
    {
        return Vec::new();
    }
    let Some(game) = data.game.get() else {
        return Vec::new();
    };
    let actions = game.user_music.actions(started.skill_id).to_vec();
    let mut commands = Vec::new();
    for action in actions {
        let command = match action {
            94 => MusicCommand::End,
            93 => {
                let notes = data.user_music.cache.borrow().notes.clone();
                let grade = data
                    .game
                    .get()
                    .unwrap()
                    .user_music
                    .grade(&notes)
                    .unwrap_or(0);
                let Some(voice) = instrument(data) else {
                    data.user_music.errors.extend([641, 1]);
                    continue;
                };
                let received = &data.user_music.actabilities;
                let proficiency = match received.get(33) {
                    Some(value) => value.point.map(|point| (point, value.step)),
                    None => received.loaded.then_some((0, 0)),
                };
                let Some(quality) = proficiency.and_then(|value| quality_at(data, value, grade))
                else {
                    continue;
                };
                let Ok(midi) = aa_midi::mml::compile(
                    compiler_input(&notes),
                    voice,
                    quality.min(1.0),
                    data.start.elapsed().as_millis() as u32,
                ) else {
                    data.user_music.errors.push(1);
                    continue;
                };
                if midi.is_empty() || midi.len() > aa_net::music::MAX_MIDI_BYTES {
                    continue;
                }
                MusicCommand::CacheMidi { midi }
            }
            _ => continue,
        };
        commands.push(Command::Music {
            character_id: owner,
            command,
        });
    }
    commands
}

fn stage(data: &mut HudData, id: &str, title: String, notes: String) -> Option<Staged> {
    ready_buff_catalog(data);
    let item_id = id.parse::<u64>().ok().filter(|id| *id != 0)?;
    if title.is_empty()
        || title.len() > 96
        || notes.is_empty()
        || notes.len() > 12000
        || title.contains('\0')
        || notes.contains('\0')
    {
        return None;
    }
    let source = super::enchant::music_paper(data, item_id)?;
    let (_, proficiency_grade) = super::crafting::proficiency(data, 33)?;
    let tables = &data.game.get()?.user_music;
    if notes.chars().count() > *tables.limits.get(&proficiency_grade)? {
        return None;
    }
    let grade = tables.grade(&notes)?;
    let voice = match instrument(data) {
        Some(voice) => voice,
        None => {
            data.user_music.errors.extend([641, 682]);
            return None;
        }
    };
    let quality = quality(data, grade)?;
    // Compilation is bounded to 1000 editor characters; rendering runs on a worker.
    // Native values >=1 all choose perfect notes; the compiler's public input is bounded to1.
    if aa_midi::mml::compile(
        compiler_input(&notes),
        voice,
        quality.min(1.0),
        data.start.elapsed().as_millis() as u32,
    )
    .is_err()
    {
        data.user_music.errors.push(682);
        return None;
    }
    let request = MusicComposeRequest::new(source, grade).ok()?;
    let actor = data
        .snapshot
        .unit_id
        .parse::<u32>()
        .ok()
        .filter(|id| *id != 0)?;
    Some(Staged {
        owner: data.character_id,
        actor,
        request,
        title,
        notes,
        timeline: None,
        sent: false,
    })
}

fn stop_preview(data: &mut HudData) {
    data.user_music.generation = data.user_music.generation.wrapping_add(1);
    data.user_music.preview = None;
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    cache::bind(lua, &bridge.borrow().user_music.cache);
    let api: Table = lua.globals().get("X2UserMusic")?;
    let state = bridge.clone();
    api.set(
        "GetCompositionLimitInfos",
        lua.create_function(move |lua, _: Value| {
            let data = state.borrow();
            let list = lua.create_table()?;
            if let Some(game) = data.game.get() {
                for (grade, name) in super::crafting::music_grades(&data) {
                    let Some(limit) = game.user_music.limits.get(&grade) else {
                        continue;
                    };
                    let row = lua.create_table()?;
                    row.set("grade", grade)?;
                    row.set("name", name)?;
                    row.set("compositionLimit", *limit)?;
                    list.raw_set(list.raw_len() + 1, row)?;
                }
            }
            Ok(list)
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "PreSaveNotes",
        lua.create_function(
            move |_, (_, id, title, notes): (Value, String, String, String)| {
                let mut data = state.borrow_mut();
                let staged = stage(&mut data, &id, title, notes);
                let accepted = staged.is_some();
                if let Some(staged) = &staged {
                    data.user_music.cache.borrow_mut().notes = staged.notes.clone();
                }
                data.user_music.staged = staged;
                Ok(accepted)
            },
        )?,
    )?;
    let state = bridge.clone();
    api.set(
        "SaveNotes",
        lua.create_function(
            move |_, (_, id, title, notes): (Value, String, String, String)| {
                let mut data = state.borrow_mut();
                if data.user_music.cast.is_some() {
                    return Ok(());
                }
                // Native SaveNotes repeats staging after confirmation: current paper and proficiency.
                if let Some(staged) = stage(&mut data, &id, title, notes) {
                    data.user_music.cache.borrow_mut().notes = staged.notes.clone();
                    data.user_music.staged = Some(staged.clone());
                    data.user_music
                        .requests
                        .push((staged.owner, staged.request.clone()));
                    data.user_music.cast = Some(staged);
                }
                Ok(())
            },
        )?,
    )?;
    let state = bridge.clone();
    api.set(
        "TestPlayNotes",
        lua.create_function(move |_, (_, notes): (Value, String)| {
            let mut data = state.borrow_mut();
            ready_buff_catalog(&mut data);
            if data.character_id == 0 || data.disconnected || notes.contains('\0') {
                return Ok(-1i64);
            }
            let Some(voice) = instrument(&data) else {
                data.user_music.errors.extend([641, 1]);
                return Ok(-1);
            };
            let Ok(midi) = aa_midi::mml::compile(compiler_input(&notes), voice, 1.0, 0) else {
                data.user_music.errors.push(1);
                return Ok(-1);
            };
            stop_preview(&mut data);
            data.user_music.preview = Some(midi);
            Ok((data.user_music.generation & i64::MAX as u64) as i64)
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "StopPlayNotes",
        lua.create_function(move |_, _: Value| {
            stop_preview(&mut state.borrow_mut());
            Ok(())
        })?,
    )?;
    Ok(())
}

fn same_cast(staged: &Staged, skill: u32, caster: &aa_net::skill::SkillCasterData) -> bool {
    skill == staged.request.skill_id()
        && matches!(caster, aa_net::skill::SkillCasterData::Item { obj_id, item_id, template_id, type1, type2 }
        if *obj_id == staged.actor && *item_id == staged.request.source().item.id && *template_id == 27902 && *type1 == staged.request.source().item.grade && *type2 == staged.request.source().item.made_unit_id)
}

fn receive(data: &mut HudData, event: &Event) -> Option<Command> {
    if matches!(
        event,
        Event::CharacterSelected(_)
            | Event::PlayerSpawn(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
    ) || matches!(event, Event::UnitsRemoved(ids) if data.snapshot.unit_id.parse::<u32>().ok().is_some_and(|actor| ids.contains(&actor)))
    {
        stop_preview(data);
        data.user_music.staged = None;
        data.user_music.cast = None;
        data.user_music.requests.clear();
        data.user_music.errors.clear();
        return None;
    }
    let pending = data.user_music.cast.as_mut()?;
    if pending.owner != data.character_id
        || data.snapshot.unit_id.parse::<u32>().ok() != Some(pending.actor)
        || data.disconnected
    {
        data.user_music.cast = None;
        return None;
    }
    match event {
        Event::MusicComposeRequested {
            character_id,
            request,
        } if *character_id == pending.owner && *request == pending.request => pending.sent = true,
        Event::MusicComposeRejected {
            character_id,
            request,
            ..
        } if *character_id == pending.owner && *request == pending.request => {
            data.user_music.cast = None
        }
        Event::SkillStarted(started)
            if pending.sent && same_cast(pending, started.skill_id, &started.raw_caster) =>
        {
            if started.result != 0 {
                data.user_music.cast = None;
            } else {
                pending.timeline = Some(started.tl);
            }
        }
        Event::CastingStopped { tl, .. } | Event::SkillEnded { tl }
            if pending.timeline == Some(*tl) =>
        {
            data.user_music.cast = None
        }
        Event::SkillStopped { obj_id, skill_id }
            if *obj_id == pending.actor && *skill_id == pending.request.skill_id() =>
        {
            data.user_music.cast = None
        }
        Event::SkillFired(fired)
            if pending.sent
                && same_cast(pending, fired.skill_id, &fired.raw_caster)
                && pending.timeline.is_none_or(|tl| tl == fired.tl) =>
        {
            let staged = data.user_music.cast.take()?;
            if fired.result.code != 0 {
                return None;
            }
            return Some(Command::Music {
                character_id: staged.owner,
                command: MusicCommand::UploadNotes {
                    item_id: staged.request.source().item.id,
                    slot_type: 2,
                    slot: staged.request.source().slot,
                    title: staged.title,
                    notes: staged.notes,
                },
            });
        }
        _ => {}
    }
    None
}

fn item_use(rt: Option<&UiRuntime>, state: &Shared, id: u64) {
    ready_buff_catalog(&mut state.borrow_mut());
    if let Some(rt) = rt {
        cache::sync_directory(&rt.lua, &state.borrow().user_music.cache);
    }
    let paper = super::enchant::music_paper(&state.borrow(), id).is_some();
    if paper {
        let allowed = composer_allowed(&state.borrow());
        match allowed {
            Some(true) => {
                if let Some(rt) = rt {
                    rt.fire_event("OPEN_MUSIC_SHEET", (true, id.to_string()));
                }
            }
            Some(false) => state.borrow_mut().user_music.errors.push(702),
            None => {} // A discarded buff history cannot establish admission.
        }
    } else if let Some(source) = super::enchant::music_sheet(&state.borrow(), id)
        && source.item.detail.kind() == 8
        && let Some(note_id) = source
            .item
            .detail_payload
            .as_deref()
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    {
        state
            .borrow()
            .user_music
            .cache
            .borrow_mut()
            .lookup(note_id, false);
    }
}

fn update(
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    mut opened: MessageReader<MusicItemUsed>,
    mut net: MessageWriter<NetCommand>,
    engine: Option<Res<aa_audio::Engine>>,
    mut playback: ResMut<playback::Playback>,
) {
    for MusicItemUsed(id) in opened.read() {
        item_use(rt.as_deref(), &bridge.0, *id);
    }
    let mut data = bridge.0.borrow_mut();
    for (character_id, command) in data.user_music.cache.borrow_mut().drain() {
        if data.character_id == character_id && !data.disconnected {
            net.write(NetCommand(Command::Music {
                character_id,
                command,
            }));
        }
    }
    let actor = data.snapshot.unit_id.parse::<u32>().ok();
    for (character_id, request) in std::mem::take(&mut data.user_music.requests) {
        if data.character_id == character_id
            && !data.disconnected
            && data.user_music.cast.as_ref().is_some_and(|pending| {
                pending.owner == character_id
                    && pending.request == request
                    && !pending.sent
                    && Some(pending.actor) == actor
            })
        {
            net.write(NetCommand(Command::ComposeMusic {
                character_id,
                request,
            }));
        }
    }
    let positions = playback
        .foreign_actors()
        .filter_map(|id| {
            data.user_music
                .actors
                .get(&id)
                .map(|position| (id, *position))
        })
        .collect();
    playback.move_foreign(
        engine.as_deref(),
        &positions,
        crate::sound::mute_other_music(),
    );
    let ended = playback.update(
        engine.as_deref(),
        data.user_music.generation,
        data.user_music.preview.take(),
    );
    if ended
        && data.character_id != 0
        && !data.disconnected
        && data
            .snapshot
            .unit_id
            .parse::<u32>()
            .ok()
            .is_some_and(|actor| {
                data.user_music
                    .cache
                    .borrow()
                    .owns(data.character_id, actor)
            })
    {
        // Native preview and received music share the own-actor tracking map.
        net.write(NetCommand(Command::Music {
            character_id: data.character_id,
            command: MusicCommand::End,
        }));
    }
    let errors = std::mem::take(&mut data.user_music.errors);
    drop(data);
    if let Some(rt) = rt {
        for code in errors {
            if let Some(text) = super::alerts::error_name(code)
                .and_then(|name| super::alerts::localized(&rt, "ERROR_MSG", name))
            {
                rt.fire_event("SYSMSG", text);
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<playback::Playback>()
        .add_message::<MusicItemUsed>()
        .add_systems(
            Update,
            observe_events
                .after(super::enchant::track)
                .after(super::sync_hud)
                .before(update)
                .before(aa_ui::UiFrame),
        )
        .add_systems(
            Update,
            update
                .after(super::sync_hud)
                .before(aa_ui::UiFrame)
                .run_if(super::hud_up),
        )
        .add_systems(
            OnExit(crate::Screen::InGame),
            |bridge: NonSend<HudBridge>,
             engine: Option<Res<aa_audio::Engine>>,
             mut playback: ResMut<playback::Playback>| {
                stop_preview(&mut bridge.0.borrow_mut());
                let generation = bridge.0.borrow().user_music.generation;
                let cache = bridge.0.borrow().user_music.cache.clone();
                cache.borrow_mut().deactivate();
                bridge.0.borrow_mut().user_music = View {
                    generation,
                    cache,
                    ..default()
                };
                bridge.0.borrow_mut().mode_actions = default();
                bridge.0.borrow_mut().input_modes.clear();
                playback.clear(engine.as_deref());
            },
        );
}

pub(super) fn receipt(rt: Option<&UiRuntime>, state: &Shared, event: &Event) {
    let cache = state.borrow().user_music.cache.clone();
    if let Some(rt) = rt {
        cache::sync_directory(&rt.lua, &cache);
    }
    cache.borrow_mut().apply(event);
    let mut data = state.borrow_mut();
    ready_buff_catalog(&mut data);
    let removed = super::mode_actions::removal(&data, event);
    buff_receipt(&mut data, event);
    data.user_music.actabilities.apply(event);
    if matches!(
        event,
        Event::Actability(aa_net::ActabilityEvent::TierChanged {
            is_upgrade: false,
            ..
        })
    ) {
        let limits: Vec<_> = (0..)
            .map_while(|grade| super::crafting::music_upper_limit(&data, grade))
            .collect();
        data.user_music
            .actabilities
            .resolve_downgrade_points(event, &limits);
    }
    // Compact receipt-local actor registry. Frame-final aa-net Units may already
    // have consumed a later removal or movement when this callback executes.
    match event {
        Event::UnitSpawned(unit) => {
            data.user_music.actors.insert(
                unit.obj_id,
                aa_world::cry_to_bevy(Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z)),
            );
        }
        Event::UnitMoved { obj_id, movement } if movement.move_type != 4 => {
            if let Some(position) = data.user_music.actors.get_mut(obj_id) {
                let pos = movement
                    .support
                    .as_ref()
                    .and_then(|s| s.world)
                    .map_or(movement.pos, |(world, _)| world);
                *position = aa_world::cry_to_bevy(Vec3::new(pos.x, pos.y, pos.z));
            }
        }
        Event::Resurrected { obj_id, pos, .. } => {
            if let Some(position) = data.user_music.actors.get_mut(obj_id) {
                *position = aa_world::cry_to_bevy(Vec3::new(pos.x, pos.y, pos.z));
            }
        }
        Event::Slave(aa_net::slave::SlaveEvent::Escaped(escape)) => {
            if let Some(position) = data.user_music.actors.get_mut(&escape.obj_id) {
                let pos = escape.position();
                *position = aa_world::cry_to_bevy(Vec3::new(pos.x, pos.y, pos.z));
            }
        }
        Event::UnitsRemoved(ids) => {
            for id in ids {
                data.user_music.actors.remove(id);
            }
        }
        Event::Notice(aa_net::notice::NoticeEvent::UserMusic { obj_id, midi, .. }) => {
            if let Some((_, own)) = cache.borrow().context()
                && *obj_id != 0
            {
                if *obj_id == own {
                    stop_preview(&mut data);
                    data.user_music.preview = Some(midi.clone());
                } else {
                    let position = data.user_music.actors.get(obj_id).copied();
                    data.user_music.received.push(Received::Play {
                        actor: *obj_id,
                        midi: midi.clone(),
                        position,
                    });
                }
            }
        }
        Event::Notice(aa_net::notice::NoticeEvent::UserMusicPaused { obj_id }) => {
            if cache
                .borrow()
                .context()
                .is_some_and(|(_, own)| own == *obj_id)
            {
                stop_preview(&mut data);
            } else {
                data.user_music.received.push(Received::Pause(*obj_id));
            }
        }
        Event::PlayerSpawn(_) => data.user_music.received.push(Received::Reset),
        Event::ReadyToConnect { .. }
        | Event::LoginSucceeded(_)
        | Event::CharacterSelected(_)
        | Event::LeftWorld { .. }
        | Event::Disconnected { .. }
        | Event::Kicked { .. } => {
            data.user_music.actors.clear();
            data.user_music.received.push(Received::Reset);
        }
        _ => {}
    }
    if let Event::SkillStarted(started) = event {
        let commands = started_commands(&mut data, started);
        data.user_music.commands.extend(commands);
    }
    if let Some(command) = receive(&mut data, event) {
        data.user_music.commands.push(command);
    }
super::mode_actions::reduce(&mut data, event, removed);
    drop(data);
    super::mode_actions::flush(rt, state);
}

fn observe_events(
    bridge: NonSend<HudBridge>,
    mut net: MessageWriter<NetCommand>,
    engine: Option<Res<aa_audio::Engine>>,
    mut playback: ResMut<playback::Playback>,
) {
    for received in std::mem::take(&mut bridge.0.borrow_mut().user_music.received) {
        match received {
            Received::Play {
                actor,
                midi,
                position,
            } => playback.play_foreign(engine.as_deref(), actor, midi, position),
            Received::Pause(actor) => playback.stop_foreign(engine.as_deref(), actor),
            Received::Reset => playback.clear(engine.as_deref()),
        }
    }
    for command in std::mem::take(&mut bridge.0.borrow_mut().user_music.commands) {
        net.write(NetCommand(command));
    }
}
