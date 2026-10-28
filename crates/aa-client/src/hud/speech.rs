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
//! * `SCChatBubble` (a skill's `BubbleEffect`): inline text, or the localized `bubble_effects.speech` of its ID, raised as
//!   `CHAT_MSG_QUEST` over the unit with the bubble kind of the packet (1 normal, 2 think, 3 system).
//! * `SCEmotionExpressed`: the line of the emote (`express_texts`: "You wave.", "Ann waves.") is raised as `CHAT_EMOTION`, which the
//!   script prints in the chat window. The clip itself is played by `ingame::combat`.
use std::collections::HashMap;

use aa_net::model::UnitKind;
use aa_net::state::{Player, Units};
use aa_net::{BubbleBody, EmoteEvent, Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::ecs::system::SystemParam;
use mlua::{Function, Lua, Table, Value};
use rusqlite::Connection;

use super::api::{HudBridge, Shared};
use super::data::any_int;
use super::plates::{PlateState, head_gap, relation_of, unit_height};
use crate::ingame::emote::{Emotes, fill};

/// The initialized unnamed native int wrapper is100; its x87 speed helper
/// truncates100 *90f32 *.01f32 to89. Its virtual setters are no-ops.
const MS_PER_CODEPOINT: u32 = 89;
const FADE_PER_CODEPOINT: f32 = f32::from_bits(0x3aaae3a1);
/// Pixels from the spot over the head (where the plate's bar is) up to the tail of the bubble: over the name (a character's is much
/// bigger than an NPC's).
const ABOVE_CHARACTER: f32 = 62.0;
const ABOVE_NPC: f32 = 30.0;

/// Localized `bubble_effects.speech`, indexed by the effect ID received in 0x154.
/// `bubbles` / `bubble_chats` are separate dialogue data, not this packet's table.
#[derive(Default)]
pub(super) struct Tables {
    effects: HashMap<u32, String>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut localized = HashMap::new();
        let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'bubble_effects' AND tbl_column_name = 'speech' AND en_us IS NOT NULL")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, r.get::<_, String>(1)?)))? {
            let (id, text) = row?;
            localized.insert(id, text);
        }
        let mut effects = HashMap::new();
        let mut q = conn.prepare("SELECT id, speech FROM bubble_effects")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, r.get::<_, Option<String>>(1)?.unwrap_or_default())))? {
            let (id, raw) = row?;
            // Native395b1350 also falls back for empty/leading-NUL translation.
            let text = localized.remove(&id).map(|s| cstr(&s).to_owned()).filter(|s| !s.is_empty())
                .unwrap_or_else(|| cstr(&raw).to_owned());
            effects.insert(id, text);
        }
        Ok(Self { effects })
    }

    fn text(&self, id: u32) -> Option<&str> {
        self.effects.get(&id).map(String::as_str)
    }
}

#[derive(Default)]
pub(super) struct View {
    /// Object id -> screen position (UI pixels) of the spot over its head.
    pos: HashMap<u32, [f32; 2]>,
    /// Object id -> (`"character"` | `"npc"`, relation to the player).
    info: HashMap<u32, (&'static str, &'static str)>,
}

/// A live object ID in the native Lua string domain; numeric values are rejected.
fn id_of(v: &Value) -> Option<u32> {
    super::unit_id::value(v)
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
            let Some(id) = id_of(&id) else { return Ok(None) };
            if let Some(previous) = &previous
                && let Some(table) = previous.call::<Option<Table>>((this, id.to_string()))?
            {
                // Legacy providers can add fields such as the pet owner. Complete the
                // shared name-colour fields from the received unit snapshot when available.
                if let Some((kind, faction)) = s.borrow().speech.info.get(&id) {
                    if table.raw_get::<Value>("type")?.is_nil() { table.raw_set("type", *kind)?; }
                    if table.raw_get::<Value>("faction")?.is_nil() { table.raw_set("faction", *faction)?; }
                }
                return Ok(Some(table));
            }
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
    super::plates::npc_of(game, u).map(|n| n.name.clone()).unwrap_or_default()
}

fn cstr(s: &str) -> &str {
    s.split('\0').next().unwrap_or_default()
}

fn timing(text: &str) -> (f32, f32) {
    let n = text.chars().count() as u32;
    //39371140 has three separate SSE operations, not the sphere path's u32 fade.
    let fade = (n as f32 * FADE_PER_CODEPOINT + 1.0) * 300.0;
    (n.wrapping_mul(MS_PER_CODEPOINT).max(500) as f32, fade)
}

