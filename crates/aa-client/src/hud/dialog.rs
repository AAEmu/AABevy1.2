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

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;
use web_time::Instant;

use aa_net::interaction::Interact;
use aa_net::model::{Unit, UnitKind};
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
use crate::ingame::units::doodad_world;
use super::data::GameData;
use super::quests::{CinemaDef, PlayerFacts, Texts, kind};
use super::{HudKeyboardFocus, HudPointerOverUi};

/// `FUN_393bf270` / `FUN_39093860`: the common interaction reach, the 3D surface gap strictly under
/// `DAT_39ac79fc` 9.0, i.e. 3.0 m for a live target, and `DAT_39ac7a00` 12.25 (3.5 m) for a dead one:
/// `+0x3c26` is the **target's** dead flag (`FUN_390b4df0` sets it; `FUN_393be4e0` reports `TARGET_DIED`).
/// The same predicate gates the interaction skill a talk uses (`FUN_393be280`) and the right-click pick:
/// the 2026-10-28 capture sent `CSStartInteraction` at 2.46 m and nothing at 4.57 m.
const INTERACT_REACH: f32 = 3.0;
/// The dead-target reach, `DAT_39ac7a00` 12.25 squared (corpse/loot).
const DEAD_INTERACT_REACH: f32 = 3.5;
/// How long a walk to an NPC may go without getting nearer before it is given up, seconds (the doodad walk's
/// bound; the native click-to-move has no recovered timeout).
const NPC_WALK_STUCK_SECS: f32 = 3.0;
/// The keys that end the walk to an NPC (the player takes over), as for the doodad walk.
const NPC_WALK_CANCEL_ACTIONS: [&str; 5] = ["moveforward", "moveback", "moveleft", "moveright", "jump"];
/// A press that moves more than this many pixels before it is released was a camera drag.
const CLICK_SLOP: f32 = 6.0;
/// The widgets the dialog leaves shown when it hides the HUD.
const DIRECTING_WIDGETS: [&str; 2] = ["questDirecting", "QuestDirectingFadeWindow"];

/// What the scripts asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Req {
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
    /// The directing window about to play a step (`HUD.quest_cinema_permission`): 0x0d0.
    CinemaPermission { quest: u32, kind: u8 },
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

/// The walk to an NPC that was right clicked beyond the interaction reach (`FUN_390a6af0`, the client's
/// click-to-move, entered from `FUN_393bf270` when the interaction skill is out of reach). The use follows
/// when the 3D surface gap is inside the reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Approach {
    /// Object id and template of the NPC clicked.
    pub npc: (u32, u32),
    /// The authoritative target when the walk started: a different `Some` target (a new pick) cancels
    /// it; the old target going away (`None`) does not (the mount walk's rule).
    target: Option<u32>,
    /// The nearest gap so far, metres, and when it was reached (`Time::elapsed_secs`).
    best: f32,
    best_at: f32,
}

