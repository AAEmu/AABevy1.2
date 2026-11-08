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
    /// The items a step hands out; the native `GetUseTypeQuestItems` reads its `SupplyItem` acts
    /// (component type 3) and their `show_action_bar` flag.
    pub const SUPPLY: u8 = 3;
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
    /// `quest_contexts.quest_idx`: the quest's position within its chapter, the second replay
    /// list sort key (`FUN_39377f30` compares `+0x24` after `+0x08`).
    pub quest_idx: u32,
    pub zone_id: u32,
    pub selective: bool,
    pub let_it_done: bool,
    /// `quest_contexts.score`: the objective score a score quest needs (0 for other quests).
    pub score: u32,
    /// `quest_contexts.repeatable`: `FUN_394a55a0`'s completed check ignores a repeatable quest.
    pub repeatable: bool,
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
    /// `quest_act_supply_items.show_action_bar`: the native shows this supplied item on the
    /// tracker (`FUN_394a55a0`).
    pub show_action_bar: bool,
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
    /// `cinemas.replay`: a completed main quest's movie can be replayed from the journal's Past tab.
    pub replay: bool,
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
    /// `quest_components.skill_id`, 0 for none. A component without a direct cinema uses its
    /// skill's `CinemaEffect` for the replay list (`FUN_393780a0`).
    pub skill: u32,
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
    /// `SupplyAaPoint`: the AA points the quest gives (`FUN_394a11a0`).
    pub aa_point: i64,
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
    /// `quest_mails.id` -> name, the `$1` of the client's `QUEST_MAIL_SENT` text (`FUN_3937a650`
    /// formats the `quest_mails` row's name).
    pub quest_mail_names: HashMap<u32, String>,
    /// `quest_cameras` by id, and `model_quest_cameras`: model id -> row id (the talk's close-up).
    pub cameras: HashMap<u32, crate::ingame::quest_framing::Row>,
    pub model_cameras: HashMap<u32, u32>,
    pub aliases: HashMap<u32, String>,
    pub monster_groups: HashMap<u32, String>,
    pub doodads: HashMap<u32, String>,
    /// Skill id -> the first `CinemaEffect` of its effects (`FUN_393780a0`), for a replay
    /// component with no `quest_components.cinema_id`.
    pub skill_cinemas: HashMap<u32, CinemaDef>,
    /// `quest_names` with `quest_name_kind_id` 2: a score quest's objective line
    /// (`X2Quest.GetScoreQuestObjective`, `FUN_397d4660`).
    pub score_objectives: HashMap<u32, String>,
    /// `zones.id` (a quest's zone) -> `zone_groups.id`.
    pub zone_group: HashMap<u32, u32>,
}

/// The linked-quest objective rows of `X2Quest.GetLinkedQuestObjectives` (`FUN_394a4fc0`).
pub struct LinkedObjectives {
    /// `(obj, count)` for every Progress component with a counted act.
    pub entries: Vec<(String, i32)>,
    /// The Ready components to list when the quest has no Progress components; the text is the
    /// locale's `report_alias` / `talk_report_npc` / `find_report_doodad`.
    pub ready_components: u32,
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
    /// `quest_act_supply_items.show_action_bar` (only that table has the column).
    show_action_bar: bool,
}

