//! Whether a skill (or an item through its use skill) can be used right now: what the original client's native
//! `ActionAccessor::CanUseSkill` decides, and what the scripts hear as the slot event `can_use`
//! (`action_slot.lua`: usable -> the icon as it is, not usable -> a black 75 % layer over it, and the hotkey letter stays red).
//!
//! The rules are the server's (`Skill.Use`: `validation::check`, `unit_reqs::validate`, `check_resources`, the casting task)
//! over the 1.2 game database, in the order the server asks them, so the text a refusal shows is the one the server would
//! have sent:
//!
//! 1. the player is dead (`SOURCE_DIED`) or alive for a skill meant for the dead (`SOURCE_ALIVE`);
//! 2. the skill's unit requirements (`unit_reqs`, owner `Skill`): level, ability level, race, gender, a worn item in a slot or
//!    a shield / ranged weapon, an owned item, a learned skill, "not in combat" (Recall, Dash and the dances go dark in a
//!    fight: `real_NN.png` of `combat_death`), buffs; the kinds the client has no data for (quests, doodads, zones) count as met;
//! 3. buff restrictions of the player: stun, sleep, knock down, "blank minded", silence (magic skills), crippled (physical);
//! 4. the player is casting (`ON_CASTING`: every slot goes dark during Recall, `combat_return`) or channeling;
//! 5. the target: a hostile skill needs one (`NO_TARGET`: the skills dim as soon as the target is gone), of the right relation
//!    (`INVALID_TARGET`) and alive or dead as the skill wants (`TARGET_DIED`: the bar dims when the rat dies);
//! 6. the range, from the skill or from the weapon in the hand (`TOO_FAR_RANGE` / `TOO_CLOSE_RANGE`);
//! 7. mana, labor points.
//!
//! The own cooldown comes last ([`Env::cooldown`]): the sweep already shows it, so it does not darken the icon. Unknown skills and
//! rules without data pass: the server has the last word and its refusal is shown too (`refusal.rs`).
//!
//! Before any of that a press can be held back without a word ([`Env::held_back`]): the skill is being cast, the global cooldown
//! has not run out, or the previous press is still on its way to the server ([`InFlight`]).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use aa_net::model::{exp, slot};
use aa_net::state::{BuffEntry, Casting, Inventory, Skills};
use bevy::prelude::{Resource, Vec3};
use rusqlite::Connection;

use super::data::{GameData, Snapshot, any_flag, any_int};
use super::api::HudData;
use super::panels::{SlotSnap, TargetSnap};

/// `SkillResult` numbers (the 1.2 client's, `SkillResultHelper.SkillResultErrorKeyToIdFor_r208022`) used here.
pub mod code {
    pub const SOURCE_DIED: u8 = 2;
    pub const SOURCE_ALIVE: u8 = 3;
    pub const TARGET_DIED: u8 = 4;
    pub const TARGET_ALIVE: u8 = 6;
    pub const ON_CASTING: u8 = 7;
    pub const COOLDOWN_TIME: u8 = 8;
    pub const NO_TARGET: u8 = 9;
    pub const LACK_MANA: u8 = 11;
    pub const TOO_CLOSE_RANGE: u8 = 14;
    pub const TOO_FAR_RANGE: u8 = 15;
    pub const CANNOT_CAST_IN_STUN: u8 = 19;
    pub const BLANK_MINDED: u8 = 22;
    pub const SILENCE: u8 = 23;
    pub const CRIPPLED: u8 = 24;
    pub const CANNOT_CAST_IN_CHANNELING: u8 = 25;
    pub const INVALID_TARGET: u8 = 34;
    pub const NEED_REAGENT: u8 = 36;
    pub const NEED_LABOR_POWER: u8 = 39;
    pub const HIGHER_BUFF: u8 = 42;
    pub const UNIT_REQS_OR_FAIL: u8 = 49;
    pub const URK_LEVEL: u8 = 64;
    pub const URK_ABILITY: u8 = 65;
    pub const URK_RACE: u8 = 66;
    pub const URK_GENDER: u8 = 67;
    pub const URK_EQUIP_SLOT: u8 = 68;
    pub const URK_EQUIP_ITEM: u8 = 69;
    pub const URK_OWN_ITEM: u8 = 70;
    pub const URK_TRAINED_SKILL: u8 = 71;
    pub const URK_COMBAT: u8 = 72;
    pub const URK_BUFF: u8 = 75;
    pub const URK_TARGET_BUFF: u8 = 76;
    pub const URK_EQUIP_SHIELD: u8 = 80;
    pub const URK_NOBUFF: u8 = 81;
    pub const URK_EQUIP_RANGED: u8 = 88;
}

/// The name the client passes to `SKILL_MSG(name, ...)` for a result number: the scripts print the text
/// `skill_<name in lower case>` of the system message texts (`formatSkillMessage` in `alert_message.lua`).
/// Read off the string table of `x2game.dll` (`URK_LEVEL`, `LACK_MANA`, ...).
pub fn result_name(result: u8) -> &'static str {
    NAMES.get(usize::from(result)).copied().unwrap_or("FAILURE")
}

/// The number of a result name (`LACK_MANA`), for the scripted refusals.
pub fn result_number(name: &str) -> Option<u8> {
    NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)).map(|i| i as u8)
}

