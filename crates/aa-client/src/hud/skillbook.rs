//! The skill book: what the skill window (K) lists and what the player may learn in it.
//!
//! The original keeps the ability skill trees in its own tables and the scripts ask natively: `X2Ability:GetAbilitySlotCount(kind)`
//! says how many slots of a kind a column has, the slots (`ISLOT_ABILITY_VIEW`, kind + 1 ..) hold a skill or a passive buff with
//! its state (`learned`, `canLearn`, `reqPoints`, `skillLevel`: the red / teal tints and the rank badge of `action_slot.alb`), and
//! the learn dialog's OK sends `CSLearnSkill` / `CSLearnBuff`. This file is that native side over the game database and aa-net's
//! state; `skillbook.lua` is the `X2Ability` layer the scripts call.
//!
//! * the trees: the 11 skills of an ability (`skills` with `ability_id`, shown, `need_learn`, by the ability level they need) and
//!   its 7 passive buffs (`passive_buffs`, by the skill points the tree must have spent). 15126 skills, but only 110 are in a tree;
//! * what is learned is `Skills::known` (ranks included) and `Skills::passive_buffs`; the points are the server's rule
//!   (`CharacterSkills.GetUsedSkillPoints`, `levels.skill_points`): every known skill costs its `skill_points`, a passive buff 1,
//!   and the player has the points of his level;
//! * `canLearn` follows the server's `learn_skill` / `learn_buff` plus the checks the server leaves to the client: the ability must
//!   be one of the three active ones, its level high enough, enough points left, and for a passive buff enough points spent in
//!   its tree. A learn that cannot work is not sent;
//! * the flat lists of the other tabs (attack, general and race skills, emotes) are `default_skills` by `skill_book_category_id`.

use std::collections::HashMap;

use aa_net::model::exp;
use aa_net::state::Skills;
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::HudBuilt;
use super::api::{HudBridge, HudData, Shared};
use super::data::{GameData, any_flag, any_int};

/// The scripts' `ABILITY_*` ids 1..=10; this is also the key of an ability in `locale.common.abilityCategory`.
pub const ABILITY_KEYS: [&str; 10] = ["fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love"];

/// `ABILITY_MAX`: no ability (an empty slot of the three).
pub const NO_ABILITY: u8 = 11;

/// An active skill of a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveSkill {
    pub id: u32,
    /// The ability level that unlocks it.
    pub level: i32,
    pub points: i32,
}

/// A passive buff of a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Passive {
    /// `passive_buffs.id`: what `CSLearnBuff` and `SCBuffLearned` name.
    pub id: u32,
    pub buff: u32,
    pub level: i32,
    /// Skill points the tree must have spent.
    pub req_points: i32,
}

/// The tables, read once from the game database.
#[derive(Default)]
pub struct Book {
    /// Per ability (index = id - 1), by the level they need.
    pub active: Vec<Vec<ActiveSkill>>,
    /// Per ability, by the points they need.
    pub passive: Vec<Vec<Passive>>,
    /// Every skill's ability and what learning it costs.
    pub cost: HashMap<u32, (u8, i32)>,
    /// The ability of every passive buff.
    pub passive_ability: HashMap<u32, u8>,
    /// Skill points of a character by level (`levels`, index = level).
    pub points: Vec<i32>,
    /// `default_skills` by skill book category: 2 attack, 3 general, 5 emotes (4 would be job and race skills).
    pub basic: HashMap<u8, Vec<u32>>,
}

/// `skill_book_category_id` of the flat lists, by the kind the scripts ask for (`ATTACK_SKILL` 100, `GENERAL_SKILL` 110,
/// `JOB_SKILL` 130, `EMOTION_SKILL` 140).
fn category_of_kind(kind: i64) -> Option<u8> {
    match kind {
        100 => Some(2),
        110 => Some(3),
        130 => Some(4),
        140 => Some(5),
        _ => None,
    }
}

