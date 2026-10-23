//! Original composition window. Staging, the item cast, and the received fired callback
//! are separate steps; a successful socket write never means that a sheet was saved.
mod data;
#[allow(dead_code)]
pub(super) mod fixture;
mod playback;
#[cfg(test)]
mod tests;
pub(super) use data::Tables;

use super::api::{HudBridge, HudData, Shared};
use aa_net::{Command, Event, MusicCommand, MusicComposeRequest, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct MusicPaperUsed(pub u64);

#[derive(Default)]
pub(super) struct View {
    generation: u64,
    preview: Option<Vec<u8>>,
    staged: Option<Staged>,
    cast: Option<Staged>,
    requests: Vec<(u32, MusicComposeRequest)>,
    errors: Vec<i16>,
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

fn instrument(data: &HudData) -> Option<aa_midi::mml::Voice> {
    let item = super::enchant::equipment(data, aa_net::model::slot::MUSICAL as u8)?;
    data.game.get()?.user_music.voice(item.template_id)
}

fn quality(data: &HudData, song_grade: u8) -> Option<f32> {
    let (points, grade) = super::crafting::proficiency(data, 33)?;
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

fn stage(data: &mut HudData, id: &str, title: String, notes: String) -> Option<Staged> {
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
        &notes,
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
            if data.character_id == 0 || data.disconnected || notes.contains('\0') {
                return Ok(-1i64);
            }
            let Some(voice) = instrument(&data) else {
                data.user_music.errors.extend([641, 1]);
                return Ok(-1);
            };
            let Ok(midi) = aa_midi::mml::compile(&notes, voice, 1.0, 0) else {
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
        && matches!(caster, aa_net::skill::SkillCasterData::Item { obj_id, item_id, template_id, type1, .. }
        if *obj_id == staged.actor && *item_id == staged.request.source().item.id && *template_id == 27902 && *type1 == staged.request.source().item.grade)
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

fn update(
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    mut opened: MessageReader<MusicPaperUsed>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
    engine: Option<Res<aa_audio::Engine>>,
    mut playback: ResMut<playback::Playback>,
) {
    for MusicPaperUsed(id) in opened.read() {
        if super::enchant::music_paper(&bridge.0.borrow(), *id).is_some()
            && let Some(rt) = rt.as_ref()
        {
            rt.fire_event("OPEN_MUSIC_SHEET", (true, id.to_string()));
        }
    }
    for NetEvent(event) in events.read() {
        if let Some(command) = receive(&mut bridge.0.borrow_mut(), event) {
            net.write(NetCommand(command));
        }
    }
    let mut data = bridge.0.borrow_mut();
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
    playback.update(
        engine.as_deref(),
        data.user_music.generation,
        data.user_music.preview.take(),
    );
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
        .add_message::<MusicPaperUsed>()
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
                bridge.0.borrow_mut().user_music = View {
                    generation,
                    ..default()
                };
                playback.clear(engine.as_deref());
            },
        );
}
