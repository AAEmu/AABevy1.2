//! The lists the scripts read besides the player's numbers: the action bars, the bag, the buffs on
//! the player and the target, and what the player targets. [`Panels`] is computed from aa-net's
//! resources every frame, compared with what the Lua state has seen, and pushed as the tables
//! `HUD.actions`, `HUD.bag`, `HUD.buffs`, `HUD.target` only when it changed (`ingame.lua` answers
//! `AAUI.SlotContent`, `X2Unit:UnitBuff`, `X2Unit:UnitName("target")` ... from them).
//!
//! Times that count down on their own (buffs, cooldowns) are pushed as absolute end times in the
//! HUD's clock (`HUD.now_ms()`: milliseconds since the plugin started), so a running countdown is not
//! a change.

use std::time::Instant;

use aa_net::buff::BuffKind;
use aa_net::items::ActionKind;
use aa_net::model::UnitKind;
use aa_net::state::{ActionBar, Buffs, Inventory, Skills, Target, Units};
use aa_net::{Command, NetCommand};
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::HudKeyboardFocus;
use super::api::HudBridge;
use super::data::GameData;

/// One action bar slot (or bag slot).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotSnap {
    /// `"skill"` or `"item"`.
    pub kind: &'static str,
    /// Skill id, or item template id.
    pub id: u32,
    pub name: String,
    pub icon: String,
    /// Items: how many (the bag total for a template slot).
    pub count: i64,
    /// Whether the count is shown on the icon (stackable items).
    pub stack: bool,
    pub grade: u8,
    /// Skills: when the cooldown ends (HUD clock, ms) and how long it is.
    pub cooldown: Option<(u64, u32)>,
    /// Items: description and level requirement (tooltips).
    pub desc: String,
    pub level: i64,
    /// Items: `item_groups.id` (bag tabs).
    pub group: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuffSnap {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub desc: String,
    pub stacks: i32,
    /// When it ends (HUD clock, ms); `None` for a permanent buff.
    pub ends_ms: Option<u64>,
}

/// The buffs of a unit, good and bad.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuffLists {
    pub good: Vec<BuffSnap>,
    pub bad: Vec<BuffSnap>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetSnap {
    pub obj_id: u32,
    pub name: String,
    pub level: u8,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,
    /// `GetUnitTypeString`: `"character"` / `"npc"`, and the detail (`"slave"`, `"housing"`, ...).
    pub unit_type: &'static str,
    pub detail: &'static str,
    pub faction_name: String,
    /// `"friendly"`, `"neutral"` or `"hostile"` towards the player.
    pub relation: &'static str,
    pub grade: &'static str,
    pub race: u8,
    pub gender: u8,
    pub buffs: BuffLists,
}

/// Everything in this module, as of this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panels {
    /// Action bar by slot index (0 based); `None` = empty. Empty when the server has not sent the
    /// bars yet.
    pub actions: Vec<Option<SlotSnap>>,
    pub actions_known: bool,
    pub bag: Vec<Option<SlotSnap>>,
    pub bag_slots: u32,
    /// Worn items by aa-net slot index (`model::slot`, the scripts' `ES_*` minus one); body parts
    /// are not items to the window.
    pub equipment: Vec<Option<SlotSnap>>,
    pub buffs: BuffLists,
    pub target: Option<TargetSnap>,
}

/// Milliseconds on the HUD clock.
pub fn now_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn item_slot(game: Option<&GameData>, template: u32, count: i64, grade: u8) -> SlotSnap {
    let info = game.and_then(|g| g.items.get(&template));
    SlotSnap {
        kind: "item",
        id: template,
        name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
        icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
        count,
        stack: info.is_some_and(|i| i.max_stack > 1),
        grade: if grade > 0 { grade } else { info.map_or(0, |i| i.fixed_grade.clamp(0, 12) as u8) },
        cooldown: None,
        desc: info.map(|i| i.desc.clone()).unwrap_or_default(),
        level: info.map_or(0, |i| i.level),
        group: info.map_or(0, |i| i.group),
    }
}