fn say(rt: &UiRuntime, obj_id: u32, author: &str, text: &str, kind: u8) {
    let text = cstr(text);
    if text.is_empty() { return; }
    if kind == 4 {
        rt.fire_event("CHAT_MSG_ALARM", text.to_owned());
    } else {
        let (show, fade) = timing(text);
        rt.fire_event("CHAT_MSG_QUEST", (text.to_owned(), cstr(author).to_owned(), format!("{obj_id:x}"), kind as f32, show, fade));
    }
}

/// Scene points, independent of name plates, camera projection and model loading.
#[derive(SystemParam)]
struct ActorPoints<'w, 's> {
    views: Option<Res<'w, crate::ingame::units::Views>>,
    houses: Option<Res<'w, crate::ingame::housing::render::DrawnHouses>>,
    own: Query<'w, 's, (&'static crate::ingame::player::LocalPlayer, &'static Transform)>,
    roots: Query<'w, 's, &'static Transform>,
}

impl ActorPoints<'_, '_> {
    fn loaded(&self, obj_id: u32, units: &Units, player: &Player) -> bool {
        if player.spawn.as_ref().is_some_and(|own| own.obj_id == obj_id) {
            return self.own.iter().any(|(actor, _)| actor.obj_id == obj_id);
        }
        units.get(obj_id).is_some_and(|unit| self.views.as_ref().is_some_and(|views| views.unit_current(unit)))
    }

    fn point(&self, obj_id: u32, units: &Units, player: &Player) -> Option<Vec3> {
        let own = player.spawn.as_ref()?.obj_id;
        if obj_id == own {
            return self.own.iter().find(|(actor, _)| actor.obj_id == own).map(|(_, root)| root.translation);
        }
        let unit = units.get(obj_id)?;
        let views = self.views.as_ref()?;
        if !views.unit_current(unit) { return None; }
        let root = if matches!(unit.kind, UnitKind::Housing { .. }) {
            self.houses.as_ref().and_then(|houses| houses.entity(obj_id))
        } else { views.unit_entity(obj_id) };
        if let Some(entity) = root {
            return self.roots.get(entity).ok().map(|root| root.translation);
        }
        views.unit_point(unit)
    }

    fn in_range(&self, obj_id: u32, units: &Units, player: &Player) -> bool {
        let Some(own) = player.spawn.as_ref().map(|spawn| spawn.obj_id) else { return false };
        let (Some(a), Some(b)) = (self.point(own, units, player), self.point(obj_id, units, player)) else { return false };
        // Native393c2650 uses current 3D points and admits equality at30m.
        a.distance(b) <= 30.0
    }
}

/// One event of the server. The actor/range decision is made by the scheduled
/// scene consumer; ordinary CHAT_EMOTION does not use the bubble range gate.
fn dispatch(rt: &UiRuntime, state: &Shared, event: &Event, units: &Units, player: &Player, emotes: Option<&Emotes>, in_range: bool) -> Option<(u32, u32)> {
    let Event::Emote(emote) = event else { return None };
    let data = state.borrow();
    let game = data.game.get();
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    match emote {
        EmoteEvent::Expressed { obj_id, target_obj_id, emotion_id } => {
            let Some(emotes) = emotes else { return None };
            let line = emotes.line(*emotion_id, *obj_id == own, *target_obj_id != 0 && *target_obj_id == own, *target_obj_id != 0);
            if line.is_empty() {
                return None;
            }
            let text = fill(line, &name_of(units, player, game, *obj_id), &name_of(units, player, game, *target_obj_id));
            drop(data);
            rt.fire_event("CHAT_EMOTION", text);
        }
        EmoteEvent::ChatBubble { obj_id, kind, body, .. } => {
            if !in_range { return None; }
            let author = name_of(units, player, game, *obj_id);
            let text = match body {
                BubbleBody::Text(text) => Some(text.clone()),
                BubbleBody::Id(id) => game.and_then(|g| g.speech.text(*id)).map(str::to_owned),
            };
            let Some(text) = text.filter(|text| !cstr(text).is_empty()) else { return None };
            drop(data);
            say(rt, *obj_id, &author, &text, *kind);
            if let BubbleBody::Id(id) = body { return Some((*obj_id, *id)); }
        }
        EmoteEvent::AreaBubble { .. } => {}
    }
    None
}