/// A timed quest's clock (`FUN_3937b240`): the server's remaining milliseconds at receipt and the
/// local receipt time. `left = end - (now - start) * 1000` (`FUN_39379e30`).
#[derive(Clone, Copy, Debug)]
pub struct QuestClock {
    pub end_ms: i32,
    pub started: Instant,
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
    /// The quest whose step the dialog is directing (`quest_context_directing.alb`'s window
    /// `questType`): set only by `HUD.quest_show` when the window starts playing a step (and
    /// cleared when directing ends), so a late refusal for an older quest cannot end a newer
    /// window. Requests do not set it.
    pub quest: Option<u32>,
    /// The top-level widgets hidden for it.
    hidden: Vec<aa_ui::tree::Id>,
    /// The movie wants every top-level widget hidden (C5).
    pub cinema_hidden: bool,
    /// The UI tree has the movie's hidden set applied and its restore is still owed.
    cinema_applied: bool,
    pub(super) cinema_restore_pending: bool,
    cinema_restore: Vec<aa_ui::tree::Id>,
    /// The active quests as the scripts last saw them, and its version (`HUD.quest_version()`).
    pub quests: Vec<Quest>,
    /// The clock of each timed quest (`X2Quest:GetQuestContextLeftTime`), on the local clock:
    /// `FUN_3937b240` stores the server's `time_left_ms` as its end and the local receipt time as
    /// its start; `FUN_39379e30` returns `end - (now - start) * 1000`.
    pub left_times: BTreeMap<u32, QuestClock>,
    /// Quest instances whose clock a refusal cleared: `FUN_3937a130` removes the timer entry and
    /// the tick must not add it again while the same quest is active.
    cleared_left: HashSet<(u32, i64)>,
    /// Quest instances whose clock has been started (Started / Updated, or the login list's first
    /// tick): an expired clock is not restarted by the tick; the next Started / Updated is
    /// (`FUN_3937b240` is the only creator).
    seeded_left: HashSet<(u32, i64)>,
    pub version: u64,
    /// Whether `quests` holds the server's list yet (the first one raises no events).
    seen: bool,
    /// The message of the last quest event ("Quest accepted: [A Friendly Reminder]"):
    /// `X2Quest:GetActiveQuestContextConditionMessage()`.
    pub condition: String,
    pub(crate) requests: Vec<Req>,
    pointer_talk: Option<PointerTalk>,
    /// The walk to a right-clicked NPC beyond the interaction reach ([`Approach`]).
    pub(super) approach: Option<Approach>,
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
        "quest_show",
        lua.create_function(move |_, qtype: u32| {
            b.borrow_mut().dialog.quest = (qtype != 0).then_some(qtype);
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "quest_left_time",
        lua.create_function(move |_, qtype: u32| {
            let b = b.borrow();
            Ok(b.dialog.left_times.get(&qtype).map_or(-1, |clock| clock_left_ms(clock, Instant::now())))
        })?,
    )?;
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
    let b = bridge.clone();
    hud.raw_set(
        "quest_cinema_permission",
        lua.create_function(move |_, (quest, which): (u32, String)| {
            let kind = match which.as_str() {
                "start" => kind::START,
                "complete" => kind::READY,
                _ => return Ok(()),
            };
            b.borrow_mut().dialog.requests.push(Req::CinemaPermission { quest, kind });
            Ok(())
        })?,
    )?;
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

/// Milliseconds left of `clock` at `now` (`FUN_39379e30`): `end - (now - start) * 1000`, with the
/// start the local time the Started / Updated packet was received (`FUN_3937b240`). The server's
/// own accept stamp is not used.
pub fn clock_left_ms(clock: &QuestClock, now: Instant) -> i32 {
    let elapsed_ms = i64::try_from(now.saturating_duration_since(clock.started).as_millis()).unwrap_or(i64::MAX);
    (i64::from(clock.end_ms) - elapsed_ms).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// One tick of the quest clock (`X2::GameClient::UpdateQuestLeftTime`, `FUN_3937cd80`): every clock
/// with at least 1000 ms left is published as `(qtype, remaining_ms)`. A clock under that, or one
/// whose quest is no longer timed, takes the native timeout path (`FUN_39379ea0`): it is removed
/// and `QUEST_LEFT_TIME_UPDATED(qtype, 0)` is published once, so the notifier hides it
/// (`quest_context_notifier.alb` F81).
pub fn tick_left_times(left: &mut BTreeMap<u32, QuestClock>, quests: &[Quest], now: Instant) -> Vec<(u32, i32)> {
    let mut events = Vec::new();
    left.retain(|qtype, clock| {
        let timed = quests.iter().any(|q| q.template_id == *qtype && q.time_left_ms > 0);
        if !timed || clock_left_ms(clock, now) < 1000 {
            events.push((*qtype, 0));
            return false;
        }
        true
    });
    for (qtype, clock) in left.iter() {
        events.push((*qtype, clock_left_ms(clock, now)));
    }
    events
}

/// One tick over the dialog's clocks: drop latches of quests no longer active, start the clock of
/// a timed active quest seen for the first time at `now` (the login list has no Started packet),
/// then [`tick_left_times`]. A refused instance stays latched until a Started / Updated releases it.
fn tick_quest_clocks(dialog: &mut DialogView, quests: &[Quest], now: Instant) -> Vec<(u32, i32)> {
    let active: HashSet<(u32, i64)> = quests.iter().map(|q| (q.template_id, q.id)).collect();
    dialog.cleared_left.retain(|key| active.contains(key));
    dialog.seeded_left.retain(|key| active.contains(key));
    let timed: Vec<Quest> = quests
        .iter()
        .filter(|q| q.status == status::PROGRESS && q.time_left_ms > 0)
        .filter(|q| !dialog.cleared_left.contains(&(q.template_id, q.id)))
        .cloned()
        .collect();
    for q in &timed {
        // The login list has no Started packet: seed its clock once, at this tick's local time.
        // An expired clock stays gone until the server sends the next Started / Updated.
        if dialog.seeded_left.insert((q.template_id, q.id)) {
            dialog.left_times.entry(q.template_id).or_insert(QuestClock { end_ms: q.time_left_ms, started: now });
        }
    }
    tick_left_times(&mut dialog.left_times, &timed, now)
}

/// The quest clock runs at most once a second (the native's 1.0 s constant `0x3999c488`) and tells
/// the tracker the remaining milliseconds of every timed quest (`QUEST_LEFT_TIME_UPDATED`).
pub fn tick_quest_left_time(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    quests: Res<Quests>,
    mut last: Local<Option<Instant>>,
) {
    let now = Instant::now();
    if last.is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
        return;
    }
    *last = Some(now);
    let events = {
        let mut d = bridge.0.borrow_mut();
        if d.game.get().is_none() {
            return;
        }
        tick_quest_clocks(&mut d.dialog, &quests.active, now)
    };
    let Some(rt) = rt else { return };
    for (qtype, left) in events {
        rt.fire_event("QUEST_LEFT_TIME_UPDATED", (qtype, left));
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

/// The NPC or vehicle a right click released over the 3D view picked (not a drag, not on the HUD),
/// with its distance along the ray.
fn right_click_pick(
    p: &Pointer,
    units: &Units,
    own: u32,
    ridden: Option<u32>,
    pressed_at: &mut Option<Vec2>,
    blocked: bool,
    pick: &mut super::pick::PickWorld,
) -> Option<(f32, u32)> {
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
    let (entry, id) = super::pick::pick_unit_hit(
        &ray,
        units,
        own,
        ridden,
        |u| matches!(u.kind, UnitKind::Npc { .. } | UnitKind::Slave { .. }),
        |unit, ray| pick.data(unit, units, ray),
    )?;
    let unit = units.get(id)?;
    let (at, model) = pick.data(unit, units, &ray);
    let along = crate::hud::pick::unit_compare_hit(&ray, unit, at, model).unwrap_or(entry);
    Some((along, id))
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

/// The interaction reach: 3.0 m for a live target, 3.5 m for a dead one (`FUN_393bf270`'s `+0x3c26`
/// target-dead variant; `FUN_39093860` compares strictly).
fn interact_reach(unit: &Unit) -> f32 {
    if unit.dead { DEAD_INTERACT_REACH } else { INTERACT_REACH }
}

/// The unit's scaled actor-model radius, the same `Unit.ModelSize` source the server and `api::distance_to` use.
fn npc_radius(game: Option<&GameData>, unit: &Unit) -> f32 {
    game.map_or(0.0, |g| g.model_size(unit.model_id, unit.scale))
}

/// The 3D surface gap from the player to an NPC (`FUN_39093860`'s geometry distance, approximated with one
/// scaled actor-model radius per unit like the slave path; the exact collider pair is not recovered).
fn npc_gap(d: &HudData, me: Vec3, at: Vec3, unit: &Unit) -> f32 {
    super::slave::surface_gap(me, d.player_radius, at, npc_radius(d.game.get(), unit))
}

/// Whether an NPC is inside the interaction reach. `FUN_39093860` returns inside only while the gap is
/// strictly under the limit, so exactly 3.0 m (3.5 m mounted) is already out.
fn npc_in_reach(d: &HudData, me: Vec3, at: Vec3, unit: &Unit) -> bool {
    npc_gap(d, me, at, unit) < interact_reach(unit)
}

/// What a right click on an NPC does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NpcClick {
    /// Inside the reach: the talk starts (`CSChangeTarget` + `CSInteractNPC`/`CSStartInteraction`).
    Talk((u32, u32)),
    /// Beyond the reach: walk there first and send nothing yet.
    Walk((u32, u32), f32),
}

/// The right-click gate: a live, non-hostile NPC is talked to inside the interaction reach and otherwise
/// walked to (`FUN_393bf270` -> `FUN_390a6af0`); a hostile NPC (an enemy is attacked, not talked to) and a
/// non-NPC are ignored.
fn plan_npc_click(
    d: &HudData,
    units: &Units,
    own_faction: u32,
    me: Vec3,
    obj_id: u32,
    at: Vec3,
) -> Option<NpcClick> {
    let game = d.game.get();
    let unit = units
        .get(obj_id)
        .filter(|u| !u.dead && super::plates::relation_of(game, own_faction, u) != "hostile")?;
    let UnitKind::Npc { template_id, .. } = unit.kind else { return None };
    let gap = npc_gap(d, me, at, unit);
    Some(if gap < interact_reach(unit) {
        NpcClick::Talk((obj_id, template_id))
    } else {
        NpcClick::Walk((obj_id, template_id), gap)
    })
}

/// The NPC nearest to the player in reach that can be talked to (alive, not hostile); with `wanted`,
/// only one that has something for the player. `range` is the surface gap limit.
#[allow(clippy::too_many_arguments)]
fn nearest_npc(
    d: &HudData,
    me: Vec3,
    units: &Units,
    own: u32,
    own_faction: u32,
    facts: &PlayerFacts,
    range: f32,
    wanted: bool,
    world: &impl Fn(&Unit) -> Option<Vec3>,
) -> Option<(u32, u32)> {
    let game = d.game.get();
    units
        .iter()
        .filter(|u| u.obj_id != own && !u.dead)
        .filter_map(|u| {
            let UnitKind::Npc { template_id, .. } = u.kind else { return None };
            let at = world(u).unwrap_or(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
            let gap = npc_gap(d, me, at, u);
            // A scripted `talk` searches its whole range; the interact key (F) only reaches what the
            // right click could talk to.
            let limit = if wanted { range } else { range.min(interact_reach(u)) };
            if gap >= limit || super::plates::relation_of(game, own_faction, u) == "hostile" {
                return None;
            }
            // with no game data there is no mark to prefer: keep the NPC rather than filtering all out
            if wanted && game.is_some_and(|g| g.quests.mark(template_id, facts).is_none()) {
                return None;
            }
            Some((gap, (u.obj_id, template_id)))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, n)| n)
}

/// The parent slave of the right-clicked ladder, if any (a ship's side ladder).
fn parent_of_climb(doodads: &Doodads, climb: Option<(f32, u32)>) -> Option<u32> {
    climb.and_then(|(_, id)| {
        doodads
            .by_id
            .get(&id)
            .map(|o| o.parent_obj_id)
            .filter(|parent| *parent != 0)
    })
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
    mut pick: super::pick::PickWorld,
    mut script: ResMut<ScriptedTalk>,
    mut script_pick: ResMut<crate::script::ScriptPick>,
    mut pressed_at: Local<Option<Vec2>>,
    (teleport, ladder_claim, ladder_grab): (
        Option<Res<crate::ingame::teleport::Teleport>>,
        Option<Res<crate::ingame::ladder::LadderClaim>>,
        Option<ResMut<crate::ingame::ladder::LadderGrab>>,
    ),
    suspended: Res<super::HudSuspended>,
) {
    let Some(rt) = rt else { return };
    if suspended.0 || teleport.is_some_and(|t| t.loading()) || bridge.0.borrow().dialog.npc_flow.barrier { return; }
    let Some(spawn) = facts.player.spawn.as_ref() else { return };
    let (own, own_faction) = (spawn.obj_id, spawn.character().faction_id);
    // seated, the server moves only the slave: the HUD position can stay at the boarding point
    let ridden = spawn.unit.attached_to.map(|a| a.obj_id);
    let me = {
        let world = |id: u32| units.get(id).and_then(|u| pick.pose(u, &units));
        super::slave::player_centre(&bridge.0.borrow(), &world)
    };
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
            let near = starter.and_then(|t| {
                doodads.by_id.values().filter(|o| o.template_id == t)
                    .filter_map(|o| Some((me.distance(doodad_world(o, &units)?), o)))
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, o)| o)
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
    // A right-clicked climb doodad takes the click only when it is in front of the unit/ship
    // pick: a nearer unit (the hull included) keeps the right click (`climb_click` is this
    // frame's nearest ladder distance and id along the ray).
    let over_climb = bridge.0.borrow().doodad.climb_click();
    let unit = right_click_pick(
        &pointer,
        &units,
        own,
        ridden,
        &mut pressed_at,
        blocked,
        &mut pick,
    );
    let ladder_parent = parent_of_climb(&doodads, over_climb);
    let ladder_wins = super::pick::doodad_beats_unit(over_climb, unit, ladder_parent);
    if ladder_wins
        && let Some((_, id)) = over_climb
        && let Some(mut grab) = ladder_grab
    {
        // inferred: the native use-target path FUN_393b8f40 is the only CSHang sender, but no
        // right-click-on-ladder capture exists
        grab.0 = Some(id);
    }
    let picked = if ladder_wins {
        None
    } else {
        unit.map(|(_, id)| id)
    };
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
    let ladder_owns_key = ladder_claim
        .as_deref()
        .is_some_and(|claim| claim.0.is_some());
    let interact_pressed = !scripted_talk && pressed("do_interaction_1", 0, 'f');
    let interact_key = interact_pressed
        && d.loot.nearest.is_none()
        && (scripted_key == Some('f') || (!d.doodad.claims_key() && !ladder_owns_key));
    if let Some(obj_id) = super::slave::action_target(&units, picked) {
        cancel_walk(&mut d);
        super::slave::interact(&mut d, &units, target.obj_id, obj_id);
        if scripted_pick == Some(obj_id) {
            // The script's `interact` only completes once this path ran; taking the pick alone
            // is not proof (it is taken before the blocked/directing checks above).
            script_pick.interacted(obj_id);
        }
        return;
    }
    let pointer_npc = picked.and_then(|obj_id| {
        // a right click: an NPC that is not hostile (an enemy is attacked, not talked to); beyond the
        // interaction reach the click walks there first (`FUN_393bf270` -> `FUN_390a6af0`) and sends nothing
        let at = units.get(obj_id).map_or(Vec3::ZERO, |u| {
            pick.pose(u, &units)
                .map_or(Vec3::new(u.pos.x, u.pos.y, u.pos.z), |(pos, _)| pos)
        });
        match plan_npc_click(&d, &units, own_faction, me, obj_id, at) {
            Some(NpcClick::Walk(npc, gap)) => {
                if seated_or_driving(&d) {
                    info!("hud: {} is {gap:.1} m away but the player is seated: not walking", units.get(obj_id).map_or("?", |u| u.name.as_str()));
                    cancel_walk(&mut d);
                    return None;
                }
                let reach = units.get(obj_id).map_or(INTERACT_REACH, interact_reach);
                info!("hud: {} is {gap:.1} m away (reach {reach:.1} m): walking there", units.get(obj_id).map_or("?", |u| u.name.as_str()));
                start_walk(&mut d, npc, gap, now, target.obj_id);
                None
            }
            Some(NpcClick::Talk(npc)) => {
                cancel_walk(&mut d);
                Some(npc)
            }
            None => {
                // a hostile unit or a non-NPC: the click is not a talk, and it ends a running walk
                cancel_walk(&mut d);
                None
            }
        }
    });
    if interact_pressed || scripted_talk {
        // F / a scripted talk is a new action: it ends a running walk, also when a doodad or loot
        // takes the key and no NPC talk follows.
        cancel_walk(&mut d);
    }
    let keyboard_npc = if scripted_talk {
        nearest_npc(&d, me, &units, own, own_faction, &player_facts, 60.0, true, &|unit| {
            pick.pose(unit, &units).map(|(pos, _)| pos)
        })
    } else if interact_key {
        nearest_npc(
            &d,
            me,
            &units,
            own,
            own_faction,
            &player_facts,
            60.0,
            false,
            &|unit| pick.pose(unit, &units).map(|(pos, _)| pos),
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

/// Starts the walk to an NPC that was right clicked beyond the interaction reach. A mount walk and a doodad
/// walk give way (they hold the same forward key).
pub(super) fn start_walk(data: &mut HudData, npc: (u32, u32), gap: f32, now: f32, target: Option<u32>) {
    super::pets::cancel_mount_walk(data);
    data.doodad.cancel_walk();
    data.dialog.approach = Some(Approach {
        npc,
        target,
        best: gap,
        best_at: now,
    });
}

/// Ends a walk to an NPC because another walk (a mount or a doodad) takes over. Returns whether one ran.
pub(super) fn cancel_walk(data: &mut HudData) -> bool {
    data.dialog.approach.take().is_some()
}

/// The player is on a slave seat, driving or sitting on a seat doodad: a forward key would drive the vehicle
/// (or the seat blocks it), so the talk walk must not run. Riding a mate still walks.
fn seated_or_driving(data: &HudData) -> bool {
    data.mounted.slave_seat().is_some() || data.mounted.local_cart.is_some() || data.doodad.seated
}

/// What one frame of the walk to a right-clicked NPC decides.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NpcWalk {
    /// No walk is running.
    Idle,
    /// Keep running at the NPC; the yaw to face it (radians, game terms).
    Toward(f32),
    /// The NPC came inside the interaction reach: the talk may start.
    Arrived((u32, u32)),
    /// The NPC is gone, the player took over or the walk made no progress.
    Cancelled,
}

/// One frame of the walk to a right-clicked NPC: the same nearest-distance progress and stuck bound as the
/// doodad walk, with `FUN_393bf270`'s 3D surface gap as the arrival test.
fn step_npc_walk(
    data: &mut HudData,
    units: &Units,
    me: Vec3,
    at: Vec3,
    now: f32,
    target: Option<u32>,
    interrupted: bool,
) -> NpcWalk {
    let Some(mut approach) = data.dialog.approach else { return NpcWalk::Idle };
    let alive = units.get(approach.npc.0).is_some_and(|u| {
        !u.dead && matches!(u.kind, UnitKind::Npc { template_id, .. } if template_id == approach.npc.1)
    });
    // A new `Some` target (Tab / left click) is the player taking over; a walk while seated or driving
    // must not hold W (it would drive the vehicle).
    let new_target = target.is_some() && target != approach.target;
    if interrupted || !alive || new_target || seated_or_driving(data) {
        data.dialog.approach = None;
        return NpcWalk::Cancelled;
    }
    let Some(unit) = units.get(approach.npc.0) else {
        data.dialog.approach = None;
        return NpcWalk::Cancelled;
    };
    let gap = npc_gap(data, me, at, unit);
    if gap < interact_reach(unit) {
        data.dialog.approach = None;
        return NpcWalk::Arrived(approach.npc);
    }
    let (best, best_at) = if gap < approach.best - 0.3 {
        (gap, now)
    } else {
        (approach.best, approach.best_at)
    };
    if now - best_at > NPC_WALK_STUCK_SECS {
        data.dialog.approach = None;
        return NpcWalk::Cancelled;
    }
    approach.best = best;
    approach.best_at = best_at;
    data.dialog.approach = Some(approach);
    let toward = (at - me).truncate();
    NpcWalk::Toward((-toward.x).atan2(toward.y))
}

/// The walk to a right-clicked NPC that is beyond the interaction reach (`FUN_390a6af0`, the click-to-move the
/// native interaction reach starts): steer at it through the normal movement path (the scripted forward key
/// `control_player` reads) and start the talk when the surface gap is inside the reach. Movement input, a lost
/// or dead NPC, a teleport and no progress for [`NPC_WALK_STUCK_SECS`] end it without a talk.
#[allow(clippy::too_many_arguments)]
pub fn walk_to_npc(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    target: Res<Target>,
    time: Res<Time>,
    poses: super::pick::UnitPoses,
    mut movers: Query<&mut crate::ingame::player::Mover, With<crate::ingame::player::LocalPlayer>>,
    mut cameras: Query<&mut crate::ingame::camera::OrbitCamera>,
    mut dev: ResMut<crate::ingame::DevOptions>,
    teleport: Option<Res<crate::ingame::teleport::Teleport>>,
    suspended: Res<super::HudSuspended>,
    mut walking: Local<bool>,
) {
    let typing = focus.0 || rt.as_ref().is_some_and(|rt| rt.keyboard_captured());
    let interrupted = !typing && NPC_WALK_CANCEL_ACTIONS.iter().any(|a| actions.pressed(a, 0));
    let mut step = NpcWalk::Idle;
    {
        let mut d = bridge.0.borrow_mut();
        if d.dialog.approach.is_some() && (suspended.0 || teleport.as_ref().is_some_and(|t| t.loading())) {
            d.dialog.approach = None;
        }
        if d.dialog.approach.is_some() {
            let world = |id: u32| units.get(id).and_then(|u| poses.get(u, &units));
            let me = super::slave::player_centre(&d, &world);
            let at = d
                .dialog
                .approach
                .and_then(|a| units.get(a.npc.0))
                .and_then(|u| poses.get(u, &units))
                .map_or(Vec3::ZERO, |(pos, _)| pos);
            step = step_npc_walk(&mut d, &units, me, at, time.elapsed_secs(), target.obj_id, interrupted);
        }
        let other_walk = d.doodad.walking() || super::pets::walking(&d);
        match step {
            NpcWalk::Toward(yaw) => {
                *walking = true;
                if let Ok(mut mover) = movers.single_mut() {
                    mover.facing = yaw;
                }
                for mut camera in &mut cameras {
                    camera.yaw = yaw;
                }
                if !dev.script_keys.contains('W') {
                    dev.script_keys.push('W');
                }
                return;
            }
            NpcWalk::Arrived(npc) => info!("hud: reached {} for the talk", npc.0),
            NpcWalk::Cancelled if interrupted => info!("hud: the walk to the NPC was taken over"),
            NpcWalk::Cancelled => info!("hud: the walk to the NPC gave up"),
            NpcWalk::Idle => {}
        }
        let was_walking = std::mem::take(&mut *walking);
        if (was_walking || matches!(step, NpcWalk::Arrived(_))) && !other_walk {
            dev.script_keys.retain(|c| c != 'W');
        }
    }
    if let NpcWalk::Arrived(npc) = step {
        dispatch_talk(&bridge.0, npc, target.obj_id);
    }
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

/// `FUN_3937b240` from `SCQuestContextStarted` / `SCQuestContextUpdated`: the clock's end is the
/// server's `time_left_ms` at local receipt, and a later Started / Updated releases the refusal
/// latch so the tick may keep it.
fn start_quest_clock(bridge: &Shared, q: &Quest, end_ms: i32, now: Instant) {
    let mut d = bridge.borrow_mut();
    d.dialog.cleared_left.remove(&(q.template_id, q.id));
    d.dialog.seeded_left.insert((q.template_id, q.id));
    d.dialog.left_times.insert(q.template_id, QuestClock { end_ms, started: now });
}

/// `FUN_3937a130`'s refusal path: end the window showing `quest_id`, clear its clock, latch the
/// instance so the tick does not restore it, and say whether a clock was cleared.
fn refuse_quest(rt: &UiRuntime, bridge: &Shared, quest_id: u32) -> bool {
    let mut d = bridge.borrow_mut();
    if d.dialog.directing && d.dialog.quest == Some(quest_id) {
        apply_directing(rt, &mut d.dialog, false);
    }
    let instance = d.dialog.quests.iter().find(|q| q.template_id == quest_id).map(|q| q.id);
    if let Some(id) = instance {
        d.dialog.cleared_left.insert((quest_id, id));
    }
    d.dialog.left_times.remove(&quest_id).is_some()
}

/// The quest packets that are notices rather than quest state: a refused request
/// (`SCQuestContextFailed`), a needed unit gone (`SCQuestUnitReqFailed`) and quest mail
/// (`SCQuestMailSent`, `SCQuestRewardedByMail`).
///
/// `FUN_391dbcd0` raises `QUEST_ERROR_INFO(err, qtype)` and `FUN_3937a130` first clears the quest's
/// timer (`FUN_39379ea0` publishes `QUEST_LEFT_TIME_UPDATED(qtype, 0)`); the directing script ends
/// its window when the event's quest is the one it shows (`quest_context_directing.alb` F37).
/// `FUN_391dbdd0` raises `QUEST_QUICK_CLOSE_EVENT(qtype)` only when the failed unit is the current
/// target (`FUN_39376590`); the directing script closes the same window (F36). The mail packets
/// show the `QUEST_MAIL_SENT` / `QUEST_REWARDED_BY_MAIL` texts through `DOODAD_PHASE_MSG`
/// (`FUN_3937a650`, `FUN_3937a520`), with the `quest_mails` name / the quest title as `$1`; a
/// failed reward mail (`success` 0) is `FUN_3937a130(qtype, 0xb)` instead.
pub fn route_quest_notices(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    target: Res<Target>,
    mut events: MessageReader<NetEvent>,
    mut skill_failed: MessageWriter<super::alerts::UseFailed>,
) {
    let Some(rt) = rt else { return };
    for NetEvent(e) in events.read() {
        match e {
            Event::QuestStarted(q) => start_quest_clock(&bridge.0, q, q.time_left_ms, Instant::now()),
            Event::QuestUpdated(q) => {
                // 723452: a status other than in progress calls FUN_3937b240(qtype, -1), which the
                // next tick removes and publishes as (qtype, 0).
                let end_ms = if q.status == status::PROGRESS { q.time_left_ms } else { -1 };
                start_quest_clock(&bridge.0, q, end_ms, Instant::now());
            }
            Event::QuestContextFailed(failed) => {
                let had_timer = refuse_quest(&rt, &bridge.0, failed.quest_id);
                info!("hud: quest {} failed ({})", failed.quest_id, failed.err);
                // FUN_3937a130 clears the quest's timer before raising the error.
                if had_timer {
                    rt.fire_event("QUEST_LEFT_TIME_UPDATED", (failed.quest_id, 0i32));
                }
                rt.fire_event("QUEST_ERROR_INFO", (u32::from(failed.err), failed.quest_id));
            }
            Event::QuestUnitReqFailed(failed) => {
                if target.obj_id != Some(failed.unit_obj_id) {
                    continue;
                }
                {
                    let mut d = bridge.0.borrow_mut();
                    if d.dialog.directing && d.dialog.quest == Some(failed.quest_id) {
                        apply_directing(&rt, &mut d.dialog, false);
                    }
                }
                info!("hud: quest {} needs unit {} (result {:?})", failed.quest_id, failed.unit_obj_id, failed.result);
                // FUN_39376590 reports the result through FUN_39390900 before it raises 0xf4.
                // The cooldown-detail silence (FUN_39390900:742294) is inside the skill-lookup
                // branch, and FUN_39376590 passes the no-skill sentinel DAT_3a173784 (718029), so
                // it does not apply here.
                let result = failed.result.code;
                let value = if failed.result.raw_flags & 4 != 0 { failed.result.value } else { u32::from(failed.result.extra) };
                skill_failed.write(super::alerts::UseFailed { skill: 0, result, value, server: true });
                rt.fire_event("QUEST_QUICK_CLOSE_EVENT", failed.quest_id);
            }
            Event::QuestMailSent(sent) => {
                // FUN_3937a650: the quest_mails row's name (FUN_398562a0 -> +4) is the $1 of
                // QUEST_MAIL_SENT; a missing row shows nothing.
                let name = bridge.0.borrow().game.get().and_then(|game| game.quests.quest_mail_names.get(&sent.id)).cloned();
                let Some(name) = name else { continue };
                let Some(text) = super::alerts::localized_args(&rt, "ERROR_MSG", "QUEST_MAIL_SENT", std::slice::from_ref(&name)) else { continue };
                info!("hud: quest mail {} sent: {text}", sent.id);
                rt.fire_event("DOODAD_PHASE_MSG", text);
            }
            Event::Interaction(InteractEvent::RewardedByMail { success, quest }) => {
                if !success {
                    // FUN_3937a520(0, qtype) -> FUN_3937a130(qtype, 0xb).
                    let had_timer = refuse_quest(&rt, &bridge.0, *quest);
                    info!("hud: quest {quest} reward mail failed");
                    if had_timer {
                        rt.fire_event("QUEST_LEFT_TIME_UPDATED", (*quest, 0i32));
                    }
                    rt.fire_event("QUEST_ERROR_INFO", (0xb_u32, *quest));
                    continue;
                }
                // FUN_3937a520: the quest's title (+0x20) is the $1 of QUEST_REWARDED_BY_MAIL; a
                // quest missing from the table shows nothing.
                let title = {
                    let d = bridge.0.borrow();
                    let Some(game) = d.game.get() else { continue };
                    let Some(def) = game.quests.def(*quest) else { continue };
                    texts(&d, game).expand(&def.name)
                };
                let Some(text) = super::alerts::localized_args(&rt, "ERROR_MSG", "QUEST_REWARDED_BY_MAIL", std::slice::from_ref(&title)) else { continue };
                info!("hud: quest {quest} rewarded by mail: {text}");
                rt.fire_event("DOODAD_PHASE_MSG", text);
            }
            _ => {}
        }
    }
}

/// The cinema of the quest's `kind` component (`quest_components.cinema_id`).
fn component_cinema(game: &GameData, quest: u32, kind: u8) -> Option<(u32, CinemaDef)> {
    let def = game.quests.def(quest)?;
    def.components.iter().filter_map(|id| game.quests.parts.get(id)).find_map(|c| (c.kind == kind).then_some(c).and_then(|c| c.cinema.clone().map(|cinema| (c.id, cinema))))
}

/// A component cinema may only drive the quest from the directing dialog; the quest log, item and
/// scripted paths send their request without a movie (capture case C3 for the unproven ones).
fn directing_cinema(d: &HudData, game: &GameData, quest: u32, kind: u8) -> Option<(u32, CinemaDef)> {
    if !d.dialog.directing {
        return None;
    }
    component_cinema(game, quest, kind)
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
    mut cinema: Option<ResMut<super::cinema::CinemaPlayer>>,
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
            Req::Accept(quest_id) => {
                let then = Interact::QuestStart { quest_id, npc_obj_id, doodad_obj_id, sphere_id: 0 };
                let started = match (cinema.as_deref_mut(), d.game.get().and_then(|g| directing_cinema(&d, g, quest_id, kind::START))) {
                    (Some(player), Some((component_id, cinema_def))) => super::cinema::start_quest_cinema(
                        player,
                        &mut net,
                        super::cinema::StartCinema { quest_id, component_id, cinema: &cinema_def, then: Some(then.clone()), hide_dialog: !cinema_def.before_bubble },
                    ),
                    _ => false,
                };
                if !started {
                    send(&mut net, then);
                }
            }
            Req::Complete(quest_id, selected) => {
                let then = Interact::QuestComplete { quest_id, npc_obj_id, doodad_obj_id, selected };
                let started = match (cinema.as_deref_mut(), d.game.get().and_then(|g| directing_cinema(&d, g, quest_id, kind::READY))) {
                    (Some(player), Some((component_id, cinema_def))) => super::cinema::start_quest_cinema(
                        player,
                        &mut net,
                        super::cinema::StartCinema { quest_id, component_id, cinema: &cinema_def, then: Some(then.clone()), hide_dialog: !cinema_def.before_bubble },
                    ),
                    _ => false,
                };
                if !started {
                    send(&mut net, then);
                }
            }
            Req::CinemaPermission { quest, kind } => {
                let found = d.game.get().and_then(|g| component_cinema(g, quest, kind));
                if d.dialog.directing
                    && let (Some((component_id, cinema_def)), Some(player)) = (found, cinema.as_deref_mut())
                {
                    send(&mut net, Interact::CinemaPermission { component_id, npc_obj_id, doodad_obj_id });
                    if cinema_def.before_bubble {
                        super::cinema::start_quest_cinema(
                            player,
                            &mut net,
                            super::cinema::StartCinema { quest_id: quest, component_id, cinema: &cinema_def, then: None, hide_dialog: false },
                        );
                    }
                }
            }
            Req::Talk(quest_id, component_id, act_id) => {
                send(&mut net, Interact::QuestTalk { npc_obj_id, quest_id, component_id, act_id });
            }
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
    d.dialog.quest = None;
    if let Some(scope) = active_scope(d).or_else(|| npc::cancel_scope(d)) { close_npc(d, scope); }
    d.dialog.start.clear(); d.dialog.complete.clear(); d.dialog.talk.clear();
}

/// The dialog has the screen to itself: every other top-level widget is hidden while it is up (the
/// original hides the UI layers below its own) and the ones that were shown come back after.
fn apply_directing(rt: &UiRuntime, dialog: &mut DialogView, on: bool) {
    if !on {
        dialog.quest = None;
    }
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

/// Hides every top-level widget for a movie (capture C5: no talk window or bars while it plays).
pub(super) fn cinema_hide(rt: Option<&UiRuntime>, dialog: &mut DialogView, on: bool) {
    dialog.cinema_hidden = on;
    if on {
        let Some(rt) = rt else { return };
        if dialog.cinema_applied {
            return;
        }
        let mut u = aa_ui::lua::ui(&rt.lua);
        let shown: Vec<aa_ui::tree::Id> = u.tree.top.iter().copied().filter(|&id| {
            let o = u.tree.get(id);
            o.alive && o.shown
        }).collect();
        for id in shown {
            u.tree.get_mut(id).shown = false;
            dialog.cinema_restore.push(id);
        }
        dialog.cinema_applied = true;
        dialog.cinema_restore_pending = false;
    } else if dialog.cinema_applied {
        let Some(rt) = rt else {
            dialog.cinema_restore_pending = true;
            return;
        };
        let mut u = aa_ui::lua::ui(&rt.lua);
        for id in std::mem::take(&mut dialog.cinema_restore) {
            let o = u.tree.get(id);
            if o.alive && !DIRECTING_WIDGETS.contains(&o.name.as_str()) {
                u.tree.get_mut(id).shown = true;
            }
        }
        drop(u);
        stop_directing(&rt.lua, dialog);
        dialog.cinema_applied = false;
        dialog.cinema_restore_pending = false;
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
        .add_systems(Update, walk_to_npc.after(talk_input).before(talk_pointer).run_if(super::hud_up).run_if(in_state(crate::Screen::InGame)))
        .add_systems(Update, talk_pointer.after(super::PointerFinished).after(walk_to_npc).run_if(super::hud_up).run_if(in_state(crate::Screen::InGame)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn talk_distance_measures_from_the_drawn_seat_not_the_stale_hud_point() {
        let state: Shared = Rc::new(RefCell::new(HudData::default()));
        let own = crate::hud::fixture_spawn();
        crate::hud::mounted::receive(
            &state,
            &Event::CharacterSelected(Box::new(own.state.clone())),
        );
        crate::hud::mounted::receive(&state, &Event::PlayerSpawn(Box::new(own.clone())));
        let mut slave = crate::hud::fixture_spawn().unit;
        slave.obj_id = 20;
        slave.kind = UnitKind::Slave {
            id: 700,
            tl_id: 700,
            template_id: 60,
            summoner_id: 0,
        };
        slave.pos = aa_net::Pos::new(100.0, 100.0, 0.0);
        crate::hud::mounted::receive(&state, &Event::UnitSpawned(Box::new(slave.clone())));
        crate::hud::mounted::receive(
            &state,
            &Event::UnitAttached {
                obj_id: own.obj_id,
                mount: slave.obj_id,
                point: 1,
                reason: 0,
            },
        );
        let mut npc = crate::hud::fixture_spawn().unit;
        npc.obj_id = 21;
        npc.kind = UnitKind::Npc {
            template_id: 7,
            owner_id: 0,
        };
        npc.pos = aa_net::Pos::new(102.0, 100.0, 0.0);
        let mut units = Units::default();
        units.by_id.insert(slave.obj_id, slave);
        units.by_id.insert(npc.obj_id, npc);
        // the HUD point stayed at the boarding point 100 m away
        state.borrow_mut().pos = Vec3::ZERO;
        let world = |id: u32| {
            units
                .get(id)
                .and_then(|u| crate::ingame::units::unit_world_pose_in(u, &units))
        };
        let me = super::super::slave::player_centre(&state.borrow(), &world);
        assert_eq!(me, Vec3::new(100.0, 100.0, 0.0));
        let facts = PlayerFacts::default();
        let by_unit = |u: &Unit| world(u.obj_id).map(|(pos, _)| pos);
        assert_eq!(
            nearest_npc(
                &state.borrow(),
                me,
                &units,
                own.obj_id,
                0,
                &facts,
                10.0,
                false,
                &by_unit
            ),
            Some((21, 7)),
            "the seat's position is within talk range"
        );
        assert_eq!(
            nearest_npc(
                &state.borrow(),
                state.borrow().pos,
                &units,
                own.obj_id,
                0,
                &facts,
                10.0,
                false,
                &by_unit
            ),
            None,
            "the stale HUD point would be 105 m away"
        );
        // `drive_npc` uses the same seat point (not the stale `data.pos`) for its late range rejection.
        let npc_unit = units.get(21).unwrap();
        assert!(npc_in_reach(
            &state.borrow(),
            me,
            Vec3::new(102.0, 100.0, 0.0),
            npc_unit
        ));
        assert!(!npc_in_reach(
            &state.borrow(),
            state.borrow().pos,
            Vec3::new(102.0, 100.0, 0.0),
            npc_unit
        ));
    }

    #[test]
    fn a_climb_doodad_keeps_the_click_only_in_front_of_a_unit() {
        assert!(
            crate::hud::pick::doodad_beats_unit(Some((3.0, 77)), Some((5.0, 20)), None),
            "ladder in front"
        );
        assert!(
            crate::hud::pick::doodad_beats_unit(Some((3.0, 77)), None, None),
            "no unit behind"
        );
        assert!(
            !crate::hud::pick::doodad_beats_unit(Some((5.0, 77)), Some((3.0, 20)), None),
            "a nearer unit or the hull keeps the right click"
        );
        assert!(
            !crate::hud::pick::doodad_beats_unit(None, Some((3.0, 20)), None),
            "no ladder"
        );
    }

    /// A ladder parented to the ship beats the ship's loose box (the box is the union of every
    /// drawn mesh, so its entry is outside the hull); a ray away from the ladder still picks the
    /// ship. Uses a real [`super::pick::PickBox`] for the hull.
    #[test]
    fn a_ladder_on_the_hull_beats_the_ships_loose_box() {
        let mut hull = crate::hud::expansion::fixture::spawn().unit;
        hull.obj_id = 20;
        hull.kind = aa_net::model::UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id: 21,
            summoner_id: 100,
        };
        hull.scale = 1.0;
        let boxed = crate::hud::pick::PickBox {
            centre: Vec3::new(0.0, 1.5, 0.0),
            // masts and sails protrude past the hull side, as the loose union box does
            half: Vec3::new(12.0, 1.5, 3.0),
            rotation: Quat::IDENTITY,
        };
        // from outside the port side, above the 1.1 m sphere: the box entry is at x 12, the
        // middle at x 0; the ladder sits on the side at x -10.1.
        let ray = Ray3d::new(
            Vec3::new(20.0, 3.0, 0.0),
            bevy::math::Dir3::NEG_X,
        );
        let mut units = Units::default();
        units.by_id.insert(20, hull.clone());
        let (entry, id) = crate::hud::pick::pick_unit_hit(
            &ray,
            &units,
            0,
            None,
            |_| true,
            |_, _| (None, Some(boxed)),
        )
        .expect("the hull box is under the ray");
        assert_eq!(id, 20);
        let unit_along = crate::hud::pick::unit_compare_hit(&ray, &hull, None, Some(boxed))
            .expect("the hull box is under the ray");
        let ladder_along = 20.0 - 10.1;
        assert!(
            entry < ladder_along,
            "the loose box entry {entry} is in front of the ladder"
        );
        // like for like: the entry is in front of the ladder, the box middle is behind it
        assert!(!crate::hud::pick::doodad_beats_unit(
            Some((ladder_along, 77)),
            Some((entry, 20)),
            None
        ));
        assert!(
            crate::hud::pick::doodad_beats_unit(
                Some((ladder_along, 77)),
                Some((unit_along, 20)),
                None
            ),
            "the ladder is in front of the box middle (ladder {ladder_along}, middle {unit_along})"
        );
        // a ladder behind the box middle still beats the hull it is attached to
        let mut doodads = Doodads::default();
        doodads.by_id.insert(
            77,
            aa_net::model::Doodad {
                obj_id: 77,
                template_id: 1187,
                owner_obj_id: 0,
                parent_obj_id: 20,
                attach_point: 24,
                pos: aa_net::Pos::new(0.0, 0.0, 0.0),
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: 1.0,
                has_loot_item: false,
                func_group_id: 15013,
                owner_id: 0,
                ucc_id: 0,
                item_template_id: 0,
                type2: 0,
                time_left: 0,
                plant_time: 0,
                quest_glow: 0,
                family: 0,
                puzzle_group: 0,
                owner_type: 0,
                owner_db_id: 0,
                data: 0,
            },
        );
        let parent = parent_of_climb(&doodads, Some((30.0, 77)));
        assert_eq!(parent, Some(20), "the ladder parent comes from Doodads");
        assert!(crate::hud::pick::doodad_beats_unit(
            Some((30.0, 77)),
            Some((unit_along, 20)),
            parent
        ));
        // the same click away from the ladder has no doodad hit: the ship stays the pick
        assert!(!crate::hud::pick::doodad_beats_unit(
            None,
            Some((unit_along, 20)),
            None
        ));
    }

    #[test]
    fn a_before_bubble_start_cinema_starts_when_the_dialog_opens() {
        if !aa_world::pak_available() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = GameData::load(&path) else { return };
        let before = game.quests.def(1352).and_then(|def| def.components.iter().filter_map(|id| game.quests.parts.get(id)).find(|c| c.kind == kind::START)).and_then(|c| c.cinema.clone());
        let Some(cinema) = before.filter(|c| c.before_bubble) else { return };
        let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
        {
            let mut data = shared.borrow_mut();
            data.game = super::super::SharedGameData::default();
            let _ = data.game.0.set(game);
            data.dialog.directing = true;
            data.dialog.npc = Some((1, 1));
            data.dialog.requests.push(Req::CinemaPermission { quest: 1352, kind: kind::START });
        }
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .init_resource::<Units>()
            .init_resource::<super::super::cinema::CinemaPlayer>()
            .insert_non_send(HudBridge(shared))
            .add_systems(Update, handle_dialog);
        app.update();
        let packets: Vec<Interact> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| if let Command::Interact(packet) = command { Some(packet) } else { None })
            .collect();
        assert_eq!(
            packets,
            [
                Interact::CinemaPermission { component_id: 6758, npc_obj_id: 1, doodad_obj_id: 0 },
                Interact::Cinema { completed: false }
            ],
            "the {} movie must start after 0x0d0 at the dialog open",
            cinema.name
        );
    }

    #[test]
    fn a_movie_hides_and_restores_the_talk_window() {
        let mut dialog = DialogView::default();
        cinema_hide(None, &mut dialog, true);
        assert!(dialog.cinema_hidden);
        cinema_hide(None, &mut dialog, false);
        assert!(!dialog.cinema_hidden);
    }

    #[test]
    fn quest_cinemas_need_the_directing_dialog() {
        if !aa_world::pak_available() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = GameData::load(&path) else { return };
        let mut d = HudData::default();
        assert!(directing_cinema(&d, &game, 1065, kind::READY).is_none());
        d.dialog.directing = true;
        assert!(directing_cinema(&d, &game, 1065, kind::READY).is_some());
        assert!(directing_cinema(&d, &game, 1065, kind::START).is_none());
    }

    fn npc(obj_id: u32, template_id: u32, x: f32) -> Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = obj_id;
        unit.kind = UnitKind::Npc {
            template_id,
            owner_id: 0,
        };
        unit.pos = aa_net::Pos::new(x, 0.0, 0.0);
        unit
    }

    #[test]
    fn the_npc_interaction_reach_is_the_3d_surface_gap() {
        let d = HudData {
            player_radius: 0.5,
            ..HudData::default()
        };
        let unit = npc(20, 42, 3.4);
        let mut units = Units::default();
        units.by_id.insert(20, unit.clone());

        // base gap = centre distance - the player radius (the test unit has no model radius).
        assert!(
            npc_in_reach(&d, Vec3::ZERO, Vec3::new(3.4, 0.0, 0.0), &unit),
            "2.9 m surface gap is inside the 3.0 m reach"
        );
        assert!(
            !npc_in_reach(&d, Vec3::ZERO, Vec3::new(3.5, 0.0, 0.0), &unit),
            "FUN_39093860 is strict: exactly 3.0 m is already out"
        );
        assert_eq!(
            plan_npc_click(&d, &units, 0, Vec3::ZERO, 20, Vec3::new(3.4, 0.0, 0.0)),
            Some(NpcClick::Talk((20, 42)))
        );
        assert_eq!(
            plan_npc_click(&d, &units, 0, Vec3::ZERO, 20, Vec3::new(10.0, 0.0, 0.0)),
            Some(NpcClick::Walk((20, 42), 9.5)),
            "beyond the reach the click walks and sends nothing"
        );
    }

    #[test]
    fn a_dead_target_uses_the_3_5_m_reach() {
        let mut unit = npc(20, 42, 3.0);
        assert_eq!(interact_reach(&unit), INTERACT_REACH);
        unit.dead = true;
        assert_eq!(interact_reach(&unit), DEAD_INTERACT_REACH);
    }

    fn npc_walk_app(npc_at: f32) -> (App, Shared) {
        let spawn = crate::hud::fixture_spawn();
        let data = HudData {
            character_id: spawn.character().id,
            snapshot: crate::hud::data::Snapshot {
                unit_id: spawn.obj_id.to_string(),
                ..crate::hud::data::Snapshot::default()
            },
            pos: Vec3::ZERO,
            ..HudData::default()
        };
        let shared: Shared = Rc::new(RefCell::new(data));
        let mut units = Units::default();
        units.by_id.insert(20, npc(20, 42, npc_at));
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::settings::Actions::default())
            .insert_resource(HudKeyboardFocus(false))
            .insert_resource(Target::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(super::super::HudSuspended::default())
            .insert_resource(crate::hud::HudBuilt {
                up: true,
                fresh: false,
            })
            .insert_resource(Units::default())
            .insert_resource(Player {
                spawn: Some(spawn.clone()),
            })
            .insert_resource(Doodads::default())
            .insert_resource(HudPointerOverUi::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(ScriptedTalk::default())
            .init_resource::<crate::script::ScriptPick>()
            .init_resource::<Quests>()
            .init_resource::<PlayerStats>()
            .init_resource::<Inventory>()
            .init_resource::<Skills>()
            .init_resource::<Buffs>()
            .insert_resource(units)
            .insert_non_send(HudBridge(shared.clone()))
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(
                Update,
                (npc::track_npc, talk_input, walk_to_npc, talk_pointer, drive_npc).chain(),
            );
        app.world_mut().spawn((
            crate::ingame::player::Mover {
                pos: Vec3::ZERO,
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            crate::ingame::player::LocalPlayer { obj_id: spawn.obj_id },
        ));
        app.world_mut()
            .spawn(crate::ingame::camera::OrbitCamera::default());
        (app, shared)
    }

    fn with_ui(app: &mut App) -> bool {
        if !aa_world::pak_available() {
            return false;
        }
        let Ok(rt) = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0) else { return false };
        app.insert_non_send(rt);
        true
    }

    fn w_held(app: &App) -> bool {
        app.world()
            .resource::<crate::ingame::DevOptions>()
            .script_keys
            .contains('W')
    }

    #[test]
    fn an_out_of_reach_npc_click_walks_and_sends_the_interaction_only_on_arrival() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
        app.world_mut()
            .write_message(NetEvent(Event::PlayerSpawn(Box::new(spawn))));
        app.world_mut().write_message(NetEvent(Event::EnteredWorld));
        let npc = app.world().resource::<Units>().get(20).unwrap().clone();
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(npc))));
        app.update();
        app.world_mut()
            .resource_mut::<crate::script::ScriptPick>()
            .right_for_test(20);
        app.update();
        assert!(shared.borrow().dialog.approach.is_some());
        assert!(w_held(&app), "the walk holds the forward key");
        assert!(shared.borrow().dialog.npc_flow.intent_count() == 0);
        assert!(
            app.world_mut()
                .resource_mut::<Messages<NetCommand>>()
                .drain()
                .next()
                .is_none(),
            "no CSStartInteraction before the walk arrives"
        );

        // The player walks (the movement path is another system; the position is what the reach reads).
        shared.borrow_mut().pos = Vec3::new(8.5, 0.0, 0.0);
        app.update();
        app.update();
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app), "the arrived walk releases the key");
        let commands: Vec<Command> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|c| c.0)
            .collect();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::Interact(Interact::Start { npc_obj_id: 20, .. }))),
            "the talk starts with CSStartInteraction once inside the reach: {commands:?}"
        );
    }


