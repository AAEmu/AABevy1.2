//! Talking to NPCs and the quest dialog (the original's "directing mode": the letterbox with what the
//! NPC says, `questcontext/quest_context_directing.lua`), and the quest data every quest window reads
//! (the tracker, the quest log, the dialog's reward panel).
//!
//! ## Flow
//!
//! The scripts are the original's; this module and `quest.lua` are the client's native side of them.
//!
//! 1. A right click on an NPC in reach, or F with one in reach and no loot (F loots first), starts the
//!    talk: `CSChangeTarget` and `CSInteractNPC`. What the NPC does for the player is worked out from
//!    the quest tables ([`QuestTables::talks`], `completes`, `offers`) and goes to the scripts, in
//!    that order of preference, as `NPC_INTERACTION_START("quest", "talk" | "complete" | "start",
//!    npcId)` (`npc_interaction.lua`). An NPC with nothing to say ends the talk at once.
//! 2. The scripts ask `X2Quest:CallQuestUi` / `CallQuestTalk`, which raise
//!    `START_QUEST_CONTEXT_NPC` / `COMPLETE_QUEST_CONTEXT_NPC` / `PROGRESS_TALK_QUEST_CONTEXT(qtype,
//!    true, npcId)`: the dialog opens (`X2Quest:EnterQuestDirectingMode`: the rest of the HUD is hidden,
//!    [`apply_directing`]) and asks `TryStartQuestContext` (...), which plays the step's lines
//!    (`quest_chat_bubbles` from the step's first one along `next_bubble`) as `CHAT_MSG_QUEST`; Next
//!    (F) / Back (D) walk them (`X2:FastForwardQuestChat`).
//! 3. The last line's button (F, or G on the reward panel) sends the request: `CSStartQuestContext`,
//!    `CSCompleteQuestContext` (with the chosen reward) or `CSQuestTalkMade`. The server's answer
//!    changes `aa_net::state::Quests`; [`sync_quests`] tells the scripts (`QUEST_CONTEXT_UPDATED(qtype,
//!    "started" | "updated" | "completed" | "dropped")`, the objective and condition messages), the
//!    dialog closes on it (after a hand-in it offers the NPC's next quest at once, like the original)
//!    and the talk ends (`CSInteractNPCEnd`).
//!
//! The quest rows the scripts read (`HUD.quest_list()`, `HUD.quest_info(qtype)`) are made here from
//! the server's quest state and the game database ([`super::quests`]).

mod npc;
pub(super) use npc::candidates::Catalog as NpcCandidateCatalog;
#[allow(dead_code)]
pub(super) mod fixture;
pub(super) use npc::{Scope as NpcScope, Selected as SelectedNpc, active_scope, choose_npc, close_npc, drive_npc, npc_menu_current, query_npc, scope_current};

use aa_net::interaction::Interact;
use aa_net::model::UnitKind;
use aa_net::quest::{Quest, status};
use aa_net::state::{Buffs, Doodads, Inventory, Player, PlayerStats, Quests, Skills, Target, Units};
use aa_net::interaction::InteractEvent;
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::{Lua, Table};

use super::api::{HudBridge, HudData, Shared};
use crate::ingame::quest_camera::{Frame as QuestFrame, QuestTalkCamera};
use super::data::GameData;
use super::quests::{PlayerFacts, Texts, kind};
use super::{HudKeyboardFocus, HudPointerOverUi};

/// How far the player reaches to talk to an NPC (F, a right click), metres.
const TALK_RANGE: f32 = 6.0;
/// How near the view ray has to pass a unit's middle for a right click to pick it, metres.
const PICK_RADIUS: f32 = 1.1;
/// A press that moves more than this many pixels before it is released was a camera drag.
const CLICK_SLOP: f32 = 6.0;
/// The widgets the dialog leaves shown when it hides the HUD.
const DIRECTING_WIDGETS: [&str; 2] = ["questDirecting", "QuestDirectingFadeWindow"];

/// What the scripts asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Req {
    Accept(u32),
    /// The quest and the chosen reward (1 based, 0 for none).
    Complete(u32, i32),
    /// Quest, component, act of a talk objective.
    Talk(u32, u32, u32),
    Drop(u32),
    /// `X2Interaction:CancelNPCInteraction()`.
    End,
    /// `X2Quest:EnterQuestDirectingMode()` / `LeaveQuestDirectingMode()`.
    Directing(bool),
    /// The talk's close-up camera on or off (`quest_camera::set_active`).
    Camera(bool),
    /// The quest bubble about to be said frames its speaker (`quest_camera::frame_bubble`).
    Frame(u32),
    /// Originating NPC lifetime, captured before a queued UI request can outlive it.
    AtNpc(NpcScope, Box<Req>),
}

/// A pending world click and its simultaneous keyboard fallback.
#[derive(Debug)]
struct PointerTalk {
    npc: (u32, u32),
    fallback: Option<(u32, u32)>,
}

