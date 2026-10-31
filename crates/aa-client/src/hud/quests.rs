//! The client's quest tables: which NPC gives and takes which quest (the marker over its head), and
//! everything the quest windows show (titles, objective texts and counters, rewards, what the NPCs say).
//!
//! The server does not say which NPC starts a quest: the client works it out from its quest tables,
//! the way the original does:
//!
//! * an NPC starts the quests whose *Start* component (`quest_components.component_kind_id` 2) has a
//!   `QuestActConAcceptNpc` act for it. The quest is on offer (a yellow "!") when the player has not
//!   done or taken it (a repeatable quest may be done before) and the component's `unit_reqs` hold:
//!   level, race, which other quests are done, items owned ... (all of them, or any when
//!   `or_unit_reqs`);
//! * an NPC is where a quest is handed in when the quest's *Ready* component (kind 6) has a
//!   `QuestActConReportNpc` act for it: a yellow "?" when the player's quest is ready, a grey "?"
//!   while it is still being worked on;
//! * an NPC has a talk objective for the player when a *Progress* component (kind 4) of an active
//!   quest has a `QuestActObjTalk` act for it that is not done yet.
//!
//! A quest's steps are its components in the server's order (kind, then id); the objectives of a step
//! are its acts that count (`QuestActObj*` and a few `QuestActCon*`), numbered across all components of
//! that kind: `Quest::objectives[i]` is how far objective `i` is. The texts are the English
//! `localized_texts` of `quest_contexts` (title), `quest_component_texts` (kind 4: what a step asks;
//! a text in `|nn;...|r` is a hint) and `quest_chat_bubbles` (what is said when a step starts), with
//! the client's macros (`@NPC_NAME(id)`, `@ITEM_NAME(id)`, `@PC_NAME(0)` ...) expanded by [`Texts`].
//! The rewards are the Reward component's supply acts plus the quest level's `quest_supplies` exp and
//! copper (what the server gives on top, `distribute_rewards`).
//!
//! The requirement kinds are the server's (`unit_reqs.rs`, `UnitReqsKindType`); one the client cannot
//! check from what it knows holds, as it does there.

use std::collections::{HashMap, HashSet};

use aa_net::quest::{Quest, status};
use aa_net::state::{Buffs, Inventory, PlayerStats, Quests, Skills};
use rusqlite::Connection;

use super::data::{GameData, any_flag, any_int};

/// Component kinds (`QuestComponentKind`).
pub mod kind {
    pub const START: u8 = 2;
    pub const PROGRESS: u8 = 4;
    pub const READY: u8 = 6;
    pub const REWARD: u8 = 8;
}

/// What marker an NPC carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A quest to take ("!").
    Available,
    /// A quest ready to be handed in ("?").
    Ready,
    /// A quest being worked on that is handed in here (grey "?").
    InProgress,
}

/// A Start component: the quest it starts and what the player needs.
#[derive(Clone, Debug, Default)]
struct StartComponent {
    quest: u32,
    any_req: bool,
    hidden: bool,
    /// `(kind, value1, value2)` of `unit_reqs`.
    reqs: Vec<(u32, u32, u32)>,
}

/// A quest (`quest_contexts`).
#[derive(Clone, Debug, Default)]
pub struct QuestDef {
    pub name: String,
    pub category: u32,
    pub level: u8,
    /// `QuestDetail`: 1 normal, 2 main, 5 hidden, 6 task, 7 daily, 8 livelihood, 9 group ...
    pub detail: u32,
    pub chapter: u32,
    pub zone_id: u32,
    pub selective: bool,
    pub let_it_done: bool,
    /// `use_quest_camera`: the talk may use the close-up camera.
    pub use_quest_camera: bool,
    /// Its components, in the server's order (kind, then id).
    pub components: Vec<u32>,
}

impl QuestDef {
    pub fn is_main(&self) -> bool {
        self.detail == 2
    }
    pub fn is_hidden(&self) -> bool {
        self.detail == 5
    }
    pub fn is_task(&self) -> bool {
        self.detail == 6
    }
    pub fn is_daily(&self) -> bool {
        matches!(self.detail, 7 | 10 | 11 | 12 | 13)
    }
    pub fn is_livelihood(&self) -> bool {
        matches!(self.detail, 8 | 11)
    }
    pub fn is_group(&self) -> bool {
        matches!(self.detail, 9 | 12)
    }
}

/// One act of a component (`quest_acts` and its detail row).
#[derive(Clone, Debug, Default)]
pub struct ActDef {
    pub id: u32,
    /// `act_detail_type` without the `QuestAct` prefix: `ConAcceptNpc`, `ObjMonsterHunt`, `SupplyExp` ...
    pub ty: String,
    /// The NPC, item, doodad, quest, monster group ... the act is about (the detail's first id column).
    pub target: u32,
    /// How many (`count`), 1 when the detail has none.
    pub count: i32,
    /// The amount of a supply act (`exp`, `amount`, `lp`, `point`) or a selective item's grade.
    pub value: i64,
    pub grade: u8,
    /// `quest_act_obj_alias_id` when `use_alias`: the objective's own name.
    pub alias: u32,
    /// Its objective number within the step, when it counts as one.
    pub objective: Option<u8>,
}

