//! Original-client stack folding, separate from the active lane's raw index-based list.
use super::BuffEntry;
use crate::{buff::{Buff, BuffKind}, session::Event};
use std::{collections::HashMap, sync::Arc, time::Instant};

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
    pub rules: Arc<HashMap<u32, NativeBuffRule>>,
    by_unit: HashMap<u32, Vec<BuffEntry>>,
    catalog_loaded: bool,
    own_identity: Option<(u32, u32)>,
}
impl NativeBuffs {
    /// Replace static rules without discarding authoritative snapshot counts. Unknown histories
    /// remain unknown until another unit snapshot; installing a catalog cannot reconstruct them.
    pub fn set_catalog(&mut self, rules: HashMap<u32, NativeBuffRule>) {
        self.rules = Arc::new(rules);
        self.catalog_loaded = true;
    }

    pub fn catalog_loaded(&self) -> bool { self.catalog_loaded }

    pub fn of(&self, obj_id: u32) -> Option<&[BuffEntry]> {
        self.by_unit.get(&obj_id).map(Vec::as_slice)
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::PlayerSpawn(s) => {
                self.own_identity = match s.unit.kind {
                    crate::model::UnitKind::Character { char_id } if char_id != 0 && char_id == s.state.character.id && s.obj_id == s.unit.obj_id => Some((s.obj_id, char_id)),
                    _ => None,
                };
                self.seed(s.obj_id, &s.unit.buffs);
            }
            Event::OwnUnitSnapshot(u) if matches!(u.kind, crate::model::UnitKind::Character { char_id } if self.own_identity == Some((u.obj_id, char_id))) => self.seed(u.obj_id, &u.buffs),
            Event::UnitSpawned(u) => self.seed(u.obj_id, &u.buffs),
            Event::BuffCreated { obj_id, buff } => {
                let Some(rule) = self.rules.get(&buff.buff_id).copied() else {
                    if !self.catalog_loaded { self.by_unit.remove(obj_id); }
                    return;
                };
                if rule.kind == BuffKind::Unknown { return; }
                let Some(list) = self.by_unit.get_mut(obj_id) else { return };
                let found = if rule.stack_rule_id == 6 {
                    None
                } else {
                    list.iter().position(|e| {
                        e.buff.kind == rule.kind && e.buff.buff_id == buff.buff_id && (!rule.per_unit_creation || e.buff.caster_char_id == buff.caster_char_id)
                    })
                };
                if found.is_none() && list.iter().filter(|e| e.buff.kind == rule.kind).count() >= kind_capacity(rule.kind) {
                    // Native allocation returns null before changing any existing row.
                    return;
                }
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
            Event::BuffUpdated { obj_id, index, stacks, charged } => {
                let Some(list) = self.by_unit.get_mut(obj_id) else { return };
                let Some(i) = first_index(list, *index) else { return };
                if list[i].buff.kind == BuffKind::Unknown {
                    self.by_unit.remove(obj_id);
                } else if self.rules.contains_key(&list[i].buff.buff_id) {
                    list[i].buff.stacks = *stacks;
                    list[i].buff.charged = *charged;
                } else if !self.catalog_loaded {
                    self.by_unit.remove(obj_id);
                }
            }
            Event::BuffRemoved { obj_id, index } => {
                let Some(list) = self.by_unit.get_mut(obj_id) else { return };
                let Some(i) = first_index(list, *index) else { return };
                if list[i].buff.kind == BuffKind::Unknown {
                    self.by_unit.remove(obj_id);
                } else if self.rules.contains_key(&list[i].buff.buff_id) {
                    list.remove(i);
                } else if !self.catalog_loaded {
                    // Presence of the native descriptor cannot yet be decided.
                    self.by_unit.remove(obj_id);
                }
                // A complete catalog proves missing descriptor: native callback does not erase.
            }
            // These diagnostics prove unread data was discarded. Normal unsupported packets
            // have a known frame boundary and do not by themselves prove a buff-history gap.
            Event::GameDecodeFailed { .. } => self.by_unit.clear(),
            Event::UnhandledGamePacket { level, len, .. } if *level == crate::game::LEVEL_COMPRESSED && *len > 0 => self.by_unit.clear(),
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.by_unit.remove(id);
                    if self.own_identity.is_some_and(|(own, _)| own == *id) { self.own_identity = None; }
                }
            }
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => { self.by_unit.clear(); self.own_identity = None; },
            _ => {}
        }
    }
    fn seed(&mut self, id: u32, buffs: &[crate::buff::Buff]) {
        if [BuffKind::Good, BuffKind::Bad, BuffKind::Hidden].into_iter()
            .any(|kind| buffs.iter().filter(|buff| buff.kind == kind).count() > kind_capacity(kind)) {
            // Preserve raw lane receipts elsewhere; malformed original framing is not emulated.
            self.by_unit.remove(&id);
            return;
        }
        let now = Instant::now();
        self.by_unit
            .insert(id, buffs.iter().cloned().map(|buff| BuffEntry { buff, received: now }).collect());
    }
}