impl Book {
    /// Skills shown in an ability tree must be learned before an action-bar shortcut can use them.
    pub fn requires_learning(&self, id: u32) -> bool {
        self.cost.get(&id)
            .and_then(|(ability, _)| ability.checked_sub(1))
            .and_then(|i| self.active.get(usize::from(i)))
            .is_some_and(|tree| tree.iter().any(|skill| skill.id == id))
    }

    pub fn load(conn: &Connection) -> rusqlite::Result<Book> {
        let mut book = Book { active: vec![Vec::new(); 10], passive: vec![Vec::new(); 10], ..Book::default() };
        {
            let mut q = conn.prepare("SELECT id, ability_id, ability_level, skill_points, SHOW, need_learn FROM skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                let ability = any_int(r, 1)?.clamp(0, 255) as u8;
                let points = any_int(r, 3)? as i32;
                book.cost.insert(id, (ability, points));
                if (1..=10).contains(&ability) && any_flag(r, 4)? && any_flag(r, 5)? {
                    book.active[usize::from(ability) - 1].push(ActiveSkill { id, level: any_int(r, 2)? as i32, points });
                }
            }
        }
        for list in &mut book.active {
            list.sort_by_key(|s| (s.level, s.id));
        }
        {
            let mut q = conn.prepare("SELECT id, ability_id, LEVEL, buff_id, req_points FROM passive_buffs ORDER BY req_points, id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                let ability = any_int(r, 1)?.clamp(0, 255) as u8;
                book.passive_ability.insert(id, ability);
                if (1..=10).contains(&ability) {
                    book.passive[usize::from(ability) - 1].push(Passive {
                        id,
                        buff: any_int(r, 3)? as u32,
                        level: any_int(r, 2)? as i32,
                        req_points: any_int(r, 4)? as i32,
                    });
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT id, skill_points FROM levels")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let level = any_int(r, 0)?.clamp(0, 255) as usize;
                if level < 200 {
                    if book.points.len() <= level {
                        book.points.resize(level + 1, 0);
                    }
                    book.points[level] = any_int(r, 1)? as i32;
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT skill_id, skill_book_category_id FROM default_skills ORDER BY skill_book_category_id, id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                book.basic.entry(any_int(r, 1)? as u8).or_default().push(any_int(r, 0)? as u32);
            }
        }
        Ok(book)
    }

    /// The skill points a character of `level` has to spend.
    pub fn points_at(&self, level: u8) -> i32 {
        self.points.get(usize::from(level)).copied().unwrap_or(0)
    }
}

/// What the player has: the learned skills (rank), the learned passive buffs, the level and the abilities.
pub struct Have<'a> {
    pub known: &'a HashMap<u32, u8>,
    pub passive: &'a [u32],
    pub level: u8,
    /// The active abilities (up to three).
    pub active: &'a [u8],
    pub ability_exp: &'a [i32; 11],
}

impl Have<'_> {
    pub fn ability_level(&self, ability: u8) -> i32 {
        if self.active.contains(&ability) || self.ability_exp[usize::from(ability.min(10))] > 0 {
            i32::from(exp::level_for_total(i64::from(self.ability_exp[usize::from(ability.min(10))])))
        } else {
            0
        }
    }
}

/// One row of a tree for the scripts.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u32,
    pub learned: bool,
    /// The rank of a learned skill.
    pub rank: u8,
    /// Whether the player could learn it now.
    pub can: bool,
}

impl Book {
    /// `GetUsedSkillPoints(ability)`: `None` counts every ability's.
    pub fn used(&self, have: &Have, ability: Option<u8>) -> i32 {
        let mut points = 0;
        for id in have.known.keys() {
            if let Some(&(a, p)) = self.cost.get(id)
                && ability.is_none_or(|x| x == a)
            {
                points += p;
            }
        }
        for id in have.passive {
            if ability.is_none_or(|x| x == self.passive_ability.get(id).copied().unwrap_or(0)) {
                points += 1;
            }
        }
        points
    }