fn skill_slot(game: Option<&GameData>, skills: Option<&Skills>, skill: u32, start: Instant) -> SlotSnap {
    let info = game.and_then(|g| g.skills.get(&skill));
    // the skill's own cooldown, or the global cooldown the last cast started (every skill that does not
    // ignore it waits for it), whichever ends later; quantized to a quarter second: the end time of a
    // running cooldown must not jitter
    let cooldown = info.and_then(|i| {
        let skills = skills?;
        let own = skills.cooldown_left(skill, i.tag, i.cooldown_ms).map(|left| (left, i.cooldown_ms));
        let global = (!i.ignore_gcd)
            .then(|| {
                let len = game?.skills.get(&skills.last_cast_skill)?.gcd_ms;
                skills.gcd_left(len).map(|left| (left, len))
            })
            .flatten();
        let (left, total) = match (own, global) {
            (Some(a), Some(b)) => if b.0 > a.0 { b } else { a },
            (a, b) => a.or(b)?,
        };
        let end = now_ms(start) + left.as_millis() as u64;
        Some(((end + 125) / 250 * 250, total))
    });
    SlotSnap {
        kind: "skill",
        id: skill,
        name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
        icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
        count: 0,
        stack: false,
        grade: 0,
        cooldown,
        desc: String::new(),
        level: 0,
        group: 0,
    }
}

fn buff_lists(game: Option<&GameData>, entries: &[aa_net::state::BuffEntry], start: Instant) -> BuffLists {
    let mut lists = BuffLists::default();
    for e in entries {
        let b = &e.buff;
        let info = game.and_then(|g| g.buffs.get(&b.buff_id));
        let good = match b.kind {
            BuffKind::Good => true,
            BuffKind::Bad => false,
            BuffKind::Hidden => continue,
            BuffKind::Unknown => match info.map(|i| i.kind) {
                Some(2) => false,
                Some(3) => continue,
                _ => true,
            },
        };
        let ends_ms = (b.duration_ms > 0).then(|| {
            let received = e.received.saturating_duration_since(start).as_millis() as u64;
            received + (i64::from(b.duration_ms) - i64::from(b.elapsed_ms)).max(0) as u64
        });
        let snap = BuffSnap {
            id: b.buff_id,
            name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
            icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
            desc: info.map(|i| i.desc.clone()).unwrap_or_default(),
            stacks: b.stacks.max(1),
            ends_ms,
        };
        if good { lists.good.push(snap) } else { lists.bad.push(snap) }
    }
    lists
}

fn unit_race_gender(race_gender: u8) -> (u8, u8) {
    (race_gender & 0x0f, race_gender >> 4)
}

/// Compute the panels from aa-net's state. `own_faction` is the player's faction id; also returns
/// the game position of the target.
pub fn refresh(world: &World, game: Option<&GameData>, own_faction: u32, own_obj_id: u32, start: Instant) -> (Panels, Option<Vec3>) {
    let mut p = Panels::default();
    let skills = world.get_resource::<Skills>();
    let inventory = world.get_resource::<Inventory>();
    if let Some(bar) = world.get_resource::<ActionBar>() {
        p.actions_known = !bar.slots.is_empty();
        p.actions = bar
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let slot = *slot;
                if slot.is_empty() {
                    return None;
                }
                match slot.kind {
                    ActionKind::Spell | ActionKind::RidePetSpell => Some(skill_slot(game, skills, slot.id as u32, start)),
                    ActionKind::ItemType | ActionKind::ItemId => {
                        let inv = inventory?;
                        let (template, grade) = match slot.item_template_id() {
                            Some(t) => (t, inv.find_template(t).map_or(0, |(_, it)| it.grade)),
                            None => {
                                let (_, _, it) = inv.find_by_id(slot.id)?;
                                (it.template_id, it.grade)
                            }
                        };
                        Some(item_slot(game, template, bar.item_count(i, inv).unwrap_or(0), grade))
                    }
                    _ => None,
                }
            })
            .collect();
    }
    if let Some(inv) = inventory {
        p.bag_slots = inv.bag_slots;
        p.bag = inv.bag.iter().map(|it| it.as_ref().map(|it| item_slot(game, it.template_id, i64::from(it.count), it.grade))).collect();
        p.equipment = inv
            .equipment
            .slots
            .iter()
            .enumerate()
            .map(|(i, it)| it.as_ref().filter(|_| !aa_net::model::slot::is_body_part(i)).map(|it| item_slot(game, it.template_id, 1, it.grade)))
            .collect();
    }
    if let Some(buffs) = world.get_resource::<Buffs>() {
        p.buffs = buff_lists(game, buffs.of(own_obj_id), start);
    }
    let mut target_pos = None;
    if let (Some(target), Some(units)) = (world.get_resource::<Target>(), world.get_resource::<Units>())
        && let Some(u) = target.unit(units)
    {
        let template = u.kind.template_id();
        let npc = game.and_then(|g| g.npcs.get(&template)).filter(|_| template != 0);
        let faction = if u.faction_id != 0 { u.faction_id } else { npc.map_or(0, |n| n.faction_id as u32) };
        let (race, gender) = unit_race_gender(u.race_gender);
        let (unit_type, detail) = match u.kind {
            UnitKind::Character { .. } => ("character", "character"),
            UnitKind::Npc { .. } => ("npc", "npc"),
            UnitKind::Slave { .. } => ("npc", "slave"),
            UnitKind::Housing { .. } => ("npc", "housing"),
            UnitKind::Transfer { .. } => ("npc", "transfer"),
            UnitKind::Mate { .. } => ("npc", "mate"),
            UnitKind::Shipyard { .. } => ("npc", "shipyard"),
        };
        let name = if !u.name.is_empty() { u.name.clone() } else { npc.map(|n| n.name.clone()).unwrap_or_default() };
        // the maxima come from the database's formulas; the highest seen is the fallback
        let unit_buffs = world.get_resource::<Buffs>().map_or(&[][..], |b| b.of(u.obj_id));
        let (max_hp, max_mp) = game.and_then(|g| super::stats::unit_max(g, u, unit_buffs)).unwrap_or((u.max_hp, u.max_mp));
        p.target = Some(TargetSnap {
            obj_id: u.obj_id,
            name,
            level: u.level.max(1),
            hp: u.hp,
            max_hp: max_hp.max(u.hp).max(1),
            mp: u.mp,
            max_mp: max_mp.max(u.mp),
            unit_type,
            detail,
            faction_name: game.and_then(|g| g.factions.get(&faction).cloned()).unwrap_or_default(),
            relation: game.map_or("neutral", |g| g.relation(own_faction, faction)),
            grade: GameData::grade_name(npc.map_or(1, |n| n.grade)),
            race,
            gender,
            buffs: world.get_resource::<Buffs>().map(|b| buff_lists(game, b.of(u.obj_id), start)).unwrap_or_default(),
        });
        target_pos = Some(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
    }
    (p, target_pos)
}