/// The acts the server counts as objectives (`ActKind::counts_as_objective`).
fn counts_as_objective(ty: &str) -> bool {
    matches!(
        ty,
        "ConAcceptBuff"
            | "ConAcceptItemEquip"
            | "ConFail"
            | "ObjAbilityLevel"
            | "ObjAggro"
            | "ObjCinema"
            | "ObjCompleteQuest"
            | "ObjCraft"
            | "ObjDistance"
            | "ObjExpressFire"
            | "ObjInteraction"
            | "ObjItemGather"
            | "ObjItemGroupGather"
            | "ObjItemGroupUse"
            | "ObjItemUse"
            | "ObjLevel"
            | "ObjMateLevel"
            | "ObjMonsterGroupHunt"
            | "ObjMonsterHunt"
            | "ObjSphere"
            | "ObjTalkNpcGroup"
            | "ObjTalk"
            | "ObjZoneKill"
            | "ObjZoneMonsterHunt"
    )
}

/// The cinema a component plays (`quest_components.cinema_id`, `cinemas.name`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CinemaDef {
    pub name: String,
    /// `play_cinema_before_bubble`: the movie comes before the step's lines instead of after them.
    pub before_bubble: bool,
}

/// A component (`quest_components`).
#[derive(Clone, Debug, Default)]
pub struct ComponentDef {
    pub id: u32,
    pub quest: u32,
    pub kind: u8,
    /// Its kind 4 texts (raw, macros not expanded), in id order.
    pub texts: Vec<String>,
    pub acts: Vec<ActDef>,
    /// The bubble the step's talk starts with (`is_start`), 0 for none.
    pub first_bubble: u32,
    /// The cinema played for this step, if any.
    pub cinema: Option<CinemaDef>,
}

impl ComponentDef {
    /// The text of what the step asks (not a `|n...|r` hint).
    pub fn main_text(&self) -> Option<&str> {
        self.texts.iter().find(|t| !t.trim_start().starts_with("|n")).or(self.texts.first()).map(String::as_str)
    }
}

/// A line of what is said when a step starts (`quest_chat_bubbles`).
#[derive(Clone, Debug, Default)]
pub struct BubbleDef {
    pub id: u32,
    /// The NPC who says it; 0 is the player (or the system, for kind 3).
    pub npc: u32,
    pub next: u32,
    /// `chat_bubble_kind_id`: 1 normal, 2 thought, 3 system.
    pub kind: u8,
    pub text: String,
    /// `change_speaker_name`.
    pub speaker: String,
    /// `camera_id`: the `quest_cameras` row that frames the line, 0 for its speaker's model's.
    pub camera: u32,
}

/// What a quest gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rewards {
    pub exp: i64,
    pub copper: i64,
    /// `(item, count, grade)`.
    pub items: Vec<(u32, i32, u8)>,
    pub selective: Vec<(u32, i32, u8)>,
    pub labor: i64,
    pub honor: i64,
    pub crime: i64,
    pub living: i64,
    pub appellation: u32,
}

/// One line of a quest's objectives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Objective {
    /// The text with the counter ("Kill the Solzreed Foxes on Desireen Peak. (1/3)").
    pub text: String,
    pub done: i32,
    pub count: i32,
}

impl Objective {
    pub fn finished(&self) -> bool {
        self.done >= self.count
    }
}

/// The tables the markers come from.
#[derive(Default)]
pub struct QuestTables {
    components: HashMap<u32, StartComponent>,
    /// NPC template -> Start components it gives out.
    starters: HashMap<u32, Vec<u32>>,
    /// NPC template -> quests handed in to it.
    reporters: HashMap<u32, Vec<u32>>,
    repeatable: HashSet<u32>,
    pub defs: HashMap<u32, QuestDef>,
    pub parts: HashMap<u32, ComponentDef>,
    pub bubbles: HashMap<u32, BubbleDef>,
    /// Quest level -> (exp, copper) given on top of the acts.
    pub supplies: HashMap<u8, (i64, i64)>,
    pub categories: HashMap<u32, String>,
    /// `quest_cameras` by id, and `model_quest_cameras`: model id -> row id (the talk's close-up).
    pub cameras: HashMap<u32, crate::ingame::quest_framing::Row>,
    pub model_cameras: HashMap<u32, u32>,
    pub aliases: HashMap<u32, String>,
    pub monster_groups: HashMap<u32, String>,
    pub doodads: HashMap<u32, String>,
    /// `zones.id` (a quest's zone) -> `zone_groups.id`.
    pub zone_group: HashMap<u32, u32>,
}

/// What the requirements read of the player.
#[derive(Default)]
pub struct PlayerFacts<'a> {
    pub level: u8,
    pub race: u8,
    pub gender: u8,
    pub faction: u32,
    pub honor: i32,
    pub vocation: i32,
    /// Item templates in the bag and worn.
    pub items: HashSet<u32>,
    pub skills: HashSet<u32>,
    pub buffs: HashSet<u32>,
    pub equipped: HashSet<u32>,
    pub quests: Option<&'a Quests>,
}