    /// The active skills of a tree with what the player can do with them.
    pub fn active_rows(&self, have: &Have, ability: u8) -> Vec<Row> {
        let Some(list) = usize::from(ability).checked_sub(1).and_then(|i| self.active.get(i)) else { return Vec::new() };
        let free = self.points_at(have.level) - self.used(have, None);
        let level = have.ability_level(ability);
        let active = have.active.contains(&ability);
        list.iter()
            .map(|s| {
                let rank = have.known.get(&s.id).copied();
                Row { id: s.id, learned: rank.is_some(), rank: rank.unwrap_or(0), can: active && rank.is_none() && level >= s.level && free >= s.points }
            })
            .collect()
    }

    /// The passive buffs of a tree.
    pub fn passive_rows(&self, have: &Have, ability: u8) -> Vec<Row> {
        let Some(list) = usize::from(ability).checked_sub(1).and_then(|i| self.passive.get(i)) else { return Vec::new() };
        let free = self.points_at(have.level) - self.used(have, None);
        let spent = self.used(have, Some(ability));
        let level = have.ability_level(ability);
        let active = have.active.contains(&ability);
        list.iter()
            .map(|p| {
                let learned = have.passive.contains(&p.id);
                Row { id: p.id, learned, rank: u8::from(learned), can: active && !learned && level >= p.level && spent >= p.req_points && free >= 1 }
            })
            .collect()
    }
}

/// The active abilities and their experience from the snapshot, with what aa-net has learned.
fn have<'a>(data: &'a HudData) -> Have<'a> {
    Have {
        known: &data.skill_levels,
        passive: &data.passive_buffs,
        level: data.snapshot.level.max(1),
        active: &data.snapshot.abilities,
        ability_exp: &data.snapshot.ability_exp,
    }
}

/// A number that changes whenever something the skill window shows does (the scripts cache the trees by it).
pub fn version(data: &HudData) -> i64 {
    let mut sum: u64 = 0;
    for (id, rank) in &data.skill_levels {
        sum = sum.wrapping_add((u64::from(*id) << 8 | u64::from(*rank)).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    for id in &data.passive_buffs {
        sum = sum.wrapping_add((u64::from(*id) | 1 << 40).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    }
    sum = sum.wrapping_add(u64::from(data.snapshot.level) << 20);
    for (i, e) in data.snapshot.ability_exp.iter().enumerate() {
        // the level an experience total is, not the total (the window shows no experience)
        sum = sum.wrapping_add((i as u64 + 1).wrapping_mul(u64::from(exp::level_for_total(i64::from(*e)))) << 12);
    }
    for a in &data.snapshot.abilities {
        sum = sum.wrapping_add(u64::from(*a) << 30);
    }
    (sum >> 1) as i64
}

fn ability_key(ability: u8) -> &'static str {
    ability.checked_sub(1).and_then(|i| ABILITY_KEYS.get(usize::from(i))).copied().unwrap_or("fight")
}

fn named(game: Option<&GameData>, skill: u32) -> (String, String) {
    game.and_then(|g| g.skills.get(&skill)).map_or_else(Default::default, |s| (s.named.name.clone(), s.named.icon.clone()))
}

/// `HUD.book_tree(ability)`: `{ ability, level, used, active = { { id, name, icon, level, points, learned, rank, can } },
/// passive = { { id, buff, name, icon, desc, level, req, learned, can } } }`.
fn tree_table(lua: &Lua, data: &HudData, ability: u8) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else { return Ok(None) };
    let book = &game.book;
    let have = have(data);
    let t = lua.create_table()?;
    t.raw_set("ability", i64::from(ability))?;
    t.raw_set("level", i64::from(have.ability_level(ability)))?;
    t.raw_set("used", i64::from(book.used(&have, Some(ability))))?;
    let (Some(active_specs), Some(passive_specs)) = (book.active.get(usize::from(ability) - 1), book.passive.get(usize::from(ability) - 1)) else {
        return Ok(None);
    };
    let rows = book.active_rows(&have, ability);
    let active = lua.create_table()?;
    for (row, spec) in rows.iter().zip(active_specs) {
        let (name, icon) = named(Some(game), row.id);
        let r = lua.create_table()?;
        r.raw_set("id", i64::from(row.id))?;
        r.raw_set("name", name)?;
        r.raw_set("icon", icon)?;
        r.raw_set("level", i64::from(spec.level))?;
        r.raw_set("points", i64::from(spec.points))?;
        r.raw_set("learned", row.learned)?;
        r.raw_set("rank", i64::from(row.rank))?;
        r.raw_set("can", row.can)?;
        active.raw_push(r)?;
    }
    t.raw_set("active", active)?;
    let rows = book.passive_rows(&have, ability);
    let passive = lua.create_table()?;
    for (row, spec) in rows.iter().zip(passive_specs) {
        let buff = game.buffs.get(&spec.buff);
        let r = lua.create_table()?;
        r.raw_set("id", i64::from(row.id))?;
        r.raw_set("buff", i64::from(spec.buff))?;
        r.raw_set("name", buff.map(|b| b.named.name.clone()).unwrap_or_default())?;
        r.raw_set("icon", buff.map(|b| b.named.icon.clone()).unwrap_or_default())?;
        r.raw_set("desc", buff.map(|b| b.desc.clone()).unwrap_or_default())?;
        r.raw_set("level", i64::from(spec.level))?;
        r.raw_set("req", i64::from(spec.req_points))?;
        r.raw_set("learned", row.learned)?;
        r.raw_set("can", row.can)?;
        passive.raw_push(r)?;
    }
    t.raw_set("passive", passive)?;
    Ok(Some(t))
}

const BOOK_LUA: &str = include_str!("skillbook.lua");

/// The `X2Ability` layer of the skill window, over the `HUD.book_*` functions.
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(BOOK_LUA).set_name("@skillbook.lua").exec()
}

