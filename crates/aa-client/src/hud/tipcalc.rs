//! The numbers in skill and buff descriptions.
//!
//! The descriptions in the database carry `#{name}` placeholders the client's C++ fills when a tooltip is
//! built (the names are the strings of `x2game.dll`): `min_damage` / `max_damage` / `avg_damage`, the same for
//! `heal`, `mana_burn`, `restore_mana`, their `total_*` over a buff's whole duration, the `buff_*` ones of
//! the buff a skill applies, `buff_ua_*` / `ua_*` (what the buff changes), `buff_duration`, `duration`,
//! `area_radius`, `damage_multiplier`, `charge_multiplier`, `buff_tooltip`.
//!
//! They come from the effect rows the skill points to and from the player: the damage and heal formulas are
//! the server's (`systems/skills/effects/combat.rs`), over the player's attack numbers ([`Attrs`], from level,
//! gear and buffs, the way `systems/skills/stats.rs` evaluates them). Which effect a placeholder means:
//!
//! * the skill's `tooltip_skill_effects` entry of that kind, else the first effect of the kind that applies at the
//!   player's level (`skill_effects.start_level..end_level`, the skill's effects in table order, then the effects
//!   of its plot);
//! * `buff_*`: the buffs the skill applies, the ones meant for friends only first, then in table order;
//!   `buff_duration`, `buff_duration2`, `buff_duration3` are the first three that last for any time (a channeled
//!   skill without such a buff has its channeling time).
//!
//! Numbers print as the client's `number_value_format` (orange, `ui_texts` 4012), durations as `%d#$second` /
//! `%.1f#$second` (the tooltip script turns `#$second` into "sec"). The damage breakdown after a number
//! (`detail_melee_damage` ..: "(63 + 117% Melee Attack)") is an option of the real client
//! ("Show Damage Breakdowns in Skill Tooltips"), off unless [`Attrs::details`] says otherwise.

use std::collections::HashMap;

use rusqlite::Connection;

use super::data::{any_flag, any_int};
use super::formula::Formula;
use super::tipitem::ItemTables;

mod charstats;
pub use charstats::PlayerSheet;

/// What an effect row is (`effects.actual_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Damage,
    Heal,
    Buff,
    ManaBurn,
    Restore,
}

impl Kind {
    fn of(actual_type: &str) -> Option<Kind> {
        Some(match actual_type {
            "DamageEffect" => Kind::Damage,
            "HealEffect" => Kind::Heal,
            "BuffEffect" => Kind::Buff,
            "ManaBurnEffect" => Kind::ManaBurn,
            "RestoreManaEffect" => Kind::Restore,
            _ => return None,
        })
    }
}

/// The effect kind a placeholder's second half names (`damage`, `heal`, `mana_burn`, `restore_mana`).
fn kind_of(what: &str) -> Option<Kind> {
    Some(match what {
        "damage" => Kind::Damage,
        "heal" => Kind::Heal,
        "mana_burn" => Kind::ManaBurn,
        "restore_mana" => Kind::Restore,
        _ => return None,
    })
}

/// An effect: the typed row (`damage_effects.id` ...).
#[derive(Clone, Copy, Debug)]
struct Fx {
    kind: Kind,
    id: u32,
}

/// An effect of a skill with the levels it applies at and who it is for.
#[derive(Clone, Copy, Debug)]
struct SkillEffect {
    fx: Fx,
    start: i32,
    end: i32,
    friendly: bool,
    hostile: bool,
}

#[derive(Clone, Debug, Default)]
struct SkillFx {
    ability: usize,
    ability_level: i32,
    level_step: i32,
    casting_ms: i32,
    channeling_ms: i32,
    radius: i32,
    mana_cost: i32,
    mana_level_md: f32,
    effects: Vec<SkillEffect>,
    tooltip: Vec<Fx>,
}

#[derive(Clone, Debug, Default)]
struct DamageFx {
    /// 1 melee, 2 magic, 3 siege, 4 ranged.
    damage_type: i32,
    fixed: Option<(i32, i32)>,
    multiplier: f32,
    mainhand: bool,
    offhand: bool,
    ranged: bool,
    level: Option<(f32, i32, i32)>,
    charged: Option<f32>,
    charged_level: f32,
    dps_multiplier: f32,
    dps_inc_multiplier: f32,
    weapon_slot: i32,
}

#[derive(Clone, Debug, Default)]
struct HealFx {
    level: Option<(f32, i32, i32)>,
    dps_multiplier: f32,
    fixed: Option<(i32, i32)>,
}

#[derive(Clone, Debug, Default)]
struct BurnFx {
    base: (i32, i32),
    level: (f32, i32, i32),
}

#[derive(Clone, Debug, Default)]
struct RestoreFx {
    fixed: Option<(i32, i32)>,
    level: Option<(f32, i32, i32)>,
}

#[derive(Clone, Debug, Default)]
struct BuffFx {
    duration: i32,
    tick: i32,
    level_duration: i32,
    /// What it does every tick, and when something happens to it (`buff_triggers`: when it ends ...).
    ticks: Vec<Fx>,
    triggers: Vec<Fx>,
}

impl BuffFx {
    /// `BuffTemplate.GetDuration(abLevel)`, ms.
    fn duration(&self, ab_level: i32) -> i32 {
        self.level_duration.wrapping_mul(ab_level).wrapping_add(self.duration).max(0)
    }
}

/// A buff's bonus (`unit_modifiers`, owner `Buff` or `Item`).
#[derive(Clone, Copy, Debug)]
pub struct Mod {
    pub attr: u8,
    pub percent: bool,
    pub value: i32,
    /// Added per ability level / 100.
    pub per_level: i32,
}

impl Mod {
    /// The value at an ability level (the server's `Bonus.Value`).
    pub fn at(&self, ab_level: i32) -> i32 {
        (f64::from(self.value) + f64::from(self.per_level) * f64::from(ab_level as f32 / 100.0)).round_ties_even() as i32
    }
}

/// The numbers of a skill's min / max damage (or heal).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amount {
    pub min: f64,
    pub max: f64,
    /// The part that does not depend on the attack stat (level based) and the weapon's part in percent: the
    /// `$1` and `$2` of the damage breakdown.
    pub flat: f64,
    pub percent: f64,
}

impl Amount {
    pub fn avg(&self) -> f64 {
        (self.min + self.max) / 2.0
    }

    fn scaled(self, k: f64) -> Amount {
        Amount { min: self.min * k, max: self.max * k, flat: self.flat * k, percent: self.percent * k }
    }
}

/// The player's attack numbers: what the damage and heal formulas read of the caster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attrs {
    pub level: i32,
    /// Character attribute 91: native 60,000 ms base with the worn-item and buff modifiers.
    pub lung_capacity_ms: u32,
    /// Worn-item/buff modifiers retained for native proficiency unit-attribute lookups.
    pub(super) modifiers: Vec<(u8, bool, i32)>,
    /// `str, dex, sta, int, spi, fai`
    pub primaries: [i32; 6],
    /// `LevelDps` of the character formulas.
    pub level_dps: f64,
    /// Weapon damage per second in 1/1000: main hand, off hand, ranged, spell, heal.
    pub dps: [i32; 5],
    /// The attack bonuses in 1/1000: melee, ranged, spell, heal.
    pub dps_inc: [i32; 4],
    /// Damage multipliers of melee, ranged, spell and heal (1.0 = none).
    pub damage_mul: [f32; 4],
    /// `holdables.damage_scale` of the main hand, off hand and ranged weapon (0 for none).
    pub scale: [i32; 3],
    /// The level of every ability (index = ability id).
    pub abilities: [i32; 11],
    /// Show the damage breakdown after the numbers.
    pub details: bool,
    /// The zone and name of the recall point the character is bound to (`return_point`), when known.
    pub return_point: Option<(String, String)>,
}

/// `equipment slot` (aa-net index) the damage formulas look at.
const SLOT_MAINHAND: i32 = 15;
const SLOT_OFFHAND: i32 = 16;
const SLOT_RANGED: i32 = 17;

/// `UnitFormulaKind`.
mod formula_kind {
    pub const LEVEL_DPS: u8 = 12;
    pub const MELEE_DPS_INC: u8 = 21;
    pub const STR: u8 = 22;
    pub const RANGED_DPS_INC: u8 = 28;
    pub const SPELL_DPS_INC: u8 = 29;
    pub const HEAL_DPS_INC: u8 = 43;
}