#[rustfmt::skip]
const NAMES: [&str; 131] = [
    "SUCCESS", "FAILURE", "SOURCE_DIED", "SOURCE_ALIVE", "TARGET_DIED", "TARGET_DESTROYED",
    "TARGET_ALIVE", "ON_CASTING", "COOLDOWN_TIME", "NO_TARGET", "LACK_HEALTH", "LACK_MANA",
    "OBSTACLE", "OUTOF_HEIGHT", "TOO_CLOSE_RANGE", "TOO_FAR_RANGE", "OUTOF_ANGLE", "CANNOT_CAST_IN_COMBAT",
    "CANNOT_CAST_WHILE_MOVING", "CANNOT_CAST_IN_STUN", "CANNOT_CAST_WHILE_WALKING", "CANNOT_CAST_IN_SWIMMING", "BLANK_MINDED", "SILENCE",
    "CRIPPLED", "CANNOT_CAST_IN_CHANNELING", "CANNOT_CAST_IN_PRISON", "NEED_STEALTH", "NEED_NOCOMBAT_TARGET", "TARGET_IMMUNE",
    "INVALID_SKILL", "INACTIVE_ABILITY", "NOT_ENOUGH_ABILITY_LEVEL", "INVALID_SOURCE", "INVALID_TARGET", "INVALID_LOCATION",
    "NEED_REAGENT", "ITEM_LOCKED", "NEED_MONEY", "NEED_LABOR_POWER", "SOURCE_IS_HANGING", "SOURCE_IS_RIDING",
    "HIGHER_BUFF", "NOT_PVP_AREA", "NOT_NOW", "NO_PERM", "BAG_FULL", "PROTECTED_FACTION",
    "PROTECTED_LEVEL", "UNIT_REQS_OR_FAIL", "SKILL_REQ_FAIL", "BACKPACK_OCCUPIED", "OBSTACLE_FOR_SPAWN_DOODAD", "CANNOT_SPAWN_DOODAD_IN_HOUSE",
    "CANNOT_USE_FOR_SELF", "NOT_PREOCCUPIED", "NOT_MY_NPC", "NOT_CHECKED_SECOND_PASS", "ZONE_BANNED", "INVALID_GRADE_ENCHANT_SUPPORT_ITEM",
    "CHECK_CHARACTER_P_STAT_MIN", "CHECK_CHARACTER_P_STAT_MAX", "ITEM_SECURED", "INVALID_ACCOUNT_ATTRIBUTE", "URK_LEVEL", "URK_ABILITY",
    "URK_RACE", "URK_GENDER", "URK_EQUIP_SLOT", "URK_EQUIP_ITEM", "URK_OWN_ITEM", "URK_TRAINED_SKILL",
    "URK_COMBAT", "URK_STEALTH", "URK_HEALTH", "URK_BUFF", "URK_TARGET_BUFF", "URK_TARGET_COMBAT",
    "URK_CAN_LEARN_CRAFT", "URK_DOODAD_RANGE", "URK_EQUIP_SHIELD", "URK_NOBUFF", "URK_TARGET_BUFF_TAG", "URK_CORPSE_RANGE",
    "URK_EQUIP_WEAPON_TYPE", "URK_TARGET_HEALTH_LESS_THAN", "URK_TARGET_NPC", "URK_TARGET_DOODAD", "URK_EQUIP_RANGED", "URK_NO_BUFF_TAG",
    "URK_COMPLETE_QUEST_CONTEXT", "URK_PROGRESS_QUEST_CONTEXT", "URK_READY_QUEST_CONTEXT", "URK_TARGET_NPC_GROUP", "URK_AREA_SPHERE", "URK_EXCEPT_COMPLETE_QUEST_CONTEXT",
    "URK_PRECOMPLETE_QUEST_CONTEXT", "URK_TARGET_OWNER_TYPE", "URK_NOT_UNDER_WATER", "URK_FACTION_MATCH", "URK_TOD", "URK_MOTHER_FACTION",
    "URK_ACTABILITY_POINT", "URK_CRIME_POINT", "URK_HONOR_POINT", "URK_LIVING_POINT", "URK_CRIME_RECORD", "URK_JURY_POINT",
    "URK_SOURCE_OWNER_TYPE", "URK_APPELATION", "URK_IN_ZONE", "URK_OUT_ZONE", "URK_DOMINION_OWNER", "URK_VERDICT_ONLY",
    "URK_FACTION_MATCH_ONLY", "URK_MOTHER_FACTION_ONLY", "URK_NATION_OWNER", "URK_FACTION_MATCH_ONLY_NOT", "URK_MOTHER_FACTION_ONLY_NOT", "URK_NATION_MEMBER",
    "URK_NATION_MEMBER_NOT", "URK_NATION_OWNER_AT_POS", "URK_DOMINION_OWNER_AT_POS", "URK_HOUSING", "URK_HEALTH_MARGIN", "URK_MANA_MARGIN",
    "URK_LABOR_POWER_MARGIN", "URK_UNKNOWN", "URK_MAX_LEVEL", "URK_EXPEDITION_OWNER", "URK_EXPEDITION_MEMBER",
];

/// A refusal: the result number and the detail the text takes (an item or buff id, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fail {
    pub result: u8,
    pub value: u32,
}

const fn fail(result: u8) -> Fail {
    Fail { result, value: 0 }
}

/// A unit requirement of a skill (`unit_reqs`): the kind (`UnitReqsKind`) and its two values.
#[derive(Clone, Copy, Debug, Default)]
struct Req {
    kind: u8,
    v1: u32,
    v2: u32,
}

/// The rules of one skill (`skills`).
#[derive(Clone, Debug, Default)]
pub struct SkillRules {
    mana: f64,
    mana_level_md: f64,
    ability: u8,
    labor: i32,
    /// `SkillTargetType`: 0 self, 1 friendly, 4 hostile, 5 any unit, 16 others, 17 friendly others, ...
    target_type: u8,
    source_alive: bool,
    source_dead: bool,
    target_alive: bool,
    target_dead: bool,
    /// Usable while stunned.
    source_stun: bool,
    min_range: f32,
    max_range: f32,
    /// `weapon_slot_for_range_id`: the equipment slot whose weapon sets the range (> 0).
    range_slot: i32,
    /// 1 melee, 2 magic, 3 siege, 4 ranged, 5 heal.
    damage_type: u8,
    /// Starting it ends a channel (the others wait for it).
    stop_channeling: bool,
    or_reqs: bool,
    reqs: Vec<Req>,
}

/// What a buff does to the unit it is on.
#[derive(Clone, Copy, Debug, Default)]
struct BuffFlags {
    stun: bool,
    sleep: bool,
    knock_down: bool,
    silence: bool,
    crippled: bool,
    blank_minded: bool,
}

/// A weapon's reach and kind (`item_weapons` -> `holdables`).
#[derive(Clone, Copy, Debug, Default)]
struct Weapon {
    min_range: f32,
    max_range: f32,
    /// `EquipmentItemSlotType` (20: shield).
    slot_type: u32,
}

/// The usage rules of the game database.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    skills: HashMap<u32, SkillRules>,
    buffs: HashMap<u32, BuffFlags>,
    weapons: HashMap<u32, Weapon>,
}

const SLOT_TYPE_SHIELD: u32 = 20;

