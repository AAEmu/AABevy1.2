//! The local player's numbers (level, HP / MP, experience, labor, money ...) and the buffs on every
//! unit in view.

use std::collections::HashMap;
use std::time::Instant;

use crate::buff::Buff;
use crate::items::ItemTask;
use crate::model::{NO_ABILITY, exp};
use crate::session::Event;

/// The labor cap of a normal account (premium accounts get [`LABOR_MAX_PREMIUM`]).
pub const LABOR_MAX: i32 = 2000;
pub const LABOR_MAX_PREMIUM: i32 = 5000;
/// How many labor changes [`PlayerStats::labor_log`] keeps.
pub const LABOR_LOG_LIMIT: usize = 16;

resource! {
    /// The numbers the HUD shows for the local player. Filled when the character is selected,
    /// refreshed when it spawns and then kept current from the server's change packets.
    ///
    /// The server never sends maximum HP / MP (the real client computes them from the formula tables
    /// of the game database: `unit_formulas` of kind MaxHealth / MaxMana over level and stamina /
    /// intelligence, plus the gear's stats and the buffs' bonuses), so `max_hp` / `max_mp` here are
    /// the highest values seen since the spawn, which are exact right after a spawn at full health.
    /// An app that has the database computes the real ones and stores them with
    /// [`PlayerStats::set_max`] (aa-client's HUD does: `hud/stats.rs`).
    pub struct PlayerStats {
        /// The player's object id (0 until the character spawned).
        pub obj_id: u32,
        pub character_id: u32,
        pub name: String,
        pub level: u8,
        pub hp: i32,
        pub mp: i32,
        pub max_hp: i32,
        pub max_mp: i32,
        /// Total experience (`levels.total_exp` scale, see [`PlayerStats::exp_progress`]).
        pub exp: i64,
        /// Experience that a resurrection at a shrine can win back.
        pub recoverable_exp: i32,
        /// Labor points as of `labor_updated` (unix seconds); the server adds 5 every 5 minutes
        /// online, up to [`LABOR_MAX`].
        pub labor: i32,
        pub labor_updated: i64,
        /// The labor changes the server announced this session, oldest first, as `(amount, labor
        /// after)`: the client prints "Gained 10 Labor Point(s). (Total LP: 105)" for each, also for
        /// the ones that came while the screen was still loading (the last [`LABOR_LOG_LIMIT`]).
        pub labor_log: Vec<(i32, i32)>,
        /// How many labor changes the server announced this session (`labor_log` keeps the last few of them).
        pub labor_seq: u32,
        /// Copper in the bag (1 gold = 10000 copper) and in the bank.
        pub money: i64,
        pub bank_money: i64,
        /// Loyalty tokens.
        pub loyalty: i64,
        pub honor: i32,
        /// Vocation (living) points.
        pub vocation: i32,
        /// The three active ability (archetype) ids; `model::NO_ABILITY` (11) for an empty one.
        /// The title ("Battlerage Novice") comes from these and `ability_exp`.
        pub active_abilities: [u8; 3],
        /// Experience per ability, index = ability id (0 is the general ability).
        pub ability_exp: [i32; 11],
        pub in_combat: bool,
        pub dead: bool,
    }
}

impl PlayerStats {
    /// `(earned, needed)` experience in the current level; `needed` is 0 at the maximum level.
    pub fn exp_progress(&self) -> (i64, i64) {
        exp::progress(self.level, self.exp)
    }

    /// The level of an ability from its experience (same table as characters).
    pub fn ability_level(&self, ability: u8) -> u8 {
        self.ability_exp
            .get(usize::from(ability))
            .map_or(1, |&e| exp::level_for_total(i64::from(e)))
    }

    /// `(gold, silver, copper)` of `money`.
    pub fn money_parts(&self) -> (i64, i64, i64) {
        (self.money / 10_000, self.money / 100 % 100, self.money % 100)
    }