/// `UnitAttribute`.
mod attr {
    pub const MELEE_DPS_INC: u8 = 33;
    pub const RANGED_DPS_INC: u8 = 34;
    pub const SPELL_DPS_INC: u8 = 35;
    pub const MELEE_DAMAGE_MUL: u8 = 51;
    pub const RANGED_DAMAGE_MUL: u8 = 52;
    pub const SPELL_DAMAGE_MUL: u8 = 53;
    pub const SPELL_DPS: u8 = 87;
    pub const MAINHAND_DPS: u8 = 96;
    pub const OFFHAND_DPS: u8 = 97;
    pub const RANGED_DPS: u8 = 98;
    pub const HEAL_MUL: u8 = 120;
    pub const HEAL_DPS: u8 = 173;
    pub const HEAL_DPS_INC: u8 = 175;
}

/// `CalculateWithBonuses`: the flat bonuses, then each percent bonus on the running value.
pub fn with_bonuses(value: f64, attr: u8, mods: &[(u8, bool, i32)]) -> f64 {
    let mut v = value;
    for (_, _, value) in mods.iter().filter(|m| m.0 == attr && !m.1) {
        v += f64::from(*value);
    }
    for (_, _, value) in mods.iter().filter(|m| m.0 == attr && m.1) {
        v += v * f64::from(*value) / 100.0;
    }
    v
}

/// The `ui_texts` the damage breakdown uses: melee, ranged, spell, heal, then the number format.
const UI_TEXTS: [u32; 6] = [4009, 4010, 4011, 4020, 4012, 1991];
const UI_DEFAULTS: [&str; 6] = [
    "|cFF83DD5F ($1 + $2% Melee Attack)|r",
    "|cFF83DD5F ($1 + $2% Ranged Attack)|r",
    "|cFF83DD5F ($1 + $2% Magic Attack)|r",
    "|cFF83DD5F ($1 + $2% Healing Power)|r",
    "|cFFFF9C27%s|r",
    "Recorded Location: $1: $2",
];

#[derive(Default)]
pub struct TipTables {
    pub items: ItemTables,
    skills: HashMap<u32, SkillFx>,
    damage: HashMap<u32, DamageFx>,
    heal: HashMap<u32, HealFx>,
    burn: HashMap<u32, BurnFx>,
    restore: HashMap<u32, RestoreFx>,
    /// `buff_effects.id` -> `buffs.id`
    buff_effects: HashMap<u32, u32>,
    buffs: HashMap<u32, BuffFx>,
    mods: HashMap<u32, Vec<Mod>>,
    formulas: HashMap<u8, Formula>,
    skill_desc: HashMap<u32, String>,
    buff_desc: HashMap<u32, String>,
    ui: [String; 6],
}

fn int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    any_int(r, i)
}

fn real(r: &rusqlite::Row, i: usize) -> rusqlite::Result<f32> {
    Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32)
}

/// The level part of a heal / damage / mana burn row: `(level_md, level_va_start, level_va_end)`.
fn level_part(r: &rusqlite::Row, i: usize) -> rusqlite::Result<(f32, i32, i32)> {
    Ok((real(r, i)?, int(r, i + 1)? as i32, int(r, i + 2)? as i32))
}

/// The server's `level_range`: the damage range of a level based part (`lvl_md` is `level_dps * level_md`).
fn level_range(lvl_md: f32, rank: i32, va_start: i32, va_end: i32) -> (f32, f32) {
    let modifier = ((rank - 1) / 49 * (va_end - va_start) + va_start) as f32 * 0.01;
    (lvl_md - modifier * lvl_md + 0.5, (modifier + 1.0) * lvl_md + 0.5)
}