fn kind_capacity(kind: BuffKind) -> usize {
    match kind { BuffKind::Good => 32, BuffKind::Bad | BuffKind::Hidden => 24, BuffKind::Unknown => 0 }
}
fn kind_order(kind: BuffKind) -> u8 {
    match kind { BuffKind::Good => 0, BuffKind::Bad => 1, BuffKind::Hidden => 2, BuffKind::Unknown => 3 }
}

fn first_index(list: &[BuffEntry], index: u32) -> Option<usize> {
    list.iter().enumerate().filter(|(_, e)| e.buff.index == index)
        .min_by_key(|(i, e)| (kind_order(e.buff.kind), *i)).map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buff::Buff;
    fn assert_entry_eq(actual: &BuffEntry, expected: &BuffEntry) {
        assert_eq!(actual.buff, expected.buff);
        assert_eq!(actual.received, expected.received);
    }
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
        Arc::make_mut(&mut v.rules).insert(
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
    fn actual_decode_loss_invalidates_only_native_history_until_snapshot() {
        let mut world=crate::state::WorldState::default();world.buffs.native=view(4,false);
        world.buffs.native.set_catalog((*world.buffs.native.rules).clone());
        let buff=Buff{index:7,buff_id:77,kind:BuffKind::Good,..Buff::default()};
        let mut snapshot=crate::state::tests::unit(9);snapshot.buffs=vec![buff.clone()];
        world.apply(&Event::UnitSpawned(Box::new(snapshot.clone())));
        for event in [Event::UnhandledGamePacket{level:crate::game::LEVEL_GAME,opcode:0x3ff,len:99},
            Event::UnhandledGamePacket{level:crate::game::LEVEL_COMPRESSED,opcode:0x3ff,len:0}] {
            world.apply(&event);assert!(world.buffs.native.of(9).is_some());
        }
        for loss in [Event::UnhandledGamePacket{level:crate::game::LEVEL_COMPRESSED,opcode:0x3ff,len:1},
            Event::GameDecodeFailed{message:"truncated actual packet".into()}] {
            world.apply(&loss);assert!(world.buffs.native.of(9).is_none());
            assert_eq!(world.buffs.of(9).len(),1);assert!(world.buffs.native.catalog_loaded());assert!(world.buffs.native.rules.contains_key(&77));
            world.apply(&created(8,10,1));assert!(world.buffs.native.of(9).is_none(),"Created cannot repair unknown history");
            world.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:8,charged:8});assert!(world.buffs.native.of(9).is_none());
            world.apply(&Event::UnitSpawned(Box::new(snapshot.clone())));assert!(world.buffs.native.of(9).is_some());
        }
    }

    #[test]
    fn buff_updated_first_group_descriptor_guards_and_no_clamp() {
        let mut v=view(4,false);let rule=v.rules[&77];let mut bad_rule=rule;bad_rule.kind=BuffKind::Bad;
        v.set_catalog([(77,rule),(88,bad_rule)].into_iter().collect());
        let good=Buff{index:7,buff_id:77,kind:BuffKind::Good,duration_ms:10000,elapsed_ms:500,tick_ms:100,raw_tick_index:Some(5),native_tick_index:Some(5),raw_cooldown_skill:Some(99),..Buff::default()};
        let bad=Buff{buff_id:88,kind:BuffKind::Bad,..good.clone()};v.seed(9,&[bad,good]);
        let old_bad=v.of(9).unwrap()[0].clone();let mut expected=v.of(9).unwrap()[1].clone();
        for (stacks,charged) in [(i32::MAX,i32::MIN),(0,0),(-1,-2)] {
            v.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks,charged});expected.buff.stacks=stacks;expected.buff.charged=charged;
            assert_entry_eq(&v.of(9).unwrap()[0],&old_bad);assert_entry_eq(&v.of(9).unwrap()[1],&expected);
        }
        v.apply(&Event::BuffUpdated{obj_id:10,index:7,stacks:99,charged:99});assert!(v.of(10).is_none());
        v.set_catalog(HashMap::new());v.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:99,charged:99});assert_entry_eq(&v.of(9).unwrap()[1],&expected);
        let snapshot=expected.buff.clone();let mut unknown=NativeBuffs::default();unknown.seed(9,&[snapshot]);
        unknown.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:99,charged:99});assert!(unknown.of(9).is_none());
        unknown.seed(9,&[]);unknown.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:99,charged:99});assert!(unknown.of(9).unwrap().is_empty());
    }

    #[test]
    fn buff_updated_world_state_keeps_received_time_and_never_creates_rows() {
        let mut world=crate::state::WorldState::default();world.buffs.native=view(4,false);
        let buff=Buff{index:7,buff_id:77,kind:BuffKind::Good,duration_ms:10000,elapsed_ms:500,stacks:1,..Buff::default()};
        world.buffs.native.seed(9,&[buff.clone()]);
        let mut raw=BuffEntry{buff,received:Instant::now()-std::time::Duration::from_secs(1)};
        world.buffs.by_unit.insert(9,vec![raw.clone()]);let mut native=world.buffs.native.of(9).unwrap()[0].clone();
        world.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:-7,charged:99});
        raw.buff.stacks=-7;raw.buff.charged=99;native.buff.stacks=-7;native.buff.charged=99;
        assert_eq!(world.buffs.of(9).len(),1);assert_entry_eq(&world.buffs.of(9)[0],&raw);
        assert_eq!(world.buffs.native.of(9).unwrap().len(),1);assert_entry_eq(&world.buffs.native.of(9).unwrap()[0],&native);
        world.apply(&Event::BuffUpdated{obj_id:10,index:7,stacks:1,charged:1});assert!(world.buffs.of(10).is_empty());assert!(world.buffs.native.of(10).is_none());
        world.buffs.by_unit.remove(&9);
        assert!(world.buffs.apply(&Event::BuffUpdated{obj_id:9,index:7,stacks:77,charged:0}),"native-only change must remain observable");
        assert_eq!(world.buffs.native.of(9).unwrap()[0].buff.stacks,77);assert!(world.buffs.of(9).is_empty());
    }

    #[test]
    fn removal_descriptor_admission_distinguishes_missing_from_unloaded_catalog() {
        let snapshot=Buff{index:7,buff_id:99,kind:BuffKind::Good,..Buff::default()};
        let mut v=NativeBuffs::default();v.seed(9,&[]);
        v.apply(&Event::BuffRemoved{obj_id:9,index:7});assert!(v.of(9).unwrap().is_empty());
        v.seed(9,&[snapshot.clone()]);v.apply(&Event::BuffRemoved{obj_id:9,index:7});assert!(v.of(9).is_none());
        v.set_catalog(HashMap::new());assert!(v.catalog_loaded());assert!(v.of(9).is_none());
        v.seed(9,&[snapshot]);v.apply(&Event::BuffRemoved{obj_id:9,index:7});assert_eq!(v.of(9).unwrap().len(),1);
        v.apply(&Event::CharacterSelected(Box::default()));assert!(v.catalog_loaded());assert!(v.of(9).is_none());
        v.seed(9,&[]);assert!(v.of(9).unwrap().is_empty());
        v.apply(&created(1,0,1));assert!(v.of(9).unwrap().is_empty(),"complete missing descriptor is a native no-op");
        v.set_catalog([(77,NativeBuffRule{kind:BuffKind::Unknown,per_unit_creation:false,stack_rule_id:4,max_stack:1,max_charge:0})].into_iter().collect());
        v.apply(&created(1,0,1));assert!(v.of(9).unwrap().is_empty(),"static invalid group cannot allocate");
        v.by_unit.remove(&9);v.apply(&created(1,0,1));
        v.apply(&Event::BuffRemoved{obj_id:9,index:7});assert!(v.of(9).is_none(),"unknown history is not reconstructed");
    }

    #[test]
    fn removal_uses_first_physical_group_order_and_stably_compacts_duplicates() {
        let kinds=[BuffKind::Hidden,BuffKind::Good,BuffKind::Bad,BuffKind::Good];
        let mut v=NativeBuffs::default();let mut buffs=Vec::new();let mut rules=HashMap::new();
        for (i,kind) in kinds.into_iter().enumerate() {
            let id=i as u32+1;buffs.push(Buff{index:7,buff_id:id,kind,..Buff::default()});
            rules.insert(id,NativeBuffRule{kind,per_unit_creation:false,stack_rule_id:4,max_stack:2,max_charge:0});
        }
        v.set_catalog(rules);v.seed(9,&buffs);
        for expected in [vec![1,3,4],vec![1,3],vec![1],vec![]] {
            v.apply(&Event::BuffRemoved{obj_id:9,index:7});
            assert_eq!(v.of(9).unwrap().iter().map(|e|e.buff.buff_id).collect::<Vec<_>>(),expected);
        }
        v.apply(&Event::BuffRemoved{obj_id:9,index:7});assert!(v.of(9).unwrap().is_empty());
    }

    #[test]
    fn native_capacity_refuses_new_records_but_allows_existing_reconciliation() {
        for kind in [BuffKind::Good,BuffKind::Bad,BuffKind::Hidden] {
            let cap=kind_capacity(kind);let mut v=view(4,false);
            let mut rule=v.rules[&77];rule.kind=kind;v.set_catalog([(77,rule),(88,rule)].into_iter().collect());
            let buffs=(0..cap).map(|i|Buff{index:i as u32,buff_id:77,kind,stacks:1,..Buff::default()}).collect::<Vec<_>>();v.seed(9,&buffs);
            let mut event=created(0,0,1);let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.buff_id=88;
            v.apply(&event);assert_eq!(v.of(9).unwrap().len(),cap,"full allocation is a no-op even with colliding index");
            let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.buff_id=77;buff.index=cap as u32;
            v.apply(&event);let list=v.of(9).unwrap();assert_eq!(list.len(),cap);assert_eq!((list[0].buff.index,list[0].buff.stacks),(cap as u32,2));
            v.apply(&Event::BuffRemoved{obj_id:9,index:cap as u32});assert_eq!(v.of(9).unwrap().len(),cap-1);
            let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.buff_id=88;buff.index=999;
            v.apply(&event);assert_eq!(v.of(9).unwrap().len(),cap);assert_eq!(v.of(9).unwrap().last().unwrap().buff.index,999);
        }
    }

    #[test]
    fn oversized_native_snapshot_is_unknown_without_truncating_lane_receipt() {
        use crate::{buff::tests::net_buff,wire::{Reader,Writer}};
        for overflow in 0..3 {
            let counts=[32usize,24,24];let mut w=Writer::new();
            for (group,count) in counts.into_iter().enumerate() {
                let n=count+usize::from(group==overflow);w.u8(n as u8);
                for index in 0..n {net_buff(&mut w,index as u32,77);}
            }
            w.u8(0xa5);let bytes=w.finish();let mut r=Reader::new(&bytes);let buffs=Buff::read_lists(&mut r).unwrap();
            assert_eq!(buffs.len(),81);assert_eq!(r.u8().unwrap(),0xa5);
            let mut v=NativeBuffs::default();v.seed(9,&buffs);assert!(v.of(9).is_none());assert_eq!(buffs.len(),81);
            v.seed(9,&[]);assert!(v.of(9).unwrap().is_empty());
        }
    }

    #[test]
    fn per_unit_source_matching_uses_persistent_caster_not_item_or_mount_identity() {
        use crate::skill::SkillCasterData;
        let item=SkillCasterData::Item{obj_id:10,item_id:99,template_id:77,type1:1,type2:2};
        let mount=SkillCasterData::Mount{obj_id:20,mount_skill_id:33};
        let mut v=view(4,true);let mut event=created(1,0,1);
        let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.raw_caster=Some(item);buff.caster=item.projection();
        v.apply(&event);
        let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.index=2;buff.raw_caster=Some(mount);buff.caster=mount.projection();
        v.apply(&event);let list=v.of(9).unwrap();assert_eq!(list.len(),1);
        assert_eq!((list[0].buff.index,list[0].buff.stacks,list[0].buff.raw_caster),(2,2,Some(item)));
        let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.index=3;buff.caster_char_id=11;
        v.apply(&event);assert_eq!(v.of(9).unwrap().len(),2);
    }
    #[test]
    fn cooldown_skill_preserves_constructor_identity_across_repeat_and_snapshot() {
        let mut v=view(4,false);let mut event=created(1,10,1);
        let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.raw_cooldown_skill=Some(77);
        v.apply(&event);assert_eq!(v.of(9).unwrap()[0].buff.raw_cooldown_skill,Some(77));
        let Event::BuffCreated{buff,..}=&mut event else{unreachable!()};buff.raw_cooldown_skill=Some(88);buff.index=2;
        v.apply(&event);let result=&v.of(9).unwrap()[0].buff;
        assert_eq!((result.index,result.raw_cooldown_skill),(2,Some(77)));
        let mut snapshot=result.clone();snapshot.raw_cooldown_skill=Some(u32::MAX);
        v.seed(9,&[snapshot]);assert_eq!(v.of(9).unwrap()[0].buff.raw_cooldown_skill,Some(u32::MAX));
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
        Arc::make_mut(&mut v.rules).clear();
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
    #[test]
    fn static_catalog_shared_by_clone_and_preserved_across_runtime_reset() {
        let mut native=view(4,false);native.set_catalog((*native.rules).clone());let copied=native.clone();
        assert!(Arc::ptr_eq(&native.rules,&copied.rules));
        native.seed(9,&[]);assert!(native.of(9).is_some());
        native.apply(&Event::Disconnected{message:String::new()});
        assert!(native.of(9).is_none());assert!(native.catalog_loaded());
        assert!(Arc::ptr_eq(&native.rules,&copied.rules));
        assert_eq!(*native.rules,*copied.rules);
        native.set_catalog(HashMap::new());
        assert!(!Arc::ptr_eq(&native.rules,&copied.rules));assert_eq!(copied.rules.len(),1);
    }

}
