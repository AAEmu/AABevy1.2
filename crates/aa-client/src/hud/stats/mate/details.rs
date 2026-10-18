//! Original current-mate information panel; full source units and independent modifier baselines.
use super::*;
mod equipment;

const ATTRIBUTES: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 16, 33, 54, 64, 67, 68, 77, 96, 178, 180];

#[derive(Default)]
pub(super) struct Tables {
    npc: HashMap<u32, Vec<Bonus>>,
    buffs: HashMap<u32, Vec<Bonus>>,
    items: HashMap<u32, Vec<Bonus>>,
    limits: HashMap<u8, (i32, i32)>,
    conditional: HashSet<u32>,
    dynamic: HashSet<u32>,
    equipment: equipment::Tables,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q=db.prepare("SELECT owner_type,owner_id,unit_attribute_id,unit_modifier_type_id,value,linear_level_bonus FROM unit_modifiers WHERE owner_type IN('Npc','Buff','Item') AND unit_attribute_id BETWEEN 0 AND 180 AND unit_modifier_type_id IN(0,1)")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let attr = super::super::int(r, 2)? as u8;
            if !ATTRIBUTES.contains(&attr) {
                continue;
            }
            let owner = super::super::int(r, 1)? as u32;
            let bonus = Bonus {
                attr,
                percent: super::super::int(r, 3)? == 1,
                value: super::super::int(r, 4)? as i32,
                per_level: super::super::int(r, 5)? as i32,
            };
            let map = match r.get::<_, String>(0)?.as_str() {
                "Npc" => &mut out.npc,
                "Buff" => &mut out.buffs,
                _ => &mut out.items,
            };
            map.entry(owner).or_default().push(bonus);
        }
        let mut q = db.prepare("SELECT unit_attribute_id,minimum,maximum FROM unit_attribute_limits WHERE unit_attribute_id BETWEEN 0 AND 180")?;
        for row in q.query_map([], |r| {
            Ok((r.get::<_, u8>(0)?, (super::super::int(r, 1)? as i32, super::super::int(r, 2)? as i32)))
        })? {
            let (attr, range) = row?;
            out.limits.insert(attr, range);
        }
        let mut q=db.prepare("SELECT b.owner_id,m.unit_attribute_id FROM buff_unit_modifiers b JOIN unit_modifiers m ON m.owner_type='BuffUnitModifier' AND m.owner_id=b.id WHERE b.owner_type='Buff'")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u16>(1)?)))? {
            let (id, attr) = row?;
            if u8::try_from(attr).ok().is_some_and(|attr| ATTRIBUTES.contains(&attr)) {
                out.conditional.insert(id);
            }
        }
        let mut q = db.prepare("SELECT buff_id,unit_attribute_id FROM dynamic_unit_modifiers")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u16>(1)?)))? {
            let (id, attr) = row?;
            if u8::try_from(attr).ok().is_some_and(|attr| ATTRIBUTES.contains(&attr)) {
                out.dynamic.insert(id);
            }
        }
        out.equipment = equipment::Tables::load(db)?;
        Ok(out)
    }
}

pub(in crate::hud) const VALUE_KEYS: [&str; 11] = [
    "melee_dps",
    "melee_critical_rate",
    "dodge_rate",
    "armor",
    "armor_percentage",
    "magic_resist",
    "magic_resist_percentage",
    "persistent_health_regen",
    "health_regen",
    "persistent_mana_regen",
    "mana_regen",
];
pub(in crate::hud) const MODIFIERS: [(&str, u8, bool); 17] = [
    ("melee_dps", 33, false),
    ("melee_attack_speed_mul", 54, false),
    ("melee_critical_rate", 16, false),
    ("dodge_rate", 178, false),
    ("dodge_mul", 180, false),
    ("armor", 8, false),
    ("magic_resist", 64, false),
    ("health_regen", 11, false),
    ("persistent_health_regen", 67, false),
    ("mana_regen", 12, false),
    ("persistent_mana_regen", 68, false),
    ("dex", 1, true),
    ("int", 3, true),
    ("spi", 4, true),
    ("max_health", 6, true),
    ("max_mana", 7, true),
    ("move_speed", 10, true),
];

#[derive(Clone, Debug, PartialEq)]
pub(in crate::hud) struct Info {
    pub values: [f32; 11],
    pub modifiers: [f32; 17],
}

