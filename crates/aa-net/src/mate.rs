//! Owned mate spawn metadata and the mate/slave dynamic state packet boundaries.
//! Summoning commands, equipment and slave ownership are separate milestones.

use std::collections::HashMap;

use crate::model::UnitKind;
use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Reader, Result, WireError};

/// Native SCMateSpawned has exactly ten slots, including zero/empty slots.
pub const MATE_SKILL_SLOTS: usize = 10;
/// Native 397b2350 has an inline array of 150 triples for each list.
pub const MAX_DYNAMIC_ENTRIES: usize = 150;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MateSpawn {
    pub tl_id: u16,
    /// Stored mate ID, not its runtime object ID or the owner's character ID.
    pub id: u32,
    pub item_id: u64,
    pub user_state: u8,
    pub experience: i32,
    pub mileage: i32,
    pub spawn_delay_time: u32,
    /// Raw mount_skills IDs from npc_mount_skills, not combat skill IDs; zero slots stay sparse.
    pub skills: [u32; MATE_SKILL_SLOTS],
}

/// Exact native list entries. Triple field meanings are deliberately not inferred.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicUnitState {
    pub skills: Vec<[u32; 3]>,
    pub tags: Vec<[u32; 3]>,
}

impl DynamicUnitState {
    fn read_list(r: &mut Reader) -> Result<Vec<[u32; 3]>> {
        let count = r.i32()?;
        if count < 0 || count as usize > MAX_DYNAMIC_ENTRIES || count as usize > r.remaining() / 12
        {
            return Err(WireError::Invalid(
                "invalid mate/slave dynamic entry count".into(),
            ));
        }
        (0..count)
            .map(|_| Ok([r.u32()?, r.u32()?, r.u32()?]))
            .collect()
    }

    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            skills: Self::read_list(r)?,
            tags: Self::read_list(r)?,
        })
    }
}

/// Visibility metadata for any slave; receipt does not imply local ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlaveState {
    pub obj_id: u32,
    pub tl_id: u16,
    /// Lane writes zero. Preserve the native u64 type field without interpreting it.
    pub raw_type: u64,
    pub dynamic: DynamicUnitState,
    pub creator_name: String,
    /// Full u32 on this packet, despite the object-ID name.
    pub owner_obj_id: u32,
    pub db_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MateEvent {
    /// Sent only to the summoning owner, before the normal visible unit spawn.
    Spawned(MateSpawn),
    State {
        obj_id: u32,
        dynamic: DynamicUnitState,
    },
    SlaveState(SlaveState),
}

impl MateEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCMateSpawnedPacket | sc::SCMateStatePacket | sc::SCSlaveStatePacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Spawned(_) => "SCMateSpawned",
            Self::State { .. } => "SCMateState",
            Self::SlaveState(_) => "SCSlaveState",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCMateSpawnedPacket => {
                let mut spawn = MateSpawn {
                    tl_id: r.u16()?,
                    id: r.u32()?,
                    item_id: r.u64()?,
                    user_state: r.u8()?,
                    experience: r.i32()?,
                    mileage: r.i32()?,
                    spawn_delay_time: r.u32()?,
                    skills: [0; MATE_SKILL_SLOTS],
                };
                for skill in &mut spawn.skills {
                    *skill = r.u32()?;
                }
                Self::Spawned(spawn)
            }
            sc::SCMateStatePacket => Self::State {
                obj_id: r.bc()?,
                dynamic: DynamicUnitState::read(r)?,
            },
            sc::SCSlaveStatePacket => Self::SlaveState(SlaveState {
                obj_id: r.bc()?,
                tl_id: r.u16()?,
                raw_type: r.u64()?,
                dynamic: DynamicUnitState::read(r)?,
                creator_name: r.string()?,
                owner_obj_id: r.u32()?,
                db_id: r.u32()?,
            }),
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown mate opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedMate {
    pub spawn: MateSpawn,
    /// None until an actual UnitSpawned matches the timeline AND local stored owner ID.
    pub obj_id: Option<u32>,
    pub dynamic: Option<DynamicUnitState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VisibleMate {
    tl_id: u16,
    owner_id: u32,
    dynamic: Option<DynamicUnitState>,
}

/// Owner-only metadata correlates with existing units; this resource never creates a Unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Mates {
    pub own: Option<OwnedMate>,
    character_id: u32,
    visible: HashMap<u32, VisibleMate>,
}

