//! Original-client stack folding, separate from the active lane's raw index-based list.
use super::BuffEntry;
use crate::{buff::{Buff, BuffKind}, session::Event};
use std::{collections::HashMap, time::Instant};

/// Exact static `buffs` columns consumed by native 3989e080. No inferred defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeBuffRule {
    pub kind: BuffKind,
    pub per_unit_creation: bool,
    pub stack_rule_id: u32,
    pub max_stack: i32,
    pub max_charge: i32,
}

/// A received-only original-client view. Install the static catalog before folding packets.
/// `None` means history/catalog is incomplete, rather than an empty known list.
#[derive(Clone, Debug, Default)]
pub struct NativeBuffs {
    pub rules: HashMap<u32, NativeBuffRule>,
    by_unit: HashMap<u32, Vec<BuffEntry>>,
}
impl NativeBuffs {
    /// Replace static rules without discarding authoritative snapshot counts. Unknown histories
    /// remain unknown until another unit snapshot; installing a catalog cannot reconstruct them.
    pub fn set_catalog(&mut self, rules: HashMap<u32, NativeBuffRule>) {
        self.rules = rules;
    }

    pub fn of(&self, obj_id: u32) -> Option<&[BuffEntry]> {
        self.by_unit.get(&obj_id).map(Vec::as_slice)
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::PlayerSpawn(s) => self.seed(s.obj_id, &s.unit.buffs),
            Event::UnitSpawned(u) => self.seed(u.obj_id, &u.buffs),
            Event::BuffCreated { obj_id, buff } => {
                let Some(rule) = self.rules.get(&buff.buff_id).copied().filter(|r| r.kind != BuffKind::Unknown) else {
                    self.by_unit.remove(obj_id);
                    return;
                };
                let Some(list) = self.by_unit.get_mut(obj_id) else { return };
                let found = if rule.stack_rule_id == 6 {
                    None
                } else {
                    list.iter().position(|e| {
                        e.buff.kind == rule.kind && e.buff.buff_id == buff.buff_id && (!rule.per_unit_creation || e.buff.caster_char_id == buff.caster_char_id)
                    })
                };
                // An index already belonging to a different record needs the manager's separate
                // replacement path. Do not guess that path or create duplicate instance IDs.
                if list.iter().position(|e| e.buff.index == buff.index).is_some_and(|i| Some(i) != found) {
                    self.by_unit.remove(obj_id);
                    return;
                }
                let now = Instant::now();
                if let Some(i) = found {
                    let old = &mut list[i];
                    // Existing runtime identity/caster/ability/skill fields are retained.
                    old.buff.index = buff.index;
                    old.buff.duration_ms = if rule.stack_rule_id == 5 {
                        old.buff.duration_ms.wrapping_add(buff.duration_ms)
                    } else {
                        buff.duration_ms
                    };
                    old.buff.elapsed_ms = if rule.stack_rule_id == 5 {
                        old.buff.elapsed_ms.max(buff.elapsed_ms)
                    } else {
                        old.buff.elapsed_ms.min(buff.elapsed_ms)
                    };
                    old.buff.tick_ms = buff.tick_ms;
                    // This receipt does not carry a new authoritative tick index.
                    old.buff.raw_tick_index = buff.raw_tick_index;
                    old.buff.native_tick_index = Some(Buff::created_tick_index(old.buff.elapsed_ms, old.buff.tick_ms));
                    match rule.stack_rule_id {
                        2 => old.buff.charged = buff.charged,
                        3 => old.buff.charged = old.buff.charged.wrapping_add(buff.charged),
                        4 => old.buff.stacks = old.buff.stacks.wrapping_add(1),
                        _ => {}
                    }
                    old.buff.stacks = old.buff.stacks.min(rule.max_stack);
                    if rule.stack_rule_id == 3 {
                        old.buff.charged = old.buff.charged.min(rule.max_charge);
                    }
                    old.received = now;
                } else {
                    let mut new = buff.clone();
                    new.kind = rule.kind;
                    new.native_tick_index = Some(Buff::created_tick_index(new.elapsed_ms, new.tick_ms));
                    new.stacks = 1.min(rule.max_stack);
                    if rule.stack_rule_id == 3 {
                        new.charged = new.charged.min(rule.max_charge);
                    }
                    list.push(BuffEntry { buff: new, received: now });
                }
            }
            Event::BuffRemoved { obj_id, index } => {
                if let Some(list) = self.by_unit.get_mut(obj_id) {
                    list.retain(|e| e.buff.index != *index);
                }
            }
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.by_unit.remove(id);
                }
            }
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => self.by_unit.clear(),
            _ => {}
        }
    }
    fn seed(&mut self, id: u32, buffs: &[crate::buff::Buff]) {
        let now = Instant::now();
        self.by_unit
            .insert(id, buffs.iter().cloned().map(|buff| BuffEntry { buff, received: now }).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buff::Buff;
    fn created(index: u32, caster: u32, charge: i32) -> Event {
        Event::BuffCreated {
            obj_id: 9,
            buff: Buff {
                index,
                buff_id: 77,
                caster_char_id: caster,
                charged: charge,
                stacks: charge,
                duration_ms: 1000,
                ability_level: 5,
                ..Buff::default()
            },
        }
    }
    fn view(rule: u32, per_unit_creation: bool) -> NativeBuffs {
        let mut v = NativeBuffs::default();
        v.rules.insert(
            77,
            NativeBuffRule {
                kind: BuffKind::Good,
                per_unit_creation,
                stack_rule_id: rule,
                max_stack: 2,
                max_charge: 10,
            },
        );
        v.seed(9, &[]);
        v
    }
    #[test]
    fn stack_rule_uses_received_history_not_charge_and_rekeys_removal() {
        let mut v = view(4, false);
        v.apply(&created(1, 10, 90));
        assert_eq!((v.of(9).unwrap()[0].buff.stacks, v.of(9).unwrap()[0].buff.charged), (1, 90));
        v.apply(&created(2, 20, 400));
        v.apply(&created(3, 30, 500));
        let b = &v.of(9).unwrap()[0].buff;
        assert_eq!((b.index, b.stacks, b.charged, b.caster_char_id), (3, 2, 90, 10));
        v.apply(&Event::BuffRemoved { obj_id: 9, index: 1 });
        assert_eq!(v.of(9).unwrap().len(), 1);
        v.apply(&Event::BuffRemoved { obj_id: 9, index: 3 });
        assert!(v.of(9).unwrap().is_empty());
    }
    #[test]
    fn caster_character_partition_and_independent_rule_are_distinct() {
        for (rule, per, expected) in [(1, false, 1), (1, true, 2), (6, false, 3)] {
            let mut v = view(rule, per);
            v.apply(&created(1, 10, 4));
            v.apply(&created(2, 20, 4));
            v.apply(&created(3, 20, 4));
            assert_eq!(v.of(9).unwrap().len(), expected);
        }
    }
    #[test]
    fn charges_change_only_under_their_rules() {
        for (rule, charge) in [(1, 7), (2, 8), (3, 10), (4, 7), (5, 7)] {
            let mut v = view(rule, false);
            v.apply(&created(1, 10, 7));
            v.apply(&created(2, 10, 8));
            let b = &v.of(9).unwrap()[0].buff;
            assert_eq!(b.charged, charge);
            assert_eq!(b.duration_ms, if rule == 5 { 2000 } else { 1000 });
        }
    }
    #[test]
    fn repeated_created_tick_projection_matches_executed_reconciliation_fragment() {
        for (rule, old_elapsed, incoming, tick, expected_elapsed, expected_index) in [
            (1,100,25,7,25,3),(1,25,100,7,25,3),(1,u32::MAX,1,1,1,1),
            (1,1,0x80000000,3,1,0),(1,100,25,0,25,0),
            (5,100,25,7,100,14),(5,25,100,7,100,14),(5,u32::MAX,1,1,u32::MAX,-1),
            (5,1,0x80000000,3,0x80000000,715827882),(5,100,25,0,100,0)] {
            let mut v=view(rule,false);
            let mut event=created(1,10,1);
            let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};
            buff.elapsed_ms=old_elapsed;buff.duration_ms=2000;
            v.apply(&event);
            let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};
            buff.elapsed_ms=incoming;buff.duration_ms=1000;buff.tick_ms=tick;
            v.apply(&event);
            let result=&v.of(9).unwrap()[0].buff;
            assert_eq!((result.elapsed_ms,result.native_tick_index),(expected_elapsed,Some(expected_index)));
            assert_eq!(result.raw_tick_index,None);
            assert_eq!(result.duration_ms,if rule==5{3000}else{1000});
        }
    }
    #[test]
    fn snapshot_tick_index_survives_seed_and_created_invalidates_it() {
        let mut v = view(4, false);
        let event = created(1, 10, 1);
        let Event::BuffCreated { buff, .. } = &event else { unreachable!() };
        let mut snapshot = buff.clone();
        snapshot.raw_tick_index = Some(-7);
        snapshot.kind = BuffKind::Good;
        v.seed(9, &[snapshot]);
        assert_eq!(v.of(9).unwrap()[0].buff.raw_tick_index, Some(-7));
        v.apply(&event);
        assert_eq!(v.of(9).unwrap()[0].buff.raw_tick_index, None);
    }
    #[test]
    fn missing_history_or_catalog_stays_unknown_and_snapshot_recovers() {
        let mut v = view(4, false);
        v.rules.clear();
        v.apply(&created(1, 10, 7));
        assert!(v.of(9).is_none());
        v = view(4, false);
        v.by_unit.clear();
        v.apply(&created(1, 10, 7));
        assert!(v.of(9).is_none());
        let snapshot = Buff {
            stacks: 6,
            charged: 40,
            ..Buff::default()
        };
        v.seed(9, &[snapshot]);
        assert_eq!(v.of(9).unwrap()[0].buff.stacks, 6);
        v.apply(&Event::UnitsRemoved(vec![9]));
        assert!(v.of(9).is_none());
        assert_eq!(v.rules.len(), 1);
    }
    #[test]
    fn lifecycle_clears_received_views_but_keeps_catalog_and_raw_integration() {
        let events = [
            Event::CharacterSelected(Box::default()),
            Event::Kicked {
                reason: 1,
                message: String::new(),
            },
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
        ];
        for event in events {
            let mut buffs = super::super::Buffs::default();
            buffs.native = view(4, false);
            buffs.apply(&created(1, 10, 7));
            assert_eq!(buffs.native.of(9).unwrap()[0].buff.stacks, 1);
            assert_eq!(buffs.of(9)[0].buff.stacks, 7);
            buffs.apply(&event);
            assert!(buffs.native.of(9).is_none());
            assert!(buffs.of(9).is_empty());
            assert_eq!(buffs.native.rules.len(), 1);
        }
    }
    #[test]
    fn catalog_install_keeps_snapshot_and_collision_marks_unknown() {
        let mut v = view(4, false);
        let snapshot = Buff {
            index: 1,
            buff_id: 77,
            kind: BuffKind::Good,
            stacks: 8,
            ..Buff::default()
        };
        v.seed(9, &[snapshot]);
        v.set_catalog(HashMap::new());
        assert_eq!(v.of(9).unwrap()[0].buff.stacks, 8);
        let mut v = view(6, false);
        v.apply(&created(1, 10, 1));
        v.apply(&created(1, 10, 1));
        assert!(v.of(9).is_none());
    }
}