/// The talk under way and the quest data the scripts read.
#[derive(Debug, Default)]
pub struct DialogView {
    /// The NPC talked to: object id and template.
    pub npc: Option<(u32, u32)>,
    /// The doodad (a notice board ...) whose quest the dialog offers (`SCDoodadQuestAccept`).
    pub doodad: Option<u32>,
    /// What it does for the player: quests it offers, quests handed in to it, talk objectives
    /// (quest, component, act).
    pub start: Vec<u32>,
    pub complete: Vec<u32>,
    pub talk: Vec<(u32, u32, u32)>,
    /// The dialog is up (the rest of the HUD hidden).
    pub directing: bool,
    /// The top-level widgets hidden for it.
    hidden: Vec<aa_ui::tree::Id>,
    /// The active quests as the scripts last saw them, and its version (`HUD.quest_version()`).
    pub quests: Vec<Quest>,
    pub version: u64,
    /// Whether `quests` holds the server's list yet (the first one raises no events).
    seen: bool,
    /// The message of the last quest event ("Quest accepted: [A Friendly Reminder]"):
    /// `X2Quest:GetActiveQuestContextConditionMessage()`.
    pub condition: String,
    pub(super) requests: Vec<Req>,
    pointer_talk: Option<PointerTalk>,
    npc_flow: npc::Flow,
}

/// The macro expander for the player of the snapshot.
fn texts<'a>(d: &'a HudData, game: &'a GameData) -> Texts<'a> {
    Texts { game, player: d.snapshot.name.as_str(), race: d.snapshot.race }
}

/// One quest as the scripts read it: the rows of `HUD.quest_list()` and `HUD.quest_info(qtype)`.
fn quest_table(lua: &Lua, game: &GameData, texts: &Texts, q: &Quest, active: bool) -> mlua::Result<Table> {
    let t = &game.quests;
    let row = lua.create_table()?;
    row.set("type", q.template_id)?;
    let Some(def) = t.def(q.template_id) else {
        row.set("name", "")?;
        row.set("objectives", lua.create_table()?)?;
        return Ok(row);
    };
    row.set("name", texts.expand(&def.name))?;
    row.set("category", def.category)?;
    row.set("categoryName", t.categories.get(&def.category).cloned().unwrap_or_default())?;
    row.set("level", def.level)?;
    row.set("status", if active { q.status } else { 0 })?;
    row.set("main", def.is_main())?;
    row.set("task", def.is_task())?;
    row.set("hidden", def.is_hidden())?;
    row.set("daily", def.is_daily())?;
    row.set("livelihood", def.is_livelihood())?;
    row.set("group", def.is_group())?;
    row.set("selective", def.selective)?;
    row.set("letItDone", def.let_it_done)?;
    row.set("chapter", def.chapter)?;
    row.set("zone", texts.zone_name(def.zone_id))?;
    // the objectives of the step the quest is at, and the paragraph above them (the step's hints, or
    // what it asks)
    let step = t.step_kind(q);
    let objectives = lua.create_table()?;
    for (i, o) in t.objectives(q, texts).iter().enumerate() {
        let e = lua.create_table()?;
        e.set("text", o.text.as_str())?;
        e.set("status", if o.count > 1 { format!("{}/{}", o.done, o.count) } else { String::new() })?;
        e.set("done", step != kind::READY && o.finished())?;
        objectives.set(i + 1, e)?;
    }
    row.set("objectives", objectives)?;
    let step_texts: Vec<&String> = t.parts_of(q.template_id, step).flat_map(|c| &c.texts).collect();
    let hints: Vec<String> = step_texts.iter().filter(|s| s.trim_start().starts_with("|n")).map(|s| texts.expand(s)).collect();
    let summary = if hints.is_empty() { step_texts.iter().map(|s| texts.expand(s)).collect::<Vec<_>>().join("\n") } else { hints.join("\n") };
    row.set("summary", summary)?;
    // what was said when it was accepted (the quest log shows it under the rewards)
    let said = lua.create_table()?;
    if let Some(start) = t.parts_of(q.template_id, kind::START).next() {
        for (i, b) in t.bubble_chain(start.id).iter().enumerate() {
            let e = lua.create_table()?;
            e.set("who", texts.speaker(b))?;
            e.set("kind", b.kind.max(1))?;
            e.set("text", texts.speech(b))?;
            said.set(i + 1, e)?;
        }
    }
    row.set("bubbles", said)?;
    let r = t.rewards(q.template_id);
    row.set("exp", r.exp.to_string())?;
    row.set("copper", r.copper)?;
    for (key, v) in [("labor", r.labor), ("honor", r.honor), ("crime", r.crime), ("living", r.living)] {
        if v != 0 {
            row.set(key, v)?;
        }
    }
    for (key, list) in [("items", &r.items), ("selectiveItems", &r.selective)] {
        let items = lua.create_table()?;
        for (i, &(id, count, grade)) in list.iter().enumerate() {
            let e = lua.create_table()?;
            e.set("id", id)?;
            e.set("name", game.items.get(&id).map(|it| it.named.name.clone()).unwrap_or_default())?;
            e.set("count", count)?;
            e.set("grade", grade.max(1))?;
            items.set(i + 1, e)?;
        }
        row.set(key, items)?;
    }
    if let Some(npc) = t.report_npc(q.template_id) {
        row.set("reportNpc", npc)?;
        row.set("reportNpcName", game.npcs.get(&npc).map(|n| n.name.clone()).unwrap_or_default())?;
    }
    Ok(row)
}