/// The functions of the scripts' `HUD` table.
pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set("book_version", lua.create_function(move |_, ()| Ok(version(&b.borrow())))?)?;
    let b = bridge.clone();
    hud.raw_set("book_learned", lua.create_function(move |_, id: u32| {
        let data = b.borrow();
        Ok(data.game.get().is_none_or(|game| !game.book.requires_learning(id) || data.skill_levels.contains_key(&id)))
    })?)?;
    let b = bridge.clone();
    hud.raw_set(
        "book_tree",
        lua.create_function(move |lua, ability: i64| {
            if !(1..=10).contains(&ability) {
                return Ok(None);
            }
            tree_table(lua, &b.borrow(), ability as u8)
        })?,
    )?;
    // (total skill points, spent): of every ability, or of the tree of the ability given
    let b = bridge.clone();
    hud.raw_set(
        "book_points",
        lua.create_function(move |_, ability: Option<i64>| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok((0, 0)) };
            let have = have(&d);
            let tree = ability.filter(|a| (1..=10).contains(a)).map(|a| a as u8);
            Ok((i64::from(game.book.points_at(have.level)), i64::from(game.book.used(&have, tree))))
        })?,
    )?;
    // the flat lists (attack, general, race skills, emotes) as { id, name, icon }: these are built into the window when it is
    // made, so wait for the database if it is still being read
    let b = bridge.clone();
    hud.raw_set(
        "book_basic",
        lua.create_function(move |lua, kind: i64| {
            let game = b.borrow().game.clone();
            let game = game.0.wait();
            let list = lua.create_table()?;
            let Some(category) = category_of_kind(kind) else { return Ok(list) };
            let mut ids: Vec<u32> = game.book.basic.get(&category).cloned().unwrap_or_default();
            if category == 4 {
                // job and race skills: what the character knows that no tree, hotbar default or emote covers
                let d = b.borrow();
                ids = job_skills(game, &d.skill_levels);
            }
            for id in ids {
                let (name, icon) = named(Some(game), id);
                let r = lua.create_table()?;
                r.raw_set("id", i64::from(id))?;
                r.raw_set("name", name)?;
                r.raw_set("icon", icon)?;
                list.raw_push(r)?;
            }
            Ok(list)
        })?,
    )?;
    // a buff's name, icon and description (the passive buffs' tooltips and the learn dialog)
    let b = bridge.clone();
    hud.raw_set(
        "book_buff",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let buff = d.game.get().and_then(|g| g.buffs.get(&id));
            Ok((buff.map(|b| b.named.name.clone()), buff.map(|b| b.named.icon.clone()), buff.map(|b| b.desc.clone())))
        })?,
    )?;
    // what the learn and reset dialogs name
    let b = bridge.clone();
    hud.raw_set(
        "book_skill",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok((None, None, 1)) };
            let (ability, points) = game.book.cost.get(&id).copied().unwrap_or((0, 1));
            Ok((game.skills.get(&id).map(|s| s.named.name.clone()), Some(ability_key(ability)), i64::from(points)))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "book_passive",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let Some(game) = d.game.get() else { return Ok((None, None)) };
            let ability = game.book.passive_ability.get(&id).copied().unwrap_or(0);
            let buff = game.book.passive.get(usize::from(ability.clamp(1, 10)) - 1).and_then(|l| l.iter().find(|p| p.id == id)).map(|p| i64::from(p.buff));
            Ok((buff, Some(ability_key(ability))))
        })?,
    )?;
    Ok(())
}