impl TipTables {
    pub fn load(conn: &Connection, text: &HashMap<(&'static str, u32), String>) -> rusqlite::Result<TipTables> {
        let mut t = TipTables { items: ItemTables::default(), ..TipTables::default() };
        for (i, d) in UI_DEFAULTS.iter().enumerate() {
            t.ui[i] = (*d).to_string();
        }
        let mut set_names: HashMap<u32, String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, idx, en_us FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND \
                 ((tbl_name = 'ui_texts' AND idx IN (4009, 4010, 4011, 4012, 4020, 1991)) OR (tbl_name = 'equip_item_sets' AND tbl_column_name = 'name'))",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let table: String = r.get(0)?;
                let idx = int(r, 1)? as u32;
                let value: String = r.get(2)?;
                if table == "ui_texts" {
                    if let Some(at) = UI_TEXTS.iter().position(|i| *i == idx) {
                        t.ui[at] = value;
                    }
                } else {
                    set_names.insert(idx, value);
                }
            }
        }
        t.items = ItemTables::load(conn, &set_names)?;
        for ((table, id), desc) in text {
            match *table {
                "skills_desc" if desc.contains("#{") => t.skill_desc.insert(*id, desc.clone()),
                "buffs_desc" => t.buff_desc.insert(*id, desc.clone()),
                _ => None,
            };
        }
        // the effect rows
        let mut effects: HashMap<u32, Fx> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, actual_id, actual_type FROM effects")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(kind) = r.get::<_, Option<String>>(2)?.as_deref().and_then(Kind::of) {
                    effects.insert(int(r, 0)? as u32, Fx { kind, id: int(r, 1)? as u32 });
                }
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, damage_type_id, fixed_min, fixed_max, multiplier, use_mainhand_weapon, use_offhand_weapon, use_ranged_weapon, \
                 use_fixed_damage, use_level_damage, level_md, level_va_start, level_va_end, use_charged_buff, charged_mul, charged_level_mul, \
                 dps_multiplier, weapon_slot_id, dps_inc_multiplier FROM damage_effects",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.damage.insert(
                    int(r, 0)? as u32,
                    DamageFx {
                        damage_type: int(r, 1)? as i32,
                        fixed: any_flag(r, 8)?.then_some((int(r, 2)? as i32, int(r, 3)? as i32)),
                        multiplier: real(r, 4)?,
                        mainhand: any_flag(r, 5)?,
                        offhand: any_flag(r, 6)?,
                        ranged: any_flag(r, 7)?,
                        level: any_flag(r, 9)?.then_some(level_part(r, 10)?),
                        charged: any_flag(r, 13)?.then_some(real(r, 14)?),
                        charged_level: real(r, 15)?,
                        dps_multiplier: real(r, 16)?,
                        weapon_slot: int(r, 17)? as i32,
                        dps_inc_multiplier: real(r, 18)?,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, use_level_heal, level_md, level_va_start, level_va_end, dps_multiplier, use_fixed_heal, fixed_min, fixed_max FROM heal_effects")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.heal.insert(
                    int(r, 0)? as u32,
                    HealFx {
                        level: any_flag(r, 1)?.then_some(level_part(r, 2)?),
                        dps_multiplier: real(r, 5)?,
                        fixed: any_flag(r, 6)?.then_some((int(r, 7)? as i32, int(r, 8)? as i32)),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, base_min, base_max, level_md, level_va_start, level_va_end FROM mana_burn_effects")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.burn.insert(int(r, 0)? as u32, BurnFx { base: (int(r, 1)? as i32, int(r, 2)? as i32), level: level_part(r, 3)? });
            }
        }
        {
            let mut q = conn.prepare("SELECT id, use_fixed_value, fixed_min, fixed_max, use_level_value, level_md, level_va_start, level_va_end FROM restore_mana_effects")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.restore.insert(
                    int(r, 0)? as u32,
                    RestoreFx {
                        fixed: any_flag(r, 1)?.then_some((int(r, 2)? as i32, int(r, 3)? as i32)),
                        level: any_flag(r, 4)?.then_some(level_part(r, 5)?),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, buff_id FROM buff_effects")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.buff_effects.insert(int(r, 0)? as u32, int(r, 1)? as u32);
            }
        }
        {
            let mut q = conn.prepare("SELECT id, duration, tick, level_duration FROM buffs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.buffs.insert(
                    int(r, 0)? as u32,
                    BuffFx { duration: int(r, 1)? as i32, tick: int(r, 2)? as i32, level_duration: int(r, 3)? as i32, ..BuffFx::default() },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT buff_id, effect_id FROM buff_tick_effects ORDER BY rowid")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let (Some(b), Some(fx)) = (t.buffs.get_mut(&(int(r, 0)? as u32)), effects.get(&(int(r, 1)? as u32))) {
                    b.ticks.push(*fx);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT buff_id, effect_id FROM buff_triggers ORDER BY rowid")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let (Some(b), Some(fx)) = (t.buffs.get_mut(&(int(r, 0)? as u32)), effects.get(&(int(r, 1)? as u32))) {
                    b.triggers.push(*fx);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT owner_id, unit_attribute_id, unit_modifier_type_id, value, linear_level_bonus FROM unit_modifiers WHERE owner_type = 'Buff'")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.mods.entry(int(r, 0)? as u32).or_default().push(Mod {
                    attr: int(r, 1)? as u8,
                    percent: int(r, 2)? == 1,
                    value: int(r, 3)? as i32,
                    per_level: int(r, 4)? as i32,
                });
            }
        }
        {
            let mut q = conn.prepare("SELECT kind_id, formula FROM unit_formulas WHERE owner_type_id = 0")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(f) = r.get::<_, Option<String>>(1)?.and_then(|s| Formula::parse(&s).ok()) {
                    t.formulas.insert(int(r, 0)? as u8, f);
                }
            }
        }
        let mut plots: HashMap<u32, u32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, ability_id, ability_level, level_step, casting_time, channeling_time, target_area_radius, plot_id, mana_cost, mana_level_md FROM skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                t.skills.insert(
                    id,
                    SkillFx {
                        ability: int(r, 1)?.clamp(0, 10) as usize,
                        ability_level: int(r, 2)? as i32,
                        level_step: int(r, 3)? as i32,
                        casting_ms: int(r, 4)? as i32,
                        channeling_ms: int(r, 5)? as i32,
                        radius: int(r, 6)? as i32,
                        mana_cost: int(r, 8)? as i32,
                        mana_level_md: real(r, 9)?,
                        ..SkillFx::default()
                    },
                );
                if int(r, 7)? > 0 {
                    plots.insert(id, int(r, 7)? as u32);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT skill_id, effect_id, start_level, end_level, friendly, non_friendly FROM skill_effects ORDER BY rowid")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let (Some(s), Some(fx)) = (t.skills.get_mut(&(int(r, 0)? as u32)), effects.get(&(int(r, 1)? as u32))) {
                    s.effects.push(SkillEffect { fx: *fx, start: int(r, 2)? as i32, end: int(r, 3)? as i32, friendly: any_flag(r, 4)?, hostile: any_flag(r, 5)? });
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT skill_id, effect_id FROM tooltip_skill_effects ORDER BY rowid")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let (Some(s), Some(fx)) = (t.skills.get_mut(&(int(r, 0)? as u32)), effects.get(&(int(r, 1)? as u32))) {
                    s.tooltip.push(*fx);
                }
            }
        }
        {
            // the effects of a skill's plot, in the order the plot runs them
            let mut by_plot: HashMap<u32, Vec<Fx>> = HashMap::new();
            let mut q = conn.prepare(
                "SELECT ev.plot_id, pe.actual_type, pe.actual_id FROM plot_events ev JOIN plot_effects pe ON pe.event_id = ev.id \
                 WHERE pe.actual_type IN ('DamageEffect', 'HealEffect', 'BuffEffect', 'ManaBurnEffect', 'RestoreManaEffect') ORDER BY ev.plot_id, ev.position, pe.position",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(kind) = r.get::<_, Option<String>>(1)?.as_deref().and_then(Kind::of) {
                    by_plot.entry(int(r, 0)? as u32).or_default().push(Fx { kind, id: int(r, 2)? as u32 });
                }
            }
            for (skill, plot) in plots {
                if let (Some(s), Some(list)) = (t.skills.get_mut(&skill), by_plot.get(&plot)) {
                    s.effects.extend(list.iter().map(|fx| SkillEffect { fx: *fx, start: i32::MIN, end: i32::MAX, friendly: false, hostile: false }));
                }
            }
        }
        Ok(t)
    }

    /// The player's attack numbers at `level` wearing `equipment` with `buffs` (buff id, ability level).
    pub fn attrs(&self, level: u8, equipment: &aa_net::model::Equipment, buffs: &[(u32, i16)], abilities: &[i32; 11]) -> Attrs {
        use aa_net::model::{slot, ItemDetail};
        let level = i32::from(level.max(1));
        // what is worn: stats, bonuses, weapons
        let mut stats = [0i32; 5];
        let mut mods: Vec<(u8, bool, i32)> = Vec::new();
        let mut weapon: [Option<(f64, f64, f64, i32)>; 3] = [None; 3];
        for (i, item) in equipment.slots.iter().enumerate() {
            let Some(item) = item.as_ref().filter(|_| !slot::is_body_part(i)) else { continue };
            let (durability, tp, tm) = match &item.detail {
                ItemDetail::Equip(d) => (Some(d.durability), d.temper_physical, d.temper_magical),
                _ => (None, 0, 0),
            };
            let grade = i32::from(item.grade);
            let broken = durability.is_some_and(|d| d == 0) && self.items.stats(item.template_id, grade, tp, tm).is_some_and(|s| s.max_durability > 0);
            if broken {
                continue;
            }
            if let Some(s) = self.items.stats(item.template_id, grade, tp, tm) {
                for (total, v) in stats.iter_mut().zip(s.stats) {
                    *total += v;
                }
                mods.extend(s.mods.iter().map(|m| (m.attr, m.percent, m.value)));
            }
            if let Some(w) = self.items.weapon_dps(item.template_id, grade, tp, tm) {
                match i {
                    slot::MAINHAND => weapon[0] = Some(w),
                    slot::OFFHAND => weapon[1] = Some(w),
                    slot::RANGED => weapon[2] = Some(w),
                    _ => {}
                }
            }
        }
        for (id, ab_level) in buffs {
            if let Some(list) = self.mods.get(id) {
                mods.extend(list.iter().map(|m| (m.attr, m.percent, m.at(i32::from(*ab_level)))));
            }
        }
        let formula = |kind: u8, vars: &HashMap<&str, f64>| self.formulas.get(&kind).map_or(0.0, |f| f.eval(vars));
        let level_vars = HashMap::from([("level", f64::from(level))]);
        let mut primaries = [0i32; 6];
        for (i, p) in primaries.iter_mut().enumerate() {
            let worn = if i < 5 { f64::from(stats[i]) } else { 0.0 };
            *p = with_bonuses(formula(formula_kind::STR + i as u8, &level_vars) + worn, i as u8, &mods) as i32;
        }
        let [str_, dex, _sta, int_, spi, fai] = primaries.map(f64::from);
        let vars = HashMap::from([
            ("level", f64::from(level)),
            ("str", str_),
            ("dex", dex),
            ("sta", f64::from(primaries[2])),
            ("int", int_),
            ("spi", spi),
            ("fai", fai),
        ]);
        let mut level_vars_ab = vars.clone();
        level_vars_ab.insert("ab_level", f64::from(level));
        let level_dps = formula(formula_kind::LEVEL_DPS, &level_vars_ab) as f32 as f64;
        // the weapon numbers in 1/1000, with the stat's share (a fifth) added
        let main = weapon[0].map_or((0.0f32, 0.0f32, 0.0f32), |w| (w.0 as f32, w.1 as f32, w.2 as f32));
        let per = |stat: f64| stat as f32 / 5.0 * 1000.0;
        let dps = with_bonuses(f64::from(main.0 * 1000.0 + per(str_)), attr::MAINHAND_DPS, &mods) as f32 as i32;
        let spell = with_bonuses(f64::from(main.1 * 1000.0 + per(int_)), attr::SPELL_DPS, &mods) as f32 as i32;
        let heal = with_bonuses(f64::from(main.2 * 1000.0) + f64::from(per(spi)), attr::HEAL_DPS, &mods) as i32;
        let offhand = (with_bonuses(weapon[1].map_or(0.0, |w| w.0), attr::OFFHAND_DPS, &mods) as f32 * 1000.0) as i32;
        let ranged = with_bonuses(f64::from(weapon[2].map_or(0.0, |w| w.0) as f32 * 1000.0 + per(dex)), attr::RANGED_DPS, &mods) as f32 as i32;
        let inc = |kind: u8, a: u8, vars: &HashMap<&str, f64>| with_bonuses(formula(kind, vars), a, &mods) as i32;
        let heal_vars = HashMap::from([("level", f64::from(level)), ("spi", spi)]);
        let mul = |a: u8| ((with_bonuses(0.0, a, &mods) + 1000.0) / 1000.0) as f32;
        Attrs {
            level,
            lung_capacity_ms: with_bonuses(60_000.0, 91, &mods) as u32,
            primaries,
            level_dps,
            dps: [dps, offhand, ranged, spell, heal],
            dps_inc: [
                inc(formula_kind::MELEE_DPS_INC, attr::MELEE_DPS_INC, &vars),
                inc(formula_kind::RANGED_DPS_INC, attr::RANGED_DPS_INC, &vars),
                inc(formula_kind::SPELL_DPS_INC, attr::SPELL_DPS_INC, &vars),
                inc(formula_kind::HEAL_DPS_INC, attr::HEAL_DPS_INC, &heal_vars),
            ],
            damage_mul: [mul(attr::MELEE_DAMAGE_MUL), mul(attr::RANGED_DAMAGE_MUL), mul(attr::SPELL_DAMAGE_MUL), mul(attr::HEAL_MUL)],
            scale: [0, 1, 2].map(|i| weapon[i].map_or(0, |w| w.3)),
            abilities: *abilities,
            details: false,
            return_point: None,
            modifiers: mods,
        }
    }

    // ---- amounts -----------------------------------------------------------------------------------------

    /// The damage of an effect at `rank` of a skill that casts for `casting_ms`; `tick` is the share of one
    /// tick of the buff it runs in (`tick / duration`), `None` for the whole.
    fn damage_amount(&self, e: &DamageFx, rank: i32, casting_ms: i32, a: &Attrs, tick: Option<f64>) -> Amount {
        if let Some((min, max)) = e.fixed {
            // the server sets the range after it scaled it to a tick: a fixed damage is the same every tick
            return Amount { min: f64::from(min), max: f64::from(max), flat: f64::from(min + max) / 2.0, percent: 0.0 };
        }
        let (mut level_min, mut level_max) = (0.0f32, 0.0f32);
        if let Some((md, start, end)) = e.level {
            let (lo, hi) = level_range(a.level_dps as f32 * md, rank, start, end);
            level_min += lo;
            level_max += hi;
        }
        // 1 melee, 2 magic, 4 ranged (3 siege has no attack stat of its own)
        let dps_inc = match e.damage_type {
            1 => a.dps_inc[0],
            2 => a.dps[3] + a.dps_inc[2],
            4 => a.dps_inc[1],
            _ => 0,
        };
        let mut max = dps_inc as f32 * 0.001 * e.dps_inc_multiplier;
        let mut weapon = 0.0f32;
        if e.mainhand {
            weapon = a.dps[0] as f32 * 0.001;
        }
        if e.offhand {
            weapon += a.dps[1] as f32 * 0.001;
        }
        if e.ranged {
            weapon += a.dps[2] as f32 * 0.001;
        }
        let weapon_part = e.dps_multiplier * weapon;
        max += weapon_part;
        let cast = if casting_ms <= 1000 { 1000.0 } else { casting_ms as f32 };
        let variable = max * cast * 0.001;
        let (min, max) = if e.weapon_slot < 0 {
            (variable + level_min, variable + level_max)
        } else {
            let scale = match e.weapon_slot {
                SLOT_MAINHAND => a.scale[0],
                SLOT_OFFHAND => a.scale[1],
                SLOT_RANGED => a.scale[2],
                _ => 0,
            };
            let scaled = scale as f32 * variable * 0.01;
            (level_min + (variable - scaled), level_max + (variable + scaled))
        };
        let (min, max) = (min * e.multiplier, max * e.multiplier);
        let mul = match e.damage_type {
            1 => a.damage_mul[0],
            2 => a.damage_mul[2],
            4 => a.damage_mul[1],
            _ => 1.0,
        };
        let mut out = Amount {
            min: f64::from((min * mul).floor()),
            max: f64::from((max * mul).ceil()),
            flat: f64::from((level_min + level_max) / 2.0 + dps_inc as f32 * 0.001 * e.dps_inc_multiplier * cast * 0.001) * f64::from(e.multiplier),
            percent: f64::from(e.dps_multiplier * cast * 0.001 * e.multiplier) * 100.0,
        };
        if let Some(k) = tick {
            out = out.scaled(k);
        }
        out
    }

    fn heal_amount(&self, e: &HealFx, rank: i32, casting_ms: i32, a: &Attrs, tick: Option<f64>) -> Amount {
        let k = tick.unwrap_or(1.0);
        let (mut level_min, mut level_max) = (0.0f32, 0.0f32);
        if let Some((md, start, end)) = e.level {
            let (lo, hi) = level_range(a.level_dps as f32 * md, rank, start, end);
            level_min += lo;
            level_max += hi;
        }
        let max = a.dps[4] as f32 * 0.001 * e.dps_multiplier;
        let cast = if casting_ms <= 1000 { 1000.0 } else { casting_ms as f32 };
        let variable = max * cast * 0.001;
        let (min, max) = (variable + level_min, variable + level_max);
        let mul = f64::from(a.damage_mul[3]);
        let mut out = Amount {
            min: (f64::from(min) * k * mul).floor(),
            max: (f64::from(max) * k * mul).floor(),
            flat: f64::from((level_min + level_max) / 2.0) * k,
            percent: f64::from(e.dps_multiplier * cast * 0.001) * 100.0 * k,
        };
        if let Some((lo, hi)) = e.fixed {
            out = Amount { min: f64::from(lo) * k, max: f64::from(hi) * k, flat: f64::from(lo + hi) / 2.0 * k, percent: 0.0 };
        }
        out
    }

    fn burn_amount(&self, e: &BurnFx, rank: i32, a: &Attrs, tick: Option<f64>) -> Amount {
        let (lo, hi) = level_range(a.level_dps as f32 * e.level.0, rank, e.level.1, e.level.2);
        let k = tick.unwrap_or(1.0);
        let (min, max) = ((f64::from(e.base.0) + f64::from(lo)) * k, (f64::from(e.base.1) + f64::from(hi)) * k);
        Amount { min, max, flat: (min + max) / 2.0, percent: 0.0 }
    }

    fn restore_amount(&self, e: &RestoreFx, rank_ab: i32, ability_level: i32, a: &Attrs) -> Amount {
        let (mut min, mut max) = (0i32, 0i32);
        if let Some((lo, hi)) = e.fixed {
            min += lo;
            max += hi;
        }
        if let Some((md, start, end)) = e.level {
            let unk = if rank_ab >= ability_level { 0.15 * (rank_ab - ability_level + 1) as f32 } else { 0.0 };
            let level_dps = a.level_dps as f32;
            let level_md = (unk + 1.0) * md;
            min += (level_dps * level_md + 0.5) as i32;
            max += ((((rank_ab - 1) as f32 * 0.020_408_163 * (end - start) as f32 + start as f32) * 0.01 + 1.0) * level_dps * level_md + 0.5) as i32;
        }
        Amount { min: f64::from(min), max: f64::from(max), flat: f64::from(min + max) / 2.0, percent: 0.0 }
    }

    /// The amount of `fx` (a damage, heal, mana burn or restore effect) in the skill's context.
    fn amount(&self, fx: Fx, ctx: &Ctx, tick: Option<f64>) -> Option<Amount> {
        let (rank, casting) = (ctx.rank, ctx.skill.map_or(0, |s| s.casting_ms));
        Some(match fx.kind {
            Kind::Damage => self.damage_amount(self.damage.get(&fx.id)?, rank, casting, ctx.attrs, tick),
            Kind::Heal => self.heal_amount(self.heal.get(&fx.id)?, rank, casting, ctx.attrs, tick),
            Kind::ManaBurn => self.burn_amount(self.burn.get(&fx.id)?, rank, ctx.attrs, tick),
            Kind::Restore => {
                let amount = self.restore_amount(self.restore.get(&fx.id)?, ctx.ab_level, ctx.skill.map_or(0, |s| s.ability_level), ctx.attrs);
                amount.scaled(tick.unwrap_or(1.0))
            }
            Kind::Buff => return None,
        })
    }

    // ---- which effect ------------------------------------------------------------------------------------

    /// The effect of `kind` the description speaks of (`ea` takes only the tooltip effects).
    fn pick(&self, skill: &SkillFx, kind: Kind, level: i32) -> Option<Fx> {
        skill
            .tooltip
            .iter()
            .find(|f| f.kind == kind)
            .copied()
            .or_else(|| skill.effects.iter().find(|e| e.fx.kind == kind && e.start <= level && level <= e.end).map(|e| e.fx))
            // none for this level (an effect of a skill that only starts at a higher one): the first there is
            .or_else(|| skill.effects.iter().find(|e| e.fx.kind == kind).map(|e| e.fx))
    }

    /// The buffs a skill applies in the order descriptions count them: those for friends only first.
    fn skill_buffs(&self, skill: &SkillFx, level: i32) -> Vec<u32> {
        let id = |e: &SkillEffect| self.buff_effects.get(&e.fx.id).copied();
        let mut out: Vec<u32> = Vec::new();
        // the effects that apply at this level, else (a skill whose buffs start at a higher one) all of them
        for any_level in [false, true] {
            let applies = |e: &&SkillEffect| e.fx.kind == Kind::Buff && (any_level || (e.start <= level && level <= e.end));
            for e in skill.effects.iter().filter(applies).filter(|e| e.friendly && !e.hostile) {
                out.extend(id(e));
            }
            for e in skill.effects.iter().filter(applies) {
                out.extend(id(e));
            }
            if !out.is_empty() {
                break;
            }
        }
        // a skill that applies its buff some other way lists it for the tooltip
        if out.is_empty() {
            out.extend(skill.tooltip.iter().filter(|f| f.kind == Kind::Buff).filter_map(|f| self.buff_effects.get(&f.id).copied()));
        }
        let mut seen = Vec::new();
        out.retain(|b| !seen.contains(b) && self.buffs.contains_key(b) && {
            seen.push(*b);
            true
        });
        out
    }

    // ---- placeholders ------------------------------------------------------------------------------------

    fn fmt_num(&self, v: f64) -> String {
        self.ui[4].replace("%s", &format!("{}", v.round() as i64))
    }

    fn fmt_value(&self, text: &str) -> String {
        self.ui[4].replace("%s", text)
    }

    /// `$1 + $2% Attack` after an amount.
    fn detail(&self, which: usize, amount: Amount, a: &Attrs) -> String {
        if !a.details {
            return String::new();
        }
        self.ui[which].replace("$1", &format!("{}", amount.flat.round() as i64)).replace("$2", &format!("{}", amount.percent.round() as i64))
    }

    /// `min_*` / `max_*` / `avg_*` of an amount, `what` the part after the prefix (`damage`, `heal`, `mana_burn`).
    fn amount_value(&self, part: &str, amount: Amount) -> Option<String> {
        let v = match part {
            "min" => amount.min,
            "max" => amount.max,
            "avg" => amount.avg(),
            _ => return None,
        };
        Some(self.fmt_num(v))
    }

    /// The text of one placeholder of a skill's description, `None` when it has no value.
    fn skill_value(&self, key: &str, ctx: &Ctx) -> Option<String> {
        let skill = ctx.skill?;
        let level = ctx.attrs.level;
        // prefixes: `buff_` (a buff the skill applies) and `ea_` (effects only the tooltip lists)
        if let Some(rest) = key.strip_prefix("buff_") {
            return self.skill_buff_value(rest, ctx);
        }
        if key.starts_with("ua_") || key == "avg_buff_damage" {
            return self.skill_buff_value(if key == "avg_buff_damage" { "total_avg_damage" } else { key }, ctx);
        }
        if key == "return_point" {
            // the native handler (`return_district_tooltip`: zone, then the point's name); empty without a bound point
            return Some(ctx.attrs.return_point.as_ref().map_or_else(String::new, |(zone, name)| self.ui[5].replace("$1", zone).replace("$2", name)));
        }
        let (rest, only_tooltip) = match key.strip_prefix("ea_") {
            Some(r) => (r, true),
            None => (key, false),
        };
        let pick = |kind: Kind| -> Option<Fx> {
            if only_tooltip { skill.tooltip.iter().find(|f| f.kind == kind).copied() } else { self.pick(skill, kind, level) }
        };
        match rest {
            "area_radius" => Some(format!("{}", skill.radius)),
            "damage_multiplier" => {
                let d = self.damage.get(&pick(Kind::Damage)?.id)?;
                Some(self.fmt_num(f64::from(d.dps_multiplier) * 100.0))
            }
            "charge_multiplier" => {
                // the damage effect that counts the stacks (the plot's, when the skill's own do not)
                let charged = |fx: &Fx| self.damage.get(&fx.id).filter(|d| d.charged.is_some());
                let d = pick(Kind::Damage).as_ref().and_then(charged).or_else(|| skill.effects.iter().filter(|e| e.fx.kind == Kind::Damage).find_map(|e| charged(&e.fx)))?;
                let m = d.charged? + ctx.rank as f32 * d.charged_level;
                Some(self.fmt_value(&trim(f64::from(m))))
            }
            "duration" => Some(self.fmt_value(&time(self.skill_duration(skill, ctx, 0)?))),
            "detail_melee_damage" | "detail_ranged_damage" | "detail_spell_damage" | "detail_siege_damage" => {
                let which = match rest {
                    "detail_melee_damage" => 0,
                    "detail_ranged_damage" => 1,
                    "detail_spell_damage" => 2,
                    _ => return Some(String::new()),
                };
                let amount = match pick(Kind::Damage) {
                    Some(fx) => self.amount(fx, ctx, None)?,
                    None => self.skill_buff_amount(skill, Kind::Damage, ctx)?,
                };
                Some(self.detail(which, amount, ctx.attrs))
            }
            "detail_heal_damage" => {
                let amount = match pick(Kind::Heal) {
                    Some(fx) => self.amount(fx, ctx, None)?,
                    None => self.skill_buff_amount(skill, Kind::Heal, ctx)?,
                };
                Some(self.detail(3, amount, ctx.attrs))
            }
            "total_min_damage" | "total_max_damage" | "total_avg_damage" => self.skill_buff_value(rest, ctx),
            _ => {
                let (part, what) = rest.split_once('_')?;
                let kind = kind_of(what)?;
                // an effect the skill's buff runs when it ends counts too
                let fx = pick(kind).or_else(|| self.buff_trigger_fx(skill, kind, level))?;
                let amount = self.amount(fx, ctx, None)?;
                self.amount_value(part, amount)
            }
        }
    }

    /// An effect of `kind` one of the skill's buffs runs on an event (when it ends ...).
    fn buff_trigger_fx(&self, skill: &SkillFx, kind: Kind, level: i32) -> Option<Fx> {
        self.skill_buffs(skill, level).into_iter().find_map(|b| self.buffs.get(&b)?.triggers.iter().find(|f| f.kind == kind).copied())
    }

    /// The whole of what the first buff of a skill with such a tick effect does over its duration.
    fn skill_buff_amount(&self, skill: &SkillFx, kind: Kind, ctx: &Ctx) -> Option<Amount> {
        self.skill_buffs(skill, ctx.attrs.level).into_iter().find_map(|id| {
            let b = self.buffs.get(&id)?;
            let fx = b.ticks.iter().find(|f| f.kind == kind)?;
            let duration = b.duration(ctx.ab_level);
            if duration > 0 && b.tick > 0 {
                let tick = self.amount(*fx, ctx, Some(f64::from(b.tick) / f64::from(duration)))?;
                Some(tick.scaled(f64::from(duration) / f64::from(b.tick)))
            } else {
                self.amount(*fx, ctx, None)
            }
        })
    }

    /// The duration of the buff a skill applies (`which` 0 first, 1 second ..), a channeled skill's own time
    /// when it has no lasting buff.
    fn skill_duration(&self, skill: &SkillFx, ctx: &Ctx, which: usize) -> Option<i32> {
        let lasting: Vec<i32> = self
            .skill_buffs(skill, ctx.attrs.level)
            .into_iter()
            .filter_map(|b| self.buffs.get(&b))
            .map(|b| b.duration(ctx.ab_level))
            .filter(|d| *d > 0)
            .collect();
        lasting.get(which).copied().or_else(|| (which == 0 && skill.channeling_ms > 0).then_some(skill.channeling_ms)).or_else(|| lasting.last().copied())
    }

    /// `buff_*` of a skill: `rest` is the key without the prefix.
    fn skill_buff_value(&self, rest: &str, ctx: &Ctx) -> Option<String> {
        let skill = ctx.skill?;
        match rest {
            "duration" => return Some(self.fmt_value(&time(self.skill_duration(skill, ctx, 0)?))),
            "duration2" => return Some(self.fmt_value(&time(self.skill_duration(skill, ctx, 1)?))),
            "duration3" => return Some(self.fmt_value(&time(self.skill_duration(skill, ctx, 2)?))),
            _ => {}
        }
        let buffs = self.skill_buffs(skill, ctx.attrs.level);
        if rest == "tooltip" {
            let desc = buffs.iter().find_map(|b| self.buff_text(*b, ctx.ab_level, ctx.attrs).filter(|d| !d.is_empty()))?;
            return Some(desc);
        }
        // the first buff the placeholder is about
        if let Some(v) = buffs.iter().find_map(|b| self.buff_value(*b, rest, ctx.ab_level, ctx.rank, ctx.skill, ctx.attrs)) {
            return Some(v);
        }
        // none of them has such a tick effect (a channeled skill's buff lasts as long as the channel): the
        // effect the tooltip lists, over the skill's own time
        let (total, rest) = match rest.strip_prefix("total_") {
            Some(r) => (true, r),
            None => (false, rest),
        };
        let (part, what) = rest.split_once('_')?;
        let fx = self.pick(skill, kind_of(what)?, ctx.attrs.level)?;
        let mut amount = self.amount(fx, ctx, None)?;
        if !total {
            let tick = buffs.iter().filter_map(|b| self.buffs.get(b)).map(|b| b.tick).find(|t| *t > 0)?;
            amount = amount.scaled(f64::from(tick) / f64::from(self.skill_duration(skill, ctx, 0)?.max(1)));
        }
        self.amount_value(part, amount)
    }

    /// One placeholder of a buff (`duration`, `tick`, `ua_*`, the amounts of its tick effects).
    fn buff_value(&self, buff: u32, key: &str, ab_level: i32, rank: i32, skill: Option<&SkillFx>, a: &Attrs) -> Option<String> {
        let b = self.buffs.get(&buff)?;
        let duration = b.duration(ab_level);
        // (a few descriptions call the duration `damage` or `buff_duration`, and a bonus by its attribute's name)
        let key = match key {
            "damage" | "buff_duration" => "duration",
            "avg_buff_damage" => "total_avg_damage",
            k if attr_id(k).is_some() => return self.buff_value(buff, &format!("ua_{k}"), ab_level, rank, skill, a),
            k => k,
        };
        match key {
            "duration" => return (duration > 0).then(|| self.fmt_value(&time(duration))),
            // "every #{tick} second(s)": the number of seconds
            "tick" => return (b.tick > 0).then(|| self.fmt_value(&trim(f64::from(b.tick) / 1000.0))),
            _ => {}
        }
        if let Some(name) = key.strip_prefix("ua_") {
            let attr = attr_id(name)?;
            let m = self.mods.get(&buff)?.iter().find(|m| m.attr == attr)?;
            return Some(self.fmt_value(&trim(modifier_value(name, f64::from(m.at(ab_level))))));
        }
        let ctx = Ctx { skill, rank, ab_level, attrs: a };
        // `total_*` is the whole of the damage over the duration, `min_*` / `max_*` / `avg_*` one tick's
        let (total, rest) = match key.strip_prefix("total_") {
            Some(r) => (true, r),
            None => (false, key),
        };
        let (part, what) = rest.split_once('_')?;
        let kind = kind_of(what)?;
        let fx = b.ticks.iter().find(|f| f.kind == kind)?;
        let amount = if duration > 0 && b.tick > 0 {
            let share = f64::from(b.tick) / f64::from(duration);
            let tick = self.amount(*fx, &ctx, Some(share))?;
            if total { tick.scaled(f64::from(duration) / f64::from(b.tick)) } else { tick }
        } else {
            self.amount(*fx, &ctx, None)?
        };
        self.amount_value(part, amount)
    }

    /// A buff's description with its placeholders filled.
    pub fn buff_text(&self, buff: u32, ab_level: i32, a: &Attrs) -> Option<String> {
        let desc = self.buff_desc.get(&buff)?;
        Some(fill(desc, |key| self.buff_value(buff, key, ab_level, 1, None, a)))
    }

    /// The `ab_level` a skill's buffs get at `rank` (the server's `buff_ab_level`).
    fn skill_ab_level(&self, skill: &SkillFx, rank: i32, a: &Attrs) -> i32 {
        let known = a.abilities[skill.ability.min(10)];
        let from_ability = if skill.level_step != 0 && known > 0 { known / skill.level_step * skill.level_step } else { 0 };
        let from_rank = skill.ability_level + (rank.max(1) - 1) * skill.level_step;
        if known > 0 { skill.ability_level.max(from_ability) } else { from_rank }
    }

    /// A skill's description with its placeholders filled for the player at `rank`; `None` when it has none
    /// (the plain text is `SkillInfo.desc`).
    pub fn skill_text(&self, id: u32, rank: u8, a: &Attrs) -> Option<String> {
        let desc = self.skill_desc.get(&id)?;
        let skill = self.skills.get(&id);
        let rank = i32::from(rank.max(1));
        let ab_level = skill.map_or(0, |s| self.skill_ab_level(s, rank, a));
        let ctx = Ctx { skill, rank, ab_level, attrs: a };
        Some(fill(desc, |key| self.skill_value(key, &ctx)))
    }

    /// What the skill costs the player in mana (the server's `Skill.ManaCost`: the cost grows with the ability level).
    pub fn mana(&self, id: u32, a: &Attrs) -> Option<i64> {
        let s = self.skills.get(&id)?;
        let ab_level = a.abilities[s.ability.min(10)].max(s.ability_level).max(1);
        let base = (f64::from(ab_level - 1) * 1.6 + 8.0) * 3.0 / 3.65;
        Some((base * f64::from(s.mana_level_md) + f64::from(s.mana_cost)) as i64)
    }

    /// `skills.target_area_radius` (m): the "Radius" line of an area skill.
    pub fn skill_radius(&self, id: u32) -> i32 {
        self.skills.get(&id).map_or(0, |s| s.radius)
    }
}