fn slot_table(lua: &Lua, s: &SlotSnap) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("kind", s.kind)?;
    t.raw_set("id", i64::from(s.id))?;
    t.raw_set("name", s.name.as_str())?;
    t.raw_set("icon", s.icon.as_str())?;
    t.raw_set("count", s.count)?;
    t.raw_set("stack", s.stack)?;
    t.raw_set("grade", i64::from(s.grade))?;
    t.raw_set("desc", s.desc.as_str())?;
    t.raw_set("level", s.level)?;
    t.raw_set("group", i64::from(s.group))?;
    if let Some((end, total)) = s.cooldown {
        t.raw_set("cd_end", end as f64)?;
        t.raw_set("cd_total", i64::from(total))?;
    }
    Ok(t)
}

fn buff_table(lua: &Lua, b: &BuffSnap) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("buff_id", i64::from(b.id))?;
    t.raw_set("name", b.name.as_str())?;
    t.raw_set("path", b.icon.as_str())?;
    t.raw_set("description", b.desc.as_str())?;
    t.raw_set("stack", i64::from(b.stacks))?;
    if let Some(end) = b.ends_ms {
        t.raw_set("ends", end as f64)?;
    }
    Ok(t)
}

fn buffs_table(lua: &Lua, l: &BuffLists) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("good", lua.create_sequence_from(l.good.iter().map(|b| buff_table(lua, b)).collect::<mlua::Result<Vec<_>>>()?)?)?;
    t.raw_set("bad", lua.create_sequence_from(l.bad.iter().map(|b| buff_table(lua, b)).collect::<mlua::Result<Vec<_>>>()?)?)?;
    Ok(t)
}

fn slots_table(lua: &Lua, slots: &[Option<SlotSnap>]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, s) in slots.iter().enumerate() {
        if let Some(s) = s {
            t.raw_set(i as i64 + 1, slot_table(lua, s)?)?;
        }
    }
    Ok(t)
}