impl Rules {
    pub fn load(conn: &Connection) -> rusqlite::Result<Rules> {
        let mut rules = Rules::default();
        let mut reqs: HashMap<u32, Vec<Req>> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT owner_id, kind_id, value1, value2 FROM unit_reqs WHERE owner_type = 'Skill'")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                reqs.entry(any_int(r, 0)? as u32).or_default().push(Req { kind: any_int(r, 1)? as u8, v1: any_int(r, 2)?.max(0) as u32, v2: any_int(r, 3)?.max(0) as u32 });
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, mana_cost, mana_level_md, ability_id, consume_lp, target_type_id, source_alive, source_dead, target_alive, target_dead, source_stun, \
                 min_range, max_range, weapon_slot_for_range_id, damage_type_id, stop_channeling_on_start_skill, or_unit_reqs FROM skills",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                rules.skills.insert(
                    id,
                    SkillRules {
                        mana: any_int(r, 1)? as f64,
                        mana_level_md: real(r, 2)?,
                        ability: any_int(r, 3)?.clamp(0, 10) as u8,
                        labor: any_int(r, 4)?.max(0) as i32,
                        target_type: any_int(r, 5)?.clamp(0, 255) as u8,
                        source_alive: any_flag(r, 6)?,
                        source_dead: any_flag(r, 7)?,
                        target_alive: any_flag(r, 8)?,
                        target_dead: any_flag(r, 9)?,
                        source_stun: any_flag(r, 10)?,
                        min_range: real(r, 11)? as f32,
                        max_range: real(r, 12)? as f32,
                        range_slot: any_int(r, 13)? as i32,
                        damage_type: any_int(r, 14)?.clamp(0, 255) as u8,
                        stop_channeling: any_flag(r, 15)?,
                        or_reqs: any_flag(r, 16)?,
                        reqs: reqs.remove(&id).unwrap_or_default(),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, stun, sleep, knock_down, silence, crippled, blank_minded FROM buffs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let flags = BuffFlags {
                    stun: any_flag(r, 1)?,
                    sleep: any_flag(r, 2)?,
                    knock_down: any_flag(r, 3)?,
                    silence: any_flag(r, 4)?,
                    crippled: any_flag(r, 5)?,
                    blank_minded: any_flag(r, 6)?,
                };
                if flags.stun || flags.sleep || flags.knock_down || flags.silence || flags.crippled || flags.blank_minded {
                    rules.buffs.insert(any_int(r, 0)? as u32, flags);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT w.item_id, h.min_range, h.max_range, h.slot_type_id FROM item_weapons w JOIN holdables h ON h.id = w.holdable_id")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                rules.weapons.insert(
                    any_int(r, 0)? as u32,
                    Weapon { min_range: real(r, 1)? as f32, max_range: real(r, 2)? as f32, slot_type: any_int(r, 3)?.max(0) as u32 },
                );
            }
        }
        Ok(rules)
    }

    #[cfg(test)]
    pub(super) fn with_skill(mut self, id: u32, s: SkillRules) -> Rules {
        self.skills.insert(id, s);
        self
    }
}