impl QuestTables {
    pub fn load(conn: &Connection) -> rusqlite::Result<QuestTables> {
        let mut t = QuestTables::default();
        // English texts of the quest tables
        let mut text: HashMap<(String, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, tbl_column_name, idx, en_us FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND (\
                 (tbl_name IN ('quest_contexts','quest_categories','quest_act_obj_aliases','quest_monster_groups','doodad_almighties','quest_mails','quest_names') AND tbl_column_name = 'name') OR \
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
        for (table, map) in [("quest_categories", &mut t.categories), ("quest_act_obj_aliases", &mut t.aliases), ("quest_monster_groups", &mut t.monster_groups), ("doodad_almighties", &mut t.doodads), ("quest_mails", &mut t.quest_mail_names)] {
            for ((tb, id), s) in &text {
                if tb == table {
                    map.insert(*id, s.clone());
                }
            }
        }
        if let Ok(mut q) = conn.prepare("SELECT id, name FROM quest_mails") {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                let base: Option<String> = r.get(1)?;
                // the en_us localized row wins; without one the base quest_mails.name is the $1
                t.quest_mail_names.insert(id, en("quest_mails", id).or(base).unwrap_or_default());
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, name, category_id, LEVEL, REPEATABLE, selective, let_it_done, detail_id, chapter_idx, quest_idx, zone_id, use_quest_camera, score FROM quest_contexts",
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
                        quest_idx: any_int(r, 9)? as u32,
                        zone_id: any_int(r, 10)? as u32,
                        use_quest_camera: any_flag(r, 11)?,
                        score: any_int(r, 12)?.max(0) as u32,
                        repeatable: t.repeatable.contains(&id),
                        components: Vec::new(),
                    },
                );
            }
        }
        // `quest_names` kind 2 is the score quest's objective line (`FUN_397d4660`).
        if let Ok(mut q) = conn.prepare("SELECT id, quest_context_id, quest_name_kind_id, name FROM quest_names ORDER BY id") {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if any_int(r, 2)? != 2 {
                    continue;
                }
                let id = any_int(r, 0)? as u32;
                let fallback: Option<String> = r.get(3)?;
                t.score_objectives.insert(any_int(r, 1)? as u32, en("quest_names", id).or(fallback).unwrap_or_default());
            }
        }
        {
            let mut q = conn.prepare("SELECT id, quest_context_id, component_kind_id, or_unit_reqs, hide_quest_marker, skill_id FROM quest_components ORDER BY quest_context_id, component_kind_id, id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (id, quest, kind) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32, any_int(r, 2)? as u8);
                if kind == kind::START {
                    t.components.insert(id, StartComponent { quest, any_req: any_flag(r, 3)?, hidden: any_flag(r, 4)?, reqs: Vec::new() });
                }
                t.parts.insert(id, ComponentDef { id, quest, kind, skill: any_int(r, 5)? as u32, ..ComponentDef::default() });
                if let Some(d) = t.defs.get_mut(&quest) {
                    d.components.push(id);
                }
            }
        }
        if let Ok(mut q) = conn.prepare(
            "SELECT qc.id, c.name, qc.play_cinema_before_bubble, c.replay FROM quest_components qc JOIN cinemas c ON c.id = qc.cinema_id",
        ) {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                if let Some(c) = t.parts.get_mut(&id) {
                    c.cinema = Some(CinemaDef { name: r.get(1)?, before_bubble: any_flag(r, 2)?, replay: any_flag(r, 3)? });
                }
            }
        }
        // `FUN_393780a0`: a replay component without `cinema_id` uses the first CinemaEffect of its
        // skill (skill_effects -> effects.actual_id -> cinema_effects.cinema_id -> cinemas).
        if let Ok(mut q) = conn.prepare(
            "SELECT se.skill_id, c.name, c.replay FROM skill_effects se \
             JOIN effects e ON e.id = se.effect_id \
             JOIN cinema_effects ce ON ce.id = e.actual_id \
             JOIN cinemas c ON c.id = ce.cinema_id \
             WHERE e.actual_type = 'CinemaEffect' ORDER BY se.skill_id, se.id",
        ) {
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let skill = any_int(r, 0)? as u32;
                t.skill_cinemas
                    .entry(skill)
                    .or_insert(CinemaDef { name: r.get(1)?, before_bubble: false, replay: any_flag(r, 2)? });
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
            let (id_col, target, count, value, grade, alias, use_alias, action_bar) = (
                col(&["id"]),
                col(&["npc_id", "item_id", "doodad_id", "quest_monster_group_id", "npc_group_id", "quest_id", "quest_context_id", "complete_component", "sphere_id", "skill_id", "buff_id", "craft_id", "appellation_id", "item_group_id", "zone_id"]),
                col(&["count"]),
                col(&["exp", "amount", "lp", "point", "level"]),
                col(&["grade_id"]),
                col(&["quest_act_obj_alias_id"]),
                col(&["use_alias"]),
                col(&["show_action_bar"]),
            );
            let Some(id_col) = id_col else { continue };
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let get = |c: Option<usize>| c.map(|c| any_int(r, c)).transpose();
                let aliased = use_alias.map(|c| any_flag(r, c)).transpose()?.unwrap_or(false);
                // `QuestActObjZoneKill` counts both PvP and NPC kills (`FUN_397d3440` case 0x31 sums
                // the detail's two counters); every other act has one `count`.
                let count = if ty == "ObjZoneKill" {
                    (get(col(&["count_pk"]))?.unwrap_or(0) + get(col(&["count_npc"]))?.unwrap_or(0)) as i32
                } else {
                    get(count)?.map_or(1, |c| c as i32)
                };
                let d = Detail {
                    target: get(target)?.unwrap_or(0) as u32,
                    count,
                    value: get(value)?.unwrap_or(0),
                    grade: get(grade)?.unwrap_or(0).clamp(0, 255) as u8,
                    alias: if aliased { get(alias)?.unwrap_or(0) as u32 } else { 0 },
                    show_action_bar: action_bar.map(|c| any_flag(r, c)).transpose()?.unwrap_or(false),
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
                c.acts.push(ActDef {
                    id,
                    ty,
                    target: d.target,
                    count: d.count,
                    value: d.value,
                    grade: d.grade,
                    alias: d.alias,
                    show_action_bar: d.show_action_bar,
                    objective: None,
                });
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

    /// The Progress step's objectives as `(done, count)`, one slot per Progress component and only
    /// the act types the native counts. `FUN_397d7070` walks the components of kind 4 in order, takes
    /// each component's first act (`FUN_397d5b90`), and stops after five slots; score quests use the
    /// narrower set of `FUN_397d5c10` (0x2b for `QuestActObjItemGroupGather`).
    fn objective_pairs(&self, q: &Quest, score: bool) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for (slot, c) in self.parts_of(q.template_id, kind::PROGRESS).enumerate() {
            if slot >= 5 {
                break;
            }
            let Some(a) = c.acts.first() else { continue };
            let counted = if score {
                matches!(a.ty.as_str(), "ObjMonsterHunt" | "ObjMonsterGroupHunt" | "ObjItemGather" | "ObjItemUse" | "ObjInteraction" | "ObjItemGroupGather")
            } else {
                matches!(a.ty.as_str(), "ObjMonsterHunt" | "ObjMonsterGroupHunt" | "ObjItemGather" | "ObjItemUse" | "ObjInteraction" | "ObjZoneKill" | "ObjCraft")
            };
            if !counted {
                continue;
            }
            out.push((q.objectives.get(slot).copied().unwrap_or(0).max(0), a.count));
        }
        out
    }

    /// The score a score quest has reached: the sum of each objective's count times its counter
    /// (`FUN_397d5c10`).
    fn score(&self, q: &Quest) -> i64 {
        self.objective_pairs(q, true).iter().map(|&(done, count)| i64::from(done) * i64::from(count.max(0))).sum()
    }

    /// `X2Quest.IsLetItDoneQuest*`: the original's early-completion check (`FUN_397d6320`, called by
    /// `FUN_39376930` / `FUN_39376990`). It holds while the quest is not a main quest, is not
    /// repeatable, is worked on, has objectives, is not complete yet, and either the score or the
    /// summed counters are at least half of what the step asks (`DAT_3999c47c` 0.5). Native passes the
    /// player's level (`player+0x4b4`, set by `FUN_390b4930`) as a third argument; the recovered
    /// `FUN_397d6320` body does not read it.
    pub fn is_let_it_done(&self, q: &Quest) -> bool {
        let Some(def) = self.defs.get(&q.template_id) else { return false };
        if def.detail == 2 || !def.let_it_done || self.repeatable.contains(&q.template_id) {
            return false;
        }
        if q.status != status::PROGRESS {
            return false;
        }
        let pairs = self.objective_pairs(q, def.score > 0);
        if pairs.is_empty() {
            return false;
        }
        if def.score > 0 {
            let score = self.score(q);
            return score < i64::from(def.score) && score * 2 >= i64::from(def.score);
        }
        let (mut done, mut count) = (0i64, 0i64);
        let mut complete = true;
        for (d, c) in pairs {
            if c <= 0 {
                continue;
            }
            let d = d.min(c);
            done += i64::from(d);
            count += i64::from(c);
            complete &= d >= c;
        }
        count > 0 && !complete && done * 2 >= count
    }

    /// `X2Quest.IsOverDoneQuest*`: the original's over-done check (`FUN_397d71b0`). It does not look
    /// at the quest status: a quest the server already marked ready is still over-done. The quest must
    /// be a non-main let-it-done quest, every objective must be complete (`FUN_397d7070`), and each
    /// `min(done, 2 * count)` must reach `ceil(1.5 * count)` (`DAT_3999c478`); score quests use their
    /// score.
    pub fn is_over_done(&self, q: &Quest) -> bool {
        let Some(def) = self.defs.get(&q.template_id) else { return false };
        if def.detail == 2 || !def.let_it_done {
            return false;
        }
        let pairs = self.objective_pairs(q, def.score > 0);
        if def.score > 0 {
            return self.score(q) * 2 >= i64::from(def.score) * 3;
        }
        !pairs.is_empty()
            && pairs.iter().all(|&(done, count)| {
                let (d, c) = (i64::from(done), i64::from(count));
                c > 0 && d >= c && d.min(c * 2) * 2 >= c * 3
            })
    }

    /// `X2Quest.IsScoreQuest` (`FUN_397d3600`): `quest_contexts.score` is above 0.
    pub fn is_score(&self, quest: u32) -> bool {
        self.defs.get(&quest).is_some_and(|def| def.score > 0)
    }

    /// `X2Quest.GetScoreQuestCurrentScore` (`FUN_3949cb00` -> `FUN_397d5c10`): the counters of the
    /// active quest multiplied by each Progress component's first counted act's amount. The native
    /// does not test the score, so a quest with those act types can answer above 0 without being a
    /// score quest; an inactive or unknown quest has no counter and the caller answers 0.
    pub fn score_current(&self, q: &Quest) -> i64 {
        self.score(q)
    }

    /// `X2Quest.GetScoreQuestDoneScore` (`FUN_3949cc10`): `quest_contexts.score`, 0 for an unknown
    /// quest or one without a score.
    pub fn score_required(&self, quest: u32) -> i64 {
        self.defs.get(&quest).map_or(0, |def| i64::from(def.score))
    }

    /// `X2Quest.GetScoreQuestObjective` (`FUN_3949ca70` -> `FUN_397d4660`): the `quest_names` row with
    /// `quest_name_kind_id` 2, or the empty string. The native only searches a quest whose score is
    /// above 0.
    pub fn score_objective(&self, quest: u32) -> String {
        if !self.is_score(quest) {
            return String::new();
        }
        self.score_objectives.get(&quest).cloned().unwrap_or_default()
    }

    /// `X2Quest.GetLinkedQuestSummary` (`FUN_394a4dc0`): the first Progress component's kind-4 text,
    /// else the first Ready component's, with its macros expanded: the native copies the text
    /// (`FUN_3901aa10`) and runs the macro expander (`FUN_39435050`) before pushing the result.
    pub fn linked_summary(&self, quest: u32, texts: &Texts) -> Option<String> {
        if !self.defs.contains_key(&quest) {
            return None;
        }
        self.parts_of(quest, kind::PROGRESS)
            .find_map(|c| c.texts.first().map(|t| texts.expand(t)))
            .or_else(|| self.parts_of(quest, kind::READY).find_map(|c| c.texts.first().map(|t| texts.expand(t))))
    }

    /// `X2Quest.GetLinkedQuestObjectives` (`FUN_394a4fc0`): one `{obj, count}` row per Progress
    /// component with a counted act. A quest without Progress components lists its Ready components
    /// instead (the caller supplies the locale's report text, count 0).
    pub fn linked_objectives(&self, quest: u32, texts: &Texts) -> Option<LinkedObjectives> {
        if !self.defs.contains_key(&quest) {
            return None;
        }
        let mut progress = self.parts_of(quest, kind::PROGRESS).peekable();
        if progress.peek().is_none() {
            return Some(LinkedObjectives { entries: Vec::new(), ready_components: self.parts_of(quest, kind::READY).count() as u32 });
        }
        let mut entries = Vec::new();
        for c in progress {
            let Some(a) = self.counted_act(c) else { continue };
            let base = self
                .aliases
                .get(&a.alias)
                .filter(|_| a.alias != 0)
                .map(|s| texts.expand(s))
                .or_else(|| c.main_text().map(|t| texts.expand(t)))
                .unwrap_or_else(|| texts.describe(a));
            entries.push((base, Self::native_act_count(a)));
        }
        Some(LinkedObjectives { entries, ready_components: 0 })
    }

    /// `X2Quest.GetFirstObjectiveCount` (`FUN_394a4c30`): the amount of the first Progress
    /// component's first counted act (`FUN_397d5b90` -> `FUN_397d3440`, see
    /// `native_act_count`). The act types `FUN_397d3440` does not handle count 0.
    pub fn first_objective_count(&self, quest: u32) -> i32 {
        let Some(c) = self.parts_of(quest, kind::PROGRESS).next() else { return 0 };
        self.counted_act(c).map_or(0, Self::native_act_count)
    }

    /// `X2Quest.GetObjectiveComponentCount` (`FUN_394a4ae0`): the Progress components of the quest.
    pub fn objective_component_count(&self, quest: u32) -> u32 {
        self.parts_of(quest, kind::PROGRESS).count() as u32
    }

    /// The cinema of a component for the replay list: its own `cinema_id`, else (when it has none)
    /// the first `CinemaEffect` of its skill (`FUN_393780a0`).
    fn replay_cinema<'a>(&'a self, c: &'a ComponentDef) -> Option<&'a CinemaDef> {
        c.cinema.as_ref().or_else(|| self.skill_cinemas.get(&c.skill))
    }

    /// The replay cinemas of `quest`, in component order: `FUN_3937b540` walks a completed main
    /// quest's components and appends each `cinemas.replay` movie.
    pub fn replay_cinemas(&self, quest: u32) -> Vec<String> {
        let Some(def) = self.defs.get(&quest) else { return Vec::new() };
        if !def.is_main() {
            return Vec::new();
        }
        def.components
            .iter()
            .filter_map(|id| self.parts.get(id))
            .filter_map(|c| self.replay_cinema(c))
            .filter(|c| c.replay)
            .map(|c| c.name.clone())
            .collect()
    }

    /// `X2Quest.GetMainQuestCutSceneReplayList` (`FUN_39378e10`): every completed main quest whose
    /// component has a `cinemas.replay` movie, as `(quest id, cinema name)`. `FUN_3937c940`
    /// rebuilds the vector from the completed blocks and `FUN_39377f30` sorts it by
    /// `(chapter_idx, quest_idx)`; the stable sort keeps a quest's components in order.
    pub fn replay_list(&self, completed: &[aa_net::quest::CompletedBlock]) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        for id in self.defs.keys().copied() {
            if !completed.iter().any(|b| b.has(id)) {
                continue;
            }
            for cinema in self.replay_cinemas(id) {
                out.push((id, cinema));
            }
        }
        out.sort_by_key(|(id, _)| self.defs.get(id).map_or((0, 0), |d| (d.chapter, d.quest_idx)));
        out
    }

    /// `FUN_397d42d0` of a quest's Ready component: whether it has a `QuestActConReportNpc` (0x14) or
    /// `QuestActConReportDoodad` (0x15) act. This is the flag of a `COMPLETE_QUEST_CONTEXT_DOODAD`
    /// event.
    pub fn ready_has_report(&self, quest: u32) -> bool {
        self.parts_of(quest, kind::READY)
            .flat_map(|c| &c.acts)
            .any(|a| matches!(a.ty.as_str(), "ConReportNpc" | "ConReportDoodad"))
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
        let mut aa_seen = false;
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
                    "SupplyAaPoint" if !aa_seen => {
                        r.aa_point = a.value;
                        aa_seen = true;
                    }
                    "SupplyAppellation" => r.appellation = a.target,
                    _ => {}
                }
            }
        }
        r
    }

    /// The items a quest supplies (`SupplyItem` acts of its kind-3 components): what
    /// `X2Quest.GetUseTypeQuestItems` reads. `action_bar_only` keeps the acts the native shows on the
    /// tracker (`show_action_bar`); the restart check reads them all. `FUN_394a55a0` drops an item
    /// without a use skill (`FUN_39695dc0`, `+0x98`; [`GameData::item_usable`]).
    pub fn supply_item_targets(&self, game: &GameData, quest: u32, action_bar_only: bool) -> Vec<u32> {
        self.parts_of(quest, kind::SUPPLY)
            .flat_map(|c| &c.acts)
            .filter(|a| a.ty == "SupplyItem" && a.target != 0 && (!action_bar_only || a.show_action_bar) && game.item_usable(a.target))
            .map(|a| a.target)
            .collect()
    }

    /// How many items a restart hands out again: the sum of the kind-3 `SupplyItem` counts.
    /// `FUN_3937a8d0` compares it with the player's free bag slots.
    pub fn supply_item_count(&self, quest: u32) -> i64 {
        self.parts_of(quest, kind::SUPPLY)
            .flat_map(|c| &c.acts)
            .filter(|a| a.ty == "SupplyItem")
            .map(|a| i64::from(a.count.max(0)))
            .sum()
    }

    /// The items a quest asks the player to use: the `ObjItemUse` targets of its Progress steps
    /// (`X2Quest.GetUseTypeQuestItems` and `IsUseItemInActiveQuest`, `FUN_394a55a0` / `FUN_3949e280`),
    /// with the same use-skill filter.
    pub fn use_item_targets(&self, game: &GameData, quest: u32) -> Vec<u32> {
        self.parts_of(quest, kind::PROGRESS)
            .flat_map(|c| &c.acts)
            .filter(|a| a.ty == "ObjItemUse" && a.target != 0 && game.item_usable(a.target))
            .map(|a| a.target)
            .collect()
    }

    /// `X2Quest.GetUseTypeQuestItemsByObjIndex` (`FUN_394a58d0`): the supply items assigned to
    /// objective `index` (1 based, counted over the not-done Progress components) plus that
    /// objective's `ObjItemUse` items. A supply item is assigned by `FUN_394a2930`'s relation
    /// (`GameData::skill_relates_to`); objective 1 gets the ones unrelated to any other objective.
    /// `ObjItemGather` / `EtcItemObtain` only feed the relation, they are never returned.
    pub fn items_by_objective(&self, game: &GameData, quest: u32, active: Option<&Quest>, index: u32) -> Vec<u32> {
        let score = self.defs.get(&quest).map_or(0, |d| d.score);
        let objectives = active.map(|q| q.objectives).unwrap_or([0; 5]);
        let mut supplies = Vec::new();
        let mut selected_use = Vec::new();
        let mut other_use = Vec::new();
        let mut selected_aux = Vec::new();
        let mut other_aux = Vec::new();
        let mut current = 0u32;
        for c in self.parts_of(quest, kind::PROGRESS) {
            let done = self.component_done(score, c, objectives);
            if !done {
                current = current.wrapping_add(1);
            }
            for a in &c.acts {
                if a.target == 0 {
                    continue;
                }
                match a.ty.as_str() {
                    "ObjItemUse" => {
                        // Only the use act's item is filtered (`FUN_39695dc0` + `+0x98`).
                        if !game.item_usable(a.target) {
                            continue;
                        }
                        if !done && current == index { selected_use.push(a.target) } else { other_use.push(a.target) }
                    }
                    "ObjItemGather" | "EtcItemObtain" => {
                        // Relation data: not filtered by the use skill.
                        if !done && current == index { selected_aux.push(a.target) } else { other_aux.push(a.target) }
                    }
                    _ => {}
                }
            }
        }
        supplies.extend(self.supply_item_targets(game, quest, true));
        let other_use: HashSet<u32> = other_use.into_iter().collect();
        let selected_use_set: HashSet<u32> = selected_use.iter().copied().collect();
        let selected_aux: HashSet<u32> = selected_aux.into_iter().collect();
        let other_aux: HashSet<u32> = other_aux.into_iter().collect();
        let mut out = Vec::new();
        for item in supplies {
            if other_use.contains(&item) || selected_use_set.contains(&item) {
                continue;
            }
            let assigned = if index == 1 {
                !game.skill_relates_to(item, &other_aux) && !game.skill_relates_to(item, &other_use)
            } else {
                game.skill_relates_to(item, &selected_aux) || game.skill_relates_to(item, &selected_use_set)
            };
            if assigned && !out.contains(&item) {
                out.push(item);
            }
        }
        for item in selected_use {
            if !out.contains(&item) {
                out.push(item);
            }
        }
        out
    }

    /// `FUN_397d5b90`'s counted act set, built by `FUN_397d6640` (`DAT_3a5e11f0` / `11f4`): act
    /// ids 0-7, 36-39, 42-46, 49, 50, 54, 55, 57-59 and 61-63, the types `FUN_397d3370` handles plus
    /// `ObjAggro` (57).
    fn native_counted(ty: &str) -> bool {
        matches!(
            ty,
            "ObjMonsterHunt"
                | "ObjMonsterGroupHunt"
                | "ObjInteraction"
                | "ObjZoneMonsterHunt"
                | "ObjItemGather"
                | "ObjItemUse"
                | "ObjZoneQuestComplete"
                | "ObjEffectFire"
                | "ObjItemGroupGather"
                | "ObjItemGroupUse"
                | "ObjExpressFire"
                | "ObjCraft"
                | "ObjZoneKill"
                | "ObjTalk"
                | "ObjSphere"
                | "ObjZoneNpcTalk"
                | "ObjTalkNpcGroup"
                | "ObjDoodadPhaseCheck"
                | "ObjCompleteQuest"
                | "ObjSendMail"
                | "ObjMateLevel"
                | "ObjLevel"
                | "ObjAbilityLevel"
                | "ObjCinema"
                | "ObjDistance"
                | "ObjCondition"
                | "ObjAggro"
        )
    }

    /// The first act in `FUN_397d5b90`'s counted set (the set `FUN_397d6640` builds).
    fn counted_act<'a>(&self, c: &'a ComponentDef) -> Option<&'a ActDef> {
        c.acts.iter().find(|a| Self::native_counted(&a.ty))
    }

    /// `FUN_397d3440`: the act's amount. Only the act types in its switch have one (0 MonsterHunt,
    /// 1 MonsterGroupHunt, 2 ItemGather, 4 ItemUse, 5 Interaction, 0x31 ZoneKill = `count_pk +
    /// count_npc`, already summed by the loader, and 0x32 Craft); the rest answer 0.
    fn native_act_count(a: &ActDef) -> i32 {
        match a.ty.as_str() {
            "ObjMonsterHunt" | "ObjMonsterGroupHunt" | "ObjItemGather" | "ObjItemUse" | "ObjInteraction"
            | "ObjZoneKill" | "ObjCraft" => a.count.max(0),
            _ => 0,
        }
    }

    /// The component's slot among the template's kind-4 components (`FUN_397d4360`); only the first
    /// five slots exist (0..=4), everything else is `None`.
    fn slot_of_component(&self, quest: u32, id: u32) -> Option<usize> {
        let def = self.defs.get(&quest)?;
        let mut slot = 0usize;
        for cid in &def.components {
            let Some(c) = self.parts.get(cid) else { continue };
            if c.kind != kind::PROGRESS {
                continue;
            }
            if slot >= 5 {
                return None;
            }
            if c.id == id {
                return Some(slot);
            }
            slot += 1;
        }
        None
    }

    /// `FUN_397d3370`: whether the first counted act's requirement is met by `counter`. Types with
    /// a detail count compare it; the rest are met by any progress. `ObjAggro` (0x39) has no case,
    /// so it is never met (the switch's default).
    fn act_met(a: &ActDef, counter: i32) -> bool {
        if !Self::native_counted(&a.ty) {
            return false;
        }
        match a.ty.as_str() {
            "ObjMonsterHunt" | "ObjMonsterGroupHunt" | "ObjInteraction" | "ObjZoneMonsterHunt"
            | "ObjItemGather" | "ObjItemUse" | "ObjZoneQuestComplete" | "ObjEffectFire"
            | "ObjItemGroupGather" | "ObjItemGroupUse" | "ObjExpressFire" | "ObjCraft"
            | "ObjZoneKill" => counter >= a.count,
            "ObjAggro" => false,
            _ => counter > 0,
        }
    }

    /// `FUN_397d6180`'s dependency test for a component's `CheckCompleteComponent` act (0xc): the
    /// referenced component must exist and its first counted act must be met in its own slot.
    fn dependency_met(&self, component: &ComponentDef, objectives: [i32; 5]) -> bool {
        let Some(act) = component.acts.iter().find(|a| a.ty == "CheckCompleteComponent") else { return true };
        if act.target == 0 || !self.parts.contains_key(&act.target) {
            return true;
        }
        let Some(slot) = self.slot_of_component(component.quest, act.target) else { return true };
        let Some(first) = self.parts.get(&act.target).and_then(|c| self.counted_act(c)) else { return true };
        Self::act_met(first, objectives[slot])
    }

    /// `FUN_397d6210`: a score quest is never done; a component without a counted act is done; else
    /// the 0xc dependency must pass, the slot must exist, and the first counted act's per-type test
    /// must be met in that slot.
    fn component_done(&self, score: u32, component: &ComponentDef, objectives: [i32; 5]) -> bool {
        if score > 0 {
            return false;
        }
        let Some(first) = self.counted_act(component) else { return true };
        if !self.dependency_met(component, objectives) {
            return false;
        }
        let Some(slot) = self.slot_of_component(component.quest, component.id) else { return false };
        Self::act_met(first, objectives[slot])
    }

    /// `X2Quest.NowIsAggroComponent` (`FUN_394a53c0`): `FUN_397d5b90` returns the first act whose
    /// type is in its counted set (the set `FUN_397d6640` builds), and that act is `ObjAggro` (0x39).
    pub fn is_aggro(&self, quest: u32) -> bool {
        self.parts_of(quest, kind::PROGRESS).any(|c| self.counted_act(c).is_some_and(|a| a.ty == "ObjAggro"))
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
            assert_eq!(c.cinema, Some(CinemaDef { name: "fly_east".to_string(), before_bubble: false, replay: false }));
        }
        // 2246: the Start component 10186 plays zq_20_karl
        if let Some(c) = t.parts.get(&10186) {
            assert_eq!(c.quest, 2246);
            assert_eq!(c.cinema, Some(CinemaDef { name: "zq_20_karl".to_string(), before_bubble: false, replay: false }));
        }
    }

    #[test]
    fn a_replay_component_without_a_cinema_uses_its_skills_cinema_effect() {
        let Some(t) = tables() else { return };
        // Quest 3556's Ready component 17120 has no cinema_id but skill 17610, whose CinemaEffect
        // leads to cinema 121 "sl_ha_024" (replay).
        let c = t.parts.get(&17120).expect("quest 3556 has component 17120");
        assert_eq!((c.quest, c.kind, c.skill, c.cinema.clone()), (3556, kind::READY, 17610, None));
        assert_eq!(t.replay_cinemas(3556), vec!["sl_ha_024".to_string()]);
        let block = CompletedBlock { id: (3556 / 64) as u16, bits: 1 << (3556 % 64) };
        assert_eq!(t.replay_list(std::slice::from_ref(&block)), vec![(3556, "sl_ha_024".to_string())]);
    }

    #[test]
    fn let_it_done_and_over_done_use_the_native_half_and_one_and_a_half() {
        // Two Progress components: 4 kills and 2 item uses. The native reads one slot per component
        // from its first act, so the second act of the first component is ignored.
        let act = |ty: &str, count| ActDef { ty: ty.into(), count, ..ActDef::default() };
        let mut t = QuestTables::default();
        t.defs.insert(7, QuestDef { let_it_done: true, detail: 1, components: vec![10, 11], ..QuestDef::default() });
        t.parts.insert(10, ComponentDef { id: 10, quest: 7, kind: kind::PROGRESS, acts: vec![act("ObjMonsterHunt", 4), act("ObjItemUse", 99)], ..ComponentDef::default() });
        t.parts.insert(11, ComponentDef { id: 11, quest: 7, kind: kind::PROGRESS, acts: vec![act("ObjItemUse", 2)], ..ComponentDef::default() });
        let quest = |status, a, b| Quest { template_id: 7, status, component_id: 10, objectives: [a, b, 0, 0, 0], ..Quest::default() };

        // 3 of 6 is exactly half: completable early, not complete.
        assert!(t.is_let_it_done(&quest(status::PROGRESS, 2, 1)));
        assert!(!t.is_over_done(&quest(status::PROGRESS, 2, 1)));
        // Under half is not.
        assert!(!t.is_let_it_done(&quest(status::PROGRESS, 1, 1)));
        // Fully done is a report, not an early completion.
        assert!(!t.is_let_it_done(&quest(status::PROGRESS, 4, 2)));
        assert!(!t.is_over_done(&quest(status::PROGRESS, 4, 2)));
        // 1.5 times every count is over-done, including when the server already marked it ready.
        assert!(t.is_over_done(&quest(status::PROGRESS, 6, 3)));
        assert!(t.is_over_done(&quest(status::READY, 6, 3)));
        assert!(!t.is_let_it_done(&quest(status::READY, 6, 3)));
        assert!(!t.is_over_done(&quest(status::READY, 5, 3)));
        assert!(!t.is_over_done(&quest(status::READY, 6, 2)));
        // A main quest, a repeatable quest and a quest without the flag never complete early or
        // count as over-done.
        t.defs.get_mut(&7).unwrap().detail = 2;
        assert!(!t.is_let_it_done(&quest(status::PROGRESS, 2, 1)));
        assert!(!t.is_over_done(&quest(status::READY, 6, 3)));
        t.defs.get_mut(&7).unwrap().detail = 1;
        t.repeatable.insert(7);
        assert!(!t.is_let_it_done(&quest(status::PROGRESS, 2, 1)));
        t.repeatable.clear();
        t.defs.get_mut(&7).unwrap().let_it_done = false;
        assert!(!t.is_let_it_done(&quest(status::PROGRESS, 2, 1)));
        assert!(!t.is_over_done(&quest(status::READY, 6, 3)));

        // Score quests: objective count times its counter; 100 points are asked.
        let mut s = QuestTables::default();
        s.defs.insert(8, QuestDef { let_it_done: true, detail: 1, score: 100, components: vec![12], ..QuestDef::default() });
        s.parts.insert(12, ComponentDef { id: 12, quest: 8, kind: kind::PROGRESS, acts: vec![act("ObjMonsterHunt", 10)], ..ComponentDef::default() });
        let score_quest = |status, v| Quest { template_id: 8, status, component_id: 12, objectives: [v, 0, 0, 0, 0], ..Quest::default() };
        assert!(s.is_let_it_done(&score_quest(status::PROGRESS, 5)));
        assert!(!s.is_let_it_done(&score_quest(status::PROGRESS, 10)));
        assert!(s.is_over_done(&score_quest(status::READY, 15)));
        assert!(!s.is_over_done(&score_quest(status::READY, 14)));
    }

    #[test]
    fn zero_counts_and_uncounted_acts_are_not_objectives() {
        let act = |ty: &str, count| ActDef { ty: ty.into(), count, ..ActDef::default() };
        let mut t = QuestTables::default();
        t.defs.insert(9, QuestDef { let_it_done: true, detail: 1, components: vec![20, 21], ..QuestDef::default() });
        t.parts.insert(20, ComponentDef { id: 20, quest: 9, kind: kind::PROGRESS, acts: vec![act("ObjTalk", 3)], ..ComponentDef::default() });
        t.parts.insert(21, ComponentDef { id: 21, quest: 9, kind: kind::PROGRESS, acts: vec![act("ObjMonsterHunt", 0)], ..ComponentDef::default() });
        let q = Quest { template_id: 9, status: status::READY, component_id: 20, objectives: [3, 9, 0, 0, 0], ..Quest::default() };
        assert!(!t.is_over_done(&q), "ObjTalk is not counted and a 0-count act is not over-done");
        assert!(!t.is_let_it_done(&q), "a 0-count act stays 0, it is not promoted to 1");
    }

    #[test]
    fn zone_kill_counts_both_of_its_fields() {
        let Some(t) = tables() else { return };
        let Some((component, act)) = t.parts.values().flat_map(|c| c.acts.iter().filter(|a| a.ty == "ObjZoneKill").map(move |a| (c.id, a))).next() else { return };
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(conn) = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
        let (pk, npc): (i64, i64) = conn
            .query_row(
                "SELECT k.count_pk, k.count_npc FROM quest_acts a JOIN quest_act_obj_zone_kills k ON k.id = a.act_detail_id WHERE a.id = ?1",
                [act.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(act.count, (pk + npc) as i32, "component {component}: FUN_397d3440 case 0x31 sums both counters");
    }

    #[test]
    fn quest_item_aggro_and_aa_point_getters_read_the_native_rows() {
        let Some(t) = tables() else { return };
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(game) = GameData::load(&path) else { return };
        // A real ObjAggro quest (1408) and the held item acts of quest 1222's kind-3 Supply step.
        assert!(t.is_aggro(1408));
        let supply = t.supply_item_targets(&game, 1222, true);
        assert!(!supply.is_empty(), "quest 1222 has an action-bar supply item");
        assert!(supply.iter().all(|id| game.item_usable(*id)));
        assert!(t.supply_item_targets(&game, 1222, false).len() >= supply.len());
        assert!(t.supply_item_count(1222) > 0);
        // Quest 26 supplies the item it asks to use (14633), so objective 1 (1 based) holds it and
        // another index does not.
        assert!(t.items_by_objective(&game, 26, None, 1).contains(&14633));
        assert!(!t.items_by_objective(&game, 26, None, 2).contains(&14633));
        assert!(t.items_by_objective(&game, 999_999, None, 1).is_empty());
        // Quest 6370's supply item 34067 has a use skill that produces the objective's item 34282
        // (`FUN_394a2930`'s relation), so both are on objective 1 (supply first).
        let list = t.items_by_objective(&game, 6370, None, 1);
        assert_eq!(list, vec![34067, 34282]);
        // The filter drops an item without a use skill.
        let act = |ty: &str, target| ActDef { ty: ty.into(), target, count: 1, ..ActDef::default() };
        let mut s = QuestTables::default();
        s.defs.insert(9, QuestDef { components: vec![30, 31], ..QuestDef::default() });
        // Item 100 exists but has no use skill; 0 does not exist.
        assert!(!game.item_usable(100) && !game.item_usable(0));
        // The getters do not resolve the use skill: an item whose skill has no `skills` row counts.
        let mut unresolved = GameData::default();
        unresolved.items.insert(4, super::super::data::ItemInfo { use_skill: 9, ..Default::default() });
        assert!(unresolved.item_usable(4));
        s.parts.insert(30, ComponentDef { id: 30, quest: 9, kind: kind::SUPPLY, acts: vec![act("SupplyItem", 100), act("SupplyItem", 0)], ..ComponentDef::default() });
        s.parts.insert(31, ComponentDef { id: 31, quest: 9, kind: kind::PROGRESS, acts: vec![act("ObjItemUse", 100), act("ObjItemUse", 0)], ..ComponentDef::default() });
        assert!(s.supply_item_targets(&game, 9, true).is_empty());
        assert!(s.use_item_targets(&game, 9).is_empty());
        // `FUN_397d5b90` takes the first act in the counted set, not the first act.
        let mut a = QuestTables::default();
        a.defs.insert(11, QuestDef { components: vec![41], ..QuestDef::default() });
        a.parts.insert(41, ComponentDef { id: 41, quest: 11, kind: kind::PROGRESS, acts: vec![act("ObjMonsterHunt", 1), act("ObjAggro", 1)], ..ComponentDef::default() });
        assert!(!a.is_aggro(11));
        a.parts.get_mut(&41).unwrap().acts.remove(0);
        assert!(a.is_aggro(11));
        // ConFail is not in FUN_397d6640's set, so ObjAggro is still the first counted act.
        a.parts.get_mut(&41).unwrap().acts.insert(0, act("ConFail", 1));
        assert!(a.is_aggro(11));
        // `SupplyAaPoint` takes the first act, not the sum.
        let point = |value| ActDef { ty: "SupplyAaPoint".into(), value, ..ActDef::default() };
        let mut p = QuestTables::default();
        p.defs.insert(10, QuestDef { components: vec![40], ..QuestDef::default() });
        p.parts.insert(40, ComponentDef { id: 40, quest: 10, kind: kind::REWARD, acts: vec![point(50), point(70)], ..ComponentDef::default() });
        assert_eq!(p.rewards(10).aa_point, 50);
    }

    #[test]
    fn objective_done_follows_fun_397d6210() {
        let act = |ty: &str, target, count| ActDef { ty: ty.into(), target, count, ..ActDef::default() };
        let mut t = QuestTables::default();
        t.defs.insert(20, QuestDef { components: vec![10, 11, 12], ..QuestDef::default() });
        t.parts.insert(10, ComponentDef { id: 10, quest: 20, kind: kind::PROGRESS, acts: vec![act("ObjMonsterHunt", 1, 2)], ..ComponentDef::default() });
        t.parts.insert(11, ComponentDef {
            id: 11,
            quest: 20,
            kind: kind::PROGRESS,
            acts: vec![act("ObjInteraction", 2, 1), act("CheckCompleteComponent", 10, 1)],
            ..ComponentDef::default()
        });
        t.parts.insert(12, ComponentDef { id: 12, quest: 20, kind: kind::PROGRESS, acts: vec![act("ObjTalk", 3, 1)], ..ComponentDef::default() });
        // The first act's count is the requirement; the slot holds the counter.
        assert!(!t.component_done(0, t.parts.get(&10).unwrap(), [1, 0, 0, 0, 0]));
        assert!(t.component_done(0, t.parts.get(&10).unwrap(), [2, 0, 0, 0, 0]));
        // The 0xc dependency must pass before the component's own first act is tested.
        assert!(!t.component_done(0, t.parts.get(&11).unwrap(), [1, 9, 0, 0, 0]));
        assert!(t.component_done(0, t.parts.get(&11).unwrap(), [2, 1, 0, 0, 0]));
        assert!(!t.component_done(0, t.parts.get(&12).unwrap(), [0, 0, 0, 0, 0]));
        assert!(t.component_done(0, t.parts.get(&12).unwrap(), [0, 0, 1, 0, 0]));
        // A component with no counted act is done and does not shift the index.
        t.parts.insert(13, ComponentDef { id: 13, quest: 20, kind: kind::PROGRESS, acts: vec![act("SupplyItem", 9, 1)], ..ComponentDef::default() });
        assert!(t.component_done(0, t.parts.get(&13).unwrap(), [0; 5]));
        // Score quests are never done.
        assert!(!t.component_done(100, t.parts.get(&10).unwrap(), [99, 99, 99, 99, 99]));
        // Only five kind-4 slots exist (FUN_397d4360).
        let mut six = QuestTables::default();
        six.defs.insert(21, QuestDef { components: (30..36).collect(), ..QuestDef::default() });
        for id in 30..36 {
            six.parts.insert(id, ComponentDef { id, quest: 21, kind: kind::PROGRESS, acts: vec![act("ObjTalk", 1, 1)], ..ComponentDef::default() });
        }
        assert!(six.component_done(0, six.parts.get(&34).unwrap(), [1; 5]));
        assert!(!six.component_done(0, six.parts.get(&35).unwrap(), [1; 5]));
    }

    #[test]
    fn gather_and_obtain_items_feed_the_relation_unfiltered() {
        let mut game = GameData::default();
        game.items.insert(1, super::super::data::ItemInfo { use_skill: 5, ..Default::default() });
        game.items.insert(2, super::super::data::ItemInfo { use_skill: 0, ..Default::default() });
        game.items.insert(3, super::super::data::ItemInfo { use_skill: 0, ..Default::default() });
        game.skills.insert(5, super::super::data::SkillInfo { products: vec![2], ..Default::default() });
        let act = |ty: &str, target, count| ActDef { ty: ty.into(), target, count, ..ActDef::default() };
        let mut t = QuestTables::default();
        t.defs.insert(30, QuestDef { components: vec![50, 51, 52], ..QuestDef::default() });
        t.parts.insert(50, ComponentDef { id: 50, quest: 30, kind: kind::PROGRESS, acts: vec![act("ObjItemUse", 3, 1)], ..ComponentDef::default() });
        t.parts.insert(51, ComponentDef { id: 51, quest: 30, kind: kind::PROGRESS, acts: vec![act("ObjItemGather", 2, 1)], ..ComponentDef::default() });
        t.parts.insert(52, ComponentDef { id: 52, quest: 30, kind: kind::SUPPLY, acts: vec![act("SupplyItem", 1, 1)], ..ComponentDef::default() });
        t.parts.get_mut(&52).unwrap().acts[0].show_action_bar = true;
        // Item 2 has no use skill, but the gather act still feeds FUN_394a2930's relation, so the
        // supply item that produces it is assigned to objective 2.
        assert_eq!(t.items_by_objective(&game, 30, None, 2), vec![1]);
    }

    #[test]
    fn check_complete_component_loads_its_target() {
        let Some(t) = tables() else { return };
        let found = t.parts.values().filter(|c| c.kind == kind::PROGRESS).find_map(|c| {
            let act = c.acts.iter().find(|a| a.ty == "CheckCompleteComponent" && a.target != 0)?;
            let target = t.parts.get(&act.target)?;
            if target.kind != kind::PROGRESS {
                return None;
            }
            let first = t.counted_act(target)?;
            let slot = t.slot_of_component(c.quest, act.target)?;
            (first.count > 0).then_some((c, act.target, slot, first.count))
        });
        let (component, target, slot, count) = found.expect("a real Progress CheckCompleteComponent with a Progress target");
        assert_ne!(target, 0, "quest_act_check_complete_components.complete_component");
        assert!(!t.dependency_met(component, [0; 5]), "the referenced component is not done");
        let mut objectives = [0; 5];
        objectives[slot] = count;
        assert!(t.dependency_met(component, objectives), "the referenced component is met");
    }

    #[test]
    fn counted_acts_include_the_fun_397d3370_types() {
        let mut game = GameData::default();
        game.items.insert(3, super::super::data::ItemInfo { use_skill: 5, ..Default::default() });
        game.skills.insert(5, super::super::data::SkillInfo::default());
        let act = |ty: &str, target, count| ActDef { ty: ty.into(), target, count, ..ActDef::default() };
        let mut t = QuestTables::default();
        t.defs.insert(31, QuestDef { components: vec![60, 61], ..QuestDef::default() });
        // ObjSendMail is one of the types FUN_397d3370 handles but the server's objective list lacks.
        t.parts.insert(60, ComponentDef { id: 60, quest: 31, kind: kind::PROGRESS, acts: vec![act("ObjSendMail", 1, 1)], ..ComponentDef::default() });
        t.parts.insert(61, ComponentDef { id: 61, quest: 31, kind: kind::PROGRESS, acts: vec![act("ObjItemUse", 3, 1)], ..ComponentDef::default() });
        assert!(t.counted_act(t.parts.get(&60).unwrap()).is_some());
        // The first component keeps index 1, so the use item is on objective 2.
        assert_eq!(t.items_by_objective(&game, 31, None, 2), vec![3]);
        assert!(t.items_by_objective(&game, 31, None, 1).is_empty());
        // ObjAggro is in FUN_397d6640's set and FUN_397d3370 never meets it: an aggro step is not
        // done and keeps objective 1.
        t.defs.insert(32, QuestDef { components: vec![62, 63], ..QuestDef::default() });
        t.parts.insert(62, ComponentDef { id: 62, quest: 32, kind: kind::PROGRESS, acts: vec![act("ObjAggro", 0, 1)], ..ComponentDef::default() });
        t.parts.insert(63, ComponentDef { id: 63, quest: 32, kind: kind::PROGRESS, acts: vec![act("ObjItemUse", 3, 1)], ..ComponentDef::default() });
        assert_eq!(t.items_by_objective(&game, 32, None, 2), vec![3]);
        assert!(t.items_by_objective(&game, 32, None, 1).is_empty());
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