/// The skills the character knows that belong to no ability tree: the race and job skills the server gave it (fishing, the glider ...).
fn job_skills(game: &GameData, known: &HashMap<u32, u8>) -> Vec<u32> {
    let mut ids: Vec<u32> = known
        .keys()
        .copied()
        .filter(|id| {
            game.book.cost.get(id).is_some_and(|(a, _)| *a == 0)
                && game.skills.get(id).is_some_and(|s| !s.named.icon.is_empty())
                && !game.book.basic.values().any(|l| l.contains(id))
        })
        .collect();
    ids.sort_unstable();
    ids
}

/// A learn the scripts asked for: checked against the rules, then sent.
pub fn learn_skill(data: &HudData, id: u32, net: &mut MessageWriter<NetCommand>) {
    let Some(game) = data.game.get() else { return };
    let have = have(data);
    let Some(&(ability, _)) = game.book.cost.get(&id) else { return };
    let ok = game.book.active_rows(&have, ability).iter().any(|r| r.id == id && r.can);
    info!("hud: learn skill {id} ({}): {}", ability_key(ability), if ok { "sent" } else { "refused: not learnable now" });
    if ok {
        net.write(NetCommand(Command::LearnSkill { skill_id: id }));
    }
}

pub fn learn_buff(data: &HudData, id: u32, net: &mut MessageWriter<NetCommand>) {
    let Some(game) = data.game.get() else { return };
    let have = have(data);
    let ability = game.book.passive_ability.get(&id).copied().unwrap_or(0);
    let ok = game.book.passive_rows(&have, ability).iter().any(|r| r.id == id && r.can);
    info!("hud: learn passive buff {id} ({}): {}", ability_key(ability), if ok { "sent" } else { "refused: not learnable now" });
    if ok {
        net.write(NetCommand(Command::LearnBuff { passive_id: id }));
    }
}

/// aa-net has no ability membership database. Remove reset skills before the HUD takes its next snapshot.
pub fn apply_resets(mut events: MessageReader<NetEvent>, bridge: NonSend<HudBridge>, mut skills: ResMut<Skills>) {
    let data = bridge.0.borrow();
    for NetEvent(event) in events.read() {
        if let Some(game) = data.game.get() {
            apply_book_event(&game.book, &mut skills, event);
        }
    }
}

fn apply_book_event(book: &Book, skills: &mut Skills, event: &Event) {
    if skills.obj_id == 0 {
        return;
    }
    match event {
        Event::SkillsReset { obj_id, ability } if *obj_id == skills.obj_id => {
            skills.known.retain(|k| book.cost.get(&k.id).is_none_or(|c| c.0 != *ability));
            skills.passive_buffs.retain(|id| book.passive_ability.get(id).is_none_or(|a| a != ability));
        }
        // NetPlugin has already folded this frame's packets. Replay learns in order with resets so
        // a reset followed by learning in the same frame keeps the newly learned skill/passive.
        Event::SkillLearned { .. } | Event::SkillUpgraded { .. } | Event::PassiveBuffLearned { .. } => { skills.apply(event); }
        _ => {}
    }
}