/// The used bubble-effect voice slot. It owns a request handle and a point
/// snapshot, independently of text lifetime or the actor's later movement.
#[derive(Resource, Default)]
struct EffectVoice(Option<aa_audio::Voice>);

impl EffectVoice {
    fn reap(&mut self, playing: impl FnOnce(aa_audio::Voice) -> bool) {
        if self.0.is_some_and(|voice| !playing(voice)) { self.0 = None; }
    }

    fn replace(
        &mut self,
        path: Option<&str>,
        stop: impl FnOnce(aa_audio::Voice),
        point: impl FnOnce() -> Option<Vec3>,
        play: impl FnOnce(&str, Vec3) -> aa_audio::Voice,
    ) {
        // A cache miss preserves the previous voice. Native IDs1/3 are misses.
        let Some(path) = path else { return };
        if let Some(old) = self.0.take() { stop(old); }
        // Admission cancels before sampling the fresh actor point, even if
        // that point disappeared during script delivery.
        let Some(point) = point() else { return };
        self.0 = Some(play(path, point));
    }
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
    points: ActorPoints,
    mut retired: Local<bool>,
    engine: Option<Res<aa_audio::Engine>>,
    packs: Option<Res<crate::sound::Packs>>,
    mut voice: ResMut<EffectVoice>,
) {
    if let Some(engine) = engine.as_deref() { voice.reap(|id| engine.is_playing(id)); }
    let rt = if built.up { rt.as_deref() } else { None };
    for NetEvent(event) in events.read() {
        match event {
            Event::PlayerSpawn(_) => *retired = false,
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => *retired = true,
            _ => {},
        }
        if *retired { continue; }
        if let (Some(rt), Some(units), Some(player)) = (rt, units.as_deref(), player.as_deref()) {
            let in_range = match event {
                Event::Emote(EmoteEvent::ChatBubble { obj_id, .. }) => points.in_range(*obj_id, units, player),
                _ => false,
            };
            if let Some((obj_id, effect_id)) = dispatch(rt, &state.0, event, units, player, emotes.as_deref(), in_range)
                && points.loaded(obj_id, units, player)
                && let (Some(engine), Some(packs)) = (engine.as_deref(), packs.as_deref().and_then(crate::sound::Packs::get))
            {
                let key = format!("bubble_effect_{effect_id}");
                voice.replace(packs.path("voice", &key), |old| engine.stop(old, true),
                    || points.point(obj_id, units, player),
                    |path, point| engine.play_with(path, Some(point), aa_audio::PlayOptions::default()));
            }
        }
    }
}

/// The relation of every unit to the player (for the colour of the names in the bubbles), twice a second.
fn refresh_info(state: NonSend<HudBridge>, units: Res<Units>, player: Res<Player>, time: Res<Time>, mut next: Local<f64>) {
    let now = time.elapsed_secs_f64();
    if now < *next {
        return;
    }
    *next = now + 0.5;
    let Some(spawn) = player.spawn.as_ref() else { return };
    let mut data = state.0.borrow_mut();
    let own_faction = spawn.character().faction_id;
    let data = &mut *data;
    let game = data.game.get();
    let info = &mut data.speech.info;
    info.clear();
    for u in units.iter() {
        let kind = if matches!(u.kind, UnitKind::Character { .. }) { "character" } else { "npc" };
        info.insert(u.obj_id, (kind, relation_of(game, own_faction, u)));
    }
    info.insert(spawn.obj_id, ("character", "friendly"));
}

/// Projects the spot over the head of every unit the scene has, every frame (a few hundred at most).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn project(
    state: NonSend<HudBridge>,
    plates: Res<PlateState>,
    units: Res<Units>,
    player: Res<Player>,
    world_camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<super::plates::PlateRoot>, Without<crate::hud::ModelViewCamera>)>,
    transforms: Query<&Transform, Without<super::plates::PlateRoot>>,
) {
    let mut data = state.0.borrow_mut();
    let Some((camera, camera_transform)) = world_camera.iter().find(|(c, _)| c.is_active) else {
        data.speech.pos.clear();
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let mut pos = std::mem::take(&mut data.speech.pos);
    pos.clear();
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
    app.init_resource::<EffectVoice>()
        .add_systems(Update, track.after(crate::ingame::units::move_units).before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud))
        .add_systems(
            Update,
            (refresh_info, project.after(super::plates::update_plates)).run_if(crate::ingame::world_screens),
        );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_packet_tests;

#[cfg(test)]
mod voice_tests;