    fn npc_events(app: &mut App) {
        let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
        app.world_mut()
            .write_message(NetEvent(Event::PlayerSpawn(Box::new(spawn))));
        app.world_mut().write_message(NetEvent(Event::EnteredWorld));
        let npc = app.world().resource::<Units>().get(20).unwrap().clone();
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(npc))));
        app.update();
    }

    fn right_click(app: &mut App, obj_id: u32) {
        app.world_mut()
            .resource_mut::<crate::script::ScriptPick>()
            .right_for_test(obj_id);
        app.update();
    }

    fn commands(app: &mut App) -> Vec<Command> {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|c| c.0)
            .collect()
    }

    #[test]
    fn an_in_reach_right_click_sends_exactly_one_cs_start_interaction() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        // A far click starts the walk; the next in-reach right click cancels it and talks.
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_some());
        shared.borrow_mut().pos = Vec3::new(8.5, 0.0, 0.0);
        right_click(&mut app, 20);
        assert!(
            shared.borrow().dialog.approach.is_none(),
            "the in-reach click cancels the walk"
        );
        assert!(!w_held(&app));
        let started = commands(&mut app)
            .into_iter()
            .filter(|c| matches!(c, Command::Interact(Interact::Start { npc_obj_id: 20, .. })))
            .count();
        assert_eq!(started, 1, "exactly one CSStartInteraction");
    }

    #[test]
    fn an_in_reach_click_on_another_npc_cancels_the_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        let mut far = npc(21, 42, 30.0);
        far.name = "Far NPC".into();
        app.world_mut().resource_mut::<Units>().by_id.insert(21, far);
        right_click(&mut app, 21);
        assert_eq!(
            shared.borrow().dialog.approach.map(|a| a.npc.0),
            Some(21),
            "the far NPC starts the walk"
        );
        shared.borrow_mut().pos = Vec3::new(8.5, 0.0, 0.0);
        right_click(&mut app, 20);
        assert!(
            shared.borrow().dialog.approach.is_none(),
            "the in-reach talk to the other NPC ends the walk"
        );
        assert!(!w_held(&app));
    }

    #[test]
    fn a_new_target_cancels_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_some());
        app.world_mut().resource_mut::<Target>().obj_id = Some(99);
        app.update();
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
    }

    #[test]
    fn f_cancels_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_some());
        app.world_mut()
            .resource_mut::<crate::settings::Actions>()
            .tap_for_test("do_interaction_1");
        app.update();
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
    }

    #[test]
    fn a_click_on_another_unit_cancels_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_some());
        let mut other = crate::hud::fixture_spawn().unit;
        other.obj_id = 30;
        other.kind = UnitKind::Character { char_id: 30 };
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(30, other);
        right_click(&mut app, 30);
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
    }

    #[test]
    fn a_driving_player_does_not_hold_w_for_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        shared.borrow_mut().mounted.local_cart = Some((31, 0.0));
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
    }

    #[test]
    fn a_player_seated_on_a_doodad_does_not_hold_w_for_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        shared.borrow_mut().doodad.seated = true;
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
    }

    #[test]
    fn riding_a_mate_still_walks_to_a_far_npc() {
        // `player_attachment` also reports a mate; only a slave seat or a cart is "seated" and blocks
        // the walk (the forward key would drive it).
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        let spawn = app.world().resource::<Player>().spawn.clone().unwrap();
        crate::hud::mounted::receive(&shared, &Event::PlayerSpawn(Box::new(spawn.clone())));
        let mut mate = crate::hud::fixture_spawn().unit;
        mate.obj_id = 31;
        mate.name = "Snowlion".into();
        mate.kind = UnitKind::Mate {
            tl_id: 1,
            template_id: 1,
            owner_id: spawn.character().id,
        };
        crate::hud::mounted::receive(&shared, &Event::UnitSpawned(Box::new(mate)));
        crate::hud::mounted::receive(
            &shared,
            &Event::UnitAttached {
                obj_id: spawn.obj_id,
                mount: 31,
                point: 2,
                reason: 0,
            },
        );
        assert!(shared.borrow().mounted.riding_mate());
        npc_events(&mut app);
        right_click(&mut app, 20);
        assert!(
            shared.borrow().dialog.approach.is_some(),
            "a mounted player still walks to a far NPC"
        );
        assert!(w_held(&app));
    }

    #[test]
    fn f_under_a_doodad_claim_cancels_the_npc_walk() {
        let (mut app, shared) = npc_walk_app(10.0);
        if !with_ui(&mut app) { return; }
        npc_events(&mut app);
        right_click(&mut app, 20);
        assert!(shared.borrow().dialog.approach.is_some());
        shared.borrow_mut().doodad.claim_key_for_test();
        app.world_mut()
            .resource_mut::<crate::settings::Actions>()
            .tap_for_test("do_interaction_1");
        app.update();
        assert!(
            shared.borrow().dialog.approach.is_none(),
            "F used by the doodad also ends the NPC walk"
        );
        assert!(!w_held(&app));
    }

    #[test]
    fn a_scripted_talk_searches_past_the_interact_reach() {
        let (app, shared) = npc_walk_app(10.0);
        let d = shared.borrow();
        let units = app.world().resource::<Units>();
        let facts = PlayerFacts::default();
        let find = |wanted| {
            nearest_npc(
                &d,
                Vec3::ZERO,
                units,
                100,
                0,
                &facts,
                60.0,
                wanted,
                &|u: &Unit| Some(Vec3::new(u.pos.x, u.pos.y, u.pos.z)),
            )
        };
        assert_eq!(find(true), Some((20, 42)), "the scripted talk finds an NPC at 10 m");
        assert_eq!(find(false), None, "the interact key stays inside the interaction reach");
    }

    fn quest_notice_app() -> Option<(App, Shared)> {
        if !aa_world::pak_available() { return None; }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let game = GameData::load(&path).ok()?;
        let rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0).ok()?;
        rt.lua
            .load(
                r#"
                OBSERVED={}
                X2Locale.LocalizeUiText=function(self,group,key,arg) return key..':'..tostring(arg) end
                local w=UIParent:CreateWidget('emptywidget','questNotice','UIParent')
                w:SetHandler('OnEvent',function(self,event,...) table.insert(OBSERVED,{event=event,args={...}}) end)
                w:RegisterEvent('QUEST_ERROR_INFO');w:RegisterEvent('QUEST_QUICK_CLOSE_EVENT');w:RegisterEvent('QUEST_LEFT_TIME_UPDATED');w:RegisterEvent('DOODAD_PHASE_MSG')
            "#,
            )
            .exec()
            .ok()?;
        let data = HudData {
            dialog: DialogView { directing: true, quest: Some(1065), npc: Some((1, 1)), ..Default::default() },
            ..HudData::default()
        };
        let _ = data.game.0.set(game);
        let shared: Shared = Rc::new(RefCell::new(data));
        let hud = rt.lua.create_table().unwrap();
        super::install(&rt.lua, &hud, &shared).unwrap();
        rt.lua.globals().set("HUD", hud).unwrap();
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .add_message::<super::super::alerts::UseFailed>()
            .insert_resource(Target::default())
            .insert_non_send(HudBridge(shared.clone()))
            .add_systems(Update, route_quest_notices);
        app.insert_non_send(rt);
        Some((app, shared))
    }

    #[test]
    fn a_refused_quest_ends_directing_only_for_that_quest_and_raises_error_info() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        shared.borrow_mut().dialog.left_times.insert(1065, QuestClock { end_ms: 5_000, started: Instant::now() });
        // The failed quest is not the one the window is directing: it stays up.
        app.world_mut()
            .write_message(NetEvent(Event::QuestContextFailed(aa_net::quest::QuestContextFailed { quest_id: 1066, err: 6 })));
        app.update();
        assert!(shared.borrow().dialog.directing);
        // The directed quest itself: the window ends and QUEST_ERROR_INFO carries (err, qtype).
        app.world_mut()
            .write_message(NetEvent(Event::QuestContextFailed(aa_net::quest::QuestContextFailed { quest_id: 1065, err: 6 })));
        app.update();
        assert!(!shared.borrow().dialog.directing);
        assert_eq!(shared.borrow().dialog.quest, None, "ending directing clears the window quest");
        assert!(shared.borrow().dialog.left_times.is_empty(), "the failure clears the quest clock");
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load(
                r#"
                assert(#OBSERVED==3 and OBSERVED[1].event=='QUEST_ERROR_INFO' and OBSERVED[1].args[1]==6 and OBSERVED[1].args[2]==1066)
                assert(OBSERVED[2].event=='QUEST_LEFT_TIME_UPDATED')
                assert(OBSERVED[2].args[1]==1065 and OBSERVED[2].args[2]==0)
                assert(OBSERVED[3].event=='QUEST_ERROR_INFO')
                assert(OBSERVED[3].args[1]==6 and OBSERVED[3].args[2]==1065)
            "#,
            )
            .exec()
            .unwrap();
    }

    #[test]
    fn a_late_refusal_for_an_old_quest_does_not_end_a_newer_window() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        // playLines tells the app which quest the window shows (HUD.quest_show); the previous
        // request's quest must not be used for the refusal comparison.
        app.world().non_send::<UiRuntime>().lua.load("HUD.quest_show(1066)").exec().unwrap();
        assert_eq!(shared.borrow().dialog.quest, Some(1066));
        app.world_mut()
            .write_message(NetEvent(Event::QuestContextFailed(aa_net::quest::QuestContextFailed { quest_id: 1065, err: 6 })));
        app.update();
        assert!(shared.borrow().dialog.directing, "a late refusal for 1065 must not end the window showing 1066");
        app.world_mut()
            .write_message(NetEvent(Event::QuestContextFailed(aa_net::quest::QuestContextFailed { quest_id: 1066, err: 6 })));
        app.update();
        assert!(!shared.borrow().dialog.directing, "the window's own quest still closes it");
    }

    #[test]
    fn a_failed_quest_unit_quick_closes_the_directing_window_only_for_the_current_target() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        app.world_mut().resource_mut::<Target>().obj_id = Some(42);
        let failed = |unit_obj_id, result| {
            NetEvent(Event::QuestUnitReqFailed(aa_net::quest::QuestUnitReqFailed {
                quest_id: 1065,
                unit_obj_id,
                result,
            }))
        };
        let refused = aa_net::skill::SkillResult { raw_flags: 5, code: 9, extra: 0, value: 7 };
        app.world_mut().write_message(failed(43, refused));
        app.update();
        assert!(shared.borrow().dialog.directing, "another unit failing does not close the window");
        assert!(
            app.world_mut().resource_mut::<Messages<super::super::alerts::UseFailed>>().drain().next().is_none(),
            "another unit failing reports nothing"
        );
        app.world_mut().write_message(failed(42, refused));
        app.update();
        assert!(!shared.borrow().dialog.directing);
        let reported: Vec<_> = app.world_mut().resource_mut::<Messages<super::super::alerts::UseFailed>>().drain().collect();
        assert_eq!(reported.len(), 1, "FUN_39376590 reports the result before 0xf4");
        assert_eq!((reported[0].skill, reported[0].result, reported[0].value, reported[0].server), (0, 9, 7, true));
        // FUN_39390900's cooldown-detail silence is inside its skill-lookup branch, and
        // FUN_39376590 passes the no-skill sentinel DAT_3a173784, so result 8 / detail 2 reports.
        app.world_mut().write_message(failed(42, aa_net::skill::SkillResult { raw_flags: 5, code: 8, extra: 0, value: 2 }));
        app.update();
        let reported: Vec<_> = app.world_mut().resource_mut::<Messages<super::super::alerts::UseFailed>>().drain().collect();
        assert_eq!(reported.len(), 1, "the no-skill sentinel skips the cooldown silence");
        assert_eq!((reported[0].result, reported[0].value), (8, 2));
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(#OBSERVED==2 and OBSERVED[1].event=='QUEST_QUICK_CLOSE_EVENT' and OBSERVED[2].args[1]==1065)")
            .exec()
            .unwrap();
    }

    #[test]
    fn quest_mail_sent_shows_the_quest_mail_name_as_the_argument() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let name = shared.borrow().game.get().and_then(|game| game.quests.quest_mail_names.get(&1)).cloned();
        let Some(name) = name else { return };
        app.world_mut().write_message(NetEvent(Event::QuestMailSent(aa_net::quest::QuestMailSent { id: 1 })));
        app.update();
        let expected = format!("QUEST_MAIL_SENT:{name}");
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua.globals().set("EXPECTED", expected).unwrap();
        rt.lua
            .load("assert(#OBSERVED==1 and OBSERVED[1].event=='DOODAD_PHASE_MSG' and OBSERVED[1].args[1]==EXPECTED)")
            .exec()
            .unwrap();
        // FUN_3937a650 shows nothing when the quest_mails lookup fails.
        app.world_mut().write_message(NetEvent(Event::QuestMailSent(aa_net::quest::QuestMailSent { id: 999_999 })));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(#OBSERVED==1)").exec().unwrap();
    }

    #[test]
    fn reward_mail_shows_the_quest_title_and_a_failed_send_raises_error_11() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let title = shared.borrow().game.get().and_then(|game| game.quests.def(330)).map(|def| def.name.clone());
        let Some(title) = title else { return };
        app.world_mut().write_message(NetEvent(Event::Interaction(InteractEvent::RewardedByMail { success: true, quest: 330 })));
        app.update();
        let expected = format!("QUEST_REWARDED_BY_MAIL:{title}");
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua.globals().set("EXPECTED", expected).unwrap();
        rt.lua
            .load("assert(#OBSERVED==1 and OBSERVED[1].event=='DOODAD_PHASE_MSG' and OBSERVED[1].args[1]==EXPECTED)")
            .exec()
            .unwrap();
        // FUN_3937a520(0, qtype) -> FUN_3937a130(qtype, 0xb), raised as (err, qtype).
        app.world_mut().write_message(NetEvent(Event::Interaction(InteractEvent::RewardedByMail { success: false, quest: 330 })));
        app.update();
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(#OBSERVED==2 and OBSERVED[2].event=='QUEST_ERROR_INFO' and OBSERVED[2].args[1]==11 and OBSERVED[2].args[2]==330)")
            .exec()
            .unwrap();
        // A quest missing from the table shows nothing, even for a successful send.
        app.world_mut().write_message(NetEvent(Event::Interaction(InteractEvent::RewardedByMail { success: true, quest: 999_999 })));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(#OBSERVED==2)").exec().unwrap();
    }

    fn timed(left_ms: i32) -> Quest {
        Quest { template_id: 330, status: status::PROGRESS, time_left_ms: left_ms, accept_time: 0, ..Quest::default() }
    }

    #[test]
    fn the_quest_clock_counts_down_from_its_local_receipt_time() {
        let start = Instant::now();
        let clock = QuestClock { end_ms: 60_000, started: start };
        assert_eq!(clock_left_ms(&clock, start), 60_000);
        assert_eq!(clock_left_ms(&clock, start + Duration::from_secs(30)), 30_000);
        assert_eq!(clock_left_ms(&clock, start + Duration::from_millis(59_500)), 500);
        assert_eq!(clock_left_ms(&clock, start + Duration::from_secs(61)), -1_000);
        // a clock seen before its start never gains time
        let future = QuestClock { end_ms: 5_000, started: start + Duration::from_secs(1) };
        assert_eq!(clock_left_ms(&future, start), 5_000);
    }

    #[test]
    fn the_quest_clock_publishes_seconds_and_clears_the_clock_at_one() {
        let start = Instant::now();
        let q = timed(60_000);
        let mut left = BTreeMap::from([
            (331, QuestClock { end_ms: 5_000, started: start }),
            (330, QuestClock { end_ms: 60_000, started: start }),
        ]);
        // the stale timed quest is cleared, then the live one is published
        assert_eq!(tick_left_times(&mut left, std::slice::from_ref(&q), start), vec![(331, 0), (330, 60_000)]);
        assert!(!left.contains_key(&331));
        // the last published second, then FUN_39379ea0's timeout path publishes 0 once
        assert_eq!(tick_left_times(&mut left, std::slice::from_ref(&q), start + Duration::from_secs(59)), vec![(330, 1_000)]);
        assert_eq!(tick_left_times(&mut left, std::slice::from_ref(&q), start + Duration::from_secs(60)), vec![(330, 0)]);
        assert_eq!(tick_left_times(&mut left, std::slice::from_ref(&q), start + Duration::from_secs(61)), Vec::new());
        assert!(left.is_empty());
        // leaving the list before its last second also hides the clock
        let mut left = BTreeMap::from([(330, QuestClock { end_ms: 5_000, started: start })]);
        assert_eq!(tick_left_times(&mut left, &[], start), vec![(330, 0)]);
        assert!(left.is_empty());
    }

    #[test]
    fn the_quest_clock_system_uses_the_local_receipt_time_and_the_script_getter() {
        if !aa_world::pak_available() { return; }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = GameData::load(&path) else { return };
        let Ok(rt) = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0) else { return };
        rt.lua
            .load(
                r#"
                LEFT={}
                local w=UIParent:CreateWidget('emptywidget','questClock','UIParent')
                w:SetHandler('OnEvent',function(self,event,...) table.insert(LEFT,{...}) end)
                w:RegisterEvent('QUEST_LEFT_TIME_UPDATED')
            "#,
            )
            .exec()
            .unwrap();
        let data = HudData { game: super::super::SharedGameData::default(), ..HudData::default() };
        let _ = data.game.0.set(game);
        let shared: Shared = Rc::new(RefCell::new(data));
        let hud = rt.lua.create_table().unwrap();
        super::install(&rt.lua, &hud, &shared).unwrap();
        rt.lua.globals().set("HUD", hud.clone()).unwrap();
        rt.lua.load("X2Quest = X2Quest or {}; X2Decal = X2Decal or {}; X2Interaction = X2Interaction or {}; X2Util = X2Util or {}; X2 = X2 or {}").exec().unwrap();
        super::load_script(&rt.lua).unwrap();
        // an accept stamp half a minute old and a full 60 s left: only the receipt time counts
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut quest = timed(60_000);
        quest.accept_time = now - 30;
        let mut quests = Quests::default();
        quests.active.push(quest);
        let mut app = App::new();
        app.insert_resource(quests)
            .insert_non_send(HudBridge(shared.clone()))
            .add_systems(Update, tick_quest_left_time);
        app.insert_non_send(rt);
        app.update();
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(#LEFT==1 and LEFT[1][1]==330 and LEFT[1][2]>59000 and LEFT[1][2]<=60000)")
            .exec()
            .unwrap();
        let left: i32 = rt.lua.load("return X2Quest:GetQuestContextLeftTime(330)").eval().unwrap();
        assert!(left > 59_000 && left <= 60_000, "the getter reads the live clock, not accept_time: {left}");
        let none: i32 = rt.lua.load("return X2Quest:GetQuestContextLeftTime(331)").eval().unwrap();
        assert_eq!(none, -1, "no entry reads -1 (FUN_39379e30)");
        // playLines tells the app which quest the window shows (HUD.quest_show).
        rt.lua.load("FAKE={fire_later=function() end}").exec().unwrap();
        rt.lua
            .load("X2Quest:EnterQuestDirectingMode(); X2Quest:TryStartQuestContext(nil, 330, 0)")
            .exec()
            .unwrap();
        assert_eq!(shared.borrow().dialog.quest, Some(330), "the window's own quest reaches the app");
        rt.lua.load("X2Quest:LeaveQuestDirectingMode()").exec().unwrap();
        assert_eq!(shared.borrow().dialog.quest, None, "LeaveQuestDirectingMode clears it");
    }

    #[test]
    fn started_and_updated_start_the_clock_at_local_receipt() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let before = Instant::now();
        let q = Quest { id: 77, accept_time: 1_000, ..timed(60_000) };
        app.world_mut().write_message(NetEvent(Event::QuestStarted(q.clone())));
        app.update();
        let clock = shared.borrow().dialog.left_times.get(&330).copied().expect("Started starts the clock");
        assert_eq!(clock.end_ms, 60_000);
        assert!(clock.started.saturating_duration_since(before) < Duration::from_secs(2));
        // FUN_3937b240 replaces both values on every Started / Updated
        let later = before + Duration::from_secs(10);
        start_quest_clock(&shared, &q, 30_000, later);
        let clock = shared.borrow().dialog.left_times.get(&330).copied().unwrap();
        assert_eq!(clock.end_ms, 30_000);
        assert_eq!(clock.started, later);
    }

    #[test]
    fn an_updated_status_change_hides_the_clock_on_the_next_tick() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let q = Quest { id: 77, ..timed(60_000) };
        app.world_mut().write_message(NetEvent(Event::QuestStarted(q.clone())));
        app.update();
        assert!(shared.borrow().dialog.left_times.contains_key(&330));
        let ready = Quest { status: status::READY, ..q };
        app.world_mut().write_message(NetEvent(Event::QuestUpdated(ready.clone())));
        app.update();
        assert_eq!(shared.borrow().dialog.left_times.get(&330).unwrap().end_ms, -1, "FUN_3937b240(qtype, -1)");
        let events = tick_quest_clocks(&mut shared.borrow_mut().dialog, &[ready], Instant::now());
        assert_eq!(events, vec![(330, 0)], "the next tick removes it and publishes 0");
        assert!(!shared.borrow().dialog.left_times.contains_key(&330));
    }

    #[test]
    fn an_expired_clock_is_not_restarted_by_the_tick() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let q = Quest { id: 77, ..timed(60_000) };
        let start = Instant::now();
        assert_eq!(
            tick_quest_clocks(&mut shared.borrow_mut().dialog, std::slice::from_ref(&q), start),
            vec![(330, 60_000)],
            "the login list seeds the clock once"
        );
        assert_eq!(
            tick_quest_clocks(&mut shared.borrow_mut().dialog, std::slice::from_ref(&q), start + Duration::from_secs(61)),
            vec![(330, 0)]
        );
        assert!(shared.borrow().dialog.left_times.is_empty());
        // FUN_3937b240 is the only creator: the tick must not restart the expired clock.
        assert!(tick_quest_clocks(&mut shared.borrow_mut().dialog, std::slice::from_ref(&q), start + Duration::from_secs(62)).is_empty());
        assert!(shared.borrow().dialog.left_times.is_empty(), "an expired clock stays gone");
        // the server's next Updated starts it again.
        app.world_mut().write_message(NetEvent(Event::QuestUpdated(q.clone())));
        app.update();
        assert!(shared.borrow().dialog.left_times.contains_key(&330), "a later Updated restarts the clock");
        let events = tick_quest_clocks(&mut shared.borrow_mut().dialog, std::slice::from_ref(&q), Instant::now());
        assert!(events.len() == 1 && events[0].0 == 330 && events[0].1 > 0, "{events:?}");
    }

    #[test]
    fn a_later_started_or_updated_releases_the_refusal_latch() {
        let Some((mut app, shared)) = quest_notice_app() else { return };
        let q = Quest { id: 77, ..timed(60_000) };
        shared.borrow_mut().dialog.quests.push(q.clone());
        app.world_mut()
            .write_message(NetEvent(Event::QuestContextFailed(aa_net::quest::QuestContextFailed { quest_id: 330, err: 5 })));
        app.update();
        assert!(shared.borrow().dialog.left_times.is_empty());
        // the tick does not restore the refused instance
        let events = tick_quest_clocks(&mut shared.borrow_mut().dialog, std::slice::from_ref(&q), Instant::now());
        assert!(events.is_empty());
        assert!(shared.borrow().dialog.left_times.is_empty(), "the refusal latch holds");
        // a later Updated of the same instance releases it and restarts the clock
        let updated = Quest { time_left_ms: 5_000, ..q };
        app.world_mut().write_message(NetEvent(Event::QuestUpdated(updated.clone())));
        app.update();
        assert_eq!(shared.borrow().dialog.left_times.get(&330).unwrap().end_ms, 5_000);
        let events = tick_quest_clocks(&mut shared.borrow_mut().dialog, &[updated], Instant::now());
        assert!(events.len() == 1 && events[0].0 == 330 && events[0].1 > 0, "the released clock is published: {events:?}");
    }

    #[test]
    fn a_lost_npc_cancels_the_walk_without_a_talk() {
        let (mut app, shared) = npc_walk_app(10.0);
        start_walk(&mut shared.borrow_mut(), (20, 42), 9.5, 0.0, None);
        app.update();
        app.world_mut().resource_mut::<Units>().by_id.clear();
        shared.borrow_mut().pos = Vec3::new(8.5, 0.0, 0.0);
        app.update();
        assert!(shared.borrow().dialog.approach.is_none());
        assert!(!w_held(&app));
        assert!(shared.borrow().dialog.npc_flow.intent_count() == 0);
        assert!(
            app.world_mut()
                .resource_mut::<Messages<NetCommand>>()
                .drain()
                .next()
                .is_none()
        );
    }
}
