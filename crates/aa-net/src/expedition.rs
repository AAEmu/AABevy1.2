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
#[derive(Clone, Debug, PartialEq)]
pub enum ExpeditionEvent {
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
        matches!(opcode, 0x008 | 0x00b | 0x016 | 0x017)
    }
    pub fn read_lane(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
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
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Expeditions {
    pub factions: BTreeMap<u32, ExpeditionFaction>,
    pub policies: BTreeMap<(u32, u8), ExpeditionRolePolicy>,
    /// Native cache is keyed by persistent character ID, not by expedition ID.
    pub members: BTreeMap<u32, ExpeditionMember>,
    /// Last received declared total for each list context. Not a completeness assertion.
    pub totals: BTreeMap<u32, u32>,
    /// Exact ordered receipts, including empty markers and duplicate rows. Cleared with session.
    pub receipts: Vec<ExpeditionEvent>,
}
impl Expeditions {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Expedition(receipt) => {
                match receipt {
                    ExpeditionEvent::Factions(rows) => {
                        for row in rows {
                            self.factions.insert(row.id, row.clone());
                        }
                    }
                    ExpeditionEvent::Policies(rows) => {
                        for row in rows {
                            self.policies.insert((row.expedition_id, row.role), row.clone());
                        }
                    }
                    ExpeditionEvent::Members { total, expedition_id, members } => {
                        self.totals.insert(*expedition_id, *total);
                        for row in members {
                            self.members.insert(row.character_id, row.clone());
                        }
                    }
                    ExpeditionEvent::MemberStatus { member, .. } => {
                        self.members.insert(member.character_id, member.clone());
                    }
                }
                self.receipts.push(receipt.clone());
                true
            }
            Event::CharacterSelected(_) | Event::Kicked { .. } | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.receipts.is_empty();
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
        assert_eq!(s.receipts.len(), 2);
        s.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![ExpeditionRolePolicy {
            expedition_id: 9,
            role: 4,
            ..Default::default()
        }])));
        s.apply(&Event::Expedition(ExpeditionEvent::Policies(vec![])));
        assert_eq!(s.policies.len(), 1);
        m.online = 0;
        m.expedition_id = 10;
        s.apply(&Event::Expedition(ExpeditionEvent::MemberStatus { member: m.clone(), flag: 255 }));
        assert_eq!(s.members[&7], m);
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
}