impl<'a> PlayerFacts<'a> {
    /// What the game state says of the player.
    pub fn of(spawn: &aa_net::PlayerSpawn, stats: &PlayerStats, inventory: &Inventory, skills: &Skills, buffs: &Buffs, quests: &'a Quests) -> PlayerFacts<'a> {
        let character = spawn.character();
        let worn = || inventory.equipment.slots.iter().flatten().map(|it| it.template_id);
        PlayerFacts {
            level: stats.level.max(character.level),
            race: character.race,
            gender: character.gender,
            faction: character.faction_id,
            honor: stats.honor,
            vocation: stats.vocation,
            items: inventory.bag_items().map(|(_, it)| it.template_id).chain(worn()).collect(),
            skills: skills.known.iter().map(|k| k.id).collect(),
            buffs: buffs.of(spawn.obj_id).iter().map(|b| b.buff.buff_id).collect(),
            equipped: worn().collect(),
            quests: Some(quests),
        }
    }

    fn completed(&self, quest: u32) -> bool {
        self.quests.is_some_and(|q| q.is_completed(quest))
    }

    fn status_of(&self, quest: u32) -> Option<u8> {
        self.quests.and_then(|q| q.active_quest(quest)).map(|q| q.status)
    }

    /// One `unit_reqs` row; the kinds are `UnitReqsKindType`.
    fn holds(&self, kind: u32, v1: u32, v2: u32) -> bool {
        let level = u32::from(self.level);
        match kind {
            0x01 => level >= v1 && (v2 == 0 || level <= v2),
            0x03 => u32::from(self.race) == v1,
            0x04 => u32::from(self.gender) == v1,
            0x09 => self.equipped.contains(&v1),
            0x0A => self.items.contains(&v1),
            0x0B => self.skills.contains(&v1),
            0x0F => self.buffs.contains(&v1),
            0x16 => !self.buffs.contains(&v1),
            0x1F => self.completed(v1),
            0x20 => self.status_of(v1) == Some(status::PROGRESS),
            0x21 => self.status_of(v1) == Some(status::READY),
            0x24 => !self.completed(v1),
            0x25 => !self.completed(v1) && self.status_of(v1).is_some(),
            0x28 | 0x37 => self.faction == v1,
            0x3A => self.faction != v1,
            0x2D => (v1 as i64..=v2 as i64).contains(&i64::from(self.honor)),
            0x32 => (v1 as i64..=v2 as i64).contains(&i64::from(self.vocation)),
            0x45 => level <= v1,
            0x48 => !self.completed(v1) && self.status_of(v1) != Some(status::PROGRESS),
            0x49 => !self.completed(v1) && self.status_of(v1) != Some(status::READY),
            _ => true,
        }
    }
}

/// `QuestActConAcceptNpc` -> `quest_act_con_accept_npcs`: the detail table of an act type.
fn detail_table(ty: &str) -> String {
    let mut out = String::from("quest_act");
    for c in ty.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out.push('s');
    out
}

/// The columns of a detail row the client uses.
#[derive(Clone, Copy, Default)]
struct Detail {
    target: u32,
    count: i32,
    value: i64,
    grade: u8,
    alias: u32,
}

impl QuestTables {
    pub fn load(conn: &Connection) -> rusqlite::Result<QuestTables> {
        let mut t = QuestTables::default();
        // English texts of the quest tables
        let mut text: HashMap<(String, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, tbl_column_name, idx, en_us FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND (\
                 (tbl_name IN ('quest_contexts','quest_categories','quest_act_obj_aliases','quest_monster_groups','doodad_almighties') AND tbl_column_name = 'name') OR \
                 (tbl_name = 'quest_component_texts' AND tbl_column_name = 'text') OR \
                 (tbl_name = 'quest_chat_bubbles' AND tbl_column_name IN ('speech','change_speaker_name')))",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (table, column): (String, String) = (r.get(0)?, r.get(1)?);
                let key = if column == "change_speaker_name" { "speaker".to_string() } else { table };
                text.insert((key, any_int(r, 2)? as u32), r.get(3)?);
            }
        }
        let en = |table: &str, id: u32| text.get(&(table.to_string(), id)).cloned();
        for (table, map) in [("quest_categories", &mut t.categories), ("quest_act_obj_aliases", &mut t.aliases), ("quest_monster_groups", &mut t.monster_groups), ("doodad_almighties", &mut t.doodads)] {
            for ((tb, id), s) in &text {
                if tb == table {
                    map.insert(*id, s.clone());
                }
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, name, category_id, LEVEL, REPEATABLE, selective, let_it_done, detail_id, chapter_idx, zone_id, use_quest_camera FROM quest_contexts",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                if any_flag(r, 4)? {
                    t.repeatable.insert(id);
                }
                let fallback: Option<String> = r.get(1)?;
                t.defs.insert(
                    id,
                    QuestDef {
                        name: en("quest_contexts", id).or(fallback).unwrap_or_default(),
                        category: any_int(r, 2)? as u32,
                        level: any_int(r, 3)?.clamp(0, 255) as u8,
                        selective: any_flag(r, 5)?,
                        let_it_done: any_flag(r, 6)?,
                        detail: any_int(r, 7)? as u32,
                        chapter: any_int(r, 8)? as u32,
                        zone_id: any_int(r, 9)? as u32,
                        use_quest_camera: any_flag(r, 10)?,
                        components: Vec::new(),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, quest_context_id, component_kind_id, or_unit_reqs, hide_quest_marker FROM quest_components ORDER BY quest_context_id, component_kind_id, id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (id, quest, kind) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32, any_int(r, 2)? as u8);
                if kind == kind::START {
                    t.components.insert(id, StartComponent { quest, any_req: any_flag(r, 3)?, hidden: any_flag(r, 4)?, reqs: Vec::new() });
                }
                t.parts.insert(id, ComponentDef { id, quest, kind, ..ComponentDef::default() });
                if let Some(d) = t.defs.get_mut(&quest) {
                    d.components.push(id);
                }
            }
        }
        if let Ok(mut q) = conn.prepare(
            "SELECT qc.id, c.name, qc.play_cinema_before_bubble FROM quest_components qc JOIN cinemas c ON c.id = qc.cinema_id",
        ) {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                if let Some(c) = t.parts.get_mut(&id) {
                    c.cinema = Some(CinemaDef { name: r.get(1)?, before_bubble: any_flag(r, 2)? });
                }
            }
        }
        // the detail rows of every act type
        let mut acts: Vec<(u32, u32, u32, String)> = Vec::new();
        {
            let mut q = conn.prepare("SELECT id, quest_component_id, act_detail_id, act_detail_type FROM quest_acts ORDER BY quest_component_id, id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let ty: String = r.get(3)?;
                acts.push((any_int(r, 0)? as u32, any_int(r, 1)? as u32, any_int(r, 2)? as u32, ty.strip_prefix("QuestAct").unwrap_or(&ty).to_string()));
            }
        }
        let mut details: HashMap<(String, u32), Detail> = HashMap::new();
        let types: HashSet<String> = acts.iter().map(|a| a.3.clone()).collect();
        for ty in types {
            let table = detail_table(&ty);
            let Ok(mut q) = conn.prepare(&format!("SELECT * FROM {table}")) else { continue };
            let names: Vec<String> = q.column_names().iter().map(|c| c.to_ascii_lowercase()).collect();
            let col = |wanted: &[&str]| wanted.iter().find_map(|w| names.iter().position(|n| n == w));
            let (id_col, target, count, value, grade, alias, use_alias) = (
                col(&["id"]),
                col(&["npc_id", "item_id", "doodad_id", "quest_monster_group_id", "npc_group_id", "quest_id", "quest_context_id", "sphere_id", "skill_id", "buff_id", "craft_id", "appellation_id", "item_group_id", "zone_id"]),
                col(&["count"]),
                col(&["exp", "amount", "lp", "point", "level"]),
                col(&["grade_id"]),
                col(&["quest_act_obj_alias_id"]),
                col(&["use_alias"]),
            );
            let Some(id_col) = id_col else { continue };
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let get = |c: Option<usize>| c.map(|c| any_int(r, c)).transpose();
                let aliased = use_alias.map(|c| any_flag(r, c)).transpose()?.unwrap_or(false);
                let d = Detail {
                    target: get(target)?.unwrap_or(0) as u32,
                    count: get(count)?.map_or(1, |c| c as i32),
                    value: get(value)?.unwrap_or(0),
                    grade: get(grade)?.unwrap_or(0).clamp(0, 255) as u8,
                    alias: if aliased { get(alias)?.unwrap_or(0) as u32 } else { 0 },
                };
                details.insert((ty.clone(), any_int(r, id_col)? as u32), d);
            }
        }
        for (id, component, detail_id, ty) in acts {
            let d = details.get(&(ty.clone(), detail_id)).copied().unwrap_or_default();
            if ty == "ConAcceptNpc" && t.components.contains_key(&component) {
                t.starters.entry(d.target).or_default().push(component);
            }
            if ty == "ConReportNpc"
                && let Some(c) = t.parts.get(&component)
                && c.kind == kind::READY
            {
                let list = t.reporters.entry(d.target).or_default();
                if !list.contains(&c.quest) {
                    list.push(c.quest);
                }
            }
            if let Some(c) = t.parts.get_mut(&component) {
                c.acts.push(ActDef { id, ty, target: d.target, count: d.count.max(1), value: d.value, grade: d.grade, alias: d.alias, objective: None });
            }
        }
        // objective numbers: per quest and component kind, across its components in order
        for def in t.defs.values() {
            let (mut index, mut last) = (0u8, 0u8);
            for cid in &def.components {
                let Some(c) = t.parts.get_mut(cid) else { continue };
                if c.kind != last {
                    index = 0;
                    last = c.kind;
                }
                for a in &mut c.acts {
                    if counts_as_objective(&a.ty) {
                        a.objective = Some(index);
                        index = index.wrapping_add(1);
                    }
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT id, quest_component_id FROM quest_component_texts WHERE quest_component_text_kind_id = 4 ORDER BY id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (id, component) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32);
                if let (Some(s), Some(c)) = (en("quest_component_texts", id), t.parts.get_mut(&component))
                    && !s.trim().is_empty()
                {
                    c.texts.push(s);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT id, quest_component_id, npc_id, next_bubble, is_start, chat_bubble_kind_id, speech, camera_id FROM quest_chat_bubbles ORDER BY id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (id, component) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32);
                let fallback: Option<String> = r.get(6)?;
                t.bubbles.insert(
                    id,
                    BubbleDef {
                        id,
                        npc: any_int(r, 2)? as u32,
                        next: any_int(r, 3)? as u32,
                        kind: any_int(r, 5)?.clamp(0, 255) as u8,
                        text: en("quest_chat_bubbles", id).or(fallback).unwrap_or_default(),
                        speaker: en("speaker", id).unwrap_or_default(),
                        camera: any_int(r, 7)?.max(0) as u32,
                    },
                );
                if any_flag(r, 4)?
                    && let Some(c) = t.parts.get_mut(&component)
                    && c.first_bubble == 0
                {
                    c.first_bubble = id;
                }
            }
        }
        // the talk's close-up: the rows that frame a line and the row each model speaks with
        if let Ok(mut q) = conn.prepare(
            "SELECT id, fov, camera_offset_x, camera_offset_y, camera_offset_z, npc_offset_x, npc_offset_y, npc_offset_z, interpolate, invisible, dof FROM quest_cameras",
        ) {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let f = |i: usize| r.get::<_, Option<f64>>(i).map(|v| v.unwrap_or(0.0) as f32);
                let id = any_int(r, 0)? as u32;
                let row = crate::ingame::quest_framing::Row {
                    id,
                    fov: f(1)?,
                    camera: [f(2)?, f(3)?, f(4)?],
                    aim: [f(5)?, f(6)?, f(7)?],
                    interpolate: any_flag(r, 8)?,
                    invisible: any_flag(r, 9)?,
                    dof: any_flag(r, 10)?,
                };
                t.cameras.insert(id, row);
            }
        }
        if let Ok(mut q) = conn.prepare("SELECT model_id, quest_camera_id FROM model_quest_cameras ORDER BY id") {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.model_cameras.entry(any_int(r, 0)? as u32).or_insert(any_int(r, 1)? as u32);
            }
        }
        {
            let mut q = conn.prepare("SELECT LEVEL, exp, copper FROM quest_supplies")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.supplies.insert(any_int(r, 0)?.clamp(0, 255) as u8, (any_int(r, 1)?, any_int(r, 2)?));
            }
        }
        {
            let mut q = conn.prepare("SELECT id, group_id FROM zones")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.zone_group.insert(any_int(r, 0)? as u32, any_int(r, 1)? as u32);
            }
        }
        {
            let mut q = conn.prepare("SELECT owner_id, kind_id, value1, value2 FROM unit_reqs WHERE owner_type = 'QuestComponent'")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(c) = t.components.get_mut(&(any_int(r, 0)? as u32)) {
                    c.reqs.push((any_int(r, 1)? as u32, any_int(r, 2)? as u32, any_int(r, 3)? as u32));
                }
            }
        }
        Ok(t)
    }

    pub fn def(&self, quest: u32) -> Option<&QuestDef> {
        self.defs.get(&quest)
    }

    /// Whether the talk of `quest` gets the close-up camera for the option's value (`quest::try_context`).
    pub fn close_up(&self, quest: u32, option: u32) -> bool {
        self.defs.get(&quest).is_some_and(|q| crate::ingame::quest_framing::close_up_on(q.use_quest_camera, q.detail, option))
    }

    /// The `quest_cameras` row that frames a line: the bubble's own, else the speaker's model's
    /// ([`crate::ingame::quest_framing::pick_row`]).
    pub fn camera_row(&self, camera_id: u32, model: u32, previous: Option<u32>) -> Option<crate::ingame::quest_framing::Row> {
        let id = crate::ingame::quest_framing::pick_row(camera_id, self.model_cameras.get(&model).copied(), previous)?;
        self.cameras.get(&id).copied()
    }

    /// The components of `quest` of `kind`, in order.
    pub fn parts_of(&self, quest: u32, kind: u8) -> impl Iterator<Item = &ComponentDef> {
        self.defs.get(&quest).into_iter().flat_map(|d| d.components.iter()).filter_map(|c| self.parts.get(c)).filter(move |c| c.kind == kind)
    }

    /// The step an active quest is at: its Progress components while it is worked on, its Ready
    /// component when it waits to be handed in.
    pub fn step_kind(&self, q: &Quest) -> u8 {
        if let Some(c) = self.parts.get(&q.component_id)
            && matches!(c.kind, kind::PROGRESS | kind::READY)
        {
            return c.kind;
        }
        if q.status == status::READY || self.parts_of(q.template_id, kind::PROGRESS).next().is_none() { kind::READY } else { kind::PROGRESS }
    }

    /// The quests the Start components of NPC `npc` offer this player now, lowest id first.
    pub fn offers(&self, npc: u32, player: &PlayerFacts) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .starters
            .get(&npc)
            .into_iter()
            .flatten()
            .filter_map(|c| self.components.get(c))
            .filter(|c| {
                if c.hidden || player.status_of(c.quest).is_some() || (player.completed(c.quest) && !self.repeatable.contains(&c.quest)) {
                    return false;
                }
                if c.reqs.is_empty() {
                    return true;
                }
                let holds = |r: &(u32, u32, u32)| player.holds(r.0, r.1, r.2);
                if c.any_req { c.reqs.iter().any(holds) } else { c.reqs.iter().all(holds) }
            })
            .map(|c| c.quest)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The player's quests that are ready and handed in at NPC `npc`.
    pub fn completes(&self, npc: u32, player: &PlayerFacts) -> Vec<u32> {
        self.reporters.get(&npc).into_iter().flatten().copied().filter(|q| player.status_of(*q) == Some(status::READY)).collect()
    }

    /// The talk objectives NPC `npc` ticks for the player now: `(quest, component, act)`.
    pub fn talks(&self, npc: u32, player: &PlayerFacts) -> Vec<(u32, u32, u32)> {
        let Some(quests) = player.quests else { return Vec::new() };
        let mut out = Vec::new();
        for q in &quests.active {
            if q.status != status::PROGRESS || self.step_kind(q) != kind::PROGRESS {
                continue;
            }
            for c in self.parts_of(q.template_id, kind::PROGRESS) {
                for a in &c.acts {
                    let done = a.objective.and_then(|i| q.objectives.get(usize::from(i))).copied().unwrap_or(0);
                    if a.ty == "ObjTalk" && a.target == npc && done < a.count {
                        out.push((q.template_id, c.id, a.id));
                    }
                }
            }
        }
        out
    }

    /// The NPC a quest is handed in at.
    pub fn report_npc(&self, quest: u32) -> Option<u32> {
        self.parts_of(quest, kind::READY).flat_map(|c| &c.acts).find(|a| a.ty == "ConReportNpc").map(|a| a.target)
    }

    /// What the quest gives: its Reward components' supply acts plus the level's `quest_supplies`.
    pub fn rewards(&self, quest: u32) -> Rewards {
        let mut r = Rewards::default();
        let Some(def) = self.defs.get(&quest) else { return r };
        if let Some(&(exp, copper)) = self.supplies.get(&def.level) {
            r.exp += exp;
            r.copper += copper;
        }
        for c in self.parts_of(quest, kind::REWARD) {
            for a in &c.acts {
                match a.ty.as_str() {
                    "SupplyExp" => r.exp += a.value,
                    "SupplyCopper" => r.copper += a.value,
                    "SupplyItem" => r.items.push((a.target, a.count, a.grade)),
                    "SupplySelectiveItem" => r.selective.push((a.target, a.count, a.grade)),
                    "SupplyLp" => r.labor += a.value,
                    "SupplyHonorPoint" => r.honor += a.value,
                    "SupplyCrimePoint" => r.crime += a.value,
                    "SupplyLivingPoint" => r.living += a.value,
                    "SupplyAppellation" => r.appellation = a.target,
                    _ => {}
                }
            }
        }
        r
    }

    /// The bubbles of a component's talk, from its start bubble along `next_bubble`.
    pub fn bubble_chain(&self, component: u32) -> Vec<&BubbleDef> {
        let mut out = Vec::new();
        let mut id = self.parts.get(&component).map_or(0, |c| c.first_bubble);
        while id != 0 && out.len() < 32 {
            let Some(b) = self.bubbles.get(&id) else { break };
            if out.iter().any(|o: &&BubbleDef| o.id == id) {
                break;
            }
            out.push(b);
            id = b.next;
        }
        out
    }

    /// The objectives of an active quest at its step, texts expanded by `texts`.
    pub fn objectives(&self, q: &Quest, texts: &Texts) -> Vec<Objective> {
        let step = self.step_kind(q);
        let mut out = Vec::new();
        if step == kind::READY {
            for c in self.parts_of(q.template_id, kind::READY) {
                if let Some(t) = c.main_text() {
                    out.push(Objective { text: texts.expand(t), done: 0, count: 1 });
                }
            }
            return out;
        }
        for c in self.parts_of(q.template_id, step) {
            for a in &c.acts {
                let Some(i) = a.objective else { continue };
                let done = q.objectives.get(usize::from(i)).copied().unwrap_or(0).clamp(0, a.count);
                let base = self.aliases.get(&a.alias).filter(|_| a.alias != 0).map(|s| texts.expand(s)).or_else(|| c.main_text().map(|t| texts.expand(t))).unwrap_or_else(|| texts.describe(a));
                let text = if a.count > 1 { format!("{base} ({done}/{})", a.count) } else { base };
                out.push(Objective { text, done, count: a.count });
            }
        }
        out
    }

    /// The marker of NPC template `npc` for this player: a quest to hand in first, then one to take,
    /// then one in progress; `None` when the NPC has nothing to do with the player's quests.
    pub fn mark(&self, npc: u32, player: &PlayerFacts) -> Option<Mark> {
        let reported = self.reporters.get(&npc).map_or(&[][..], Vec::as_slice);
        let statuses = || reported.iter().filter_map(|q| player.status_of(*q));
        if statuses().any(|s| s == status::READY) || !self.talks(npc, player).is_empty() {
            return Some(Mark::Ready);
        }
        if !self.offers(npc, player).is_empty() {
            return Some(Mark::Available);
        }
        statuses().any(|s| s == status::PROGRESS).then_some(Mark::InProgress)
    }
}

