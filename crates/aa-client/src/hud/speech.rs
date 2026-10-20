//! Speech bubbles and the chat lines of emotes.
//!
//! The bubbles are the original script's (`x2ui/chat/chatbubble.lua`: the `chatBubbleFrame` that listens for `CHAT_MESSAGE`,
//! `CHAT_MSG_QUEST`, `CHAT_MSG_DOODAD` and `CHAT_EMOTION`). What it asks of the game is answered here:
//!
//! * `X2Unit:GetUnitScreenNameTagOffset(id)`: the screen position (x, y, 1) of the spot over a unit's head, for the unit's object id
//!   (a string). The script calls it every frame for every bubble it shows, and a bubble that gets no answer on its first frame
//!   hides and is forgotten, so [`project`] works the position out every frame for every unit the scene has (the plates' heights
//!   and unit entities) and the answer is the last result; a unit that is off screen or not drawn gives none and the bubble hides.
//! * `X2Unit:GetUnitInfoById(id)`: `{ faction = "hostile" | ..., type = "character" | "npc" }`, which colours the speaker's name.
//! * say-chat: `chat.rs` passes the speaker's object id and says the speaker is in range (`speakerInChatBound`), which is all the
//!   script needs to draw the bubble of a say, party, raid or guild line over the speaker.
//! * `SCChatBubble` (a skill's `BubbleEffect`): the text, or the lines of the `bubbles` row (`bubble_chats`, along `next`), raised as
//!   `CHAT_MSG_QUEST` over the unit with the bubble kind of the packet (1 normal, 2 think, 3 system).
//! * `SCEmotionExpressed`: the line of the emote (`express_texts`: "You wave.", "Ann waves.") is raised as `CHAT_EMOTION`, which the
//!   script prints in the chat window. The clip itself is played by `ingame::combat`.
use std::collections::HashMap;

use aa_net::model::UnitKind;
use aa_net::state::{Player, Units};
use aa_net::{BubbleBody, EmoteEvent, Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Function, Lua, Table, Value};
use rusqlite::Connection;

use super::api::{HudBridge, Shared};
use super::data::any_int;
use super::plates::{PlateState, head_gap, relation_of, unit_height};
use crate::ingame::emote::{Emotes, fill};

/// Seconds a bubble stays up (the script's own default is 5 s too).
const SHOW_MS: u32 = 5000;
/// Seconds between the lines of a `bubble_chats` conversation.
const LINE_GAP: f64 = 3.5;
/// Pixels from the spot over the head (where the plate's bar is) up to the tail of the bubble: over the name (a character's is much
/// bigger than an NPC's).
const ABOVE_CHARACTER: f32 = 62.0;
const ABOVE_NPC: f32 = 30.0;

/// One line of a `bubbles` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Line {
    pub text: String,
    /// `npc_id` of the speaker (0: the unit the bubble was announced for).
    pub npc_id: u32,
    pub kind: u8,
}

/// The conversations of the `bubbles` rows, by `bubbles.id`.
#[derive(Default)]
pub(super) struct Tables {
    chats: HashMap<u32, Vec<Line>>,
}

impl Tables {
    /// `bubble_chats` followed from its START row along `next`, with the English `speech`.
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut texts: HashMap<u32, String> = HashMap::new();
        if let Ok(mut q) = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'bubble_chats' AND tbl_column_name = 'speech' AND en_us IS NOT NULL AND en_us <> ''") {
            for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))?.flatten() {
                texts.insert(row.0, row.1);
            }
        }
        struct Row {
            bubble: u32,
            start: bool,
            next: u32,
            npc_id: u32,
            kind: u8,
        }
        let mut rows: HashMap<u32, Row> = HashMap::new();
        let mut q = conn.prepare("SELECT id, bubble_id, \"START\", next, npc_id, kind_id FROM bubble_chats")?;
        let mut cursor = q.query([])?;
        while let Some(r) = cursor.next()? {
            let start = matches!(r.get::<_, rusqlite::types::Value>(2)?, rusqlite::types::Value::Text(t) if t == "t");
            rows.insert(any_int(r, 0)? as u32, Row { bubble: any_int(r, 1)? as u32, start, next: any_int(r, 3)? as u32, npc_id: any_int(r, 4)? as u32, kind: any_int(r, 5)? as u8 });
        }
        let mut out = Tables::default();
        let mut starts: Vec<(u32, u32)> = rows.iter().filter(|(_, r)| r.start).map(|(id, r)| (r.bubble, *id)).collect();
        starts.sort();
        for (bubble, first) in starts {
            let mut lines = Vec::new();
            let mut at = first;
            // (a `next` that loops back ends the chain)
            while let Some(row) = rows.get(&at).filter(|_| lines.len() < 16) {
                if let Some(text) = texts.get(&at) {
                    lines.push(Line { text: text.clone(), npc_id: row.npc_id, kind: row.kind });
                }
                if row.next == 0 || row.next == at {
                    break;
                }
                at = row.next;
            }
            if !lines.is_empty() {
                out.chats.entry(bubble).or_insert(lines);
            }
        }
        Ok(out)
    }

    fn lines(&self, bubble: u32) -> Option<&[Line]> {
        self.chats.get(&bubble).map(Vec::as_slice)
    }
}

