//! Slave creation announcements and native-owned timeline lifecycle, without phantom units.

use std::collections::HashMap;

use crate::model::UnitKind;
use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Reader, Result, WireError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlaveCreated {
    pub owner_obj_id: u32,
    pub tl_id: u16,
    pub obj_id: u32,
    pub hide_spawn_effect: bool,
    /// Native i64 field; the lane writes zero. No database/item identity is inferred.
    pub raw_id: i64,
    pub creator_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlaveEvent {
    /// Broadcast around the summoning owner, not owner-only proof by itself.
    Created(SlaveCreated),
    /// Raw ID role differs between lane normal deletion and Rider's Escape.
    Removed { obj_id: u32, tl_id: u16 },
    /// Vehicle object/effect despawn; distinct from the native owned-timeline clear.
    Despawn { obj_id: u32 },
}

impl SlaveEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCSlaveCreatedPacket | sc::SCSlaveRemovedPacket | sc::SCSlaveDespawnPacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Created(_) => "SCSlaveCreated",
            Self::Removed { .. } => "SCSlaveRemoved",
            Self::Despawn { .. } => "SCSlaveDespawn",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCSlaveCreatedPacket => Self::Created(SlaveCreated {
                owner_obj_id: r.bc()?,
                tl_id: r.u16()?,
                obj_id: r.bc()?,
                hide_spawn_effect: r.bool()?,
                raw_id: r.i64()?,
                creator_name: r.string()?,
            }),
            sc::SCSlaveRemovedPacket => Self::Removed {
                obj_id: r.bc()?,
                tl_id: r.u16()?,
            },
            sc::SCSlaveDespawnPacket => Self::Despawn { obj_id: r.bc()? },
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown slave opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Slaves {
    /// Last native-owned announcement. Does not imply that a Unit is currently visible.
    /// Cleared by matched owner-ID/timeline removal, replacement, object reuse or teardown.
    pub own: Option<SlaveCreated>,
    /// Creation/effect metadata; these are not UnitSpawned objects or transform information.
    pub created: HashMap<u32, SlaveCreated>,
    own_obj_id: u32,
}

impl Slaves {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::CharacterSelected(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. } => {
                if *self == Self::default() {
                    return false;
                }
                *self = Self::default();
            }
            Event::PlayerSpawn(player) => {
                if self.own_obj_id == player.obj_id {
                    return false;
                }
                if self.own_obj_id != 0 {
                    *self = Self::default();
                }
                self.own_obj_id = player.obj_id;
                self.own = self
                    .created
                    .values()
                    .find(|entry| entry.owner_obj_id == player.obj_id && player.obj_id != 0)
                    .cloned();
            }
            Event::Slave(SlaveEvent::Created(entry)) => {
                if entry.owner_obj_id != 0 {
                    // The lane CreateForOwner replaces its prior active timeline.
                    self.created
                        .retain(|_, old| old.owner_obj_id != entry.owner_obj_id);
                }
                self.created.insert(entry.obj_id, entry.clone());
                if self.own_obj_id != 0 && entry.owner_obj_id == self.own_obj_id {
                    self.own = Some(entry.clone());
                } else if self
                    .own
                    .as_ref()
                    .is_some_and(|own| own.obj_id == entry.obj_id)
                {
                    self.own = None; // A runtime object ID was reused by another owner.
                }
            }
            Event::Slave(SlaveEvent::Removed { obj_id, tl_id }) => {
                let old_len = self.created.len();
                self.created.retain(|id, entry| {
                    entry.tl_id != *tl_id || (*id != *obj_id && entry.owner_obj_id != *obj_id)
                });
                let matched_own = self.own_obj_id != 0
                    && *obj_id == self.own_obj_id
                    && self.own.as_ref().is_some_and(|own| own.tl_id == *tl_id);
                if matched_own {
                    self.own = None;
                }
                if !matched_own && old_len == self.created.len() {
                    return false;
                }
            }
            Event::Slave(SlaveEvent::Despawn { obj_id }) => {
                if self.created.remove(obj_id).is_none() {
                    return false;
                }
            }
            Event::UnitsRemoved(ids) => {
                let old_len = self.created.len();
                self.created.retain(|id, _| !ids.contains(id));
                if old_len == self.created.len() {
                    return false;
                }
                // Native own timeline can survive visibility loss; only a matching 062 clears it.
            }
            Event::UnitSpawned(unit) => {
                let matches = |entry: &SlaveCreated| matches!(unit.kind, UnitKind::Slave { tl_id, .. } if tl_id == entry.tl_id);
                let stale_created = self
                    .created
                    .get(&unit.obj_id)
                    .is_some_and(|entry| !matches(entry));
                let stale_own = self
                    .own
                    .as_ref()
                    .is_some_and(|entry| entry.obj_id == unit.obj_id && !matches(entry));
                if !stale_created && !stale_own {
                    return false;
                }
                if stale_created {
                    self.created.remove(&unit.obj_id);
                }
                if stale_own {
                    self.own = None;
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::model::CharacterState;
    use crate::session::PlayerSpawn;
    use crate::wire::Writer;

    pub(crate) fn fixture() -> SlaveCreated {
        SlaveCreated {
            owner_obj_id: 0x112233,
            tl_id: 0x4567,
            obj_id: 0xaabbcc,
            hide_spawn_effect: true,
            raw_id: -0x102030405060708,
            creator_name: "Owned\u{8239}".into(),
        }
    }

    fn player() -> Event {
        let mut state = CharacterState::default();
        state.character.id = 42;
        Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 0x112233,
            state,
            unit: crate::state::tests::unit(0x112233),
        }))
    }

    #[test]
    fn three_native_lifecycle_shapes_consume_exactly_and_keep_following_dd04_packets() {
        let created = fixture();
        assert_eq!(created.creator_name.len(), 8);
        let mut w = Writer::new();
        w.bc(created.owner_obj_id)
            .u16(created.tl_id)
            .bc(created.obj_id)
            .bool(true);
        w.i64(created.raw_id).string(&created.creator_name);
        let packets = vec![
            (0x61, w.finish(), SlaveEvent::Created(created.clone())),
            (
                0x62,
                vec![0x33, 0x22, 0x11, 0x67, 0x45],
                SlaveEvent::Removed {
                    obj_id: 0x112233,
                    tl_id: 0x4567,
                },
            ),
            (
                0x63,
                vec![0xcc, 0xbb, 0xaa],
                SlaveEvent::Despawn { obj_id: 0xaabbcc },
            ),
        ];
        assert_eq!(
            &packets[0].1[..9],
            &[0x33, 0x22, 0x11, 0x67, 0x45, 0xcc, 0xbb, 0xaa, 1]
        );
        let mut inner = Writer::new();
        let mut expected = Vec::new();
        for (opcode, raw, event) in &packets {
            let mut r = Reader::new(raw);
            assert_eq!(SlaveEvent::read(*opcode, &mut r).unwrap(), *event);
            assert!(r.is_empty());
            for n in 0..raw.len() {
                assert!(
                    SlaveEvent::read(*opcode, &mut Reader::new(&raw[..n])).is_err(),
                    "op {opcode:x} cut {n}"
                );
            }
            inner.u16(0).u16(*opcode).raw(raw);
            expected.push(ServerMsg::Slave(event.clone()));
        }
        inner.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        expected.push(ServerMsg::Snow(true));
        let inner = inner.finish();
        let mut outer = Writer::new();
        let len = inner.len() as u16;
        outer
            .raw(&[0xdd, 4])
            .u16(4)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&inner);
        assert_eq!(
            ServerMsg::decode(&outer.finish()).unwrap(),
            ServerMsg::Batch(expected)
        );
    }

    #[test]
    fn creation_correlates_runtime_owner_in_either_order_without_units_or_stored_id_guess() {
        for announced_first in [false, true] {
            let mut state = crate::state::WorldState::default();
            let event = Event::Slave(SlaveEvent::Created(fixture()));
            if announced_first {
                state.apply(&event);
                assert!(state.slaves.own.is_none());
            }
            state.apply(&player());
            if !announced_first {
                state.apply(&event);
            }
            assert_eq!(state.slaves.own, Some(fixture()));
            assert_eq!(state.slaves.created.len(), 1);
            assert!(state.units.by_id.is_empty());
            let stored_id = SlaveCreated {
                owner_obj_id: 42,
                obj_id: 99,
                ..fixture()
            };
            state.apply(&Event::Slave(SlaveEvent::Created(stored_id)));
            assert_eq!(state.slaves.own, Some(fixture()));
            assert!(state.units.by_id.is_empty());
        }
    }

    #[test]
    fn native_owner_timeline_clear_is_distinct_from_despawn_visibility_and_escape() {
        let mut slaves = Slaves::default();
        slaves.apply(&player());
        let entry = fixture();
        slaves.apply(&Event::Slave(SlaveEvent::Created(entry.clone())));
        slaves.apply(&Event::Slave(SlaveEvent::Despawn {
            obj_id: entry.obj_id,
        }));
        assert!(slaves.created.is_empty());
        assert_eq!(slaves.own, Some(entry.clone()));
        // Rider's Escape uses the slave object ID here, unlike normal owner-origin deletion.
        assert!(!slaves.apply(&Event::Slave(SlaveEvent::Removed {
            obj_id: entry.obj_id,
            tl_id: entry.tl_id
        })));
        assert_eq!(slaves.own, Some(entry.clone()));
        assert!(!slaves.apply(&Event::Slave(SlaveEvent::Removed {
            obj_id: entry.owner_obj_id,
            tl_id: entry.tl_id - 1
        })));
        assert!(slaves.apply(&Event::Slave(SlaveEvent::Removed {
            obj_id: entry.owner_obj_id,
            tl_id: entry.tl_id
        })));
        assert!(slaves.own.is_none());
        slaves.apply(&Event::Slave(SlaveEvent::Created(entry.clone())));
        slaves.apply(&Event::UnitsRemoved(vec![entry.obj_id]));
        assert!(slaves.created.is_empty());
        assert_eq!(
            slaves.own,
            Some(entry.clone()),
            "own timeline can outlive visibility"
        );
        let next = SlaveCreated {
            obj_id: 77,
            tl_id: 88,
            ..entry.clone()
        };
        slaves.apply(&Event::Slave(SlaveEvent::Created(next.clone())));
        slaves.apply(&Event::Slave(SlaveEvent::Removed {
            obj_id: entry.owner_obj_id,
            tl_id: entry.tl_id,
        }));
        assert_eq!(
            slaves.own,
            Some(next.clone()),
            "old timeline must not clear replacement"
        );
        slaves.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(
            next.obj_id,
        ))));
        assert!(slaves.own.is_none());
        assert!(slaves.created.is_empty());
        slaves.apply(&Event::Slave(SlaveEvent::Created(entry)));
        slaves.apply(&Event::Disconnected {
            message: "test".into(),
        });
        assert_eq!(slaves, Slaves::default());
    }
}