    pub fn hp_fraction(&self) -> f32 {
        if self.max_hp <= 0 { 0.0 } else { (self.hp as f32 / self.max_hp as f32).clamp(0.0, 1.0) }
    }

    pub fn mp_fraction(&self) -> f32 {
        if self.max_mp <= 0 { 0.0 } else { (self.mp as f32 / self.max_mp as f32).clamp(0.0, 1.0) }
    }

    fn is_self(&self, obj_id: u32) -> bool {
        self.obj_id != 0 && self.obj_id == obj_id
    }

    /// Stores the maxima an app worked out from the game database (never below the current points).
    pub fn set_max(&mut self, max_hp: i32, max_mp: i32) {
        (self.max_hp, self.max_mp) = (max_hp.max(self.hp), max_mp.max(self.mp));
    }

    fn set_points(&mut self, hp: i32, mp: i32) {
        (self.hp, self.mp) = (hp, mp);
        self.max_hp = self.max_hp.max(hp);
        self.max_mp = self.max_mp.max(mp);
    }

    /// Returns true when the data changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::CharacterSelected(cs) => {
                let c = &cs.character;
                *self = Self {
                    character_id: c.id,
                    name: c.name.clone(),
                    level: c.level,
                    hp: c.hp,
                    mp: c.mp,
                    max_hp: c.hp,
                    max_mp: c.mp,
                    exp: i64::from(cs.experience),
                    recoverable_exp: cs.recoverable_exp,
                    // Native header hydration MOVZXs the raw 16-bit wire value.
                    labor: i32::from(c.labor_power as u16),
                    labor_updated: c.labor_power_modified,
                    money: c.money,
                    bank_money: c.money2,
                    loyalty: c.bm_point,
                    active_abilities: c.abilities,
                    ability_exp: cs.ability_exp,
                    ..Self::default()
                };
            }
            Event::PlayerSpawn(s) => {
                self.obj_id = s.obj_id;
                self.level = s.unit.level;
                self.max_hp = 0;
                self.max_mp = 0;
                self.set_points(s.unit.hp, s.unit.mp);
                self.dead = s.unit.hp <= 0;
                // The character state carries zeros for the ability experience (the server writes 11 zero words);
                // the real values come with the player's own unit state, one per ability from id 1 (from id 0 when
                // the general ability is listed too).
                let first = usize::from(s.unit.ability_exp.len() > 10);
                for (i, (exp, _order)) in s.unit.ability_exp.iter().enumerate() {
                    if let Some(e) = self.ability_exp.get_mut(i + 1 - first) {
                        *e = *exp;
                    }
                }
            }
            Event::UnitPoints { obj_id, hp, mp } if self.is_self(*obj_id) => {
                self.set_points(*hp, *mp);
                if *hp > 0 {
                    self.dead = false;
                }
            }
            Event::ExpChanged { obj_id, delta, add_ability_exp } if self.is_self(*obj_id) => {
                self.exp += i64::from(*delta);
                if *add_ability_exp {
                    // The server's `Abilities.AddActiveExp`: the same delta on every active ability, capped at the
                    // total of the last level.
                    let cap = exp::total_for_level(exp::MAX_LEVEL).min(i64::from(i32::MAX)) as i32;
                    for a in self.active_abilities {
                        if let Some(e) = self.ability_exp.get_mut(usize::from(a)).filter(|_| a != NO_ABILITY) {
                            *e = e.saturating_add(*delta).min(cap);
                        }
                    }
                }
            }
            Event::RecoverableExp { obj_id, recoverable, .. } if self.is_self(*obj_id) => {
                self.recoverable_exp = *recoverable;
            }
            Event::LevelChanged { obj_id, level } if self.is_self(*obj_id) => self.level = *level,
            Event::AbilityExpChanged { obj_id, ability, exp } if self.is_self(*obj_id) => {
                let Some(slot) = self.ability_exp.get_mut(usize::from(*ability)) else { return false };
                *slot = *exp;
            }
            Event::AbilitySwapped { obj_id, old_ability, ability } if self.is_self(*obj_id) => {
                // Only the first matching slot changes: taking a second ability fills one empty slot, not both.
                let Some(slot) = self.active_abilities.iter().position(|a| a == old_ability) else { return false };
                self.active_abilities[slot] = *ability;
                // An empty slot taken starts the new ability at the experience of the first one (the server's
                // `CharacterAbilities.Swap`; it sends no `SCAbilityExpChanged` for it).
                if *old_ability == NO_ABILITY && slot > 0 {
                    let first = usize::from(self.active_abilities[0]);
                    let exp = self.ability_exp.get(first).copied().unwrap_or(0);
                    if let Some(e) = self.ability_exp.get_mut(usize::from(*ability)) {
                        *e = exp;
                    }
                }
            }
            Event::LaborChanged { amount, .. } => {
                // SC045 ->3930b4d0 adds the raw signed delta to header+5fc.
                self.labor = self.labor.wrapping_add(*amount);
                self.labor_updated = unix_now();
                if self.labor_log.len() >= LABOR_LOG_LIMIT {
                    self.labor_log.remove(0);
                }
                self.labor_log.push((*amount, self.labor));
                self.labor_seq += 1;
            }
            Event::GamePoints { honor, vocation, .. } => (self.honor, self.vocation) = (*honor, *vocation),
            Event::GamePointChanged { kind, amount } => match kind {
                0 => self.honor += amount,
                1 => self.vocation += amount,
                _ => return false,
            },
            Event::LoyaltyChanged { bm_point } => self.loyalty = *bm_point,
            Event::ItemsChanged(changes) => {
                let mut changed = false;
                for task in &changes.tasks {
                    match task {
                        ItemTask::Money(a) => self.money += i64::from(*a),
                        ItemTask::BankMoney(a) => self.bank_money += i64::from(*a),
                        ItemTask::GamePoint { kind: 0, amount } => self.honor += amount,
                        ItemTask::GamePoint { kind: 1, amount } => self.vocation += amount,
                        _ => continue,
                    }
                    changed = true;
                }
                return changed;
            }
            Event::CombatChanged { obj_id, engaged } if self.is_self(*obj_id) => self.in_combat = *engaged,
            Event::UnitDeath { obj_id } if self.is_self(*obj_id) => {
                self.dead = true;
                self.hp = 0;
                self.in_combat = false;
            }
            Event::Resurrected { obj_id, .. } if self.is_self(*obj_id) => self.dead = false,
            Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.character_id == 0 && self.obj_id == 0 {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A buff and the moment it arrived, for countdowns.
#[derive(Clone, Debug)]
pub struct BuffEntry {
    pub buff: Buff,
    pub received: Instant,
}

impl BuffEntry {
    /// Milliseconds left, `None` for a permanent buff.
    pub fn remaining_ms(&self) -> Option<u64> {
        self.buff.remaining_ms(self.received.elapsed().as_millis() as u64)
    }
}

resource! {
    /// Buffs and debuffs by unit object id, for the player's own buff bar and the target frame.
    /// A unit's list starts from its unit-state packet and follows the buff packets.
    pub struct Buffs {
        /// Catalog-aware original-client fold; raw lane entries remain in `by_unit`.
        pub native: super::NativeBuffs,
        pub by_unit: HashMap<u32, Vec<BuffEntry>>,
    }
}

impl Buffs {
    /// The buffs on a unit (empty when none are known).
    pub fn of(&self, obj_id: u32) -> &[BuffEntry] {
        self.by_unit.get(&obj_id).map_or(&[], Vec::as_slice)
    }

    /// The buffs a unit packet carried, in the server's order (good, bad, hidden).
    fn seed(&mut self, obj_id: u32, buffs: &[Buff]) {
        let now = Instant::now();
        let list = buffs.iter().cloned().map(|buff| BuffEntry { buff, received: now }).collect();
        self.by_unit.insert(obj_id, list);
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        self.native.apply(e);
        match e {
            Event::PlayerSpawn(s) => self.seed(s.obj_id, &s.unit.buffs),
            Event::UnitSpawned(u) => self.seed(u.obj_id, &u.buffs),
            Event::UnitsRemoved(ids) => {
                let mut changed = false;
                for id in ids {
                    changed |= self.by_unit.remove(id).is_some();
                }
                return changed;
            }
            Event::BuffCreated { obj_id, buff } => {
                let list = self.by_unit.entry(*obj_id).or_default();
                let entry = BuffEntry { buff: buff.clone(), received: Instant::now() };
                match list.iter_mut().find(|b| b.buff.index == buff.index) {
                    Some(old) => *old = entry,
                    None => list.push(entry),
                }
            }
            Event::BuffUpdated { obj_id, index, stacks, charged } => {
                if let Some(entry) = self.by_unit.get_mut(obj_id).and_then(|list| list.iter_mut().find(|e| e.buff.index == *index)) {
                    entry.buff.stacks = *stacks;
                    entry.buff.charged = *charged;
                }
                // Native history may have changed even if raw lane projection has no matching row.
            }
            Event::BuffRemoved { obj_id, index } => {
                let Some(list) = self.by_unit.get_mut(obj_id) else { return false };
                let before = list.len();
                list.retain(|b| b.buff.index != *index);
                return list.len() != before;
            }
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_unit.is_empty() {
                    return false;
                }
                self.by_unit.clear();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buff::{BuffCaster, BuffKind};
    use crate::model::{Character, CharacterState};

    fn buff(index: u32, buff_id: u32) -> Buff {
        Buff {
            index,
            buff_id,
            caster: BuffCaster::default(),
            duration_ms: 10_000,
            stacks: 1,
            kind: BuffKind::Good,
            ..Buff::default()
        }
    }

    fn selected() -> Event {
        let character = Character {
            id: 5,
            name: "Aa".into(),
            level: 3,
            hp: 700,
            mp: 300,
            labor_power: 1900,
            money: 123_456,
            money2: 9_000_000,
            bm_point: 12,
            abilities: [1, 11, 11],
            ..Character::default()
        };
        Event::CharacterSelected(Box::new(CharacterState {
            character,
            experience: 1_000,
            recoverable_exp: 50,
            ability_exp: [0, 2_000, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            ..CharacterState::default()
        }))
    }

    #[test]
    fn stats_follow_the_select_and_change_packets() {
        let mut s = PlayerStats::default();
        assert!(s.apply(&selected()));
        assert_eq!((s.level, s.hp, s.mp, s.exp, s.labor, s.money, s.bank_money, s.loyalty), (3, 700, 300, 1_000, 1900, 123_456, 9_000_000, 12));
        // Level 3 starts at 1400? no: 1000 exp is level 2 territory but the server says level 3;
        // the bar clamps instead of going negative.
        assert_eq!(s.exp_progress().0, 0);
        assert_eq!(s.money_parts(), (12, 34, 56));
        assert_eq!(s.ability_level(1), 3); // 2000 exp: level 3 (1400..3200)

        // Events about other units do not touch the player.
        s.obj_id = 0x77;
        assert!(!s.apply(&Event::UnitPoints { obj_id: 0x99, hp: 1, mp: 1 }));
        assert!(s.apply(&Event::UnitPoints { obj_id: 0x77, hp: 650, mp: 320 }));
        assert_eq!((s.hp, s.mp, s.max_hp, s.max_mp), (650, 320, 700, 320));
        // an app that knows the formulas stores the real maxima (never below the current points)
        s.set_max(900, 100);
        assert_eq!((s.max_hp, s.max_mp), (900, 320));
        assert!((s.hp_fraction() - 650.0 / 900.0).abs() < 1e-6);
        assert!(s.apply(&Event::ExpChanged { obj_id: 0x77, delta: 500, add_ability_exp: true }));
        assert_eq!(s.exp, 1_500);
        assert!(s.apply(&Event::LevelChanged { obj_id: 0x77, level: 4 }));
        assert_eq!(s.level, 4);
        assert!(s.apply(&Event::LaborChanged { amount: -50, action: 3, point: 5, step: 0 }));
        assert_eq!(s.labor, 1850);
        assert!(s.apply(&Event::LaborChanged { amount: 10, action: 0, point: 0, step: 0 }));
        assert_eq!(s.labor, 1860);
        assert_eq!(s.labor_log, vec![(-50, 1850), (10, 1860)]);
        assert_eq!(s.labor_seq, 2);
        assert!(s.apply(&Event::GamePoints { honor: 5, vocation: 6, other: [0; 8] }));
        assert!(s.apply(&Event::GamePointChanged { kind: 1, amount: 4 }));
        assert_eq!((s.honor, s.vocation), (5, 10));
        assert!(s.apply(&Event::AbilitySwapped { obj_id: 0x77, old_ability: 1, ability: 7 }));
        assert_eq!(s.active_abilities, [7, 11, 11]);
        // an empty slot taken: one slot only, at the experience of the first ability
        s.ability_exp[7] = 4_410_000;
        assert!(s.apply(&Event::AbilitySwapped { obj_id: 0x77, old_ability: 11, ability: 6 }));
        assert_eq!(s.active_abilities, [7, 6, 11]);
        assert_eq!(s.ability_exp[6], 4_410_000);
        assert!(!s.apply(&Event::AbilitySwapped { obj_id: 0x77, old_ability: 3, ability: 4 }));
        // experience with `add_ability_exp` goes to the active abilities too, capped at the last level
        assert!(s.apply(&Event::ExpChanged { obj_id: 0x77, delta: 1_000, add_ability_exp: true }));
        assert_eq!((s.ability_exp[7], s.ability_exp[6], s.ability_exp[10]), (4_411_000, 4_411_000, 0));
        assert!(s.apply(&Event::ExpChanged { obj_id: 0x77, delta: i32::MAX, add_ability_exp: true }));
        assert_eq!(i64::from(s.ability_exp[7]), exp::total_for_level(exp::MAX_LEVEL));
        assert!(s.apply(&Event::ExpChanged { obj_id: 0x77, delta: 5, add_ability_exp: false }));
        assert_eq!(i64::from(s.ability_exp[6]), exp::total_for_level(exp::MAX_LEVEL));
        assert!(s.apply(&Event::CombatChanged { obj_id: 0x77, engaged: true }));
        assert!(s.in_combat);
        assert!(s.apply(&Event::UnitDeath { obj_id: 0x77 }));
        assert!(s.dead && s.hp == 0 && !s.in_combat);
        assert!(s.apply(&Event::Resurrected { obj_id: 0x77, pos: Default::default(), yaw: 0.0 }));
        assert!(!s.dead);
        assert!(s.apply(&Event::LeftWorld { target: 1 }));
        assert_eq!(s.character_id, 0);
    }

    #[test]
    fn ability_exp_comes_from_the_own_unit_state() {
        let mut s = PlayerStats::default();
        assert!(s.apply(&selected()));
        let mut unit = crate::state::tests::unit(0x77);
        // ten abilities from id 1 (the server's character model), order 255 = not active
        unit.ability_exp = (1..=10).map(|id| (id * 1_000, if id == 1 { 0 } else { 255 })).collect();
        let spawn = |unit: crate::model::Unit| {
            Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn { obj_id: 0x77, state: CharacterState::default(), unit }))
        };
        assert!(s.apply(&spawn(unit.clone())));
        assert_eq!(s.ability_exp, [0, 1_000, 2_000, 3_000, 4_000, 5_000, 6_000, 7_000, 8_000, 9_000, 10_000]);
        // eleven: the general ability first
        unit.ability_exp.insert(0, (42, 255));
        assert!(s.apply(&spawn(unit)));
        assert_eq!(s.ability_exp[..3], [42, 1_000, 2_000]);
    }

    #[test]
    fn exp_bar_uses_the_level_table() {
        let s = PlayerStats { level: 2, exp: 900, ..PlayerStats::default() };
        // Level 2 starts at 400 and level 3 at 1400.
        assert_eq!(s.exp_progress(), (500, 1000));
        let max = PlayerStats { level: 55, exp: 179_307_360 + 5, ..PlayerStats::default() };
        assert_eq!(max.exp_progress(), (5, 0));
    }

    #[test]
    fn buffs_start_from_the_unit_and_follow_the_packets() {
        let mut b = Buffs::default();
        let mut unit = crate::state::tests::unit(5);
        unit.buffs = vec![buff(2, 8_000_011), buff(3, 2423)];
        assert!(b.apply(&Event::UnitSpawned(Box::new(unit))));
        assert_eq!(b.of(5).len(), 2);
        assert_eq!(b.of(5)[0].buff.buff_id, 8_000_011);
        assert!(b.of(5)[0].remaining_ms().unwrap() <= 10_000);
        // A new buff, and a refresh of an existing index replaces it.
        assert!(b.apply(&Event::BuffCreated { obj_id: 5, buff: buff(9, 100) }));
        assert!(b.apply(&Event::BuffCreated { obj_id: 5, buff: Buff { stacks: 3, ..buff(9, 100) } }));
        assert_eq!(b.of(5).len(), 3);
        assert_eq!(b.of(5)[2].buff.stacks, 3);
        assert!(b.apply(&Event::BuffRemoved { obj_id: 5, index: 2 }));
        assert!(!b.apply(&Event::BuffRemoved { obj_id: 5, index: 2 }));
        assert_eq!(b.of(5).iter().map(|e| e.buff.index).collect::<Vec<_>>(), vec![3, 9]);
        assert!(b.of(404).is_empty());
        assert!(b.apply(&Event::UnitsRemoved(vec![5])));
        assert!(b.of(5).is_empty());
    }

    #[test]
    fn labor_matches_executed_native_signed_add_without_clamping() {
        // Original3930b4d0; native-labor-delta-oracle.json. No regeneration estimate.
        for (initial, amount, expected) in [
            (50, -60, -10),
            (i32::MAX, 1, i32::MIN),
            (i32::MIN, -1, i32::MAX),
            (-10, 20, 10),
            (0, 0, 0),
            (-1, 1, 0),
            (i32::MAX, i32::MAX, -2),
        ] {
            let mut stats = PlayerStats { labor: initial, ..PlayerStats::default() };
            assert!(stats.apply(&Event::LaborChanged { amount, action: 0, point: 0, step: 0 }));
            assert_eq!(stats.labor, expected);
            // This existing bounded history describes app receipt observations,
            // including zero, rather than claiming native zero-delta callbacks.
            assert_eq!(stats.labor_log, [(amount, expected)]);
            assert_eq!(stats.labor_seq, 1);
        }
        let mut stats = PlayerStats { labor: 50, ..PlayerStats::default() };
        for amount in [-60, 20] {
            stats.apply(&Event::LaborChanged { amount, action: 0, point: 0, step: 0 });
        }
        assert_eq!(stats.labor, 10);
        assert_eq!(stats.labor_log, [(-60, -10), (20, 10)]);
    }

    #[test]
    fn labor_header_matches_executed_native_unsigned_projection() {
        //391b9884 passes raw header+10 to39089d50; MOVZX stores raw+5fc.
        for (wire, expected) in [(0i16, 0), (50, 50), (32767, 32767), (i16::MIN, 32768), (-1, 65535)] {
            let Event::CharacterSelected(mut selected) = selected() else { unreachable!() };
            selected.character.labor_power = wire;
            let mut stats = PlayerStats::default();
            stats.apply(&Event::CharacterSelected(selected));
            assert_eq!(stats.labor, expected);
        }
    }

}