/// Text colour of the client's `|nn;` (a name or a key in a quest text) and `|nb;` (something to use)
/// highlights (the colours are a guess: the codes are the client's renderer's own).
const HIGHLIGHT_NAME: &str = "|cFFFF9C27";
const HIGHLIGHT_OBJECT: &str = "|cFF97D5F9";

/// Expands the client's text macros: `@NPC_NAME(id)`, `@ITEM_NAME(id)`, `@DOODAD_NAME(id)`,
/// `@NPC_GROUP_NAME(id)`, `@QUEST_NAME(id)`, `@ZONE_NAME(id)`, `@PC_NAME(0)`, `@PC_RACE(0)`, and the
/// `|nn;` / `|nb;` highlights.
pub struct Texts<'a> {
    pub game: &'a GameData,
    pub player: &'a str,
    pub race: u8,
}

impl Texts<'_> {
    pub fn expand(&self, text: &str) -> String {
        let text = text.replace("\r\n", "\n").replace("|nn;", HIGHLIGHT_NAME).replace("|nb;", HIGHLIGHT_OBJECT);
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find('@') {
            out.push_str(&rest[..at]);
            let tail = &rest[at + 1..];
            let name_len = tail.find(|c: char| !(c.is_ascii_uppercase() || c == '_')).unwrap_or(tail.len());
            let (name, after) = tail.split_at(name_len);
            if name.is_empty() || !after.starts_with('(') {
                out.push('@');
                rest = tail;
                continue;
            }
            let Some(close) = after.find(')') else {
                out.push('@');
                rest = tail;
                continue;
            };
            let id: u32 = after[1..close].trim().parse().unwrap_or(0);
            match self.macro_value(name, id) {
                Some(v) => out.push_str(&v),
                // not one of the quest macros: left for whoever knows it
                None => out.push_str(&rest[at..at + 1 + name.len() + close + 1]),
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        out
    }

    fn macro_value(&self, name: &str, id: u32) -> Option<String> {
        let g = self.game;
        Some(match name {
            "NPC_NAME" => g.npcs.get(&id).map(|n| n.name.clone()).unwrap_or_default(),
            "ITEM_NAME" => g.items.get(&id).map(|i| i.named.name.clone()).unwrap_or_default(),
            "DOODAD_NAME" => g.quests.doodads.get(&id).cloned().unwrap_or_default(),
            "NPC_GROUP_NAME" => g.quests.monster_groups.get(&id).cloned().unwrap_or_default(),
            "QUEST_NAME" => g.quests.defs.get(&id).map(|d| d.name.clone()).unwrap_or_default(),
            "ZONE_NAME" => self.zone_name(id),
            "PC_NAME" => self.player.to_string(),
            "PC_RACE" => race_name(self.race).to_string(),
            _ => return None,
        })
    }

    /// The English name of zone `zones.id` (its zone group's).
    pub fn zone_name(&self, zone: u32) -> String {
        let g = self.game;
        g.quests.zone_group.get(&zone).and_then(|group| g.zone_groups.get(group)).map(|z| z.text.clone()).unwrap_or_default()
    }

    /// A text for an objective act without one in the database.
    pub fn describe(&self, a: &ActDef) -> String {
        let g = self.game;
        let npc = || g.npcs.get(&a.target).map(|n| n.name.clone()).unwrap_or_default();
        let item = || g.items.get(&a.target).map(|i| i.named.name.clone()).unwrap_or_default();
        match a.ty.as_str() {
            "ObjMonsterHunt" => format!("Kill {}", npc()),
            "ObjMonsterGroupHunt" => format!("Kill {}", g.quests.monster_groups.get(&a.target).cloned().unwrap_or_default()),
            "ObjTalk" => format!("Talk to {}", npc()),
            "ObjItemGather" => format!("Collect {}", item()),
            "ObjItemUse" => format!("Use {}", item()),
            "ObjInteraction" => g.quests.doodads.get(&a.target).cloned().unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The name a bubble's line is said by.
    pub fn speaker(&self, b: &BubbleDef) -> String {
        if !b.speaker.is_empty() {
            return self.expand(&b.speaker);
        }
        match (b.npc, b.kind) {
            (_, 3) => String::new(),
            (0, _) => self.player.to_string(),
            (npc, _) => self.game.npcs.get(&npc).map(|n| n.name.clone()).unwrap_or_default(),
        }
    }

    /// A bubble's text without the gesture it starts with ("/haha Ha! ..." plays the laugh).
    pub fn speech(&self, b: &BubbleDef) -> String {
        let text = b.text.trim_start();
        let text = match text.strip_prefix('/') {
            Some(rest) => rest.split_once(' ').map_or("", |(_, said)| said),
            None => text,
        };
        self.expand(text)
    }
}

fn race_name(race: u8) -> &'static str {
    match race {
        1 => "Nuian",
        2 => "Fairy",
        3 => "Dwarf",
        4 => "Elf",
        5 => "Harani",
        6 => "Firran",
        7 => "Returned",
        8 => "Warborn",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::quest::{CompletedBlock, Quest};

    fn tables() -> Option<QuestTables> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        QuestTables::load(&conn).ok()
    }

    #[test]
    fn detail_tables_are_named_after_the_act_type() {
        assert_eq!(detail_table("ConAcceptNpc"), "quest_act_con_accept_npcs");
        assert_eq!(detail_table("ObjTalkNpcGroup"), "quest_act_obj_talk_npc_groups");
        assert_eq!(detail_table("SupplyLp"), "quest_act_supply_lps");
    }

    #[test]
    fn maude_offers_the_first_quest_to_a_new_character() {
        let Some(t) = tables() else { return };
        // Maude (npcs.id 3597) starts quest 330 for a level 1 character; Maison (10688) starts nothing
        let quests = Quests::default();
        let new = PlayerFacts { level: 1, race: 1, quests: Some(&quests), ..PlayerFacts::default() };
        assert_eq!(t.mark(3597, &new), Some(Mark::Available));
        assert_eq!(t.offers(3597, &new), vec![330]);
        assert_eq!(t.mark(10688, &new), None);
        // taken: no longer on offer, and Maude is not where it is handed in (or: it is, in progress)
        let taken = Quests { active: vec![Quest { template_id: 330, status: status::READY, ..Quest::default() }], ..Quests::default() };
        let facts = PlayerFacts { level: 1, race: 1, quests: Some(&taken), ..PlayerFacts::default() };
        assert_ne!(t.mark(3597, &facts), Some(Mark::Available));
        // ready: handed in at Woodcutter Gallagher (3511)
        assert_eq!(t.completes(3511, &facts), vec![330]);
        assert_eq!(t.mark(3511, &facts), Some(Mark::Ready));
        assert_eq!(t.report_npc(330), Some(3511));
        // done: gone
        let done = Quests { completed: vec![CompletedBlock { id: 330 / 64, bits: 1 << (330 % 64) }], ..Quests::default() };
        let facts = PlayerFacts { level: 1, race: 1, quests: Some(&done), ..PlayerFacts::default() };
        assert_eq!(t.mark(3597, &facts), None);
        // Gallagher then offers "The Mayor's Advice"
        assert_eq!(t.offers(3511, &facts), vec![6198]);
    }

    #[test]
    fn quest_component_cinemas_come_from_the_component_row() {
        let Some(t) = tables() else { return };
        // 1065 "Evacuation Protocol": the Ready component 5336 reports through cinema 17 (fly_east)
        if let Some(c) = t.parts.get(&5336) {
            assert_eq!(c.quest, 1065);
            assert_eq!(c.cinema, Some(CinemaDef { name: "fly_east".to_string(), before_bubble: false }));
        }
        // 2246: the Start component 10186 plays zq_20_karl
        if let Some(c) = t.parts.get(&10186) {
            assert_eq!(c.quest, 2246);
            assert_eq!(c.cinema, Some(CinemaDef { name: "zq_20_karl".to_string(), before_bubble: false }));
        }
    }

    #[test]
    fn quest_texts_bubbles_and_rewards() {
        let Some(t) = tables() else { return };
        let q = t.def(330).unwrap();
        assert_eq!(q.name, "A Friendly Reminder");
        // the start component's talk: Maude twice, then the system hint
        let start = t.parts_of(330, kind::START).next().unwrap();
        let chain: Vec<u32> = t.bubble_chain(start.id).iter().map(|b| b.id).collect();
        assert_eq!(chain, vec![3203, 3205, 24017]);
        assert_eq!(t.bubbles[&24017].kind, 3);
        // level 1: 420 exp and 33 copper from quest_supplies, no acts
        let r = t.rewards(330);
        assert_eq!((r.exp, r.copper, r.items.len()), (420, 33, 0));
        // Fox Infestation: the hunt counts 3, objective 0 of the progress step, 110 exp on top
        let fox = t.parts_of(250, kind::PROGRESS).next().unwrap();
        let hunt = fox.acts.iter().find(|a| a.ty == "ObjMonsterHunt").unwrap();
        assert_eq!((hunt.target, hunt.count, hunt.objective), (3492, 3, Some(0)));
        assert!(fox.main_text().unwrap().starts_with("Kill"));
        assert_eq!(t.rewards(250).exp, 420 + 110);
    }
}