/// A number column, whatever the database stored it as.
fn real(r: &rusqlite::Row, i: usize) -> rusqlite::Result<f64> {
    use rusqlite::types::Value;
    Ok(match r.get::<_, Value>(i)? {
        Value::Integer(n) => n as f64,
        Value::Real(f) => f,
        Value::Text(t) => t.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

/// What the checks look at: the player, the target and the world state aa-net keeps.
pub struct Env<'a> {
    pub game: Option<&'a GameData>,
    pub me: &'a Snapshot,
    /// The player's position and the target's, in the server's coordinates.
    pub pos: Vec3,
    pub target: Option<&'a TargetSnap>,
    pub target_pos: Option<Vec3>,
    pub skills: Option<&'a Skills>,
    pub buffs: &'a [BuffEntry],
    pub inventory: Option<&'a Inventory>,
}

/// How long a press waits for the server's answer before the next one may go out.
const FLIGHT_MS: u64 = 500;

/// The global cooldown left under which a press goes out anyway: the client counts it from the server's answer, which is a
/// little behind the server's own clock.
const GCD_SLACK_MS: u32 = 100;

/// The last press of a skill that starts the global cooldown, until the server has answered it. The client's own estimate of
/// the cooldown only starts with that answer, so the presses in between would all be sent.
#[derive(Resource, Default, Debug)]
pub struct InFlight(Option<Instant>);

impl InFlight {
    /// A skill that waits for the global cooldown was sent.
    pub fn sent(&mut self, now: Instant) {
        self.0 = Some(now);
    }

    /// The answer has not come: the player started a skill or a cast failed since the press (aa-net stamps both), and the wait
    /// is not over.
    pub fn pending(&self, skills: &Skills, now: Instant) -> bool {
        let Some(at) = self.0 else { return false };
        let answered = skills.last_cast.is_some_and(|c| c >= at) || skills.last_failure.as_ref().is_some_and(|f| f.at >= at);
        !answered && now.saturating_duration_since(at) < Duration::from_millis(FLIGHT_MS)
    }
}

/// A cast with a time that has not fired yet (one whose end packet was lost stops counting a couple of seconds late).
fn in_cast(c: &Casting) -> bool {
    c.fired.is_none() && c.cast_ms > 0 && c.started.elapsed().as_millis() < u128::from(c.cast_ms) + 2000
}

fn in_channel(c: &Casting) -> bool {
    c.fired.is_some_and(|at| c.channeling_ms > 0 && at.elapsed().as_millis() < c.channeling_ms as u128)
}

/// The distance the bodies take off the centre to centre distance (the server measures edge to edge).
const BODIES: f32 = 1.0;

/// A target further than this over the reach is refused here at once; closer to the limit the server decides (it knows the
/// size of the bodies).
pub const SEND_SLACK: f32 = 2.0;

impl<'a> Env<'a> {
    /// The checks over what the HUD keeps ([`HudData`]: the player's numbers, the target and where both are) and aa-net's
    /// state of the player.
    pub fn of(data: &'a HudData, skills: Option<&'a Skills>, buffs: &'a [BuffEntry], inventory: Option<&'a Inventory>) -> Env<'a> {
        Env { game: data.game.get(), me: &data.snapshot, pos: data.pos, target: data.panels.target.as_ref(), target_pos: data.target_pos, skills, buffs, inventory }
    }
}

impl Env<'_> {
    fn rules(&self) -> Option<&Rules> {
        self.game.map(|g| &g.rules)
    }

    fn dead(&self) -> bool {
        self.me.dead || self.me.hp <= 0
    }

    fn worn(&self, slot: u32) -> Option<&aa_net::model::Item> {
        self.inventory?.equipment.get(slot as usize)
    }

    /// The target is the player.
    fn is_self(&self, t: &TargetSnap) -> bool {
        self.skills.is_some_and(|s| s.obj_id != 0 && s.obj_id == t.obj_id)
    }

    fn has_buff(&self, id: u32) -> bool {
        self.buffs.iter().any(|b| b.buff.buff_id == id)
    }

    fn ability_level(&self, ability: u8) -> i32 {
        if ability == 0 {
            return i32::from(self.me.level.max(1));
        }
        i32::from(exp::level_for_total(i64::from(self.me.ability_exp.get(usize::from(ability)).copied().unwrap_or(0))))
    }

    /// The mana a skill takes (`skill::mana_cost` of the server without the buff modifiers).
    pub fn mana_cost(&self, r: &SkillRules) -> i32 {
        let base = (f64::from(self.ability_level(r.ability) - 1) * 1.6 + 8.0) * 3.0 / 3.65;
        (base * r.mana_level_md + r.mana) as i32
    }

    /// How far the target is (centre to centre).
    fn distance(&self) -> Option<f32> {
        Some(self.pos.distance(self.target_pos?))
    }

    /// The reach of a skill: its own, or the weapon's in the slot it names (fists: 3 m).
    fn reach(&self, r: &SkillRules) -> (f32, f32) {
        if r.range_slot > 0 {
            let weapon = self.worn(r.range_slot as u32).and_then(|it| self.rules()?.weapons.get(&it.template_id));
            return weapon.map_or((0.0, 3.0), |w| (w.min_range, w.max_range));
        }
        (r.min_range, r.max_range)
    }

    fn requirement(&self, req: &Req) -> Option<Fail> {
        let me = self.me;
        let met = |ok: bool, result: u8, value: u32| (!ok).then_some(Fail { result, value });
        match req.kind {
            0x01 => met(u32::from(me.level) >= req.v1 && (req.v2 == 0 || u32::from(me.level) <= req.v2), code::URK_LEVEL, 0),
            0x02 => met(self.ability_level(req.v1.min(10) as u8) >= req.v2 as i32, code::URK_ABILITY, 0),
            0x03 => met(u32::from(me.race) == req.v1, code::URK_RACE, 0),
            0x04 => met(u32::from(me.gender) == req.v1, code::URK_GENDER, 0),
            0x08 => met(self.worn(req.v1).is_some(), code::URK_EQUIP_SLOT, 0),
            0x09 => met(self.inventory.is_some_and(|i| i.equipment.slots.iter().flatten().any(|it| it.template_id == req.v1)), code::URK_EQUIP_ITEM, 0),
            0x0A => met(self.inventory.is_some_and(|i| i.count_of(req.v1) > 0), code::URK_OWN_ITEM, req.v1),
            0x0B => met(self.skills.is_some_and(|s| s.knows(req.v1)), code::URK_TRAINED_SKILL, 0),
            0x0C => met(!me.in_combat, code::URK_COMBAT, 0),
            0x0F => met(self.has_buff(req.v1), code::URK_BUFF, req.v1),
            0x10 => met(self.target.is_some_and(|t| t.buffs.good.iter().chain(&t.buffs.bad).any(|b| b.id == req.v1)), code::URK_TARGET_BUFF, req.v1),
            0x15 => met(
                self.worn(slot::OFFHAND as u32).and_then(|it| self.rules()?.weapons.get(&it.template_id)).is_some_and(|w| w.slot_type == SLOT_TYPE_SHIELD),
                code::URK_EQUIP_SHIELD,
                0,
            ),
            0x16 => met(!self.has_buff(req.v1), code::URK_NOBUFF, req.v1),
            0x1D => met(self.worn(if req.v1 == 1 { slot::MUSICAL } else { slot::RANGED } as u32).is_some(), code::URK_EQUIP_RANGED, req.v1),
            // quests, doodads, zones, factions ...: the server knows
            _ => None,
        }
    }

    fn requirements(&self, r: &SkillRules) -> Option<Fail> {
        let mut first = None;
        let mut any = r.reqs.is_empty();
        for req in &r.reqs {
            match self.requirement(req) {
                Some(f) => {
                    first.get_or_insert(f);
                }
                None => any = true,
            }
        }
        if r.or_reqs { (!any).then_some(fail(code::UNIT_REQS_OR_FAIL)) } else { first }
    }

    /// The cast or channel the player is in the middle of. The skill being cast itself stays lit (`combat_return`: the
    /// teleport book is bright while Recall is cast, everything else is dark); a cast that never ended (its end packet was
    /// lost) stops counting a couple of seconds after it should have.
    fn busy(&self, skill: u32, r: &SkillRules) -> Option<Fail> {
        let skills = self.skills?;
        let c = skills.casting_of(skills.obj_id)?;
        if c.skill_id == skill {
            return None;
        }
        match c.fired {
            None => in_cast(c).then_some(fail(code::ON_CASTING)),
            Some(_) => (in_channel(c) && !r.stop_channeling).then_some(fail(code::CANNOT_CAST_IN_CHANNELING)),
        }
    }

    /// The skill is the cast or channel the player is in the middle of.
    fn casting(&self, skill: u32) -> bool {
        let Some(skills) = self.skills else { return false };
        skills.casting_of(skills.obj_id).is_some_and(|c| c.skill_id == skill && (in_cast(c) || in_channel(c)))
    }

    fn buff_restriction(&self, r: &SkillRules) -> Option<Fail> {
        let rules = self.rules()?;
        for b in self.buffs {
            let Some(f) = rules.buffs.get(&b.buff.buff_id) else { continue };
            if !r.source_stun && (f.stun || f.sleep || f.knock_down) {
                return Some(fail(code::CANNOT_CAST_IN_STUN));
            }
            if f.blank_minded && !r.source_stun {
                return Some(fail(code::BLANK_MINDED));
            }
            if f.silence && r.damage_type == 2 {
                return Some(fail(code::SILENCE));
            }
            if f.crippled && matches!(r.damage_type, 1 | 4) {
                return Some(fail(code::CRIPPLED));
            }
        }
        None
    }

    fn target_rules(&self, r: &SkillRules) -> Option<Fail> {
        // what the server's `initial_target` asks of each kind: 4 hostile, 16 others and 17 friendly others need a target,
        // the friendly and any-unit skills fall back to the caster
        let needs = matches!(r.target_type, 4 | 16 | 17);
        let unit_skill = matches!(r.target_type, 1 | 4 | 5 | 16 | 17);
        let Some(t) = self.target else {
            return needs.then_some(fail(code::NO_TARGET));
        };
        if !unit_skill {
            return None;
        }
        let own = self.is_self(t);
        let hostile = t.relation == "hostile";
        let friendly = t.relation == "friendly";
        match r.target_type {
            1 if hostile => return Some(fail(code::INVALID_TARGET)),
            4 if friendly => return Some(fail(code::INVALID_TARGET)),
            16 | 17 if own => return Some(fail(code::NO_TARGET)),
            17 if hostile => return Some(fail(code::INVALID_TARGET)),
            _ => {}
        }
        if t.hp <= 0 && r.target_alive && !r.target_dead {
            return Some(fail(code::TARGET_DIED));
        }
        if t.hp > 0 && r.target_dead && !r.target_alive {
            return Some(fail(code::TARGET_ALIVE));
        }
        None
    }

    fn range(&self, r: &SkillRules, slack: f32) -> Option<Fail> {
        let t = self.target?;
        if !matches!(r.target_type, 1 | 4 | 5 | 16 | 17) || self.is_self(t) {
            return None;
        }
        let distance = (self.distance()? - BODIES).max(0.0);
        let (min, max) = self.reach(r);
        if max <= 0.0 && min <= 0.0 {
            return None;
        }
        if distance < min {
            return Some(fail(code::TOO_CLOSE_RANGE));
        }
        (distance > max + slack).then_some(fail(code::TOO_FAR_RANGE))
    }

    fn resources(&self, r: &SkillRules) -> Option<Fail> {
        if self.me.mp < self.mana_cost(r) {
            return Some(fail(code::LACK_MANA));
        }
        (r.labor > self.me.labor).then_some(fail(code::NEED_LABOR_POWER))
    }

    /// Why the skill cannot be used now, `slack` metres of range given (0 for the slot's look, [`SEND_SLACK`] before sending).
    fn why_not(&self, skill: u32, slack: f32) -> Option<Fail> {
        if self.game.is_some_and(|g| g.book.requires_learning(skill)) && self.skills.is_some_and(|s| !s.knows(skill)) {
            return Some(fail(code::URK_TRAINED_SKILL));
        }
        let r = self.rules()?.skills.get(&skill)?;
        let dead = self.dead();
        if dead && r.source_alive && !r.source_dead {
            return Some(fail(code::SOURCE_DIED));
        }
        if !dead && r.source_dead && !r.source_alive {
            return Some(fail(code::SOURCE_ALIVE));
        }
        if dead {
            return None;
        }
        self.requirements(r)
            .or_else(|| self.buff_restriction(r))
            .or_else(|| self.busy(skill, r))
            .or_else(|| self.target_rules(r))
            .or_else(|| self.range(r, slack))
            .or_else(|| self.resources(r))
    }

    /// The global cooldown the skill still waits for, in milliseconds (0 for the skills that ignore it).
    pub fn gcd_left(&self, skill: u32) -> u32 {
        let (Some(game), Some(skills)) = (self.game, self.skills) else { return 0 };
        super::cooldown::gcd_at(game, skills, skill, Instant::now()).0
    }

    /// The skill starts the global cooldown (it is known and does not ignore it).
    pub fn uses_gcd(&self, skill: u32) -> bool {
        self.game.and_then(|g| g.skills.get(&skill)).is_some_and(|i| !i.ignore_gcd)
    }

    /// A press the client drops without a word, so that holding a key sends nothing the server would refuse: the skill is
    /// being cast, the global cooldown has more than [`GCD_SLACK_MS`] left, or the last press has no answer yet.
    pub fn held_back(&self, skill: u32, flight: &InFlight) -> bool {
        if self.casting(skill) {
            return true;
        }
        self.uses_gcd(skill) && (self.gcd_left(skill) > GCD_SLACK_MS || self.skills.is_some_and(|s| flight.pending(s, Instant::now())))
    }

    /// What the client refuses before it asks the server: everything that darkens the slot ([`Env::slot_blocked`]), except
    /// that a target a little over the reach is left to the server (it knows the size of the bodies), and the skill's own
    /// cooldown (the sweep shows it, the slot stays lit).
    pub fn precheck(&self, skill: u32) -> Option<Fail> {
        self.why_not(skill, SEND_SLACK).or_else(|| self.cooldown(skill))
    }

    /// Why an action bar slot is dark: the result that refuses it, `None` for a lit slot. An item needs to be in the bag
    /// and its skill usable (`combat_return`: the potions go dark while Recall is cast).
    pub fn slot_blocked(&self, slot: &SlotSnap) -> Option<u8> {
        match slot.kind {
            "skill" => self.why_not(slot.id, 0.0).map(|f| f.result),
            "item" if slot.count <= 0 => Some(code::NEED_REAGENT),
            "item" => {
                let skill = self.game?.items.get(&slot.id)?.use_skill;
                (skill != 0).then(|| self.why_not(skill, 0.0).map(|f| f.result)).flatten()
            }
            _ => None,
        }
    }

    /// What the client refuses before it uses an item: its skill.
    pub fn precheck_item(&self, template: u32) -> Option<(u32, Fail)> {
        let skill = self.game?.items.get(&template)?.use_skill;
        if skill == 0 {
            return None;
        }
        self.precheck(skill).map(|f| (skill, f))
    }

    /// The skill's own cooldown is running (the global cooldown is not counted: [`Env::held_back`] drops those presses quietly).
    pub fn cooldown(&self, skill: u32) -> Option<Fail> {
        let info = self.game?.skills.get(&skill)?;
        let skills = self.skills?;
        // Explicit skill/tag timers are own cooldowns even when the DB duration is zero or
        // shorter than a GCD. Their wire total does not determine whether they are active.
        if super::cooldown::native_at(info, skills, skill, std::time::Instant::now()).0 > 0 {
            return Some(fail(code::COOLDOWN_TIME));
        }
        let (left, _) = super::cooldown::observed_at(info, skills, skill, std::time::Instant::now());
        (left > 0).then_some(fail(code::COOLDOWN_TIME))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    use aa_net::state::{Casting, Inventory};

    use crate::hud::panels::BuffLists;

    fn melee() -> SkillRules {
        SkillRules { target_type: 4, source_alive: true, target_alive: true, min_range: 0.0, max_range: 4.0, damage_type: 1, ..SkillRules::default() }
    }

    fn game(rules: Rules) -> GameData {
        GameData { rules, ..GameData::default() }
    }

    fn target(hp: i32, relation: &'static str) -> TargetSnap {
        TargetSnap { obj_id: 77, hp, max_hp: 100, relation, buffs: BuffLists::default(), ..TargetSnap::default() }
    }

    fn me() -> Snapshot {
        Snapshot { unit_id: "5".into(), level: 1, hp: 700, max_hp: 700, mp: 100, max_mp: 100, labor: 50, ..Snapshot::default() }
    }

    fn env<'a>(g: &'a GameData, me: &'a Snapshot, t: Option<&'a TargetSnap>, distance: f32) -> Env<'a> {
        Env { game: Some(g), me, pos: Vec3::ZERO, target: t, target_pos: Some(Vec3::new(distance, 0.0, 0.0)), skills: None, buffs: &[], inventory: None }
    }

    #[test]
    fn target_and_range() {
        let g = game(Rules::default().with_skill(1, melee()));
        let me = me();
        let rat = target(100, "hostile");
        // a hostile skill needs a target, a living hostile one, in reach
        assert_eq!(env(&g, &me, None, 3.0).why_not(1, 0.0), Some(fail(code::NO_TARGET)));
        assert_eq!(env(&g, &me, Some(&rat), 3.0).why_not(1, 0.0), None);
        assert_eq!(env(&g, &me, Some(&rat), 12.0).why_not(1, 0.0), Some(fail(code::TOO_FAR_RANGE)));
        // the reach plus the bodies is fine, a little over it is the server's call when sending
        assert_eq!(env(&g, &me, Some(&rat), 4.9).why_not(1, 0.0), None);
        assert_eq!(env(&g, &me, Some(&rat), 6.5).why_not(1, 0.0), Some(fail(code::TOO_FAR_RANGE)));
        assert_eq!(env(&g, &me, Some(&rat), 6.5).precheck(1), None);
        assert_eq!(env(&g, &me, Some(&rat), 9.0).precheck(1), Some(fail(code::TOO_FAR_RANGE)));
        // the rat is dead: `combat_death` real_16, the bar goes dark
        assert_eq!(env(&g, &me, Some(&target(0, "hostile")), 3.0).why_not(1, 0.0), Some(fail(code::TARGET_DIED)));
        // a friendly unit is no target for a hostile skill
        assert_eq!(env(&g, &me, Some(&target(100, "friendly")), 3.0).why_not(1, 0.0), Some(fail(code::INVALID_TARGET)));
    }

    #[test]
    fn an_unlearned_tree_shortcut_refuses_until_it_is_learned() {
        let mut g = game(Rules::default().with_skill(1, SkillRules::default()));
        g.book.active = vec![vec![crate::hud::skillbook::ActiveSkill { id: 1, level: 1, points: 1 }]];
        g.book.cost.insert(1, (1, 1));
        let me = me();
        let mut skills = Skills { obj_id: 5, ..Skills::default() };
        let blocked = |skills: &Skills| Env { skills: Some(skills), ..env(&g, &me, None, 0.0) }.precheck(1);
        assert_eq!(blocked(&skills), Some(fail(code::URK_TRAINED_SKILL)));
        skills.apply(&aa_net::Event::SkillLearned { skill_id: 1, level: 1 });
        assert_eq!(blocked(&skills), None);
        // General/attack skills that are not purchased from a tree remain available.
        assert!(!g.book.requires_learning(2));
    }

    #[test]
    fn mana_combat_and_death() {
        let mana = SkillRules { mana: 30.0, ..melee() };
        let recall = SkillRules { reqs: vec![Req { kind: 0x0C, v1: 0, v2: 0 }], source_alive: true, target_type: 0, ..SkillRules::default() };
        let g = game(Rules::default().with_skill(1, mana).with_skill(2, recall));
        let rat = target(100, "hostile");
        let mut m = me();
        assert_eq!(env(&g, &m, Some(&rat), 2.0).why_not(1, 0.0), None);
        m.mp = 29;
        assert_eq!(env(&g, &m, Some(&rat), 2.0).why_not(1, 0.0), Some(fail(code::LACK_MANA)));
        // Recall and Dash are dark in a fight (the first Shift slot of `combat_death`), lit again after "Battle End!"
        let mut m = me();
        assert_eq!(env(&g, &m, None, 0.0).why_not(2, 0.0), None);
        m.in_combat = true;
        assert_eq!(env(&g, &m, None, 0.0).why_not(2, 0.0), Some(fail(code::URK_COMBAT)));
        // the dead can use nothing that wants a living caster
        let mut m = me();
        m.dead = true;
        assert_eq!(env(&g, &m, Some(&rat), 2.0).why_not(1, 0.0), Some(fail(code::SOURCE_DIED)));
        assert_eq!(env(&g, &m, None, 0.0).why_not(2, 0.0), Some(fail(code::SOURCE_DIED)));
    }

    fn entry(id: u32) -> BuffEntry {
        BuffEntry { buff: aa_net::buff::Buff { buff_id: id, ..Default::default() }, received: Instant::now() }
    }

    fn casting(skill: u32, cast_ms: u32, started: Instant, fired: Option<Instant>, channeling_ms: i32) -> Casting {
        Casting { skill_id: skill, tl: 1, target: aa_net::skill::SkillTarget::unit(9), cast_ms, base_cast_ms: cast_ms, started, fired, channeling_ms }
    }

    #[test]
    fn buff_restrictions() {
        let magic = SkillRules { damage_type: 2, target_type: 0, source_alive: true, ..SkillRules::default() };
        let mut rules = Rules::default().with_skill(1, magic.clone()).with_skill(2, melee()).with_skill(3, SkillRules { source_stun: true, ..magic });
        rules.buffs.insert(10, BuffFlags { silence: true, ..Default::default() });
        rules.buffs.insert(11, BuffFlags { stun: true, ..Default::default() });
        rules.buffs.insert(12, BuffFlags { crippled: true, ..Default::default() });
        rules.buffs.insert(13, BuffFlags { sleep: true, ..Default::default() });
        let g = game(rules);
        let me = me();
        let rat = target(100, "hostile");
        let with = |ids: &[u32]| {
            let buffs: Vec<BuffEntry> = ids.iter().map(|id| entry(*id)).collect();
            let e = Env { buffs: &buffs, ..env(&g, &me, Some(&rat), 2.0) };
            (e.why_not(1, 0.0), e.why_not(2, 0.0), e.why_not(3, 0.0))
        };
        assert_eq!(with(&[]), (None, None, None));
        // silence stops magic only
        assert_eq!(with(&[10]), (Some(fail(code::SILENCE)), None, Some(fail(code::SILENCE))));
        // crippled stops physical skills only
        assert_eq!(with(&[12]), (None, Some(fail(code::CRIPPLED)), None));
        // stun and sleep stop everything but the skills made for it
        assert_eq!(with(&[11]), (Some(fail(code::CANNOT_CAST_IN_STUN)), Some(fail(code::CANNOT_CAST_IN_STUN)), None));
        assert_eq!(with(&[13]).1, Some(fail(code::CANNOT_CAST_IN_STUN)));
        // a buff without restrictions does nothing
        assert_eq!(with(&[99]), (None, None, None));
    }

    #[test]
    fn casting_darkens_everything_but_the_cast() {
        let g = game(Rules::default().with_skill(1, melee()).with_skill(9, melee()));
        let me = me();
        let rat = target(100, "hostile");
        let now = Instant::now();
        let check = |c: Option<Casting>| {
            let mut skills = Skills { obj_id: 5, ..Skills::default() };
            if let Some(c) = c {
                skills.casting.insert(5, c);
            }
            let e = Env { skills: Some(&skills), ..env(&g, &me, Some(&rat), 2.0) };
            (e.why_not(1, 0.0), e.why_not(9, 0.0))
        };
        assert_eq!(check(None), (None, None));
        // Recall's 5 s cast: the slots go dark (`combat_return`), the cast skill itself stays lit
        assert_eq!(check(Some(casting(9, 5000, now, None, 0))), (Some(fail(code::ON_CASTING)), None));
        // an instant skill is no cast
        assert_eq!(check(Some(casting(9, 0, now, None, 0))), (None, None));
        // a channel blocks the others until it ends
        assert_eq!(check(Some(casting(9, 0, now, Some(now), 4000))).0, Some(fail(code::CANNOT_CAST_IN_CHANNELING)));
        // a cast whose end never came does not block for ever
        assert_eq!(check(Some(casting(9, 1000, now - Duration::from_secs(10), None, 0))), (None, None));
    }

    #[test]
    fn gear_and_mana_cost() {
        let sword = SkillRules { reqs: vec![Req { kind: 0x08, v1: 15, v2: 0 }], ..melee() };
        let shot = SkillRules { range_slot: 17, min_range: 0.0, max_range: 0.0, ..melee() };
        let spell = SkillRules { mana: 10.0, mana_level_md: 1.0, ability: 1, target_type: 0, ..SkillRules::default() };
        let mut rules = Rules::default().with_skill(1, sword).with_skill(2, shot).with_skill(3, spell.clone());
        rules.weapons.insert(200, Weapon { min_range: 0.0, max_range: 20.0, slot_type: 17 });
        let g = game(rules);
        let me = me();
        let rat = target(100, "hostile");
        let mut inv = Inventory::default();
        let at_ten = |inv: &Inventory, skill| Env { inventory: Some(inv), ..env(&g, &me, Some(&rat), 11.0) }.why_not(skill, 0.0);
        // no sword in the hand, no bow: fists reach 3 m
        assert_eq!(at_ten(&inv, 1), Some(fail(code::URK_EQUIP_SLOT)));
        assert_eq!(at_ten(&inv, 2), Some(fail(code::TOO_FAR_RANGE)));
        inv.equipment.slots[slot::MAINHAND] = Some(aa_net::model::Item { template_id: 100, ..Default::default() });
        inv.equipment.slots[slot::RANGED] = Some(aa_net::model::Item { template_id: 200, ..Default::default() });
        // the sword is worn (at ten metres the target is out of reach, but the sword is not missing); the bow's 20 m reach it
        assert_eq!(at_ten(&inv, 1), Some(fail(code::TOO_FAR_RANGE)));
        assert_eq!(at_ten(&inv, 2), None);
        // the mana of a skill grows with the level of its ability: ((level - 1) * 1.6 + 8) * 3 / 3.65 + cost
        let mut m = me.clone();
        assert_eq!(env(&g, &m, None, 0.0).mana_cost(&spell), 16);
        m.ability_exp[1] = 400;
        assert_eq!(env(&g, &m, None, 0.0).mana_cost(&spell), 17);
        m.mp = 16;
        assert_eq!(env(&g, &m, None, 0.0).why_not(3, 0.0), Some(fail(code::LACK_MANA)));
    }

    #[test]
    fn items_and_cooldowns() {
        let mut g = game(Rules::default().with_skill(1, melee()).with_skill(2, SkillRules { target_type: 0, source_alive: true, ..SkillRules::default() }));
        g.items.insert(5, crate::hud::data::ItemInfo { use_skill: 1, ..Default::default() });
        g.items.insert(6, crate::hud::data::ItemInfo { use_skill: 2, ..Default::default() });
        g.skills.insert(2, crate::hud::data::SkillInfo { cooldown_ms: 30_000, gcd_ms: 1000, ..Default::default() });
        g.skills.insert(1, crate::hud::data::SkillInfo { cooldown_ms: 0, gcd_ms: 1000, ..Default::default() });
        let me = me();
        let e = env(&g, &me, None, 0.0);
        let slot_of = |kind, id, count| SlotSnap { kind, id, count, ..SlotSnap::default() };
        // an empty stack is dark; an item whose skill needs a target is dark without one
        assert_eq!(e.slot_blocked(&slot_of("item", 6, 0)), Some(code::NEED_REAGENT));
        assert_eq!(e.slot_blocked(&slot_of("item", 6, 3)), None);
        assert_eq!(e.slot_blocked(&slot_of("item", 5, 3)), Some(code::NO_TARGET));
        assert_eq!(e.slot_blocked(&slot_of("skill", 1, 0)), Some(code::NO_TARGET));
        assert_eq!(e.precheck_item(5).map(|(skill, f)| (skill, f.result)), Some((1, code::NO_TARGET)));
        assert_eq!(e.precheck_item(6), None);
        // the own cooldown refuses the click but does not darken the slot (its sweep shows it); the global cooldown is silent
        let mut skills = Skills { obj_id: 5, ..Skills::default() };
        skills.last_used.insert(2, Instant::now());
        skills.last_used.insert(1, Instant::now());
        let e = Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) };
        assert_eq!(e.slot_blocked(&slot_of("skill", 2, 0)), None);
        assert_eq!(e.precheck(2), Some(fail(code::COOLDOWN_TIME)));
        assert_eq!(e.cooldown(1), None);
        // a cooldown as short as the global one is still the skill's own
        g.skills.insert(1, crate::hud::data::SkillInfo { cooldown_ms: 1000, gcd_ms: 1000, ..Default::default() });
        let e = Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) };
        assert_eq!(e.cooldown(1), Some(fail(code::COOLDOWN_TIME)));
    }

    fn gcd_game() -> GameData {
        let mut g = game(Rules::default().with_skill(1, SkillRules::default()).with_skill(2, SkillRules::default()).with_skill(3, SkillRules::default()));
        g.skills.insert(1, crate::hud::data::SkillInfo { gcd_ms: 1000, ..Default::default() });
        g.skills.insert(2, crate::hud::data::SkillInfo { gcd_ms: 1000, ..Default::default() });
        g.skills.insert(3, crate::hud::data::SkillInfo { gcd_ms: 1000, ignore_gcd: true, ..Default::default() });
        g
    }

    /// The player cast skill 1 `ago` ago.
    fn cast_ago(ago: Duration) -> Skills {
        Skills { obj_id: 5, last_cast: Some(Instant::now() - ago), last_cast_skill: 1, ..Skills::default() }
    }

    #[test]
    fn the_global_cooldown_counts_from_the_last_cast_unless_ignored() {
        let g = gcd_game();
        let me = me();
        let left = |skills: &Skills, skill| Env { skills: Some(skills), ..env(&g, &me, None, 0.0) }.gcd_left(skill);
        let skills = cast_ago(Duration::from_millis(200));
        assert!((700..=800).contains(&left(&skills, 2)), "{}", left(&skills, 2));
        assert_eq!(left(&skills, 3), 0, "an ignore_gcd skill does not wait");
        assert_eq!(left(&skills, 99), 0, "an unknown skill does not wait");
        assert_eq!(left(&cast_ago(Duration::from_secs(2)), 2), 0);
        assert_eq!(left(&Skills::default(), 2), 0);
    }

    #[test]
    fn a_plot_steps_global_cooldown_holds_presses_back() {
        let g = gcd_game();
        let me = me();
        let flight = InFlight::default();
        // Triple Slash: the plot started 900 ms ago (its own 500 ms gcd is over), its finishing step set 2 s 300 ms ago
        let mut skills = Skills { obj_id: 5, last_cast: Some(Instant::now() - Duration::from_millis(900)), last_cast_skill: 1, ..Skills::default() };
        skills.plot_gcd = Some((Instant::now() - Duration::from_millis(300), 2000));
        let e = Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) };
        assert!((1600..=1700).contains(&e.gcd_left(2)), "{}", e.gcd_left(2));
        assert!(e.held_back(2, &flight));
        assert!(!e.held_back(3, &flight), "an ignore_gcd skill does not wait");
        skills.plot_gcd = Some((Instant::now() - Duration::from_millis(1950), 2000));
        assert!(!Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) }.held_back(2, &flight));
    }

    #[test]
    fn the_plot_global_cooldowns_come_from_the_database() -> Result<(), Box<dyn std::error::Error>> {
        if !std::path::Path::new(aa_anim::db::DEFAULT_DB).exists() {
            return Ok(());
        }
        let game = crate::hud::data::GameData::load(aa_anim::db::DEFAULT_DB)?;
        // Triple Slash (plot 1528): the finishing step 12148 carries GlobalCooldown 2000
        assert_eq!(game.plot_gcd.get(&12148), Some(&2000));
        assert!(game.plot_gcd.len() > 10, "{}", game.plot_gcd.len());
        Ok(())
    }

    #[test]
    fn presses_during_the_global_cooldown_are_dropped_quietly() {
        let g = gcd_game();
        let me = me();
        let flight = InFlight::default();
        let held = |skills: &Skills, skill| Env { skills: Some(skills), ..env(&g, &me, None, 0.0) }.held_back(skill, &flight);
        assert!(held(&cast_ago(Duration::from_millis(300)), 2));
        // the last 100 ms are the server's to judge, and an ignore_gcd skill never waits
        assert!(!held(&cast_ago(Duration::from_millis(950)), 2));
        assert!(!held(&cast_ago(Duration::from_millis(300)), 3));
        assert!(!held(&Skills::default(), 2));
        // the gcd is no reason to refuse with an alert
        assert_eq!(Env { skills: Some(&cast_ago(Duration::from_millis(300))), ..env(&g, &me, None, 0.0) }.precheck(2), None);
    }

    #[test]
    fn the_skill_being_cast_is_not_sent_again() {
        let g = gcd_game();
        let me = me();
        let now = Instant::now();
        let mut skills = Skills { obj_id: 5, ..Skills::default() };
        let flight = InFlight::default();
        let mut held = |c: Option<Casting>, skill| {
            skills.casting.clear();
            if let Some(c) = c {
                skills.casting.insert(5, c);
            }
            Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) }.held_back(skill, &flight)
        };
        assert!(held(Some(casting(3, 5000, now, None, 0)), 3), "the cast itself");
        assert!(!held(Some(casting(3, 5000, now, None, 0)), 2), "another skill is the alert's business ([`Env::busy`])");
        assert!(held(Some(casting(3, 0, now, Some(now), 4000)), 3), "the channel itself");
        assert!(!held(Some(casting(3, 0, now, None, 0)), 3), "an instant skill is no cast");
        assert!(!held(Some(casting(3, 1000, now - Duration::from_secs(10), None, 0)), 3), "a cast whose end never came");
        assert!(!held(None, 3));
    }

    #[test]
    fn a_press_waits_for_the_servers_answer() {
        let g = gcd_game();
        let me = me();
        let t0 = Instant::now() - Duration::from_millis(100);
        let mut flight = InFlight::default();
        let mut skills = Skills { obj_id: 5, ..Skills::default() };
        assert!(!flight.pending(&skills, Instant::now()));
        flight.sent(t0);
        assert!(flight.pending(&skills, Instant::now()));
        let held = |flight: &InFlight, skills: &Skills, skill| Env { skills: Some(skills), ..env(&g, &me, None, 0.0) }.held_back(skill, flight);
        assert!(held(&flight, &skills, 2));
        assert!(!held(&flight, &skills, 3), "an ignore_gcd skill goes through");
        // the reply: the player's skill started (this also starts the gcd), or the server refused it
        let mut started = skills.clone();
        started.last_cast = Some(Instant::now());
        assert!(!flight.pending(&started, Instant::now()));
        skills.last_failure = Some(aa_net::state::SkillFailure { skill_id: 2, result: code::COOLDOWN_TIME, value: 0, at: Instant::now() });
        assert!(!flight.pending(&skills, Instant::now()));
        // a cast from before the press is no answer; no answer at all does not block for ever
        skills.last_failure = None;
        skills.last_cast = Some(t0 - Duration::from_secs(1));
        assert!(flight.pending(&skills, Instant::now()));
        assert!(!flight.pending(&skills, t0 + Duration::from_millis(FLIGHT_MS)));
    }

    #[test]
    fn received_own_cooldowns_refuse_clicks_independently_of_db_and_gcd_duration() {
        let mut g = game(Rules::default().with_skill(1, SkillRules::default()));
        g.skills.insert(1, crate::hud::data::SkillInfo { tag: 9, tags: vec![9], gcd_ms: 1000, ..Default::default() });
        let me = me();
        let mut skills = Skills::default();
        skills.native_skill_cooldowns.push(aa_net::state::NativeCooldown {
            cooldown: aa_net::Cooldown { id: 1, total_ms: 0, remaining_ms: 30_000 },
            updated_at: Instant::now(), origin: aa_net::state::NativeCooldownOrigin::Snapshot, reset: false,
        });
        let check = |skills: &Skills| Env { skills: Some(skills), ..env(&g, &me, None, 0.0) }.precheck(1);
        assert_eq!(check(&skills), Some(fail(code::COOLDOWN_TIME)));
        // Native sweep conveys a cooldown; it does not introduce a separate dark-slot reason.
        let slot = SlotSnap { kind: "skill", id: 1, ..Default::default() };
        assert_eq!(Env { skills: Some(&skills), ..env(&g, &me, None, 0.0) }.slot_blocked(&slot), None);
        skills.native_skill_cooldowns[0].reset = true;
        assert_eq!(check(&skills), None);
        skills.native_tag_cooldowns.push(aa_net::state::NativeCooldown {
            cooldown: aa_net::Cooldown { id: 9, total_ms: 500, remaining_ms: 30_000 },
            updated_at: Instant::now(), origin: aa_net::state::NativeCooldownOrigin::Snapshot, reset: false,
        });
        assert_eq!(check(&skills), Some(fail(code::COOLDOWN_TIME)));
        skills.native_tag_cooldowns[0].updated_at = Instant::now() - Duration::from_secs(31);
        assert_eq!(check(&skills), Some(fail(code::COOLDOWN_TIME)), "queries read frame-stored remaining time");
        skills.advance_native_frame(31_000);
        assert_eq!(check(&skills), None);
    }

    /// The rules as the game database has them (skipped where the database is not installed).
    #[test]
    fn real_database_rules() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(conn) = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
        let rules = Rules::load(&conn).unwrap();
        // Triple Slash: 30 mana, a hostile target, 4 m
        let r = &rules.skills[&18132];
        assert_eq!((r.mana, r.target_type, r.max_range, r.damage_type), (30.0, 4, 4.0, 1));
        // Melee Attack reaches as far as the weapon in the main hand; Dash does not work in a fight
        assert_eq!(rules.skills[&2].range_slot, 15);
        assert!(rules.skills[&16287].reqs.iter().any(|q| q.kind == 12));
        assert!(rules.buffs.values().any(|b| b.stun) && rules.buffs.values().any(|b| b.silence));
        assert!(rules.weapons.values().any(|w| w.max_range >= 20.0));
    }

    #[test]
    fn names_follow_the_numbers() {
        assert_eq!(result_name(code::LACK_MANA), "LACK_MANA");
        assert_eq!(result_name(code::TOO_FAR_RANGE), "TOO_FAR_RANGE");
        assert_eq!(result_name(code::NO_TARGET), "NO_TARGET");
        assert_eq!(result_name(code::URK_COMBAT), "URK_COMBAT");
        assert_eq!(result_name(code::URK_EQUIP_RANGED), "URK_EQUIP_RANGED");
        assert_eq!(result_name(code::CANNOT_CAST_IN_STUN), "CANNOT_CAST_IN_STUN");
        assert_eq!(result_name(code::INVALID_TARGET), "INVALID_TARGET");
        assert_eq!(result_name(200), "FAILURE");
    }
}