#[derive(Clone, Copy)]
struct Pair {
    flat: i32,
    percent: i32,
}
impl Default for Pair {
    fn default() -> Self {
        Self { flat: 0, percent: 100 }
    }
}
impl Pair {
    fn merge(self, other: Self) -> Self {
        Self {
            flat: self.flat.wrapping_add(other.flat),
            percent: self.percent.wrapping_add(other.percent).wrapping_sub(100),
        }
    }
    fn final_value(self, base: i32, limit: Option<&(i32, i32)>) -> i32 {
        let flat = base.wrapping_add(self.flat);
        let value = if self.percent == 100 {
            flat
        } else {
            trunc(flat as f32 * self.percent as f32 * 0.01f32)
        };
        limit.map_or(value, |(min, max)| value.clamp(*min, *max))
    }
    fn modifier(self, base: i32, static_pair: Self) -> f32 {
        if self.flat == 0 && self.percent == 100 {
            return 0.;
        }
        let baseline = (base as f32 + static_pair.flat as f32) * static_pair.percent as f32 * 0.01f32;
        // Native x87 holds this difference without intermediate integer truncation or attribute limits.
        (((f64::from(baseline) + f64::from(self.flat)) * f64::from(self.percent)) * f64::from(0.01f32) - f64::from(baseline)) as f32
    }
}
pub(super) fn trunc(value: f32) -> i32 {
    if !(-2147483648.0..2147483648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}
type Pairs = [Pair; 181];
fn add(out: &mut Pairs, list: Option<&Vec<Bonus>>, level: i32, multiplier: i32) {
    for bonus in list.into_iter().flatten() {
        let pair = &mut out[usize::from(bonus.attr)];
        let target = if bonus.percent { &mut pair.percent } else { &mut pair.flat };
        *target = target.wrapping_add(super::amount(bonus, level, multiplier));
    }
}

struct Attributes<'a> {
    stats: &'a StatTables,
    tables: &'a Tables,
    kind: u32,
    level: u8,
    primary: [i32; 6],
    equipment: [i32; 3],
    static_pairs: Pairs,
    temporary: Pairs,
    main: Pairs,
    secondary: Pairs,
    cache: [Option<i32>; 181],
}
impl Attributes<'_> {
    fn final_value(&mut self, attr: u8) -> Option<i32> {
        let index = usize::from(attr);
        if let Some(value) = self.cache[index] {
            return Some(value);
        }
        let base = self.base(attr)?;
        let pair = self.static_pairs[index]
            .merge(self.temporary[index])
            .merge(self.main[index])
            .merge(self.secondary[index]);
        let value = pair.final_value(base, self.tables.limits.get(&attr));
        self.cache[index] = Some(value);
        Some(value)
    }
    fn formula(&self, kind: u8) -> Option<i32> {
        super::evaluate(self.stats, self.kind, self.level, kind, self.primary)
    }
    fn base(&mut self, attr: u8) -> Option<i32> {
        Some(match attr {
            0..=5 => super::evaluate(self.stats, self.kind, self.level, 22 + attr, [0; 6])?,
            6 => self.formula(14)?,
            7 => self.formula(15)?,
            8 => self.equipment[0],
            10 | 54 | 77 | 180 => 0,
            11 => self.formula(16)?,
            12 => self.formula(17)?,
            13 => self.formula(20)?,
            33 => self.formula(21)?,
            64 => self.formula(19)?.wrapping_add(self.equipment[1]),
            67 => self.formula(31)?,
            68 => self.formula(32)?,
            96 => self.equipment[2],
            16 | 178 => {
                let (kind, multiplier) = if attr == 16 { (1, 77) } else { (46, 180) };
                let base = self.formula(kind)?;
                let denominator = self.final_value(13)? as f32;
                let multiplier = self.final_value(multiplier)? as f32;
                base.wrapping_sub(trunc(multiplier * (denominator * -0.001f32)))
            }
            _ => return None,
        })
    }
    fn modifier(&mut self, attr: u8, skip_main: bool) -> Option<f32> {
        let index = usize::from(attr);
        let mut pair = self.temporary[index].merge(self.secondary[index]);
        if !skip_main {
            pair = pair.merge(self.main[index]);
        }
        if pair.flat == 0 && pair.percent == 100 {
            return Some(0.);
        }
        Some(pair.modifier(self.base(attr)?, self.static_pairs[index]))
    }
}