/// The lines of a step: `kind` "start" (the Start component), "complete" (the Ready one) or "talk"
/// (component `component`). `npc` is the NPC talked to, who says its lines in the world too.
fn bubble_table(lua: &Lua, game: &GameData, texts: &Texts, which: &str, quest: u32, component: u32, npc: Option<(u32, u32)>, own: &str) -> mlua::Result<Table> {
    let t = &game.quests;
    let part = match which {
        "start" => t.parts_of(quest, kind::START).next().map(|c| c.id),
        "complete" => t.parts_of(quest, kind::READY).next().map(|c| c.id),
        _ => Some(component),
    };
    let out = lua.create_table()?;
    for (i, b) in part.map(|c| t.bubble_chain(c)).unwrap_or_default().iter().enumerate() {
        let e = lua.create_table()?;
        e.set("id", b.id)?;
        e.set("text", texts.speech(b))?;
        e.set("author", texts.speaker(b))?;
        let author_id = match (b.npc, b.kind) {
            (_, 3) => String::new(),
            (0, _) => own.to_string(),
            (template, _) => npc.filter(|n| n.1 == template).map(|n| n.0.to_string()).unwrap_or_default(),
        };
        e.set("authorId", author_id)?;
        e.set("kind", b.kind.max(1))?;
        out.set(i + 1, e)?;
    }
    Ok(out)
}

/// `HUD.quest_*`: the quest rows, the talk under way and the requests.
pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set("quest_version", lua.create_function(move |_, ()| Ok(b.borrow().dialog.version))?)?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_list",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let out = lua.create_table()?;
            let Some(game) = d.game.get() else { return Ok(out) };
            let texts = texts(&d, game);
            for (i, q) in d.dialog.quests.iter().enumerate() {
                out.set(i + 1, quest_table(lua, game, &texts, q, true)?)?;
            }
            Ok(out)
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_info",
        lua.create_function(move |lua, qtype: u32| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok(None) };
            if game.quests.def(qtype).is_none() {
                return Ok(None);
            }
            let q = Quest { template_id: qtype, ..Quest::default() };
            Ok(Some(quest_table(lua, game, &texts(&d, game), &q, false)?))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_npc",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let Some((obj_id, _)) = d.dialog.npc else { return Ok(None) };
            let t = lua.create_table()?;
            t.set("npcId", obj_id.to_string())?;
            t.set("start", d.dialog.start.clone())?;
            t.set("complete", d.dialog.complete.clone())?;
            let talk = lua.create_table()?;
            for (i, &(qtype, ctype, atype)) in d.dialog.talk.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("qtype", qtype)?;
                e.set("ctype", ctype)?;
                e.set("atype", atype)?;
                talk.set(i + 1, e)?;
            }
            t.set("talk", talk)?;
            Ok(Some(t))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_bubbles",
        lua.create_function(move |lua, (which, qtype, ctype): (String, u32, Option<u32>)| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok(None) };
            Ok(Some(bubble_table(lua, game, &texts(&d, game), &which, qtype, ctype.unwrap_or(0), d.dialog.npc, &d.snapshot.unit_id)?))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set("quest_condition", lua.create_function(move |_, ()| Ok(b.borrow().dialog.condition.clone()))?)?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_macro",
        lua.create_function(move |_, text: Option<String>| {
            let d = b.borrow();
            let text = text.unwrap_or_default();
            Ok(match d.game.get() {
                Some(game) => texts(&d, game).expand(&text),
                None => text,
            })
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_category",
        lua.create_function(move |_, id: u32| Ok(b.borrow().game.get().and_then(|g| g.quests.categories.get(&id).cloned()).unwrap_or_default()))?,
    )?;
    let request = |name: &str, f: fn(mlua::MultiValue) -> Option<Req>| -> mlua::Result<()> {
        let b = bridge.clone();
        hud.raw_set(
            name,
            lua.create_function(move |lua, args: mlua::MultiValue| {
                if let Some(r) = f(args) {
                    let mut data = b.borrow_mut();
                    if matches!(r, Req::End) && (npc::cancel_scope(&data).is_some() || data.dialog.doodad.is_none()) {
                        drop(data);
                        npc::cancel_public(lua, &b);
                        return Ok(());
                    }
                    if let Req::Directing(on) = r {
                        data.input_modes.set(4, u32::from(on));
                    }
                    let scope = active_scope(&data);
                    if matches!(r, Req::End | Req::Directing(false)) {
                        if let Some(scope) = scope.or_else(|| npc::cancel_scope(&data)) {
                            close_npc(&mut data, scope);
                            if matches!(r, Req::Directing(false)) {
                                data.dialog.requests.push(Req::AtNpc(scope, Box::new(r)));
                            }
                            return Ok(());
                        }
                        if data.dialog.doodad.is_none() { return Ok(()); }
                    }
                    data.dialog.requests.push(if let Some(scope) = scope.filter(|_| !matches!(r, Req::Drop(_))) { Req::AtNpc(scope, Box::new(r)) } else { r });
                }
                Ok(())
            })?,
        )
    };
    fn num(args: &mlua::MultiValue, i: usize) -> i64 {
        match args.get(i) {
            Some(mlua::Value::Integer(n)) => i64::from(*n),
            Some(mlua::Value::Number(n)) => *n as i64,
            Some(mlua::Value::String(s)) => s.to_str().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0),
            _ => 0,
        }
    }
    request("quest_accept", |a| Some(Req::Accept(num(&a, 0) as u32)))?;
    request("quest_complete", |a| Some(Req::Complete(num(&a, 0) as u32, num(&a, 1) as i32)))?;
    request("quest_talk", |a| Some(Req::Talk(num(&a, 0) as u32, num(&a, 1) as u32, num(&a, 2) as u32)))?;
    request("quest_drop", |a| Some(Req::Drop(num(&a, 0) as u32)))?;
    request("quest_end", |_| Some(Req::End))?;
    request("quest_directing", |a| Some(Req::Directing(matches!(a.front(), Some(mlua::Value::Boolean(true))))))?;
    // the close-up camera's requests follow the talk itself, not the NPC's scope: the camera must come off even after the scope ended
    let camera_request = |name: &str, f: fn(mlua::MultiValue) -> Req| -> mlua::Result<()> {
        let b = bridge.clone();
        hud.raw_set(
            name,
            lua.create_function(move |_, args: mlua::MultiValue| {
                b.borrow_mut().dialog.requests.push(f(args));
                Ok(())
            })?,
        )
    };
    camera_request("quest_camera", |a| Req::Camera(matches!(a.front(), Some(mlua::Value::Boolean(true)))))?;
    camera_request("quest_frame", |a| Req::Frame(num(&a, 0) as u32))?;
    // `quest::try_context`: whether the close-up is on for the quest, by its `use_quest_camera` and the option's bit
    let b = bridge.clone();
    hud.raw_set(
        "quest_close_up",
        lua.create_function(move |_, (quest, option): (u32, u32)| Ok(b.borrow().game.get().is_some_and(|g| g.quests.close_up(quest, option))))?,
    )?;
    Ok(())
}

