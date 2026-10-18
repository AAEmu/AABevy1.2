//! Received expedition/faction caches. Lists upsert; empty lists are not reset markers.
use crate::{
    session::Event,
    wire::{Reader, Result, WireError},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpeditionFaction {
    pub id: u32,
    pub aggro_link: u8,
    pub mother_id: u32,
    pub name: Vec<u8>,
    pub owner_id: u32,
    pub owner_name: Vec<u8>,
    pub unit_owner_type: i8,
    pub political_system: u8,
    pub created: i64,
    pub diplomacy_target: u8,
    pub allow_change_name: u8,
}
impl ExpeditionFaction {
    pub fn read_lane(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            aggro_link: r.u8()?,
            mother_id: r.u32()?,
            name: r.blob()?.to_vec(),
            owner_id: r.u32()?,
            owner_name: r.blob()?.to_vec(),
            unit_owner_type: r.i8()?,
            political_system: r.u8()?,
            created: r.i64()?,
            diplomacy_target: r.u8()?,
            allow_change_name: r.u8()?,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpeditionRolePolicy {
    pub expedition_id: u32,
    pub role: u8,
    pub name: Vec<u8>,
    /// Native order: dominion_declare, invite, expel, promote, dismiss, chat,
    /// manager_chat, siege_master, join_siege. Preserve raw boolean bytes.
    pub permissions: [u8; 9],
}
impl ExpeditionRolePolicy {
    pub fn read_lane(r: &mut Reader) -> Result<Self> {
        let expedition_id = r.u32()?;
        let role = r.u8()?;
        let name = r.blob()?.to_vec();
        let mut permissions = [0; 9];
        for value in &mut permissions {
            *value = r.u8()?;
        }
        Ok(Self {
            expedition_id,
            role,
            name,
            permissions,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExpeditionMember {
    pub expedition_id: u32,
    pub character_id: u32,
    pub in_party: u8,
    pub online: u8,
    pub last_world_leave_time: i64,
    pub name: Vec<u8>,
    pub level: u8,
    pub zone_id: u32,
    /// Native serializer calls this field id; no further meaning inferred.
    pub id3: u32,
    pub abilities: [u8; 3],
    pub role: u8,
    /// Original fixed-position wire integers, not re-encoded floating coordinates.
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub memo: Vec<u8>,
    pub transfer_requested_time: i64,
}
impl ExpeditionMember {
    /// Native397bd700 and lane ExpeditionMember::write have this field order.
    pub fn read_lane(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            expedition_id: r.u32()?,
            character_id: r.u32()?,
            in_party: r.u8()?,
            online: r.u8()?,
            last_world_leave_time: r.i64()?,
            name: r.blob()?.to_vec(),
            level: r.u8()?,
            zone_id: r.u32()?,
            id3: r.u32()?,
            abilities: [r.u8()?, r.u8()?, r.u8()?],
            role: r.u8()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            memo: r.blob()?.to_vec(),
            transfer_requested_time: r.i64()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpeditionCreatedMember {
    pub obj_id:u32,
    pub character_id:u32,
    pub name:Vec<u8>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ExpeditionEvent {
    ShowRenameUi,
    Renamed { expedition_id:u32, name:Vec<u8>, by_gm:u8 },
    Created { faction:ExpeditionFaction, owner_obj_id:u32, members:Vec<ExpeditionCreatedMember> },
    UnitFactionChanged { obj_id:u32, name:Vec<u8>, old_faction_id:u32, new_faction_id:u32, temporary:u8 },
    PolicyChanged { policy:ExpeditionRolePolicy, success:u8 },
    RoleChanged { character_id:u32, role:u8, name:Vec<u8> },
    OwnerChanged { old_owner_id:u32, new_owner_id:u32, name:Vec<u8> },
    Dismissed { expedition_id:u32, success:u8 },
    MembershipChanged { obj_id:u32, character_id:u32, kicker:Vec<u8>, name:Vec<u8>, old_expedition_id:u32, new_expedition_id:u32, expel:u8 },
    /// SC01a: complete raw invitation; task admission belongs to the native UI bridge.
    Invitation { inviter_id:u32, inviter_name:Vec<u8>, expedition_id:u32, expedition_name:Vec<u8> },
    Factions(Vec<ExpeditionFaction>),
    Policies(Vec<ExpeditionRolePolicy>),
    Members {
        total: u32,
        expedition_id: u32,
        members: Vec<ExpeditionMember>,
    },
    MemberStatus {
        member: ExpeditionMember,
        flag: u8,
    },
}
impl ExpeditionEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x008 | 0x00b..=0x00f | 0x011 | 0x013 | 0x015..=0x017 | 0x01a..=0x01c)
    }
    pub fn read_lane(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x00f => Self::ShowRenameUi,
            0x011 => Self::Renamed { expedition_id:r.u32()?,name:r.blob()?.to_vec(),by_gm:r.u8()? },
            0x01b => Self::UnitFactionChanged { obj_id:r.bc()?,name:r.blob()?.to_vec(),old_faction_id:r.u32()?,new_faction_id:r.u32()?,temporary:r.u8()? },
            0x013 => {
                let faction=ExpeditionFaction::read_lane(r)?;let owner_obj_id=r.bc()?;let count=r.u8()?;
                let mut members=Vec::with_capacity(count.into());
                for _ in 0..count {members.push(ExpeditionCreatedMember{obj_id:r.bc()?,character_id:r.u32()?,name:r.blob()?.to_vec()});}
                Self::Created{faction,owner_obj_id,members}
            }
            0x00c => Self::PolicyChanged { policy:ExpeditionRolePolicy::read_lane(r)?,success:r.u8()? },
            0x00d => Self::RoleChanged { character_id:r.u32()?,role:r.u8()?,name:r.blob()?.to_vec() },
            0x00e => Self::OwnerChanged { old_owner_id:r.u32()?,new_owner_id:r.u32()?,name:r.blob()?.to_vec() },
            0x015 => Self::Dismissed { expedition_id:r.u32()?,success:r.u8()? },
            0x01c => Self::MembershipChanged { obj_id:r.bc()?,character_id:r.u32()?,kicker:r.blob()?.to_vec(),name:r.blob()?.to_vec(),old_expedition_id:r.u32()?,new_expedition_id:r.u32()?,expel:r.u8()? },
            0x01a => Self::Invitation { inviter_id:r.u32()?,inviter_name:r.blob()?.to_vec(),expedition_id:r.u32()?,expedition_name:r.blob()?.to_vec() },
            0x008 => {
                let count = r.u8()?;
                let mut rows = Vec::with_capacity(count.into());
                for _ in 0..count {
                    rows.push(ExpeditionFaction::read_lane(r)?);
                }
                Self::Factions(rows)
            }
            0x00b => {
                let count = r.u8()?;
                let mut rows = Vec::with_capacity(count.into());
                for _ in 0..count {
                    rows.push(ExpeditionRolePolicy::read_lane(r)?);
                }
                Self::Policies(rows)
            }
            0x016 => {
                let total = r.u32()?;
                let count = r.u8()?;
                let expedition_id = r.u32()?;
                let mut members = Vec::with_capacity(count.into());
                for _ in 0..count {
                    members.push(ExpeditionMember::read_lane(r)?);
                }
                Self::Members { total, expedition_id, members }
            }
            0x017 => Self::MemberStatus {
                member: ExpeditionMember::read_lane(r)?,
                flag: r.u8()?,
            },
            _ => return Err(WireError::Invalid("unknown expedition opcode".into())),
        })
    }
}
/// Source-order, no-argument original Lua callbacks. No receipt is manufactured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpeditionNotice { PolicyChanged, RoleChanged, Updated, MemberStatusChanged, MemberNameChanged, UpdateMainMenu }
impl ExpeditionNotice {
    pub fn event_name(self)->&'static str {
        match self {
            Self::PolicyChanged=>"EXPEDITION_MANAGEMENT_POLICY_CHANGED",
            Self::RoleChanged=>"EXPEDITION_MANAGEMENT_ROLE_CHANGED",
            Self::Updated=>"EXPEDITION_MANAGEMENT_UPDATED",
            Self::MemberStatusChanged=>"EXPEDITION_MANAGEMENT_MEMBER_STATUS_CHANGED",
            Self::MemberNameChanged=>"EXPEDITION_MANAGEMENT_MEMBER_NAME_CHANGED",
            Self::UpdateMainMenu=>"UPDATE_MAIN_MENU_EXPEDITION",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Expeditions {
    /// Actual selected/spawned local persistent identity, never inferred from roster order.
    pub local_character_id: Option<u32>,
    pub local_object_id: Option<u32>,
    /// Native39d44b94: CharacterState faction, then accepted local SC01b changes.
    pub current_faction_id:u32,
    local_actor_faction_id:Option<u32>,
    /// Native manager constructor/reset values are all zero (invalid expedition sentinel).
    pub own_id: u32,
    pub own_role: u8,
    pub loaded_id: u32,
    pub factions: BTreeMap<u32, ExpeditionFaction>,
    pub policies: BTreeMap<(u32, u8), ExpeditionRolePolicy>,
    /// Native cache is keyed by persistent character ID, not by expedition ID.
    pub members: BTreeMap<u32, ExpeditionMember>,
    /// Last received declared total for each list context. Not a completeness assertion.
    pub totals: BTreeMap<u32, u32>,
    /// Last complete receipt, including its raw row order and duplicates. All receipts are
    /// delivered in order through Event::Expedition; the cache does not keep a status history.
    pub last_receipt: Option<ExpeditionEvent>,
}
impl Expeditions {
    /// Native392f8ad0 compares the loaded context with own ID, including equal reset sentinels.
    pub fn info_loaded(&self) -> bool { self.loaded_id == self.own_id }
    pub fn my_policy(&self) -> Option<&ExpeditionRolePolicy> {
        self.policies.get(&(self.own_id, self.own_role))
    }
    pub fn own_policies(&self) -> impl Iterator<Item=&ExpeditionRolePolicy> {
        self.policies.range((self.own_id,0)..=(self.own_id,u8::MAX)).map(|(_,row)|row)
    }
    fn hydrate_character(&mut self, character:&crate::model::Character) {
        // Native392f8080 resets member/policy view but retains global faction metadata.
        let factions=std::mem::take(&mut self.factions);
        *self=Self { factions, local_character_id:(character.id!=0).then_some(character.id),
            own_id:character.expedition_id, current_faction_id:character.faction_id, ..Self::default() };
    }
    fn cache_member(&mut self, member:&ExpeditionMember) {
        let mut row=member.clone();row.name.truncate(128);row.memo.truncate(63);
        self.members.insert(row.character_id,row);
    }
    /// The compatibility reducer discards notices; callers needing UI callbacks should call
    /// apply_with_notices INSTEAD, never fold the same receipt twice.
    pub fn apply(&mut self, event: &Event) -> bool { self.apply_with_notices(event).0 }
    pub fn apply_with_notices(&mut self,event:&Event)->(bool,Vec<ExpeditionNotice>) {
        let mut notices=Vec::new();let changed=self.apply_inner(event,&mut notices);(changed,notices)
    }
    /// Evaluate before folding a Dismissed receipt: native removes metadata before actor cleanup.
    pub fn admitted_dismissal(&self,event:&Event)->Option<u32> {
        let Event::Expedition(ExpeditionEvent::Dismissed{expedition_id,success})=event else{return None};
        (*success!=0 && self.factions.get(expedition_id).is_some_and(|f|f.mother_id!=0)).then_some(*expedition_id)
    }
    fn reset_native_view(&mut self) {
        self.own_id=0;self.own_role=0;self.loaded_id=0;
        self.members.clear();self.policies.clear();self.totals.clear();
    }
    fn apply_membership(&mut self,obj_id:u32,character_id:u32,old_expedition_id:u32,new_expedition_id:u32,notices:&mut Vec<ExpeditionNotice>) {
        let context=if old_expedition_id==0 {new_expedition_id} else {old_expedition_id};
        if context!=0 && self.factions.contains_key(&context) {
            if self.local_object_id==Some(obj_id) && self.local_character_id==Some(character_id) {
                self.reset_native_view();
                if old_expedition_id==0 {self.own_id=new_expedition_id;notices.push(ExpeditionNotice::UpdateMainMenu);}
                else if new_expedition_id==0 {notices.push(ExpeditionNotice::UpdateMainMenu);}
            } else if self.local_character_id!=Some(character_id) && self.own_id!=0
                && old_expedition_id==self.own_id && new_expedition_id==0 {
                // Native392fe4bf: offline targets need no runtime actor to leave the keyed roster.
                self.members.remove(&character_id);notices.push(ExpeditionNotice::Updated);
            }
        }
    }
    /// Native recursive mother lookup, with an app guard against malformed cache cycles.
    pub fn top_level_faction(&self)->Option<u32> {
        let mut id=self.current_faction_id;
        for _ in 0..=self.factions.len() {
            match self.factions.get(&id) {Some(row) if row.mother_id!=0 => id=row.mother_id,_=>return Some(id)}
        }
        None
    }
    fn apply_inner(&mut self, event: &Event, notices:&mut Vec<ExpeditionNotice>) -> bool {
        match event {
            Event::Expedition(receipt) => {
                match receipt {
                    ExpeditionEvent::ShowRenameUi => {}
                    ExpeditionEvent::Renamed{expedition_id,name,by_gm} => {
                        if let Some(row)=self.factions.get_mut(expedition_id) {
                            row.name=name.iter().copied().take(128).collect();
                            if *by_gm==0 {row.allow_change_name=0;}
                        }
                    }
                    ExpeditionEvent::UnitFactionChanged{obj_id,old_faction_id,new_faction_id,..} => {
                        if self.local_object_id==Some(*obj_id) && self.local_actor_faction_id==Some(*old_faction_id) {
                            self.local_actor_faction_id=Some(*new_faction_id);self.current_faction_id=*new_faction_id;
                        }
                    }
                    ExpeditionEvent::Created{faction,owner_obj_id,members} => {
                        // Native398a5280 inserts absent metadata; duplicate creation never replaces it.
                        self.factions.entry(faction.id).or_insert_with(|| {
                            let mut row=faction.clone();row.name.truncate(128);row.owner_name.truncate(128);row
                        });
                        if faction.id!=0 && faction.political_system==0 {
                            if self.local_character_id==Some(faction.owner_id) && faction.mother_id!=0 {
                                notices.push(ExpeditionNotice::UpdateMainMenu);
                            }
                            self.apply_membership(*owner_obj_id,faction.owner_id,0,faction.id,notices);
                            for member in members.iter().take(4) {
                                self.apply_membership(member.obj_id,member.character_id,0,faction.id,notices);
                            }
                        }
                    }
                    ExpeditionEvent::PolicyChanged{policy,success} => {
                        let key=(policy.expedition_id,policy.role);
                        if *success==0 { notices.push(ExpeditionNotice::PolicyChanged); }
                        else if policy.expedition_id==self.own_id && self.policies.contains_key(&key) {
                            let mut row=policy.clone();row.name.truncate(128);self.policies.insert(key,row);
                            notices.push(ExpeditionNotice::PolicyChanged);
                        }
                    }
                    ExpeditionEvent::RoleChanged{character_id,role,..} => {
                        if self.policies.contains_key(&(self.own_id,*role)) {
                            if self.local_character_id==Some(*character_id) {
                                self.own_role=*role;notices.push(ExpeditionNotice::RoleChanged);
                            }
                            // Native operator[] zero-initializes only the ROW; identity is the tree key.
                            self.members.entry(*character_id).or_default().role=*role;
                            notices.push(ExpeditionNotice::Updated);
                        }
                    }
                    ExpeditionEvent::OwnerChanged{old_owner_id,new_owner_id,name} => {
                        if let Some(faction)=self.factions.get_mut(&self.own_id).filter(|f|f.owner_id==*old_owner_id) {
                            faction.owner_id=*new_owner_id;faction.owner_name=name.iter().copied().take(128).collect();
                            if self.policies.contains_key(&(self.own_id,255)) && self.policies.contains_key(&(self.own_id,0)) {
                                if let Some(row)=self.members.get_mut(old_owner_id) {row.role=0;}
                                if let Some(row)=self.members.get_mut(new_owner_id) {row.role=255;}
                                if self.local_character_id==Some(*old_owner_id) {self.own_role=0;notices.push(ExpeditionNotice::RoleChanged);}
                                else if self.local_character_id==Some(*new_owner_id) {self.own_role=255;notices.push(ExpeditionNotice::RoleChanged);}
                                notices.push(ExpeditionNotice::Updated);
                            }
                        }
                    }
                    ExpeditionEvent::Dismissed{expedition_id,success} => {
                        if *success!=0 && self.factions.get(expedition_id).is_some_and(|f|f.mother_id!=0) {
                            self.factions.remove(expedition_id);
                            if self.own_id==*expedition_id {self.reset_native_view();notices.push(ExpeditionNotice::UpdateMainMenu);}
                        }
                    }
                    ExpeditionEvent::MembershipChanged{obj_id,character_id,old_expedition_id,new_expedition_id,..} => {
                        self.apply_membership(*obj_id,*character_id,*old_expedition_id,*new_expedition_id,notices);
                    }
                    ExpeditionEvent::Invitation { .. } => {}
                    ExpeditionEvent::Factions(rows) => {
                        for row in rows.iter().take(20) {
                            let mut row=row.clone();row.name.truncate(128);row.owner_name.truncate(128);
                            self.factions.insert(row.id, row);
                        }
                    }
                    ExpeditionEvent::Policies(rows) => {
                        for row in rows.iter().take(20) {
                            let mut row=row.clone();row.name.truncate(128);
                            self.policies.insert((row.expedition_id, row.role), row);
                        }
                    }
                    ExpeditionEvent::Members { total, expedition_id, members } => {
                        self.totals.insert(*expedition_id, *total);
                        for row in members.iter().take(20) {
                            self.cache_member(row);
                        }
                        // Native392fb160: only this callback reconciles loaded membership.
                        // Policies/status callbacks do not retroactively run this condition.
                        if !self.policies.is_empty() && *total as usize == self.members.len() {
                            if let Some(member)=self.local_character_id.and_then(|id|self.members.get(&id)) {
                                self.own_id=member.expedition_id;
                                self.own_role=member.role;
                                self.loaded_id=*expedition_id;
                            }
                        }
                    }
                    ExpeditionEvent::MemberStatus { member, flag } => {
                        let notice=if !self.members.contains_key(&member.character_id) {ExpeditionNotice::Updated}
                            else if flag&1!=0 {ExpeditionNotice::MemberNameChanged} else {ExpeditionNotice::MemberStatusChanged};
                        self.cache_member(member);notices.push(notice);
                    }
                }
                self.last_receipt = Some(receipt.clone());
                true
            }
            Event::CharacterSelected(state) => { self.hydrate_character(&state.character); true }
            Event::PlayerSpawn(spawn) => {
                // Session emits PlayerSpawn only for the actual selected Character-kind actor.
                let crate::model::UnitKind::Character{char_id}=spawn.unit.kind else {return false};
                if char_id==0 || char_id!=spawn.state.character.id {return false;}
                if self.local_character_id==Some(char_id) {
                    let changed=self.local_object_id!=Some(spawn.unit.obj_id) || self.local_actor_faction_id!=Some(spawn.unit.faction_id);self.local_object_id=Some(spawn.unit.obj_id);self.local_actor_faction_id=Some(spawn.unit.faction_id);return changed;
                }
                self.hydrate_character(&spawn.state.character);self.local_object_id=Some(spawn.unit.obj_id);self.local_actor_faction_id=Some(spawn.unit.faction_id); true
            }
            Event::UnitsRemoved(ids) if self.local_object_id.is_some_and(|id|ids.contains(&id)) => {
                self.local_object_id=None;self.local_actor_faction_id=None;true
            }
            Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = self.local_character_id.is_some() || self.local_object_id.is_some() || self.own_id != 0 || self.own_role != 0 || self.loaded_id != 0
                    || self.current_faction_id!=0 || self.local_actor_faction_id.is_some() || !self.factions.is_empty() || !self.policies.is_empty() || !self.members.is_empty()
                    || !self.totals.is_empty() || self.last_receipt.is_some();
                *self = Self::default();
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;
    #[test]
    fn creation_rename_and_faction_wire_preserve_raw_and_dd04_alignment() {
        let faction=[9,0,0,0,2,101,0,0,0,1,0,255,7,0,0,0,0,0,255,0,1,0,0,0,0,0,0,0,4,5];
        let mut w=Writer::new();w.raw(&faction).bc(77).u8(5);
        for n in 0..5 {w.bc(80+n).u32(7+n).blob(&[255,n as u8]);}
        let created=w.finish();
        let renamed=vec![9,0,0,0,3,0,255,0,65,2];
        let mut w=Writer::new();w.bc(77).blob(&[255,0]).u32(101).u32(102).u8(3);let faction_changed=w.finish();
        for (op,body) in [(0x13,created),(0x11,renamed),(0x1b,faction_changed),(0xf,vec![])] {
            let mut bytes=body.clone();bytes.extend_from_slice(&0x12345678u32.to_le_bytes());
            let mut r=Reader::new(&bytes);let event=ExpeditionEvent::read_lane(op,&mut r).unwrap();
            assert_eq!(r.u32().unwrap(),0x12345678);assert!(r.is_empty());
            if let ExpeditionEvent::Created{members,..}=&event {assert_eq!(members.len(),5);assert_eq!(members[4].name,[255,4]);}
            if let ExpeditionEvent::Renamed{by_gm,name,..}=&event {assert_eq!((*by_gm,name.as_slice()),(2,&[255,0,65][..]));}
            if let ExpeditionEvent::UnitFactionChanged{temporary,..}=&event {assert_eq!(*temporary,3);}
            for n in 0..body.len() {assert!(ExpeditionEvent::read_lane(op,&mut Reader::new(&body[..n])).is_err(),"{op:x}/{n}");}
            let decoded=crate::game::ServerMsg::decode(&frame(op,&body)).unwrap();assert!(matches!(decoded,crate::game::ServerMsg::Expedition(_)));
            use std::io::Write;
            let mut raw=Writer::new();raw.u16(0).u16(op).raw(&body).u16(0).u16(0xf);
            let mut compressor=flate2::write::DeflateEncoder::new(Vec::new(),flate2::Compression::fast());compressor.write_all(&raw.finish()).unwrap();
            let mut batch=Writer::new();batch.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(2).raw(&compressor.finish().unwrap());
            let crate::game::ServerMsg::Batch(messages)=crate::game::ServerMsg::decode(&batch.finish()).unwrap() else{panic!()};
            assert_eq!(messages.len(),2);assert!(matches!(&messages[1],crate::game::ServerMsg::Expedition(ExpeditionEvent::ShowRenameUi)));

        }
        // Empty SC00f must consume no bytes before the following genuine packet.
        let trailing=frame(0xf,&[]);assert!(matches!(crate::game::ServerMsg::decode(&trailing).unwrap(),crate::game::ServerMsg::Expedition(ExpeditionEvent::ShowRenameUi)));
    }
    #[test]
    fn current_faction_uses_selected_identity_then_matching_existing_actor_old_faction() {
        let mut selected=selected(7,0);let Event::CharacterSelected(ref mut state)=selected else{panic!()};state.character.faction_id=101;
        let mut world=crate::state::WorldState::default();world.apply(&selected);
        assert_eq!(world.expeditions.current_faction_id,101);
        let change=|obj,old,new|Event::Expedition(ExpeditionEvent::UnitFactionChanged{obj_id:obj,name:vec![255],old_faction_id:old,new_faction_id:new,temporary:2});
        world.apply(&change(77,101,999));assert_eq!(world.expeditions.current_faction_id,101); // No actor yet.
        let mut own=spawn(7,0);let Event::PlayerSpawn(ref mut own_spawn)=own else{panic!()};own_spawn.unit.faction_id=101;own_spawn.state.character.faction_id=101;
        world.apply(&own);world.units.by_id.insert(77,world.player.spawn.as_ref().unwrap().unit.clone());
        world.apply(&change(88,101,999));world.apply(&change(77,102,999));assert_eq!(world.expeditions.current_faction_id,101);
        world.apply(&change(77,101,102));assert_eq!(world.expeditions.current_faction_id,102);assert_eq!(world.units.by_id[&77].faction_id,102);
        assert_eq!(world.player.spawn.as_ref().unwrap().state.character.faction_id,102);
        world.apply(&change(77,101,999));assert_eq!(world.expeditions.current_faction_id,102);
        let legacy=|old,new|Event::UnitFactionChanged{obj_id:77,name:String::new(),faction_id:old,faction_id2:new,temporary:false};
        assert!(!world.units.apply(&legacy(101,999)));assert_eq!(world.units.by_id[&77].faction_id,102);
        assert!(world.units.apply(&legacy(102,103)));assert_eq!(world.units.by_id[&77].faction_id,103);

        world.apply(&Event::UnitsRemoved(vec![77]));world.apply(&change(77,102,999));assert_eq!(world.expeditions.current_faction_id,102);
        world.apply(&Event::LeftWorld{target:0});assert_eq!(world.expeditions.current_faction_id,0);
    }
    #[test]
    fn top_level_lookup_missing_and_cycle_boundaries_and_rename_existing_only() {
        let mut state=Expeditions::default();state.current_faction_id=9;assert_eq!(state.top_level_faction(),Some(9));
        for (id,mother) in [(9,101),(101,148),(148,0)] {state.factions.insert(id,ExpeditionFaction{id,mother_id:mother,name:vec![65],allow_change_name:1,..Default::default()});}
        assert_eq!(state.top_level_faction(),Some(148));state.factions.get_mut(&148).unwrap().mother_id=9;assert_eq!(state.top_level_faction(),None);
        for (id,gm) in [(999,0),(9,2),(101,0)] {state.apply(&Event::Expedition(ExpeditionEvent::Renamed{expedition_id:id,name:vec![255;140],by_gm:gm}));}
        assert!(!state.factions.contains_key(&999));assert_eq!(state.factions[&9].name.len(),128);assert_eq!(state.factions[&9].allow_change_name,1);assert_eq!(state.factions[&101].allow_change_name,0);
        assert_eq!(state.own_id,0);
    }
    #[test]
    fn creation_admits_owner_then_four_rows_without_overwriting_existing_metadata_or_inventing_membership() {
        let mut state=Expeditions::default();state.apply(&spawn(7,0));
        let faction=ExpeditionFaction{id:9,mother_id:101,owner_id:7,name:vec![255;140],..Default::default()};
        let rows=(0..5).map(|n|ExpeditionCreatedMember{obj_id:80+n,character_id:8+n,name:vec![n as u8]}).collect::<Vec<_>>();
        let event=Event::Expedition(ExpeditionEvent::Created{faction:faction.clone(),owner_obj_id:77,members:rows.clone()});
        let (_,notices)=state.apply_with_notices(&event);assert_eq!(notices,vec![ExpeditionNotice::UpdateMainMenu,ExpeditionNotice::UpdateMainMenu]);assert_eq!(state.own_id,9);assert_eq!(state.factions[&9].name.len(),128);
        let mut duplicate=faction.clone();duplicate.name=vec![66];state.apply(&Event::Expedition(ExpeditionEvent::Created{faction:duplicate,owner_obj_id:0,members:vec![]}));assert_eq!(state.factions[&9].name.len(),128);
        for (index,accepted) in [(3,true),(4,false)] {
            let mut state=Expeditions::default();state.apply(&spawn(7,0));let mut rows=rows.clone();rows[index].obj_id=77;rows[index].character_id=7;
            state.apply(&Event::Expedition(ExpeditionEvent::Created{faction:ExpeditionFaction{owner_id:99,..faction.clone()},owner_obj_id:999,members:rows}));assert_eq!(state.own_id,if accepted{9}else{0});
        }
        let mut world=crate::state::WorldState::default();world.apply(&spawn(7,0));world.units.by_id.insert(77,world.player.spawn.as_ref().unwrap().unit.clone());world.apply(&event);
        assert_eq!(world.player.spawn.as_ref().unwrap().unit.expedition_id,9);assert_eq!(world.units.by_id[&77].expedition_id,9);assert_eq!(world.units.by_id.len(),1);
    }
    #[test]
    fn native_lane_invitation_preserves_bytes_and_never_infers_membership() {
        let bytes=[7,0,0,0,3,0,255,0,65,9,0,0,0,2,0,128,0];
        let expected=ExpeditionEvent::Invitation{inviter_id:7,inviter_name:vec![255,0,65],expedition_id:9,expedition_name:vec![128,0]};
        let mut r=Reader::new(&bytes); assert_eq!(ExpeditionEvent::read_lane(0x1a,&mut r).unwrap(),expected); assert!(r.is_empty());
        for n in 0..bytes.len() {assert!(ExpeditionEvent::read_lane(0x1a,&mut Reader::new(&bytes[..n])).is_err());}
        let mut state=Expeditions::default();state.apply(&Event::Expedition(expected.clone()));
        assert_eq!(state.own_id,0);assert!(state.members.is_empty()&&state.factions.is_empty());assert_eq!(state.last_receipt,Some(expected));
    }
    fn member_bytes() -> Vec<u8> {
        // Independent lane writer field sequence, with binary strings and raw booleans.
        let mut w = Writer::new();
        w.u32(9).u32(7).u8(2).u8(1).i64(-3).blob(&[0xff, 0, b'A']).u8(55).u32(0x1234).u32(99);
        w.u8(1).u8(2).u8(3).u8(4).i64(-100).i64(200).f32(3.5).blob(&[0xfe, 0]).i64(123);
        w.finish()
    }
    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(0xdd).u8(crate::game::LEVEL_GAME).u16(0).u16(opcode).raw(body);
        w.finish()
    }
    #[test]
    fn exact_member_wire_raw_bytes_and_every_truncation() {
        let bytes = member_bytes();
        let mut r = Reader::new(&bytes);
        let m = ExpeditionMember::read_lane(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!((m.expedition_id, m.character_id, m.in_party, m.online), (9, 7, 2, 1));
        assert_eq!(m.name, [0xff, 0, b'A']);
        assert_eq!(m.memo, [0xfe, 0]);
        assert_eq!((m.x, m.y, m.z), (-100, 200, 3.5));
        assert_eq!(m.last_world_leave_time, -3);
        assert_eq!(m.transfer_requested_time, 123);
        for n in 0..bytes.len() {
            assert!(ExpeditionMember::read_lane(&mut Reader::new(&bytes[..n])).is_err(), "{n}");
        }
    }
    #[test]
    fn framing_counts_markers_and_next_packet_alignment() {
        let mut w = Writer::new();
        w.u32(21).u8(1).u32(9).raw(&member_bytes()).u32(0x12345678);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert!(matches!(ExpeditionEvent::read_lane(0x16,&mut r).unwrap(),ExpeditionEvent::Members{total:21,expedition_id:9,members} if members.len()==1));
        assert_eq!(r.u32().unwrap(), 0x12345678);
        let mut malformed = bytes[..bytes.len() - 4].to_vec();
        malformed[4] = 2;
        assert!(ExpeditionEvent::read_lane(0x16, &mut Reader::new(&malformed)).is_err());
        for op in [8, 11] {
            assert!(matches!(
                crate::game::ServerMsg::decode(&frame(op, &[0])).unwrap(),
                crate::game::ServerMsg::Expedition(_)
            ));
        }
        assert!(ExpeditionEvent::read_lane(0x17, &mut Reader::new(&member_bytes())).is_err());
    }
    #[test]
    fn policy_and_faction_goldens_keep_noncanonical_boolean_bytes() {
        let policy = [9, 0, 0, 0, 4, 1, 0, 0xff, 0, 1, 2, 3, 4, 5, 6, 7, 255];
        let mut r = Reader::new(&policy);
        let p = ExpeditionRolePolicy::read_lane(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(p.name, [255]);
        assert_eq!(p.permissions, [0, 1, 2, 3, 4, 5, 6, 7, 255]);
        let faction = [9, 0, 0, 0, 2, 101, 0, 0, 0, 1, 0, 0xff, 7, 0, 0, 0, 0, 0, 255, 3, 1, 0, 0, 0, 0, 0, 0, 0, 4, 5];
        let mut r = Reader::new(&faction);
        let f = ExpeditionFaction::read_lane(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(
            (
                f.id,
                f.aggro_link,
                f.mother_id,
                f.unit_owner_type,
                f.political_system,
                f.created,
                f.diplomacy_target,
                f.allow_change_name
            ),
            (9, 2, 101, -1, 3, 1, 4, 5)
        );
        for n in 0..policy.len() {
            assert!(ExpeditionRolePolicy::read_lane(&mut Reader::new(&policy[..n])).is_err());
        }
        for n in 0..faction.len() {
            assert!(ExpeditionFaction::read_lane(&mut Reader::new(&faction[..n])).is_err());
        }
    }
    #[test]
    fn native_keyed_upsert_keeps_receipts_chunks_and_empty_markers() {
        let mut s = Expeditions::default();
        let mut m = ExpeditionMember::read_lane(&mut Reader::new(&member_bytes())).unwrap();
        let first = m.clone();
        m.name = b"last".to_vec();
        let event = Event::Expedition(ExpeditionEvent::Members {
            total: 21,
            expedition_id: 9,
            members: vec![first, m.clone()],
        });
        s.apply(&event);
        s.apply(&event);
        assert_eq!(s.members.len(), 1);
        assert_eq!(s.members[&7], m);
        let Event::Expedition(receipt) = &event else { panic!() };
        assert_eq!(s.last_receipt.as_ref(), Some(receipt));
        s.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![ExpeditionRolePolicy {
            expedition_id: 9,
            role: 4,
            ..Default::default()
        }])));
        s.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![])));
        assert_eq!(s.policies.len(), 1);
        assert!(matches!(&s.last_receipt, Some(ExpeditionEvent::Policies(rows)) if rows.is_empty()));
        m.online = 0;
        m.expedition_id = 10;
        s.apply(&Event::Expedition(ExpeditionEvent::MemberStatus { member: m.clone(), flag: 255 }));
        assert_eq!(s.members[&7], m);
        assert!(matches!(&s.last_receipt, Some(ExpeditionEvent::MemberStatus{member,flag:255}) if member==&m));
        assert_eq!(s.totals[&9], 21);
        let mut world = crate::state::WorldState::default();
        world.apply(&event);
        assert_eq!(world.expeditions.members.len(), 1);
        world.apply(&Event::Disconnected { message: String::new() });
        assert_eq!(world.expeditions, Expeditions::default());
        for e in [
            Event::LeftWorld { target: 0 },
            Event::Kicked {
                reason: 0,
                message: String::new(),
            },
        ] {
            s.apply(&event);
            s.apply(&e);
            assert_eq!(s, Expeditions::default());
        }
    }
    #[test]
    fn lane_over_inline_native_cap_keeps_all_rows_and_dd04_tail() {
        use std::io::Write;
        let mut raw = Writer::new();
        raw.u16(0).u16(0x16).u32(21).u8(21).u32(9);
        for _ in 0..21 {
            raw.raw(&member_bytes());
        }
        raw.u16(0).u16(0x0b).u8(0);
        let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&raw.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(4).u16(2).raw(&compressor.finish().unwrap());
        let crate::game::ServerMsg::Batch(messages) = crate::game::ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(messages.len(), 2);
        assert!(matches!(&messages[0],crate::game::ServerMsg::Expedition(ExpeditionEvent::Members{members,..}) if members.len()==21));
        assert!(matches!(&messages[1],crate::game::ServerMsg::Expedition(ExpeditionEvent::Policies(rows)) if rows.is_empty()));
    }
    fn selected(id:u32,expedition_id:u32)->Event {
        let mut state=crate::model::CharacterState::default();state.character.id=id;state.character.expedition_id=expedition_id;
        Event::CharacterSelected(Box::new(state))
    }
    fn spawn(id:u32,expedition_id:u32)->Event {
        let Event::CharacterSelected(state)=selected(id,expedition_id) else{panic!()};
        let mut unit=crate::state::tests::unit(77);unit.kind=crate::model::UnitKind::Character{char_id:id};
        Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn{obj_id:77,state:*state,unit}))
    }
    #[test]
    fn native_identity_hydration_and_members_only_reconciliation_preserve_callback_order() {
        let mut state=Expeditions::default();assert!(state.info_loaded());
        state.apply(&Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction{id:9,..Default::default()}])));
        state.apply(&selected(7,9));assert_eq!((state.local_character_id,state.own_id,state.own_role,state.loaded_id),(Some(7),9,0,0));
        assert!(!state.info_loaded());assert!(state.factions.contains_key(&9));
        let mut own=ExpeditionMember{character_id:7,expedition_id:9,role:255,..Default::default()};
        let members=|total,rows|Event::Expedition(ExpeditionEvent::Members{total,expedition_id:9,members:rows});
        state.apply(&members(1,vec![own.clone()]));assert!(!state.info_loaded(),"no policy cache yet");
        state.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![ExpeditionRolePolicy{expedition_id:9,role:255,..Default::default()}])));
        assert!(!state.info_loaded(),"policy receipt never retroactively reconciles");
        state.apply(&members(2,vec![own.clone()]));assert!(!state.info_loaded(),"declared count does not match keyed cache");
        state.apply(&members(1,vec![own.clone()]));assert!(state.info_loaded());assert_eq!(state.own_role,255);
        assert!(state.my_policy().is_some());assert_eq!(state.own_policies().count(),1);
        state.apply(&spawn(7,0));assert_eq!((state.own_id,state.loaded_id,state.own_role),(9,9,255),"matching spawn must not overwrite already received view with older selection fields");
        own.expedition_id=10;own.role=2;
        state.apply(&Event::Expedition(ExpeditionEvent::MemberStatus{member:own.clone(),flag:0}));
        assert_eq!((state.own_id,state.own_role),(9,255),"status callback does not reconcile own manager");
        state.apply(&members(1,vec![own]));assert_eq!((state.own_id,state.loaded_id,state.own_role),(10,9,2));
        assert!(!state.info_loaded(),"packet context need not equal own row expedition; native preserves both");
    }
    #[test]
    fn native_view_uses_actual_local_row_and_resets_on_real_identity_change() {
        let mut state=Expeditions::default();state.apply(&selected(7,9));
        state.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![ExpeditionRolePolicy{expedition_id:8,role:0,..Default::default()}])));
        let list=Event::Expedition(ExpeditionEvent::Members{total:1,expedition_id:8,members:vec![ExpeditionMember{character_id:99,expedition_id:8,role:255,..Default::default()}]});
        state.apply(&list);assert_eq!((state.own_id,state.loaded_id),(9,0),"first roster row is not local identity");
        state.apply(&Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction{id:8,..Default::default()}])));
        state.apply(&spawn(10,12));assert_eq!((state.local_character_id,state.own_id,state.loaded_id,state.own_role),(Some(10),12,0,0));
        assert!(state.members.is_empty()&&state.policies.is_empty()&&state.last_receipt.is_none());assert!(state.factions.contains_key(&8));
        state.apply(&Event::Disconnected{message:String::new()});assert_eq!(state,Expeditions::default());
    }

    #[test]
    fn safe_inline_cache_limits_never_truncate_complete_received_events() {
        let mut state=Expeditions::default();state.apply(&selected(21,9));
        let factions=(1..=21).map(|id|ExpeditionFaction{id,name:vec![b'A';129],owner_name:vec![b'B';130],..Default::default()}).collect::<Vec<_>>();
        state.apply(&Event::Expedition(ExpeditionEvent::Factions(factions.clone())));
        assert_eq!(state.factions.len(),20);assert_eq!(state.factions[&1].name.len(),128);assert_eq!(state.factions[&1].owner_name.len(),128);
        assert!(matches!(&state.last_receipt,Some(ExpeditionEvent::Factions(rows)) if rows==&factions));
        let policies=(0..21).map(|role|ExpeditionRolePolicy{expedition_id:9,role,name:vec![b'C';129],..Default::default()}).collect::<Vec<_>>();
        state.apply(&Event::Expedition(ExpeditionEvent::Policies(policies.clone())));
        assert_eq!(state.policies.len(),20);assert_eq!(state.policies[&(9,0)].name.len(),128);
        assert!(matches!(&state.last_receipt,Some(ExpeditionEvent::Policies(rows)) if rows==&policies));
        let members=(1..=21).map(|character_id|ExpeditionMember{character_id,expedition_id:9,name:vec![b'D';129],memo:vec![b'E';64],..Default::default()}).collect::<Vec<_>>();
        let receipt=ExpeditionEvent::Members{total:21,expedition_id:9,members:members.clone()};
        state.apply(&Event::Expedition(receipt.clone()));assert_eq!(state.members.len(),20);assert!(!state.info_loaded());
        assert_eq!(state.last_receipt,Some(receipt));assert_eq!(state.members[&1].name.len(),128);assert_eq!(state.members[&1].memo.len(),63);
        state.apply(&Event::Expedition(ExpeditionEvent::MemberStatus{member:members[20].clone(),flag:0}));
        assert_eq!(state.members[&21].memo.len(),63);assert!(!state.info_loaded(),"status does not reconcile");
        state.apply(&Event::Expedition(ExpeditionEvent::Members{total:21,expedition_id:9,members:vec![]}));
        assert!(state.info_loaded(),"native callback count/cache conditions now hold with actual local row");
    }

    #[test]
    fn five_mutation_layouts_preserve_raw_bytes_truncations_and_dd04_tail() {
        let cases=vec![
            (0x0c,vec![9,0,0,0,2,1,0,255,1,0,1,0,1,0,1,0,1,2]),
            (0x0d,vec![7,0,0,0,2,2,0,255,0]),
            (0x0e,vec![7,0,0,0,8,0,0,0,2,0,255,0]),
            (0x15,vec![9,0,0,0,255]),
            (0x1c,vec![77,0,0,7,0,0,0,1,0,255,2,0,128,0,9,0,0,0,0,0,0,0,2]),
        ];
        for (opcode,bytes) in cases {
            let mut r=Reader::new(&bytes);let receipt=ExpeditionEvent::read_lane(opcode,&mut r).unwrap();assert!(r.is_empty());
            for n in 0..bytes.len(){assert!(ExpeditionEvent::read_lane(opcode,&mut Reader::new(&bytes[..n])).is_err(),"opcode{opcode:x} at{n}");}
            use std::io::Write;
            let mut raw=Writer::new();raw.u16(0).u16(opcode).raw(&bytes).u16(0).u16(0x0b).u8(0);
            let mut compressor=flate2::write::DeflateEncoder::new(Vec::new(),flate2::Compression::fast());compressor.write_all(&raw.finish()).unwrap();
            let mut batch=Writer::new();batch.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(2).raw(&compressor.finish().unwrap());
            let crate::game::ServerMsg::Batch(messages)=crate::game::ServerMsg::decode(&batch.finish()).unwrap() else {panic!()};
            assert!(matches!(&messages[0],crate::game::ServerMsg::Expedition(value) if value==&receipt));
            assert!(matches!(&messages[1],crate::game::ServerMsg::Expedition(ExpeditionEvent::Policies(rows)) if rows.is_empty()));
        }
    }
    #[test]
    fn mutation_guards_and_notices_follow_native_cache_order_without_optimistic_success() {
        use ExpeditionNotice::*;
        let mut state=Expeditions::default();state.apply(&selected(7,9));
        let mut policy=ExpeditionRolePolicy{expedition_id:9,role:2,name:vec![65],..Default::default()};
        let notice=|s:&mut Expeditions,event|s.apply_with_notices(&Event::Expedition(event)).1;
        assert!(notice(&mut state,ExpeditionEvent::PolicyChanged{policy:policy.clone(),success:1}).is_empty());
        assert_eq!(notice(&mut state,ExpeditionEvent::PolicyChanged{policy:policy.clone(),success:0}),vec![PolicyChanged]);
        state.policies.insert((9,2),policy.clone());policy.name=vec![255;129];
        assert_eq!(notice(&mut state,ExpeditionEvent::PolicyChanged{policy:policy.clone(),success:2}),vec![PolicyChanged]);
        assert_eq!(state.policies[&(9,2)].name.len(),128);
        assert!(notice(&mut state,ExpeditionEvent::RoleChanged{character_id:7,role:3,name:vec![]}).is_empty());
        assert_eq!(notice(&mut state,ExpeditionEvent::RoleChanged{character_id:7,role:2,name:vec![65]}),vec![RoleChanged,Updated]);
        assert_eq!(state.own_role,2);assert_eq!(state.members[&7].character_id,0);assert!(state.members[&7].name.is_empty());
        state.factions.insert(9,ExpeditionFaction{id:9,mother_id:1,owner_id:7,..Default::default()});
        let owner=ExpeditionEvent::OwnerChanged{old_owner_id:7,new_owner_id:8,name:vec![255;129]};
        assert!(notice(&mut state,owner.clone()).is_empty());assert_eq!(state.factions[&9].owner_id,8,"faction updates even when role policies are missing");
        state.policies.insert((9,0),ExpeditionRolePolicy::default());state.policies.insert((9,255),ExpeditionRolePolicy::default());
        assert!(notice(&mut state,owner).is_empty(),"stale oldowner cannot rewrite current owner");
        assert_eq!(notice(&mut state,ExpeditionEvent::OwnerChanged{old_owner_id:8,new_owner_id:7,name:vec![66]}),vec![RoleChanged,Updated]);
        assert_eq!(state.own_role,255);assert_eq!(state.members[&7].role,255);assert!(!state.members.contains_key(&8));
        assert!(notice(&mut state,ExpeditionEvent::Dismissed{expedition_id:9,success:0}).is_empty());
        assert_eq!(notice(&mut state,ExpeditionEvent::Dismissed{expedition_id:9,success:2}),vec![UpdateMainMenu]);
        assert_eq!((state.own_id,state.loaded_id,state.own_role),(0,0,0));assert!(state.members.is_empty()&&state.policies.is_empty());assert!(!state.factions.contains_key(&9));
    }
    #[test]
    fn actual_membership_identity_and_existing_unit_guards_preserve_unknown_receipts() {
        use ExpeditionNotice::*;
        let mut world=crate::state::WorldState::default();world.apply(&spawn(7,0));
        world.apply(&Event::UnitSpawned(Box::new(world.player.spawn.as_ref().unwrap().unit.clone())));
        assert_eq!(world.expeditions.local_object_id,Some(77));
        world.expeditions.factions.insert(9,ExpeditionFaction{id:9,mother_id:1,..Default::default()});
        let receipt=|obj_id,character_id,old_expedition_id,new_expedition_id|ExpeditionEvent::MembershipChanged{obj_id,character_id,kicker:vec![255],name:vec![0],old_expedition_id,new_expedition_id,expel:2};
        assert!(world.expeditions.apply_with_notices(&Event::Expedition(receipt(77,8,0,9))).1.is_empty());
        assert!(world.expeditions.apply_with_notices(&Event::Expedition(receipt(88,7,0,9))).1.is_empty());
        let accepted=Event::Expedition(receipt(77,7,0,9));
        assert_eq!(world.expeditions.apply_with_notices(&accepted).1,vec![UpdateMainMenu]);
        assert_eq!(world.expeditions.own_id,9);assert!(!world.expeditions.info_loaded());
        world.player.apply(&accepted);world.units.apply(&accepted);
        assert_eq!(world.player.spawn.as_ref().unwrap().unit.expedition_id,9);
        assert_eq!(world.player.spawn.as_ref().unwrap().state.character.expedition_id,9);
        assert_eq!(world.units.by_id[&77].expedition_id,9);
        world.units.apply(&Event::Expedition(receipt(77,8,9,0)));assert_eq!(world.units.by_id[&77].expedition_id,9);
        world.units.apply(&Event::Expedition(receipt(999,7,9,0)));assert!(!world.units.by_id.contains_key(&999));
        assert_eq!(world.expeditions.apply_with_notices(&Event::Expedition(receipt(77,7,9,0))).1,vec![UpdateMainMenu]);
        assert_eq!(world.expeditions.own_id,0);assert!(world.expeditions.factions.contains_key(&9));
        let status=ExpeditionEvent::MemberStatus{member:ExpeditionMember{character_id:8,..Default::default()},flag:1};
        assert_eq!(world.expeditions.apply_with_notices(&Event::Expedition(status.clone())).1,vec![Updated]);
        assert_eq!(world.expeditions.apply_with_notices(&Event::Expedition(status)).1,vec![MemberNameChanged]);
        world.apply(&Event::LeftWorld{target:0});assert!(world.expeditions.local_object_id.is_none());
    }

    #[test]
    fn admitted_dismissal_clears_only_existing_matching_actors_before_metadata_is_removed() {
        for (success,mother,present,admitted) in [(0,1,true,false),(1,0,true,false),(1,1,false,false),(2,1,true,true)] {
            let mut world=crate::state::WorldState::default();let mut own=spawn(7,9);
            if let Event::PlayerSpawn(value)=&mut own {value.unit.expedition_id=9;}
            world.apply(&own);
            let mut unit=crate::state::tests::unit(88);unit.expedition_id=9;world.units.by_id.insert(88,unit);
            let mut other=crate::state::tests::unit(89);other.expedition_id=10;world.units.by_id.insert(89,other);
            if present {world.expeditions.factions.insert(9,ExpeditionFaction{id:9,mother_id:mother,..Default::default()});}
            let event=Event::Expedition(ExpeditionEvent::Dismissed{expedition_id:9,success});
            assert_eq!(world.expeditions.admitted_dismissal(&event),admitted.then_some(9));world.apply(&event);
            assert_eq!(world.units.by_id[&88].expedition_id,if admitted{0}else{9});
            assert_eq!(world.player.spawn.as_ref().unwrap().unit.expedition_id,if admitted{0}else{9});
            assert_eq!(world.units.by_id[&89].expedition_id,10);assert_eq!(world.units.by_id.len(),2);
            assert_eq!(world.expeditions.factions.contains_key(&9),present&&!admitted);
        }
    }

    #[test]
    fn nonlocal_leave_and_offline_kick_remove_persistent_row_and_emit_even_if_missing() {
        let mut state=Expeditions::default();state.apply(&spawn(7,9));
        state.factions.insert(9,ExpeditionFaction{id:9,mother_id:1,..Default::default()});
        state.members.insert(8,ExpeditionMember{character_id:8,expedition_id:9,..Default::default()});
        let kick=Event::Expedition(ExpeditionEvent::MembershipChanged{obj_id:0,character_id:8,kicker:vec![255],name:vec![0],old_expedition_id:9,new_expedition_id:0,expel:2});
        for _ in 0..2 {
            assert_eq!(state.apply_with_notices(&kick).1,vec![ExpeditionNotice::Updated]);
            assert!(!state.members.contains_key(&8));assert_eq!(state.own_id,9);
        }
        let mut unknown=state.clone();unknown.factions.clear();unknown.members.insert(8,ExpeditionMember::default());
        assert!(unknown.apply_with_notices(&kick).1.is_empty());assert!(unknown.members.contains_key(&8));
    }

}