/// A bubble line that waits for its turn.
struct Waiting {
    at: f64,
    obj_id: u32,
    kind: u8,
    text: String,
}

#[derive(Default)]
pub(super) struct View {
    /// Object id -> screen position (UI pixels) of the spot over its head.
    pos: HashMap<u32, [f32; 2]>,
    /// Object id -> (`"character"` | `"npc"`, relation to the player).
    info: HashMap<u32, (&'static str, &'static str)>,
    waiting: Vec<Waiting>,
}

/// An object id as the scripts pass it: the string (or number) of `unit_id`.
fn id_of(v: &Value) -> Option<u32> {
    match v {
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        Value::Integer(n) => u32::try_from(*n).ok(),
        Value::Number(n) => Some(*n as u32),
        _ => None,
    }
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let unit: Table = lua.globals().get("X2Unit")?;
    let s = state.clone();
    unit.raw_set(
        "GetUnitScreenNameTagOffset",
        lua.create_function(move |_, (_, id): (Value, Value)| {
            let Some(id) = id_of(&id) else { return Ok((None, None, None)) };
            let data = s.borrow();
            Ok(match data.speech.pos.get(&id) {
                Some([x, y]) => (Some(*x), Some(*y), Some(1.0f32)),
                None => (None, None, None),
            })
        })?,
    )?;
    // what other modules answer for their own units (the owned pet) comes first
    let previous: Option<Function> = unit.raw_get("GetUnitInfoById").ok();
    let s = state.clone();
    unit.raw_set(
        "GetUnitInfoById",
        lua.create_function(move |lua, (this, id): (Value, Value)| {
            if let Some(previous) = &previous
                && let Some(table) = previous.call::<Option<Table>>((this, id.clone()))?
            {
                return Ok(Some(table));
            }
            let Some(id) = id_of(&id) else { return Ok(None) };
            let data = s.borrow();
            let Some((kind, faction)) = data.speech.info.get(&id) else { return Ok(None) };
            let t = lua.create_table()?;
            t.raw_set("type", *kind)?;
            t.raw_set("faction", *faction)?;
            Ok(Some(t))
        })?,
    )?;
    // (GetDoodadScreenPosition is not answered: the bubbles over doodads are not drawn)
    Ok(())
}

/// The name of a unit for the texts: a player's own name, a unit's, an NPC's template name.
fn name_of(units: &Units, player: &Player, game: Option<&super::data::GameData>, obj_id: u32) -> String {
    if player.spawn.as_ref().is_some_and(|s| s.obj_id == obj_id) {
        return player.spawn.as_ref().map(|s| s.name().to_string()).unwrap_or_default();
    }
    let Some(u) = units.get(obj_id) else { return String::new() };
    if !u.name.is_empty() {
        return u.name.clone();
    }
    game.and_then(|g| g.npcs.get(&u.kind.template_id())).map(|n| n.name.clone()).unwrap_or_default()
}

/// The kind a bubble packet's `kind` stands for: `CBK_NORMAL` 1, `CBK_THINK` 2, `CBK_SYSTEM` 3 (4: no bubble, blue text).
fn bubble_kind(kind: u8) -> u8 {
    match kind {
        2 => 2,
        3 | 4 => 3,
        _ => 1,
    }
}

fn say(rt: &UiRuntime, obj_id: u32, author: &str, text: &str, kind: u8) {
    rt.fire_event("CHAT_MSG_QUEST", (text.to_string(), author.to_string(), obj_id.to_string(), bubble_kind(kind) as i64, SHOW_MS as i64, 1000i64, 0i64, 0i64, false));
}