/// Put the changed parts of `new` into the `HUD` table (all of it when `old` is `None`); returns
/// what changed.
pub fn push(lua: &Lua, hud: &Table, new: &Panels, old: Option<&Panels>) -> mlua::Result<Changed> {
    let mut c = Changed::default();
    if old.is_none_or(|o| o.actions != new.actions || o.actions_known != new.actions_known) {
        hud.raw_set("actions", slots_table(lua, &new.actions)?)?;
        hud.raw_set("actions_known", new.actions_known)?;
        c.slots = true;
    }
    if old.is_none_or(|o| o.bag != new.bag || o.bag_slots != new.bag_slots) {
        hud.raw_set("bag", slots_table(lua, &new.bag)?)?;
        hud.raw_set("bag_slots", i64::from(new.bag_slots))?;
        c.slots = true;
        c.bag = true;
    }
    if old.is_none_or(|o| o.equipment != new.equipment) {
        hud.raw_set("equipment", slots_table(lua, &new.equipment)?)?;
        c.equipment = true;
    }
    if old.is_none_or(|o| o.buffs != new.buffs) {
        hud.raw_set("buffs", buffs_table(lua, &new.buffs)?)?;
        c.buffs = true;
    }
    if old.is_none_or(|o| o.target != new.target) {
        match &new.target {
            None => hud.raw_set("target", mlua::Value::Nil)?,
            Some(t) => {
                let x = lua.create_table()?;
                x.raw_set("unit_id", t.obj_id.to_string())?;
                x.raw_set("name", t.name.as_str())?;
                x.raw_set("level", i64::from(t.level))?;
                x.raw_set("hp", i64::from(t.hp))?;
                x.raw_set("max_hp", i64::from(t.max_hp))?;
                x.raw_set("mp", i64::from(t.mp))?;
                x.raw_set("max_mp", i64::from(t.max_mp))?;
                x.raw_set("dead", t.hp <= 0)?;
                x.raw_set("in_combat", false)?;
                x.raw_set("unit_type", t.unit_type)?;
                x.raw_set("detail", t.detail)?;
                x.raw_set("faction", t.faction_name.as_str())?;
                x.raw_set("relation", t.relation)?;
                x.raw_set("grade", t.grade)?;
                x.raw_set("race", i64::from(t.race))?;
                x.raw_set("gender", i64::from(t.gender))?;
                x.raw_set("buffs", buffs_table(lua, &t.buffs)?)?;
                hud.raw_set("target", x)?;
            }
        }
        c.target = true;
    }
    Ok(c)
}

/// Which parts of the panels changed in a [`push`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Changed {
    /// Action bar or bag: the slot widgets should ask again.
    pub slots: bool,
    pub bag: bool,
    pub equipment: bool,
    pub buffs: bool,
    pub target: bool,
}

/// Number keys use the action bar slots: 1..9, 0, -, = are slots 1..12, with Shift 13..24.
const SLOT_KEYS: [KeyCode; 12] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Digit0,
    KeyCode::Minus,
    KeyCode::Equal,
];

/// Use what is in action bar slot `slot` (0 based): cast the skill, use the item.
pub fn use_slot(slot: usize, game: Option<&GameData>, bar: &ActionBar, inventory: &Inventory, target: &Target, net: &mut MessageWriter<NetCommand>) {
    let Some(action) = bar.get(slot) else { return };
    let target_obj_id = target.obj_id.unwrap_or(0);
    match action.kind {
        ActionKind::Spell | ActionKind::RidePetSpell => {
            net.write(NetCommand(Command::UseSkill { skill_id: action.id as u32, target_obj_id }));
        }
        ActionKind::ItemType | ActionKind::ItemId => {
            let found = match action.item_template_id() {
                Some(t) => inventory.find_template(t).map(|(_, it)| it),
                None => inventory.find_by_id(action.id).map(|(_, _, it)| it),
            };
            let Some(item) = found else { return };
            use_item(item, game, target_obj_id, net);
        }
        _ => {}
    }
}

/// Use a bag item (a right click in the bag window): its skill, on the target.
pub fn use_bag_slot(slot: usize, game: Option<&GameData>, inventory: &Inventory, target: &Target, net: &mut MessageWriter<NetCommand>) {
    if let Some(Some(item)) = inventory.bag.get(slot) {
        use_item(item, game, target.obj_id.unwrap_or(0), net);
    }
}

fn use_item(item: &aa_net::model::Item, game: Option<&GameData>, target_obj_id: u32, net: &mut MessageWriter<NetCommand>) {
    let skill_id = game.and_then(|g| g.items.get(&item.template_id)).map_or(0, |i| i.use_skill);
    net.write(NetCommand(Command::UseItem { item_id: item.id, template_id: item.template_id, skill_id, target_obj_id }));
}

/// The action bar's number keys.
#[allow(clippy::too_many_arguments)]
pub fn action_keys(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<HudKeyboardFocus>,
    bridge: NonSend<HudBridge>,
    bar: Res<ActionBar>,
    inventory: Res<Inventory>,
    target: Res<Target>,
    mut script: ResMut<super::chat::ScriptedKeys>,
    mut net: MessageWriter<NetCommand>,
) {
    if let Some(slot) = script.take_slot() {
        use_slot(slot, bridge.0.borrow().game.get(), &bar, &inventory, &target, &mut net);
    }
    if focus.0 {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    for (i, key) in SLOT_KEYS.iter().enumerate() {
        if keys.just_pressed(*key) {
            let data = bridge.0.borrow();
            use_slot(i + if shift { 12 } else { 0 }, data.game.get(), &bar, &inventory, &target, &mut net);
        }
    }
}