/// What the bubble `bubble` frames: the speaker's unit and the `quest_cameras` row (`quest_camera::frame_speaker`). The speaker is the
/// bubble's NPC when it is in view (the nearest of its template to the talk's NPC), else the talk's NPC (also for the player's own lines
/// and the system's); the row is the bubble's `camera_id`, else the speaker's model's.
fn framing(d: &HudData, units: &Units, bubble: u32, previous: Option<&QuestFrame>) -> Option<QuestFrame> {
    let game = d.game.get()?;
    let b = game.quests.bubbles.get(&bubble)?;
    let (talk_obj, talk_template) = d.dialog.npc?;
    let pos = |u: &aa_net::model::Unit| Vec3::new(u.pos.x, u.pos.y, u.pos.z);
    let at = units.by_id.get(&talk_obj).map(pos);
    let own_voice = if b.npc != 0 && b.npc != talk_template {
        units
            .by_id
            .values()
            .filter(|u| matches!(u.kind, UnitKind::Npc { template_id, .. } if template_id == b.npc) && !u.dead)
            .min_by(|x, y| {
                let dist = |u: &aa_net::model::Unit| at.map_or(0.0, |a| a.distance(pos(u)));
                dist(x).total_cmp(&dist(y))
            })
            .map(|u| u.obj_id)
    } else {
        None
    };
    let speaker = own_voice.unwrap_or(talk_obj);
    let model = units.by_id.get(&speaker)?.model_id;
    let row = game.quests.camera_row(b.camera, model, previous.map(|p| p.row.id))?;
    // the facing is taken again only when the speaker changed
    let facing = previous.filter(|p| p.speaker == speaker).and_then(|p| p.facing);
    Some(QuestFrame { speaker, row, facing })
}

/// What the requirements read of the player.
#[derive(SystemParam)]
pub struct Facts<'w> {
    player: Res<'w, Player>,
    quests: Res<'w, Quests>,
    stats: Res<'w, PlayerStats>,
    inventory: Res<'w, Inventory>,
    skills: Res<'w, Skills>,
    buffs: Res<'w, Buffs>,
}

impl Facts<'_> {
    fn get(&self) -> Option<PlayerFacts<'_>> {
        let spawn = self.player.spawn.as_ref()?;
        Some(PlayerFacts::of(spawn, &self.stats, &self.inventory, &self.skills, &self.buffs, &self.quests))
    }
}

/// What NPC `template` does for the player now.
fn refresh_npc(d: &mut HudData, facts: &PlayerFacts) {
    d.dialog.start.clear();
    d.dialog.complete.clear();
    d.dialog.talk.clear();
    let Some((_, template)) = d.dialog.npc else { return };
    let Some(game) = d.game.get() else { return };
    let (start, complete, talk) = (game.quests.offers(template, facts), game.quests.completes(template, facts), game.quests.talks(template, facts));
    d.dialog.start = start;
    d.dialog.complete = complete;
    d.dialog.talk = talk;
}