/// One event of the server.
fn dispatch(rt: &UiRuntime, state: &Shared, event: &Event, units: &Units, player: &Player, emotes: Option<&Emotes>, now: f64) {
    let Event::Emote(emote) = event else { return };
    let data = state.borrow();
    let game = data.game.get();
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    match emote {
        EmoteEvent::Expressed { obj_id, target_obj_id, emotion_id } => {
            let Some(emotes) = emotes else { return };
            let line = emotes.line(*emotion_id, *obj_id == own, *target_obj_id != 0 && *target_obj_id == own, *target_obj_id != 0);
            if line.is_empty() {
                return;
            }
            let text = fill(line, &name_of(units, player, game, *obj_id), &name_of(units, player, game, *target_obj_id));
            drop(data);
            rt.fire_event("CHAT_EMOTION", text);
        }
        EmoteEvent::ChatBubble { obj_id, kind, body, .. } => {
            let author = name_of(units, player, game, *obj_id);
            let lines: Vec<Line> = match body {
                BubbleBody::Text(text) => vec![Line { text: text.clone(), npc_id: 0, kind: *kind }],
                BubbleBody::Id(id) => game.and_then(|g| g.speech.lines(*id)).map(<[Line]>::to_vec).unwrap_or_default(),
            };
            let Some(first) = lines.first() else { return };
            // the first line now, the others in turn
            let kind_of = |line: &Line| if *kind == 0 { line.kind } else { *kind };
            let waiting: Vec<Waiting> = lines
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, line)| Waiting { at: now + LINE_GAP * i as f64, obj_id: speaker_of(line, units, *obj_id), kind: kind_of(line), text: line.text.clone() })
                .collect();
            let speaker = speaker_of(first, units, *obj_id);
            let kind = kind_of(first);
            drop(data);
            say(rt, speaker, &author, &first.text, kind);
            state.borrow_mut().speech.waiting.extend(waiting);
        }
        EmoteEvent::AreaBubble { .. } => {}
    }
}

/// Who says a line: its own NPC when one is around, else the unit the bubble was announced for.
fn speaker_of(line: &Line, units: &Units, default: u32) -> u32 {
    if line.npc_id == 0 {
        return default;
    }
    units.iter().find(|u| u.kind.template_id() == line.npc_id && !matches!(u.kind, UnitKind::Character { .. })).map_or(default, |u| u.obj_id)
}

#[allow(clippy::too_many_arguments)]
fn track(
    mut events: MessageReader<NetEvent>,
    state: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<super::HudBuilt>,
    units: Option<Res<Units>>,
    player: Option<Res<Player>>,
    emotes: Option<Res<Emotes>>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let (Some(units), Some(player)) = (units, player) else {
        events.clear();
        return;
    };
    let rt = if built.up { rt.as_deref() } else { None };
    for NetEvent(event) in events.read() {
        if let Some(rt) = rt {
            dispatch(rt, &state.0, event, &units, &player, emotes.as_deref(), now);
        }
    }
    // the waiting lines of a conversation
    let due: Vec<Waiting> = {
        let mut data = state.0.borrow_mut();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut data.speech.waiting).into_iter().partition(|w| w.at <= now);
        data.speech.waiting = later;
        due
    };
    if let Some(rt) = rt {
        let data = state.0.borrow();
        let game = data.game.get();
        for w in due {
            say(rt, w.obj_id, &name_of(&units, &player, game, w.obj_id), &w.text, w.kind);
        }
    }
}

/// The relation of every unit to the player (for the colour of the names in the bubbles), twice a second.
fn refresh_info(state: NonSend<HudBridge>, units: Res<Units>, player: Res<Player>, mut frame: Local<u32>) {
    *frame += 1;
    if !(*frame).is_multiple_of(30) {
        return;
    }
    let Some(spawn) = player.spawn.as_ref() else { return };
    let mut data = state.0.borrow_mut();
    let own_faction = spawn.character().faction_id;
    let mut info = HashMap::new();
    {
        let game = data.game.get();
        for u in units.iter() {
            let kind = if matches!(u.kind, UnitKind::Character { .. }) { "character" } else { "npc" };
            info.insert(u.obj_id, (kind, relation_of(game, own_faction, u)));
        }
    }
    info.insert(spawn.obj_id, ("character", "friendly"));
    data.speech.info = info;
}

/// Projects the spot over the head of every unit the scene has, every frame (a few hundred at most).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn project(
    state: NonSend<HudBridge>,
    plates: Res<PlateState>,
    units: Res<Units>,
    player: Res<Player>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<super::plates::PlateRoot>)>,
    transforms: Query<&Transform, Without<super::plates::PlateRoot>>,
) {
    let mut data = state.0.borrow_mut();
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else {
        data.speech.pos.clear();
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let mut pos = HashMap::with_capacity(plates.entities.len());
    for (&id, &entity) in &plates.entities {
        let Ok(root) = transforms.get(entity) else { continue };
        let (scale, character) = if id == own {
            (1.0, true)
        } else {
            units.get(id).map_or((1.0, false), |u| (u.scale, matches!(u.kind, UnitKind::Character { .. })))
        };
        let height = unit_height(plates.heights.get(&id).map(|h| h.0), scale, character);
        let top = root.translation + Vec3::Y * (height + head_gap(height, super::plates::HEAD_GAP));
        let Ok(screen) = camera.world_to_viewport(&camera_global, top) else { continue };
        pos.insert(id, [screen.x.round(), screen.y.round() - if character { ABOVE_CHARACTER } else { ABOVE_NPC }]);
    }
    data.speech.pos = pos;
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, track.before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud))
        .add_systems(
            Update,
            (refresh_info, project.after(super::plates::update_plates)).run_if(crate::ingame::world_screens),
        );
}

#[cfg(test)]
mod tests;