/// What a skill placeholder is worked out in.
struct Ctx<'a> {
    skill: Option<&'a SkillFx>,
    /// The learned rank (1 based).
    rank: i32,
    /// The `ab_level` the skill's buffs are applied with.
    ab_level: i32,
    attrs: &'a Attrs,
}

/// The description with `#{name}` replaced by `value(name)`; a placeholder without a value stays a `?`.
pub fn fill(text: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len() + 32);
    let mut rest = text.as_str();
    while let Some(at) = rest.find("#{") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let Some(end) = after.find('}') else {
            rest = &rest[at..];
            break;
        };
        out.push_str(&value(&after[..end]).unwrap_or_else(|| "?".into()));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A time as the client prints it into a description: `3#$second`, `1.5#$second`, `2#$minute`; the tooltip script
/// turns the `#$..` into "sec", "m", "h".
fn time(ms: i32) -> String {
    let ms = i64::from(ms);
    let (hours, minutes, rest) = (ms / 3_600_000, ms / 60_000 % 60, ms % 60_000);
    let mut parts: Vec<String> = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}#$hour"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}#$minute"));
    }
    if rest > 0 || parts.is_empty() {
        parts.push(if rest % 1000 == 0 { format!("{}#$second", rest / 1000) } else { format!("{:.1}#$second", rest as f64 / 1000.0) });
    }
    parts.join(" ")
}