/// An event for the scripts, raised once the shared state is released.
enum Fire {
    Updated(u32, &'static str),
    Objective(String),
    Condition(&'static str),
}

/// Folds the server's quest state into what the scripts read and raises the quest events: a new
/// quest (`started`, with "Quest accepted: [...]"), a counter or status change (`updated`, with the
/// objective's new text), a quest handed in (`completed`) or given up (`dropped`).
pub fn sync_quests(rt: Option<NonSend<UiRuntime>>, bridge: NonSend<HudBridge>, facts: Facts) {
    if bridge.0.borrow().game.get().is_none() {
        return;
    }
    let fresh = !bridge.0.borrow().dialog.seen;
    if !fresh && !facts.quests.is_changed() {
        return;
    }
    let quests = &facts.quests;
    let mut fire: Vec<Fire> = Vec::new();
    {
        let mut d = bridge.0.borrow_mut();
        let d = &mut *d;
        let Some(game) = d.game.get() else { return };
        let old = std::mem::take(&mut d.dialog.quests);
        let texts = Texts { game, player: d.snapshot.name.as_str(), race: d.snapshot.race };
        let name = |id: u32| game.quests.def(id).map(|q| texts.expand(&q.name)).unwrap_or_default();
        let mut condition = None;
        if !fresh {
            for q in &quests.active {
                match old.iter().find(|o| o.template_id == q.template_id) {
                    None => {
                        condition = Some(format!("Quest accepted: [{}]", name(q.template_id)));
                        fire.push(Fire::Updated(q.template_id, "started"));
                        fire.push(Fire::Condition("started"));
                    }
                    Some(o) if o != q => {
                        fire.push(Fire::Updated(q.template_id, "updated"));
                        // the objectives that moved, with their new counters
                        let (before, after) = (game.quests.objectives(o, &texts), game.quests.objectives(q, &texts));
                        for (i, a) in after.iter().enumerate() {
                            if q.status == status::PROGRESS && before.get(i).is_none_or(|b| b.done != a.done) && a.done > 0 {
                                fire.push(Fire::Objective(a.text.clone()));
                            }
                        }
                        if o.status != q.status && q.status == status::READY {
                            for a in &after {
                                fire.push(Fire::Objective(a.text.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            for o in &old {
                if quests.active.iter().any(|q| q.template_id == o.template_id) {
                    continue;
                }
                if quests.is_completed(o.template_id) {
                    condition = Some(format!("Completed Quest: [{}]", name(o.template_id)));
                    fire.push(Fire::Updated(o.template_id, "completed"));
                    fire.push(Fire::Condition("completed"));
                } else {
                    fire.push(Fire::Updated(o.template_id, "dropped"));
                }
            }
        }
        if let Some(c) = condition {
            d.dialog.condition = c;
        }
        d.dialog.quests = quests.active.clone();
        d.dialog.version += 1;
        d.dialog.seen = true;
        if let Some(f) = facts.get() {
            refresh_npc(d, &f);
        }
    }
    let Some(rt) = rt else { return };
    for f in fire {
        match f {
            Fire::Updated(qtype, condition) => {
                info!("hud: quest {qtype} {condition}");
                rt.fire_event("QUEST_CONTEXT_UPDATED", (qtype, condition));
            }
            Fire::Objective(text) => rt.fire_event("QUEST_CONTEXT_OBJECTIVE_EVENT", text),
            Fire::Condition(condition) => rt.fire_event("QUEST_CONTEXT_CONDITION_EVENT", ("", condition)),
        }
    }
}

/// The pointer and the 3D view, for the right click.
#[derive(SystemParam)]
pub struct Pointer<'w, 's> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    over_ui: Res<'w, HudPointerOverUi>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
}

/// The NPC a right click released over the 3D view picked (not a drag, not on the HUD).
fn right_click_pick(p: &Pointer, units: &Units, own: u32, pressed_at: &mut Option<Vec2>, blocked: bool) -> Option<u32> {
    if blocked { *pressed_at = None; return None; }
    let window = p.windows.single().ok()?;
    let cursor = window.cursor_position();
    if p.mouse.just_pressed(MouseButton::Right) {
        *pressed_at = if p.over_ui.0 { None } else { cursor };
    }
    if !p.mouse.just_released(MouseButton::Right) {
        return None;
    }
    let (start, end) = (pressed_at.take()?, cursor?);
    if start.distance(end) > CLICK_SLOP || p.over_ui.0 {
        return None;
    }
    let (camera, transform) = p.cameras.iter().find(|(c, _)| c.is_active)?;
    let ray = camera.viewport_to_world(transform, end).ok()?;
    let mut best: Option<(f32, u32)> = None;
    for u in units.iter().filter(|u| {
        u.obj_id != own && matches!(u.kind, UnitKind::Npc { .. } | UnitKind::Slave { .. })
    }) {
        let centre = aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z + 0.9 * u.scale.max(0.5)));
        let along = (centre - ray.origin).dot(*ray.direction);
        if along <= 0.0 {
            continue;
        }
        let miss = (ray.origin + *ray.direction * along).distance(centre);
        if miss <= PICK_RADIUS * u.scale.max(0.7) && best.is_none_or(|(a, _)| along < a) {
            best = Some((along, u.obj_id));
        }
    }
    best.map(|(_, id)| id)
}

/// `--hud-talk-nearest T` (talk to the nearest NPC that has something for the player, T seconds
/// after the HUD is up), `--hud-quest-key K T` (K = d / f / g: the dialog's keys; f with no dialog
/// talks to the nearest NPC, like the key), `--hud-quest-accept Q T` (ask for quest Q with no NPC,
/// as an item or a sphere does) and `--hud-doodad-quest Q T` (the nearest doodad that starts quest Q
/// offers it, as the server's `SCDoodadQuestAccept` does when the doodad is used: the client cannot use
/// doodads yet): dev flags for unattended runs.
#[derive(Resource, Default)]
pub struct ScriptedTalk {
    talk_at: Option<f32>,
    keys: Vec<(f32, char)>,
    accept: Vec<(f32, u32)>,
    doodad_quest: Vec<(f32, u32)>,
    started: Option<f32>,
}

impl ScriptedTalk {
    pub fn from_args() -> ScriptedTalk {
        let args: Vec<String> = aa_dev::args();
        let mut s = ScriptedTalk::default();
        for (i, a) in args.iter().enumerate() {
            let time = |k: usize| args.get(i + k).and_then(|t| t.parse::<f32>().ok());
            if a == "--hud-talk-nearest" {
                s.talk_at = time(1);
            } else if a == "--hud-quest-key"
                && let (Some(k), Some(t)) = (args.get(i + 1).and_then(|k| k.chars().next()), time(2))
            {
                s.keys.push((t, k.to_ascii_lowercase()));
            } else if (a == "--hud-quest-accept" || a == "--hud-doodad-quest")
                && let (Some(q), Some(t)) = (args.get(i + 1).and_then(|q| q.parse::<u32>().ok()), time(2))
            {
                if a == "--hud-quest-accept" { s.accept.push((t, q)) } else { s.doodad_quest.push((t, q)) }
            }
        }
        // popped from the end
        s.keys.sort_by(|a, b| b.0.total_cmp(&a.0));
        s.accept.sort_by(|a, b| b.0.total_cmp(&a.0));
        s.doodad_quest.sort_by(|a, b| b.0.total_cmp(&a.0));
        s
    }
}

/// The NPC nearest to the player in reach that can be talked to (alive, not hostile); with `wanted`,
/// only one that has something for the player.
fn nearest_npc(d: &HudData, units: &Units, own: u32, own_faction: u32, facts: &PlayerFacts, range: f32, wanted: bool) -> Option<(u32, u32)> {
    let game = d.game.get();
    units
        .iter()
        .filter(|u| u.obj_id != own && !u.dead)
        .filter_map(|u| {
            let UnitKind::Npc { template_id, .. } = u.kind else { return None };
            let dist = d.pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
            if dist > range || super::plates::relation_of(game, own_faction, u) == "hostile" {
                return None;
            }
            if wanted && game.is_none_or(|g| g.quests.mark(template_id, facts).is_none()) {
                return None;
            }
            Some((dist, (u.obj_id, template_id)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, n)| n)
}

/// A right click on an NPC or F near one starts the talk; D / F / G drive the dialog while it is up.
#[allow(clippy::too_many_arguments)]
pub fn talk_input(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    pointer: Pointer,
    time: Res<Time>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    doodads: Res<Doodads>,
    target: Res<Target>,
    facts: Facts,
    mut script: ResMut<ScriptedTalk>,
    mut script_pick: ResMut<crate::script::ScriptPick>,
    mut pressed_at: Local<Option<Vec2>>,
    teleport: Option<Res<crate::ingame::teleport::Teleport>>,
    suspended: Res<super::HudSuspended>,
) {
    let Some(rt) = rt else { return };
    if suspended.0 || teleport.is_some_and(|t| t.loading()) || bridge.0.borrow().dialog.npc_flow.barrier { return; }
    let Some(spawn) = facts.player.spawn.as_ref() else { return };
    let (own, own_faction) = (spawn.obj_id, spawn.character().faction_id);
    let Some(player_facts) = facts.get() else { return };
    // the scripted keys and talk
    let now = time.elapsed_secs();
    let started = *script.started.get_or_insert(now);
    let mut scripted_key = None;
    while script.keys.last().is_some_and(|k| now - started >= k.0) {
        scripted_key = script.keys.pop().map(|k| k.1);
    }
    while script.accept.last().is_some_and(|a| now - started >= a.0) {
        if let Some((_, q)) = script.accept.pop() {
            bridge.0.borrow_mut().dialog.requests.push(Req::Accept(q));
        }
    }
    let mut doodad_offer = None;
    while script.doodad_quest.last().is_some_and(|a| now - started >= a.0) {
        doodad_offer = script.doodad_quest.pop().map(|a| a.1);
    }
    if let Some(quest) = doodad_offer {
        let offer = {
            let mut d = bridge.0.borrow_mut();
            let starter = d.game.get().and_then(|g| g.quests.parts_of(quest, kind::START).flat_map(|c| &c.acts).find(|a| a.ty == "ConAcceptDoodad").map(|a| a.target));
            let me = d.pos;
            let near = starter.and_then(|t| {
                doodads.by_id.values().filter(|o| o.template_id == t).min_by(|a, b| {
                    me.distance(Vec3::new(a.pos.x, a.pos.y, a.pos.z)).total_cmp(&me.distance(Vec3::new(b.pos.x, b.pos.y, b.pos.z)))
                })
            });
            match near {
                Some(o) if !d.dialog.directing => {
                    d.dialog.doodad = Some(o.obj_id);
                    Some(o.obj_id)
                }
                _ => {
                    warn!("hud: no doodad in view starts quest {quest} (template {starter:?})");
                    None
                }
            }
        };
        if let Some(obj_id) = offer {
            info!("hud: doodad {obj_id} offers quest {quest} (dev flag)");
            rt.fire_event("START_QUEST_CONTEXT_DOODAD", (quest, true, obj_id.to_string()));
        }
    }
    let scripted_talk = script.talk_at.is_some_and(|t| now - started >= t);
    if scripted_talk {
        script.talk_at = None;
    }
    let typing = focus.0 || rt.keyboard_captured();
    let pressed = |action: &str, index: u32, c: char| scripted_key == Some(c) || (!typing && actions.just_pressed(action, index));
    // a pick the script runner made (`interact`) takes the place of the ray cast, everything after it is the same
    let blocked = super::blocks_look_pointer(&bridge.0.borrow(), MouseButton::Right);
    let picked = right_click_pick(&pointer, &units, own, &mut pressed_at, blocked);
    let scripted_pick = script_pick.take_right();
    let picked = (!blocked).then_some(scripted_pick).flatten().or(picked);

    let mut d = bridge.0.borrow_mut();
    if d.dialog.directing {
        for (c, arg) in [('d', 1), ('f', 2), ('g', 3)] {
            if pressed("quest_directing_interaction", arg, c) {
                debug!("hud: dialog key {c}");
                drop(d);
                rt.fire_event("QUEST_DIRECTING_MODE_HOT_KEY", arg);
                return;
            }
        }
        return;
    }
    // a right click on a slave (cart, ship ...) sends the 1.2 interaction request (0x02c + 0x068,
    // never 0x031); the interact key (F) sends nothing for a slave and keeps the NPC talk path
    let interact_key = !scripted_talk
        && pressed("do_interaction_1", 0, 'f')
        && d.loot.nearest.is_none()
        && (scripted_key == Some('f') || !d.doodad.claims_key());
    if let Some(obj_id) = super::slave::action_target(&units, picked) {
        super::slave::interact(&mut d, &units, target.obj_id, obj_id);
        if scripted_pick == Some(obj_id) {
            // The script's `interact` only completes once this path ran; taking the pick alone
            // is not proof (it is taken before the blocked/directing checks above).
            script_pick.interacted(obj_id);
        }
        return;
    }
    let pointer_npc = if let Some(obj_id) = picked {
        // a right click: an NPC in reach that is not hostile (an enemy is attacked, not talked to)
        let game = d.game.get();
        units.get(obj_id).filter(|u| !u.dead && super::plates::relation_of(game, own_faction, u) != "hostile").and_then(|u| {
            let UnitKind::Npc { template_id, .. } = u.kind else { return None };
            let dist = d.pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
            if dist > TALK_RANGE {
                info!("hud: {} is too far away to talk to ({dist:.1} m)", u.name);
                return None;
            }
            Some((obj_id, template_id))
        })
    } else { None };
    let keyboard_npc = if scripted_talk {
        nearest_npc(&d, &units, own, own_faction, &player_facts, 60.0, true)
    } else if interact_key {
        nearest_npc(
            &d,
            &units,
            own,
            own_faction,
            &player_facts,
            TALK_RANGE,
            false,
        )
    } else {
        None
    };
    if let Some(npc) = pointer_npc {
        d.dialog.pointer_talk = Some(PointerTalk { npc, fallback: keyboard_npc });
        return;
    }
    let npc = keyboard_npc;
    let Some(npc) = npc else { return };
    drop(d);
    dispatch_talk(&bridge.0, npc, target.obj_id);
}

fn dispatch_talk(shared: &Shared, npc: (u32, u32), target: Option<u32>) {
    let mut data = shared.borrow_mut();
    if super::portal::use_gate(&mut data, npc.0, npc.1) { return; }
    query_npc(&mut data, npc, target);
}

pub(super) fn talk_pointer(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    target: Res<Target>,
    teleport: Option<Res<crate::ingame::teleport::Teleport>>,
    suspended: Res<super::HudSuspended>,
) {
    let pending = bridge.0.borrow_mut().dialog.pointer_talk.take();
    let (Some(PointerTalk { npc: pointer, fallback }), Some(_rt)) = (pending, rt) else {
        return;
    };
    if suspended.0 || teleport.is_some_and(|t| t.loading()) || bridge.0.borrow().dialog.npc_flow.barrier { return; }
    let candidate = {
        let data = bridge.0.borrow();
        if data.dialog.directing {
            return;
        }
        if super::blocks_look_pointer(&data, MouseButton::Right) {
            fallback
        } else {
            Some(pointer)
        }
    };
    if let Some(npc) = candidate {
        dispatch_talk(&bridge.0, npc, target.obj_id);
    }
}

/// The scripts' requests become packets; the dialog hides the HUD and brings it back. A doodad's quest
/// offer (`SCDoodadQuestAccept`, when the player used a notice board ...) opens the dialog on it.
pub fn handle_dialog(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
    units: Res<Units>,
    mut camera: Option<ResMut<QuestTalkCamera>>,
) {
    for NetEvent(e) in events.read() {
        if let Event::Interaction(InteractEvent::DoodadQuestAccept { doodad_obj_id, quest_id }) = e
            && let Some(rt) = rt.as_deref()
        {
            let directing = {
                let mut d = bridge.0.borrow_mut();
                if !d.dialog.directing {
                    d.dialog.doodad = Some(*doodad_obj_id);
                }
                d.dialog.directing
            };
            if !directing {
                info!("hud: doodad {doodad_obj_id} offers quest {quest_id}");
                rt.fire_event("START_QUEST_CONTEXT_DOODAD", (*quest_id, true, doodad_obj_id.to_string()));
            }
        }
    }
    let mut d = bridge.0.borrow_mut();
    let requests = std::mem::take(&mut d.dialog.requests);
    for r in requests {
        let r = if let Req::AtNpc(scope, request) = r {
            if !scope_current(&d, scope) {
                if matches!(*request, Req::Directing(false)) && active_scope(&d).is_none() {
                    if let Some(rt) = rt.as_deref() { apply_directing(rt, &mut d.dialog, false); }
                }
                continue;
            }
            *request
        } else { r };
        let npc_obj_id = d.dialog.npc.map_or(0, |n| n.0);
        let doodad_obj_id = if npc_obj_id == 0 { d.dialog.doodad.unwrap_or(0) } else { 0 };
        let send = |net: &mut MessageWriter<NetCommand>, m: Interact| {
            info!("hud: quest request {m:?}");
            net.write(NetCommand(Command::Interact(m)));
        };
        match r {
            Req::Accept(quest_id) => send(&mut net, Interact::QuestStart { quest_id, npc_obj_id, doodad_obj_id, sphere_id: 0 }),
            Req::Complete(quest_id, selected) => send(&mut net, Interact::QuestComplete { quest_id, npc_obj_id, doodad_obj_id, selected }),
            Req::Talk(quest_id, component_id, act_id) => send(&mut net, Interact::QuestTalk { npc_obj_id, quest_id, component_id, act_id }),
            Req::AtNpc(..) => unreachable!("NPC request is unwrapped once"),
            Req::Drop(quest_id) => send(&mut net, Interact::QuestDrop { quest_id }),
            Req::End => end_talk(&mut d, &mut net),
            Req::Directing(on) => {
                if let Some(rt) = rt.as_deref() {
                    apply_directing(rt, &mut d.dialog, on);
                }
                if !on {
                    end_talk(&mut d, &mut net);
                }
            }
            Req::Camera(on) => {
                if let Some(camera) = camera.as_deref_mut() {
                    camera.on = on;
                    if !on {
                        camera.frame = None;
                    }
                }
            }
            Req::Frame(bubble) => {
                // no speaker found leaves the close-up (`quest_camera::leave`)
                if let Some(camera) = camera.as_deref_mut() {
                    let previous = camera.frame;
                    camera.frame = framing(&d, &units, bubble, previous.as_ref());
                }
            }
        }
    }
    // the close-up lives only inside the talk's directing mode, however the talk ended
    // (read first: taking the resource mutably marks it changed every frame)
    if !d.dialog.directing
        && camera.as_deref().is_some_and(|c| c.on || c.frame.is_some())
        && let Some(camera) = camera.as_deref_mut()
    {
        *camera = QuestTalkCamera::default();
    }
    // what came up since (the target frame of the NPC talked to ...) waits for the dialog to end too
    if d.dialog.directing
        && let Some(rt) = rt.as_deref()
    {
        hide_others(rt, &mut d.dialog);
    }
}

fn end_talk(d: &mut HudData, _net: &mut MessageWriter<NetCommand>) {
    d.dialog.doodad = None;
    if let Some(scope) = active_scope(d).or_else(|| npc::cancel_scope(d)) { close_npc(d, scope); }
    d.dialog.start.clear(); d.dialog.complete.clear(); d.dialog.talk.clear();
}

/// The dialog has the screen to itself: every other top-level widget is hidden while it is up (the
/// original hides the UI layers below its own) and the ones that were shown come back after.
fn apply_directing(rt: &UiRuntime, dialog: &mut DialogView, on: bool) {
    if dialog.directing == on {
        return;
    }
    dialog.directing = on;
    if on {
        dialog.hidden.clear();
        hide_others(rt, dialog);
    } else {
        stop_directing(&rt.lua, dialog);
    }
}

fn stop_directing(lua: &Lua, dialog: &mut DialogView) {
    dialog.directing = false;
    let mut u = aa_ui::lua::ui(lua);
    for id in std::mem::take(&mut dialog.hidden) {
        if u.tree.get(id).alive { u.tree.get_mut(id).shown = true; }
    }
}

/// Hides the top-level widgets shown besides the dialog's own, remembering them.
fn hide_others(rt: &UiRuntime, dialog: &mut DialogView) {
    let mut u = aa_ui::lua::ui(&rt.lua);
    let shown: Vec<aa_ui::tree::Id> = u
        .tree
        .top
        .iter()
        .copied()
        .filter(|&id| {
            let o = u.tree.get(id);
            o.alive && o.shown && !DIRECTING_WIDGETS.contains(&o.name.as_str())
        })
        .collect();
    for id in shown {
        u.tree.get_mut(id).shown = false;
        dialog.hidden.push(id);
    }
}

/// The client side of the quest windows' `X2Quest` calls, run after `ingame.lua`.
const QUEST_LUA: &str = include_str!("quest.lua");

pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(QUEST_LUA).set_name("@quest.lua").exec()
}

pub(crate) fn npc_fixture() { npc::fixture::run(); }

/// A new game starts with no talk and no quests seen.
fn reset(bridge: NonSend<HudBridge>, camera: Option<ResMut<QuestTalkCamera>>) {
    if let Some(mut camera) = camera {
        *camera = QuestTalkCamera::default();
    }
    let mut d = bridge.0.borrow_mut();
    // Loading does not imply a new PlayerSpawn. Keep the received tracker and
    // monotonic stamps, while retiring all pending/active UI work.
    let mut npc_flow = std::mem::take(&mut d.dialog.npc_flow);
    npc_flow.retire();
    d.dialog = DialogView { npc_flow, ..Default::default() };
}

pub fn plugin(app: &mut App) {
    app.add_systems(Update, npc::track_npc.before(aa_ui::UiInput).before(super::sync_hud).before(super::prebuild_hud).before(super::build_hud));
    app.insert_resource(ScriptedTalk::from_args()).add_systems(OnExit(crate::Screen::InGame), reset)
        .add_systems(Update, talk_pointer.after(super::PointerFinished).after(talk_input).run_if(super::hud_up).run_if(in_state(crate::Screen::InGame)));
}