impl Mates {
    fn correlate(&mut self) {
        let Some(own) = &mut self.own else { return };
        let mut matches = self.visible.iter().filter(|(_, mate)| {
            self.character_id != 0
                && own.spawn.tl_id != 0
                && mate.tl_id == own.spawn.tl_id
                && mate.owner_id == self.character_id
        });
        let first = matches.next();
        let unique = matches.next().is_none();
        (own.obj_id, own.dynamic) = match (first, unique) {
            (Some((obj_id, mate)), true) => (Some(*obj_id), mate.dynamic.clone()),
            _ => (None, None),
        };
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::CharacterSelected(state) => {
                *self = Self {
                    character_id: state.character.id,
                    ..Self::default()
                };
            }
            Event::PlayerSpawn(player) => {
                let id = player.state.character.id;
                if id != self.character_id {
                    *self = Self {
                        character_id: id,
                        ..Self::default()
                    };
                } else {
                    return false;
                }
            }
            Event::Mate(MateEvent::Spawned(spawn)) => {
                self.own = Some(OwnedMate {
                    spawn: spawn.clone(),
                    obj_id: None,
                    dynamic: None,
                });
            }
            Event::UnitSpawned(unit) => {
                let next = match unit.kind {
                    UnitKind::Mate {
                        tl_id, owner_id, ..
                    } => {
                        let dynamic = self
                            .visible
                            .get(&unit.obj_id)
                            .filter(|old| old.tl_id == tl_id && old.owner_id == owner_id)
                            .and_then(|old| old.dynamic.clone());
                        Some(VisibleMate {
                            tl_id,
                            owner_id,
                            dynamic,
                        })
                    }
                    _ => None,
                };
                if next == self.visible.get(&unit.obj_id).cloned() {
                    return false;
                }
                let reused_owned_id = self.own.as_ref().is_some_and(|own| {
                    own.obj_id == Some(unit.obj_id)
                        && !next.as_ref().is_some_and(|mate| {
                            mate.tl_id == own.spawn.tl_id && mate.owner_id == self.character_id
                        })
                });
                if reused_owned_id {
                    self.own = None;
                }
                if let Some(next) = next {
                    self.visible.insert(unit.obj_id, next);
                } else {
                    self.visible.remove(&unit.obj_id);
                }
            }
            Event::Mate(MateEvent::State { obj_id, dynamic }) => {
                let Some(mate) = self.visible.get_mut(obj_id) else {
                    return false;
                };
                if mate.dynamic.as_ref() == Some(dynamic) {
                    return false;
                }
                mate.dynamic = Some(dynamic.clone());
            }
            Event::UnitsRemoved(ids) => {
                let mut changed = false;
                if self
                    .own
                    .as_ref()
                    .is_some_and(|own| own.obj_id.is_some_and(|id| ids.contains(&id)))
                {
                    self.own = None;
                    changed = true;
                }
                for id in ids {
                    changed |= self.visible.remove(id).is_some();
                }
                if !changed {
                    return false;
                }
            }
            Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                let changed = *self != Self::default();
                *self = Self::default();
                return changed;
            }
            _ => return false,
        }
        self.correlate();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::model::{Character, CharacterState};
    use crate::wire::Writer;

    pub(crate) fn spawn_fixture() -> MateSpawn {
        MateSpawn {
            tl_id: 0x0102,
            id: 7,
            item_id: 0x1122334455667788,
            user_state: 1,
            experience: 300,
            mileage: 5,
            spawn_delay_time: 0,
            skills: [11, 12, 0, 0, 0, 0, 0, 0, 0, 0],
        }
    }

    fn spawn_bytes() -> Vec<u8> {
        // Exact vehicles/packets.rs::mate_spawned_pads_skills_to_ten golden.
        let mut body = vec![
            2, 1, 7, 0, 0, 0, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 1, 0x2c, 1, 0, 0, 5,
            0, 0, 0, 0, 0, 0, 0, 11, 0, 0, 0, 12, 0, 0, 0,
        ];
        body.extend([0; 32]);
        assert_eq!(body.len(), 67);
        body
    }

    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xdd, 1, 0, 0];
        out.extend(opcode.to_le_bytes());
        out.extend(body);
        out
    }

    fn batch(packets: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut raw = Writer::new();
        for (opcode, body) in packets {
            raw.u16(0).u16(*opcode).raw(body);
        }
        let raw = raw.finish();
        let size = u16::try_from(raw.len()).unwrap();
        let mut out = vec![0xdd, 4];
        out.extend((packets.len() as u16).to_le_bytes());
        out.push(1); // One final raw-DEFLATE stored block.
        out.extend(size.to_le_bytes());
        out.extend((!size).to_le_bytes());
        out.extend(raw);
        out
    }

    fn dynamic_bytes(dynamic: &DynamicUnitState, w: &mut Writer) {
        for entries in [&dynamic.skills, &dynamic.tags] {
            w.i32(entries.len() as i32);
            for row in entries {
                for value in row {
                    w.u32(*value);
                }
            }
        }
    }

    fn fixtures() -> Vec<(u16, Vec<u8>, MateEvent)> {
        let mut packets = vec![
            (
                sc::SCMateSpawnedPacket,
                spawn_bytes(),
                MateEvent::Spawned(spawn_fixture()),
            ),
            (
                sc::SCMateStatePacket,
                vec![0x33, 0x22, 0x11, 0, 0, 0, 0, 0, 0, 0, 0],
                MateEvent::State {
                    obj_id: 0x112233,
                    dynamic: DynamicUnitState::default(),
                },
            ),
        ];
        for dynamic in [
            DynamicUnitState::default(),
            DynamicUnitState {
                skills: vec![[u32::MAX, 0x12345678, 0x80000000]],
                tags: vec![[0, 0, 0], [12, 34, 56]],
            },
        ] {
            let mut body = Writer::new();
            body.bc(0x010203).u16(9).u64(0xfedcba9876543210);
            dynamic_bytes(&dynamic, &mut body);
            body.string("Ab").u32(0xf1234567).u32(0x89abcdef);
            packets.push((
                sc::SCSlaveStatePacket,
                body.finish(),
                MateEvent::SlaveState(SlaveState {
                    obj_id: 0x010203,
                    tl_id: 9,
                    raw_type: 0xfedcba9876543210,
                    dynamic: dynamic.clone(),
                    creator_name: "Ab".into(),
                    owner_obj_id: 0xf1234567,
                    db_id: 0x89abcdef,
                }),
            ));
            if !dynamic.skills.is_empty() {
                let mut body = Writer::new();
                body.bc(0x112233);
                dynamic_bytes(&dynamic, &mut body);
                packets.push((
                    sc::SCMateStatePacket,
                    body.finish(),
                    MateEvent::State {
                        obj_id: 0x112233,
                        dynamic,
                    },
                ));
            }
        }
        packets
    }

    #[test]
    fn mate_spawn_and_shared_native_dynamic_lists_preserve_every_dd04_boundary() {
        let mut packets = Vec::new();
        let mut expected = Vec::new();
        for (opcode, body, event) in fixtures() {
            assert_eq!(
                ServerMsg::decode(&frame(opcode, &body)).unwrap(),
                ServerMsg::Mate(event.clone())
            );
            let mut reader = Reader::new(&body);
            assert_eq!(MateEvent::read(opcode, &mut reader).unwrap(), event);
            assert!(reader.is_empty());
            for cut in 0..body.len() {
                assert!(
                    ServerMsg::decode(&frame(opcode, &body[..cut])).is_err(),
                    "opcode {opcode:03x}, cut {cut}"
                );
            }
            packets.push((opcode, body));
            expected.push(ServerMsg::Mate(event));
        }
        let mut points = Writer::new();
        points.bc(0x112233).i32(12300).i32(45600);
        packets.push((sc::SCUnitPointsPacket, points.finish()));
        expected.push(ServerMsg::UnitPoints {
            obj_id: 0x112233,
            hp: 123,
            mp: 456,
        });
        assert_eq!(
            ServerMsg::decode(&batch(&packets)).unwrap(),
            ServerMsg::Batch(expected)
        );
    }

    #[test]
    fn dynamic_counts_have_native_150_entry_bounds_for_both_lists() {
        for skills in [true, false] {
            for count in [-1, 151, i32::MAX] {
                let mut body = Writer::new();
                body.bc(1);
                if !skills {
                    body.i32(0);
                }
                body.i32(count);
                assert!(ServerMsg::decode(&frame(sc::SCMateStatePacket, &body.finish())).is_err());
            }
        }
        let dynamic = DynamicUnitState {
            skills: vec![[1, 2, u32::MAX]; MAX_DYNAMIC_ENTRIES],
            tags: vec![[4, 5, 0x80000000]; MAX_DYNAMIC_ENTRIES],
        };
        let mut body = Writer::new();
        body.bc(1);
        dynamic_bytes(&dynamic, &mut body);
        let packets = [
            (sc::SCMateStatePacket, body.finish()),
            (sc::SCOnOffSnowPacket, vec![1]),
        ];
        assert_eq!(
            ServerMsg::decode(&batch(&packets)).unwrap(),
            ServerMsg::Batch(vec![
                ServerMsg::Mate(MateEvent::State { obj_id: 1, dynamic }),
                ServerMsg::Snow(true),
            ])
        );
    }

    fn select(id: u32) -> Event {
        Event::CharacterSelected(Box::new(CharacterState {
            character: Character {
                id,
                ..Default::default()
            },
            ..Default::default()
        }))
    }

    fn unit(obj_id: u32, tl_id: u16, owner_id: u32) -> Event {
        let mut unit = crate::state::tests::unit(obj_id);
        unit.kind = UnitKind::Mate {
            tl_id,
            template_id: 100,
            owner_id,
        };
        Event::UnitSpawned(Box::new(unit))
    }

    #[test]
    fn ownership_needs_both_timeline_and_stored_local_identity_in_either_order() {
        for owner_packet_first in [true, false] {
            let mut state = Mates::default();
            state.apply(&select(42));
            let spawned = Event::Mate(MateEvent::Spawned(spawn_fixture()));
            let visible = unit(51, 0x0102, 42);
            let remote = unit(52, 0x0102, 43);
            state.apply(&remote);
            let dynamic = DynamicUnitState {
                skills: vec![[1, 2, 3]],
                tags: vec![],
            };
            let update = Event::Mate(MateEvent::State {
                obj_id: 51,
                dynamic: dynamic.clone(),
            });
            assert!(
                !state.apply(&update),
                "no phantom visible mate for an unknown state packet"
            );
            if owner_packet_first {
                state.apply(&spawned);
                assert_eq!(state.own.as_ref().unwrap().obj_id, None);
                state.apply(&visible);
                state.apply(&update);
            } else {
                state.apply(&visible);
                state.apply(&update);
                assert!(
                    state.own.is_none(),
                    "a visible mate is not owner-only spawn proof"
                );
                state.apply(&spawned);
            }
            let own = state.own.as_ref().unwrap();
            assert_eq!(own.spawn, spawn_fixture());
            assert_eq!(own.obj_id, Some(51));
            assert_eq!(own.dynamic, Some(dynamic.clone()));
            assert!(
                !state.apply(&visible),
                "duplicate unit spawn preserves matching dynamic state"
            );
            assert!(!state.apply(&update));
            assert!(!state.apply(&Event::UnitPoints {
                obj_id: 51,
                hp: 1,
                mp: 2
            }));
            state.apply(&Event::UnitsRemoved(vec![52]));
            assert_eq!(state.own.as_ref().unwrap().obj_id, Some(51));
            state.apply(&Event::UnitsRemoved(vec![51]));
            assert!(state.own.is_none());
            assert!(!state.apply(&update));
        }
    }

    #[test]
    fn pending_replacements_ambiguous_timelines_and_object_reuse_never_keep_stale_ownership() {
        let mut state = Mates::default();
        state.apply(&select(42));
        state.apply(&unit(51, 0x0102, 42));
        state.apply(&Event::Mate(MateEvent::Spawned(spawn_fixture())));
        let mut replacement = spawn_fixture();
        replacement.tl_id = 8;
        replacement.id = 9;
        state.apply(&Event::Mate(MateEvent::Spawned(replacement.clone())));
        assert_eq!(state.own.as_ref().unwrap().obj_id, None);
        state.apply(&Event::UnitsRemoved(vec![51]));
        assert_eq!(state.own.as_ref().unwrap().spawn, replacement);
        state.apply(&unit(52, 8, 42));
        state.apply(&unit(53, 8, 42));
        assert_eq!(
            state.own.as_ref().unwrap().obj_id,
            None,
            "ambiguous IDs are not arbitrarily chosen"
        );
        state.apply(&Event::UnitsRemoved(vec![53]));
        assert_eq!(state.own.as_ref().unwrap().obj_id, Some(52));
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(52))));
        assert!(
            state.own.is_none(),
            "same object ID reused for a different kind clears the binding"
        );
        state.apply(&Event::Mate(MateEvent::Spawned(replacement)));
        for reset in [
            select(43),
            Event::LeftWorld { target: 1 },
            Event::Disconnected {
                message: "test".into(),
            },
            Event::Kicked {
                reason: 1,
                message: "test".into(),
            },
        ] {
            let mut state = state.clone();
            assert!(state.apply(&reset));
            assert!(state.own.is_none());
            assert!(state.visible.is_empty());
        }
    }
}