/// A number without a pointless `.0`.
fn trim(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 { format!("{}", v.round() as i64) } else { format!("{v:.1}") }
}

/// `UnitAttribute` id of a buff placeholder's name (`ua_<name>`).
fn attr_id(name: &str) -> Option<u8> {
    ATTRIBUTES.iter().find(|(_, n)| *n == name).map(|(id, _)| *id)
}
pub(super) fn attribute_name(id: u8) -> Option<&'static str> {
    ATTRIBUTES.iter().find(|(value, _)| *value == id).map(|(_, name)| *name)
}

/// What an attribute's modifier value reads as in a description: the tooltip script's
/// `GetCalculatedAddModifierValue`, without the sign the text writes itself.
fn modifier_value(name: &str, v: f64) -> f64 {
    match name {
        "lung_capacity" => v / 1000.0,
        "incoming_melee_damage_add" | "incoming_ranged_damage_add" | "incoming_spell_damage_add" | "fall_damage_mul" => -v,
        "casting_tolerance" => (1.0 - 100.0 / (100.0 + v)) * 100.0,
        "casting_time_mul" | "incoming_damage_mul" => -(v / 10.0),
        "move_speed_mul" => (v * 0.1).abs(),
        "swim_speed_mul" | "aggro_mul" | "detect_stealth_range_mul" => v.abs(),
        "persistent_health_regen" | "persistent_mana_regen" => v / 5.0,
        "melee_critical_mul" | "ranged_critical_mul" | "spell_critical_mul" | "melee_damage_mul" | "ranged_damage_mul" | "spell_damage_mul"
        | "backattack_melee_damage_mul" | "backattack_ranged_damage_mul" | "backattack_spell_damage_mul" | "incoming_heal_mul" | "dodge_mul"
        | "block_mul" | "melee_parry_mul" | "heal_critical_mul" | "incoming_melee_damage_mul" | "incoming_ranged_damage_mul"
        | "incoming_spell_damage_mul" => v / 10.0,
        "melee_dps_inc" | "ranged_dps_inc" | "spell_dps_inc" | "heal_dps_inc" | "heal_dps" | "spell_dps" | "ranged_dps" | "mainhand_dps" => v / 1000.0,
        "melee_critical_bonus" | "ranged_critical_bonus" | "spell_critical_bonus" | "heal_critical_bonus" => v * 0.2,
        "melee_speed_mul" | "ranged_speed_mul" | "global_cooldown_mul" | "attack_anim_speed_mul" => ((1.0 - 1000.0 / (1000.0 + v)) * 1000.0).round() * 0.1,
        _ => v.abs(),
    }
}