pub(in crate::hud) fn get(game: &crate::hud::data::GameData, unit: &aa_net::Unit, buffs: Option<&[BuffEntry]>) -> Option<Info> {
    let buffs = buffs?;
    let (kind, pools) = super::context(game, unit, Some(buffs))?;
    let stats = &game.stats;
    let tables = &stats.mate.details;
    let primary = super::primaries(stats, kind, unit.level, &pools)?;
    let aa_net::model::UnitKind::Mate { template_id, .. } = unit.kind else {
        return None;
    };
    let mut attrs = Attributes {
        stats,
        tables,
        kind,
        level: unit.level,
        primary,
        equipment: tables.equipment.bases(game, &unit.equipment)?,
        static_pairs: [Pair::default(); 181],
        temporary: [Pair::default(); 181],
        main: [Pair::default(); 181],
        secondary: [Pair::default(); 181],
        cache: [None; 181],
    };
    add(&mut attrs.static_pairs, tables.npc.get(&template_id), i32::from(unit.level), 1);
    for entry in buffs {
        let buff = &entry.buff;
        if stats.mate.one_time.contains(&buff.buff_id) {
            continue;
        }
        if tables.conditional.contains(&buff.buff_id) || (tables.dynamic.contains(&buff.buff_id) && buff.tick_ms != 0 && buff.duration_ms != 0) {
            return None;
        }
        add(
            &mut attrs.temporary,
            tables.buffs.get(&buff.buff_id),
            i32::from(buff.ability_level as u16),
            buff.stacks,
        );
    }
    for slot in (0..=18).chain(std::iter::once(27)) {
        let Some(item) = unit.equipment.slots[slot].as_ref() else { continue };
        if !super::eligible(game, item.template_id, slot) {
            continue;
        }
        let detail = match &item.detail {
            ItemDetail::Equip(detail) => Some(detail),
            _ => None,
        };
        if detail.is_none_or(|detail| detail.durability == 0) && game.tips.items.durability(item.template_id, item.grade).is_some() {
            continue;
        }
        for attr in 0..5 {
            attrs.main[attr].flat = attrs.main[attr].flat.wrapping_add(super::item_stat(stats, item.template_id, item.grade, attr));
        }
        if let Some(level) = stats.mate.item_levels.get(&item.template_id) {
            add(&mut attrs.main, tables.items.get(&item.template_id), i32::from(*level), 1);
        }
        if let Some(detail) = detail {
            for id in std::iter::once(detail.rune_id)
                .filter(|id| *id != 0)
                .chain(detail.gem_ids.iter().copied().take_while(|id| *id != 0))
            {
                if let Some(level) = stats.mate.item_levels.get(&id) {
                    add(&mut attrs.secondary, tables.items.get(&id), i32::from(*level), 1);
                }
            }
        }
    }
    for (index, value) in primary.into_iter().enumerate() {
        attrs.cache[index] = Some(value);
    }
    let weapon = attrs.final_value(96)? as f32;
    let additional = attrs.final_value(33)? as f32;
    let inv = 1.0f32 / (attrs.final_value(13)? as f32);
    let critical = attrs.final_value(16)? as f32;
    let dodge = attrs.final_value(178)? as f32;
    let armor = attrs.final_value(8)? as f32;
    let resist = attrs.final_value(64)? as f32;
    let persistent_health = attrs.final_value(67)? as f32 * 0.2f32;
    let health = attrs.final_value(11)? as f32 + persistent_health;
    let persistent_mana = attrs.final_value(68)? as f32 * 0.2f32;
    let mana = attrs.final_value(12)? as f32 + persistent_mana;
    let mut modifiers = [0.; 17];
    for (index, (_, attr, skip)) in MODIFIERS.into_iter().enumerate() {
        modifiers[index] = attrs.modifier(attr, skip)?;
    }
    Some(Info {
        values: [
            weapon * 0.001f32 + additional * 0.001f32,
            critical * inv * 100.,
            dodge * inv * 100.,
            armor,
            armor / (armor + 5300.) * 100.,
            resist,
            resist / (resist + 5300.) * 100.,
            persistent_health,
            health,
            persistent_mana,
            mana,
        ],
        modifiers,
    })
}

#[cfg(test)]
mod tests;