/// The skill window's events from what the server says: learned (`SKILL_LEARNED(name)`), upgraded and reset.
pub fn route_book(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    built: Res<HudBuilt>,
    bridge: NonSend<HudBridge>,
    skills: Res<Skills>,
) {
    let Some(rt) = rt.filter(|_| built.up) else {
        events.clear();
        return;
    };
    let own = skills.obj_id;
    // what to tell the scripts, worked out with the data borrowed and sent without it (their handlers may ask for more)
    enum Fire {
        Learned(String),
        Changed(String, i64, &'static str, i64),
        Reset(&'static str),
    }
    let mut fire = Vec::new();
    {
        let data = bridge.0.borrow();
        let game = data.game.get();
        for NetEvent(event) in events.read() {
            match event {
                Event::SkillLearned { skill_id, level } => {
                    let (name, _) = named(game, *skill_id);
                    info!("hud: learned skill {skill_id} {name:?} (rank {level})");
                    fire.push(Fire::Learned(name));
                }
                Event::SkillUpgraded { skill_id, level } => {
                    let (name, _) = named(game, *skill_id);
                    let ability = game.and_then(|g| g.book.cost.get(skill_id)).map_or(0, |c| c.0);
                    info!("hud: skill {skill_id} {name:?} is rank {level} now");
                    fire.push(Fire::Changed(name, i64::from(*level), ability_key(ability), i64::from(*skill_id)));
                }
                Event::PassiveBuffLearned { obj_id, buff_id } if own != 0 && *obj_id == own => {
                    let buff = game.and_then(|g| g.book.passive.iter().flatten().find(|p| p.id == *buff_id).and_then(|p| g.buffs.get(&p.buff)));
                    let name = buff.map(|b| b.named.name.clone()).unwrap_or_default();
                    info!("hud: learned passive buff {buff_id} {name:?}");
                    fire.push(Fire::Learned(name));
                }
                Event::SkillsReset { obj_id, ability } if own != 0 && *obj_id == own => {
                    info!("hud: the skills of ability {ability} were reset");
                    fire.push(Fire::Reset(ability_key(*ability)));
                }
                _ => {}
            }
        }
    }
    for f in fire {
        match f {
            Fire::Learned(name) => rt.fire_event("SKILL_LEARNED", name),
            Fire::Changed(name, level, ability, id) => {
                rt.fire_event("SKILL_CHANGED", (name, level, ability));
                rt.fire_event("SKILL_UPGRADED", id);
            }
            Fire::Reset(ability) => rt.fire_event("SKILLS_RESET", ability),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        let mut b = Book { active: vec![Vec::new(); 10], passive: vec![Vec::new(); 10], ..Book::default() };
        // Battlerage: Triple Slash from level 1, Charge from level 3, one passive buff that wants 2 points spent
        b.active[0] = vec![ActiveSkill { id: 18132, level: 1, points: 1 }, ActiveSkill { id: 11918, level: 3, points: 1 }];
        b.passive[0] = vec![Passive { id: 3, buff: 481, level: 1, req_points: 2 }];
        for (id, a) in [(18132, 1), (11918, 1)] {
            b.cost.insert(id, (a, 1));
        }
        b.cost.insert(2, (0, 1));
        b.passive_ability.insert(3, 1);
        b.points = vec![0, 1, 1, 2, 2, 4];
        b
    }

    fn hv<'a>(known: &'a HashMap<u32, u8>, passive: &'a [u32], level: u8, exp: &'a [i32; 11], active: &'a [u8]) -> Have<'a> {
        Have { known, passive, level, active, ability_exp: exp }
    }

    #[test]
    fn points_and_learnable_skills_follow_the_servers_rules() {
        let b = book();
        let mut exp = [0; 11];
        exp[1] = 2_000; // Battlerage level 3
        let mut known = HashMap::new();
        known.insert(18132, 1u8);
        // a level 1 character has its one point in Triple Slash: nothing to spend
        let h = hv(&known, &[], 1, &exp, &[1]);
        assert_eq!((b.points_at(1), b.used(&h, None)), (1, 1));
        assert!(b.active_rows(&h, 1).iter().all(|r| !r.can));
        assert!(b.active_rows(&h, 1)[0].learned);
        // level 3: two points, one spent, Charge (ability level 3) can be learned
        let h = hv(&known, &[], 3, &exp, &[1]);
        let rows = b.active_rows(&h, 1);
        assert_eq!((rows[0].learned, rows[1].can), (true, true));
        // a skill of an ability that is not active cannot be learned (the preview of another ability)
        let h = hv(&known, &[], 3, &exp, &[6]);
        assert!(b.active_rows(&h, 1).iter().all(|r| !r.can));
        // too low an ability level: the point is there, the level is not
        exp[1] = 0;
        let h = hv(&known, &[], 3, &exp, &[1]);
        assert!(!b.active_rows(&h, 1)[1].can);
    }

    #[test]
    fn a_passive_buff_wants_the_points_of_its_tree() {
        let b = book();
        let mut exp = [0; 11];
        exp[1] = 2_000;
        let mut known = HashMap::new();
        known.insert(18132, 1u8);
        // level 5: 4 points, 1 spent in the tree: the buff wants 2 spent
        let h = hv(&known, &[], 5, &exp, &[1]);
        assert!(!b.passive_rows(&h, 1)[0].can);
        known.insert(11918, 1);
        let h = hv(&known, &[], 5, &exp, &[1]);
        assert_eq!(b.used(&h, Some(1)), 2);
        assert!(b.passive_rows(&h, 1)[0].can);
        // learned: it costs a point and cannot be learned again; the version moves
        let h = hv(&known, &[3], 5, &exp, &[1]);
        assert_eq!(b.used(&h, None), 3);
        assert!(b.passive_rows(&h, 1)[0].learned && !b.passive_rows(&h, 1)[0].can);
        // other abilities' points are not this tree's
        assert_eq!(b.used(&h, Some(6)), 0);
    }

    #[test]
    fn reset_and_relearn_in_one_frame_keep_the_new_skills() {
        let b = book();
        let mut skills = Skills { obj_id: 5, ..Skills::default() };
        // This is already the state NetPlugin produced after receiving the entire batch.
        skills.apply(&Event::SkillLearned { skill_id: 18132, level: 1 });
        skills.apply(&Event::SkillLearned { skill_id: 11918, level: 1 });
        skills.apply(&Event::SkillLearned { skill_id: 2, level: 1 });
        skills.apply(&Event::PassiveBuffLearned { obj_id: 5, buff_id: 3 });
        apply_book_event(&b, &mut skills, &Event::SkillsReset { obj_id: 6, ability: 1 });
        assert!(skills.knows(11918));
        apply_book_event(&b, &mut skills, &Event::SkillsReset { obj_id: 5, ability: 1 });
        apply_book_event(&b, &mut skills, &Event::SkillLearned { skill_id: 18132, level: 2 });
        apply_book_event(&b, &mut skills, &Event::PassiveBuffLearned { obj_id: 5, buff_id: 3 });
        assert!(skills.knows(2) && skills.knows(18132) && !skills.knows(11918));
        assert_eq!(skills.known.iter().find(|s| s.id == 18132).unwrap().level, 2);
        assert_eq!(skills.passive_buffs, [3]);
    }

    /// The real database: every ability has its tree (11 skills at most 15 slots, 7 passive buffs at most 8), the level 1
    /// character has the one point Triple Slash costs.
    #[test]
    fn real_trees() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(conn) = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
        let b = Book::load(&conn).unwrap();
        for a in 0..10 {
            assert!((10..=11).contains(&b.active[a].len()), "ability {} has {} skills", a + 1, b.active[a].len());
            assert_eq!(b.passive[a].len(), 7);
        }
        assert_eq!(b.active[0][0], ActiveSkill { id: 18132, level: 1, points: 1 });
        assert_eq!((b.points_at(1), b.points_at(10)), (1, 7));
        assert_eq!(b.basic[&2].len(), 6);
        assert_eq!(b.basic[&5].len(), 46);
    }
}