/// `UnitAttribute` (the server's `models/unit.rs`): id and the snake case name the descriptions use (`ua_<name>`).
const ATTRIBUTES: &[(u8, &str)] = &[
    (0, "str"), (1, "dex"), (2, "sta"), (3, "int"), (4, "spi"), (5, "fai"), (6, "max_health"), (7, "max_mana"),
    (8, "armor"), (10, "move_speed_mul"), (11, "health_regen"), (12, "mana_regen"), (13, "facets"),
    (16, "melee_critical"), (17, "melee_critical_bonus"), (18, "melee_anti_miss"), (20, "melee_dodge"),
    (21, "melee_block"), (22, "melee_parry"), (23, "ranged_anti_miss"), (25, "ranged_critical"),
    (26, "ranged_critical_bonus"), (28, "spell_anti_miss"), (30, "spell_critical"), (31, "spell_critical_bonus"),
    (33, "melee_dps_inc"), (34, "ranged_dps_inc"), (35, "spell_dps_inc"), (36, "mainhand_speed"),
    (37, "mainhand_damage_min"), (38, "mainhand_damage_max"), (41, "offhand_speed"), (42, "offhand_damage_min"),
    (43, "offhand_damage_max"), (46, "ranged_speed"), (47, "ranged_damage_min"), (48, "ranged_damage_max"),
    (51, "melee_damage_mul"), (52, "ranged_damage_mul"), (53, "spell_damage_mul"), (54, "melee_speed_mul"),
    (55, "ranged_speed_mul"), (56, "incoming_heal_mul"), (57, "ignore_armor"), (58, "incoming_damage_mul"),
    (59, "ranged_dodge"), (60, "ranged_block"), (61, "aggro_range_mul"), (62, "incoming_aggro_mul"),
    (63, "hovering"), (64, "magic_resist"), (65, "magic_stability"), (66, "swim_speed_mul"),
    (67, "persistent_health_regen"), (68, "persistent_mana_regen"), (69, "armor_type"), (70, "armor_type_coverage"),
    (71, "casting_time_mul"), (72, "turn_speed"), (73, "gravity_mul"), (74, "global_cooldown_mul"),
    (75, "twohand_speed_mul"), (77, "melee_critical_mul"), (78, "melee_anti_miss_mul"), (79, "melee_dodge_mul"),
    (80, "melee_block_mul"), (81, "melee_parry_mul"), (82, "ranged_critical_mul"), (83, "ranged_anti_miss_mul"),
    (84, "ranged_dodge_mul"), (85, "ranged_block_mul"), (86, "spell_critical_mul"), (87, "spell_dps"),
    (88, "spell_anti_miss_mul"), (89, "casting_tolerance"), (90, "aggro_mul"), (91, "lung_capacity"),
    (92, "fall_damage_mul"), (93, "ladder_speed_mul"), (94, "detect_stealth_range_mul"), (95, "exp_mul"),
    (96, "mainhand_dps"), (97, "offhand_dps"), (98, "ranged_dps"), (99, "actability_archemy"),
    (100, "actability_architecture"), (101, "actability_cook"), (102, "actability_handicraft"),
    (103, "actability_livestock"), (104, "actability_farm"), (105, "actability_fish"), (106, "actability_lumber"),
    (107, "actability_collection"), (108, "actability_machinery"), (109, "actability_metal"),
    (110, "actability_print"), (111, "actability_mine"), (112, "actability_stonemason"), (113, "actability_sewing"),
    (114, "actability_skin"), (115, "actability_weapon"), (116, "actability_carpentry"), (117, "actability_theft"),
    (118, "actability_business"), (119, "attack_anim_speed_mul"), (120, "heal_mul"),
    (121, "backattack_melee_damage_mul"), (122, "backattack_ranged_damage_mul"),
    (123, "backattack_spell_damage_mul"), (124, "friction_mul"), (125, "honor_point_lose_mul"),
    (126, "honor_point_gain_battle_field"), (127, "honor_point_gain_battle_field_mul"),
    (128, "honor_point_gain_npc_kill"), (129, "honor_point_gain_npc_kill_mul"), (130, "honor_point_gain_trial"),
    (131, "honor_point_gain_trial_mul"), (132, "honor_point_gain_war"), (133, "honor_point_gain_war_mul"),
    (134, "honor_point_gain_quest"), (135, "honor_point_gain_quest_mul"), (136, "living_point_gain"),
    (137, "living_point_gain_mul"), (138, "actability_composition"), (139, "underwater_swim_speed_mul"),
    (140, "drop_rate_mul"), (141, "loot_gold_mul"), (142, "incoming_melee_damage_add"),
    (143, "incoming_melee_damage_mul"), (144, "incoming_ranged_damage_add"), (145, "incoming_ranged_damage_mul"),
    (146, "incoming_spell_damage_add"), (147, "incoming_spell_damage_mul"), (148, "incoming_siege_damage_add"),
    (149, "incoming_siege_damage_mul"), (150, "spell_damage_critical"), (151, "spell_damage_critical_mul"),
    (152, "spell_damage_critical_bonus"), (153, "ranged_parry"), (154, "ranged_parry_mul"), (173, "heal_dps"),
    (174, "heal_critical"), (175, "heal_dps_inc"), (176, "heal_critical_bonus"), (177, "block"), (178, "dodge"),
    (179, "block_mul"), (180, "dodge_mul"), (181, "bulls_eye"), (182, "battle_resist"), (183, "flexibility"),
    (184, "magic_penetration"), (185, "heal_critical_mul"), (186, "exp_by_labor_power_mul"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lung_capacity_uses_the_native_base_and_current_attribute_91_modifiers() {
        let mut t = TipTables::default();
        let equipment = aa_net::model::Equipment::default();
        assert_eq!(t.attrs(1, &equipment, &[], &[0; 11]).lung_capacity_ms, 60_000);
        t.mods.insert(700, vec![
            Mod { attr: 91, percent: false, value: 30_000, per_level: 0 },
            Mod { attr: 91, percent: true, value: 50, per_level: 0 },
        ]);
        assert_eq!(t.attrs(1, &equipment, &[(700, 1)], &[0; 11]).lung_capacity_ms, 135_000);
        assert_eq!(t.attrs(1, &equipment, &[], &[0; 11]).lung_capacity_ms, 60_000);
    }

    fn connection() -> Option<Connection> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
    }

    /// The tables, with the descriptions of skills and buffs `GameData::load` reads.
    fn tables(conn: &Connection) -> TipTables {
        let mut text: HashMap<(&'static str, u32), String> = HashMap::new();
        let mut q = conn
            .prepare("SELECT tbl_name, idx, en_us FROM localized_texts WHERE tbl_column_name = 'desc' AND tbl_name IN ('skills', 'buffs') AND en_us IS NOT NULL AND en_us <> ''")
            .unwrap();
        let mut rows = q.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let table = if r.get::<_, String>(0).unwrap() == "skills" { "skills_desc" } else { "buffs_desc" };
            text.insert((table, r.get::<_, i64>(1).unwrap() as u32), r.get(2).unwrap());
        }
        TipTables::load(conn, &text).unwrap()
    }

    /// A player of `level` with no gear and the three starter abilities at `ability` level.
    fn player(t: &TipTables, level: u8, ability: i32) -> Attrs {
        let abilities = [0, ability, 0, 0, 0, ability, ability, ability, ability, 0, ability];
        t.attrs(level, &aa_net::model::Equipment::default(), &[], &abilities)
    }

    /// Teleport to the Memory Tome ("Return"): `#{return_point}` prints the bound point as `return_district_tooltip`.
    #[test]
    fn the_return_point_names_the_bound_district() {
        let Some(conn) = connection() else { return };
        let t = tables(&conn);
        let mut a = player(&t, 50, 40);
        a.return_point = Some(("Lilyut Hills".into(), "Marianople".into()));
        let text = t.skill_text(10594, 1, &a).expect("skill 10594 has a description");
        assert!(text.contains("Lilyut Hills: Marianople"), "{text}");
        a.return_point = None;
        let text = t.skill_text(10594, 1, &a).unwrap();
        assert!(!text.contains("#{") && !text.contains("Lilyut"), "{text}");
    }

    fn ids(conn: &Connection, sql: &str) -> Vec<u32> {
        let mut q = conn.prepare(sql).unwrap();
        q.query_map([], |r| r.get::<_, i64>(0)).unwrap().map(|r| r.unwrap() as u32).collect()
    }

    /// Every skill the skill window shows for the six ability trees (Battlerage, Occultism, Archery, Sorcery,
    /// Shadowplay, Auramancy) and the general ones (Dash ...): no placeholder is left.
    #[test]
    fn the_starter_skills_have_every_number() {
        let Some(conn) = connection() else { return };
        let t = tables(&conn);
        let a = player(&t, 50, 40);
        // every visible skill of the ability trees plus the ones every character has (Dash, the dances, ...); the
        // ability-0 skills of mobs, vehicles and events are not player tooltips
        let skills = ids(&conn, "SELECT id FROM skills WHERE ability_id BETWEEN 1 AND 11 AND ability_level < 99 AND show = 't' UNION SELECT skill_id FROM default_skills");
        let mut checked = 0;
        let mut open = Vec::new();
        for id in skills {
            let Some(text) = t.skill_text(id, 1, &a) else { continue };
            checked += 1;
            if text.contains('?') {
                open.push((id, text));
            }
        }
        assert!(checked > 50, "{checked}");
        let ids: Vec<u32> = open.iter().map(|(id, _)| *id).collect();
        assert!(open.is_empty(), "{} of {checked}: {ids:?}
{:#?}", open.len(), &open[..open.len().min(4)]);
    }

    /// The new character of `reference/ingame/character_info` (Battlerage, level 1: the starter clothes, sword, shield, bow
    /// and instrument): the real character window shows 19.42 melee, 16.81 ranged, 2.00 magic attack, 2.00 healing
    /// power, 269 physical and 377 magic defense.
    #[test]
    fn a_new_characters_numbers_match_the_capture() {
        use aa_net::model::{slot, EquipDetail, Equipment, Item, ItemDetail};
        let Some(conn) = connection() else { return };
        let t = tables(&conn);
        let mut equipment = Equipment::default();
        for (at, template) in [(slot::CHEST, 23387), (slot::LEGS, 23388), (slot::FEET, 23390), (slot::MAINHAND, 5569), (slot::OFFHAND, 6152), (slot::RANGED, 6127), (slot::MUSICAL, 6177)] {
            let detail = ItemDetail::Equip(EquipDetail { durability: 100, ..EquipDetail::default() });
            equipment.slots[at] = Some(Item { template_id: template, detail, ..Item::default() });
        }
        let a = t.attrs(1, &equipment, &[], &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let near = |got: i32, want: f64| (f64::from(got) / 1000.0 - want).abs() < 0.006;
        assert!(near(a.dps[0], 19.42) && near(a.dps[2], 16.81) && near(a.dps[3], 2.0) && near(a.dps[4], 2.0), "{:?}", a.dps);
        assert_eq!(a.primaries, [10, 10, 10, 10, 10, 10]);
        let worn: Vec<_> = equipment.slots.iter().flatten().filter_map(|it| t.items.stats(it.template_id, 0, 0, 0)).collect();
        // the character formulas add nothing to the defense and the spirit to the magic defense; the shield's
        // gear buff "Equip Shield" (8226) gives 200 of both
        assert_eq!(worn.iter().map(|s| s.armor).sum::<i32>() + 200, 269);
        assert_eq!(worn.iter().map(|s| s.magic_resistance).sum::<i32>() + a.primaries[4] + 200, 377);
    }

    /// The numbers of one skill: Flamebolt hits for a magic range, and a buff with a duration prints it.
    #[test]
    fn a_damage_skill_prints_a_range() {
        let Some(conn) = connection() else { return };
        let t = tables(&conn);
        let a = player(&t, 50, 40);
        // Flamebolt (10752): "Deals #{avg_damage}.. Magic Damage", burning for #{buff_duration}
        let text = t.skill_text(10752, 1, &a).unwrap();
        assert!(!text.contains("#{") && !text.contains('?'), "{text}");
        let amount = text.split("|cFFFF9C27").nth(1).and_then(|s| s.split("|r").next()).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        assert!(amount > 20, "{text}");
        assert!(text.contains("#$second"), "{text}");
    }

    /// Prints what is left over all skills and buffs (`cargo test -p aa-client tips_coverage -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn tips_coverage() {
        let Some(conn) = connection() else { return };
        let t = tables(&conn);
        let a = player(&t, 50, 40);
        let mut left: HashMap<String, Vec<u32>> = HashMap::new();
        let (mut skills, mut buffs) = (0, 0);
        for id in ids(&conn, "SELECT id FROM skills") {
            let Some(text) = t.skill_text(id, 1, &a) else { continue };
            skills += 1;
            let desc = &t.skill_desc[&id];
            let raw: Vec<&str> = desc.split("#{").skip(1).filter_map(|p| p.split('}').next()).collect();
            if text.contains('?') {
                for key in raw {
                    if t.skill_value(key, &Ctx { skill: t.skills.get(&id), rank: 1, ab_level: 1, attrs: &a }).is_none() {
                        left.entry(format!("skill {key}")).or_default().push(id);
                    }
                }
            }
        }
        for id in ids(&conn, "SELECT id FROM buffs") {
            let Some(text) = t.buff_text(id, 50, &a) else { continue };
            buffs += 1;
            if text.contains('?') {
                let desc = &t.buff_desc[&id];
                for key in desc.split("#{").skip(1).filter_map(|p| p.split('}').next()) {
                    if t.buff_value(id, key, 50, 1, None, &a).is_none() {
                        left.entry(format!("buff {key}")).or_default().push(id);
                    }
                }
            }
        }
        let mut keys: Vec<_> = left.into_iter().collect();
        keys.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        println!("{skills} skills and {buffs} buffs have placeholders; not resolved:");
        for (k, v) in keys {
            println!("  {k}: {} e.g. {:?}", v.len(), &v[..v.len().min(5)]);
        }
    }
}
