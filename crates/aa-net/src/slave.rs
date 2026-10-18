//! Slave creation announcements and native-owned timeline lifecycle, without phantom units.

use std::collections::HashMap;

use crate::model::UnitKind;
use crate::opcodes::sc;
use crate::session::Event;
use crate::slave_control::{SlaveCommand, SlaveItemLocation};
use crate::wire::{Pos, Reader, Result, WireError, long_to_coord};

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

/// Full received ship/vehicle equipment transaction. Wire order is preserved;
/// the two items are not interpreted as accepted destinations or bag settlement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlaveEquipmentChange {
    pub character_id: u32,
    pub slave_tl_id: u16,
    pub db_slave_id: u32,
    pub bts: bool,
    pub first_item: Option<crate::model::Item>,
    pub second_item: Option<crate::model::Item>,
    pub first_location: SlaveItemLocation,
    pub second_location: SlaveItemLocation,
    pub success: bool,
}

impl SlaveEquipmentChange {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            character_id: r.u32()?, slave_tl_id: r.u16()?, db_slave_id: r.u32()?, bts: r.bool()?,
            first_item: crate::model::Item::read(r)?, second_item: crate::model::Item::read(r)?,
            first_location: SlaveItemLocation { container: r.u8()?, slot: r.u8()? },
            second_location: SlaveItemLocation { container: r.u8()?, slot: r.u8()? }, success: r.bool()?,
        })
    }
}

/// SCMySlave's owner-only position/HP report; raw XY retain the native fixed-point bits.
#[derive(Clone, Debug, PartialEq)]
pub struct SlavePositionInfo {
    pub obj_id: u32,
    pub tl_id: u16,
    pub name: String,
    pub template_id: u32,
    pub hp: i32,
    pub max_hp: i32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
}
impl SlavePositionInfo {
    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlaveEscape {
    pub obj_id: u32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub yaw: f32,
}
impl SlaveEscape {
    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SlaveEvent {
    EquipmentChanged(SlaveEquipmentChange),
    /// Broadcast around the summoning owner, not owner-only proof by itself.
    Created(SlaveCreated),
    /// Stored master character ID, independent of summoner ownership and runtime player ID.
    Bound {
        master_id: u32,
        obj_id: u32,
    },
    Position(SlavePositionInfo),
    Escaped(SlaveEscape),
    /// Local socket write only; never a completed summon/movement/attachment acknowledgement.
    Requested(SlaveCommand),
    RequestRejected {
        command: SlaveCommand,
        message: String,
    },
    /// Raw ID role differs between lane normal deletion and Rider's Escape.
    Removed {
        obj_id: u32,
        tl_id: u16,
    },
    /// Vehicle object/effect despawn; distinct from the native owned-timeline clear.
    Despawn {
        obj_id: u32,
    },
}

impl SlaveEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCSlaveEquipmentChangedPacket
                | sc::SCSlaveCreatedPacket
                | sc::SCSlaveRemovedPacket
                | sc::SCSlaveDespawnPacket
                | sc::SCSlaveBoundPacket
                | sc::SCMySlavePacket
                | sc::SCEscapeSlavePacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::EquipmentChanged(_) => "SCSlaveEquipmentChanged",
            Self::Created(_) => "SCSlaveCreated",
            Self::Bound { .. } => "SCSlaveBound",
            Self::Position(_) => "SCMySlave",
            Self::Escaped(_) => "SCEscapeSlave",
            Self::Requested(_) => "SlaveRequested",
            Self::RequestRejected { .. } => "SlaveRequestRejected",
            Self::Removed { .. } => "SCSlaveRemoved",
            Self::Despawn { .. } => "SCSlaveDespawn",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCSlaveEquipmentChangedPacket => Self::EquipmentChanged(SlaveEquipmentChange::read(r)?),
            sc::SCSlaveBoundPacket => Self::Bound {
                master_id: r.u32()?,
                obj_id: r.bc()?,
            },
            sc::SCMySlavePacket => Self::Position(SlavePositionInfo {
                obj_id: r.bc()?,
                tl_id: r.u16()?,
                name: r.string()?,
                template_id: r.u32()?,
                hp: r.i32()?,
                max_hp: r.i32()?,
                x: r.i64()?,
                y: r.i64()?,
                z: r.f32()?,
            }),
            sc::SCEscapeSlavePacket => Self::Escaped(SlaveEscape {
                obj_id: r.bc()?,
                x: r.i64()?,
                y: r.i64()?,
                z: r.f32()?,
                yaw: r.f32()?,
            }),
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

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Slaves {
    /// Last native-owned announcement. Does not imply that a Unit is currently visible.
    /// Cleared by matched owner-ID/timeline removal, replacement, object reuse or teardown.
    pub own: Option<SlaveCreated>,
    /// Latest SCMySlave report matching the already-announced owned timeline. Never a Unit.
    pub position: Option<SlavePositionInfo>,
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
            Event::Slave(SlaveEvent::Position(info)) => {
                if !self.own.as_ref().is_some_and(|own| own.tl_id == info.tl_id) {
                    return false;
                }
                self.position = Some(info.clone());
            }
            Event::Slave(SlaveEvent::Created(entry)) => {
                if entry.owner_obj_id != 0 {
                    // The lane CreateForOwner replaces its prior active timeline.
                    self.created
                        .retain(|_, old| old.owner_obj_id != entry.owner_obj_id);
                }
                self.created.insert(entry.obj_id, entry.clone());
                if self.own_obj_id != 0 && entry.owner_obj_id == self.own_obj_id {
                    if !self
                        .own
                        .as_ref()
                        .is_some_and(|old| old.tl_id == entry.tl_id)
                    {
                        self.position = None;
                    }
                    self.own = Some(entry.clone());
                } else if self
                    .own
                    .as_ref()
                    .is_some_and(|own| own.obj_id == entry.obj_id)
                {
                    self.own = None;
                    self.position = None; // A runtime object ID was reused by another owner.
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
                    self.position = None;
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
                    self.position = None;
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

    fn position_info() -> SlavePositionInfo {
        SlavePositionInfo {
            obj_id: 0xaabbcc,
            tl_id: 0x4567,
            name: "Boat".into(),
            template_id: 68,
            hp: -12,
            max_hp: 200,
            x: 0x0102030405060708,
            y: -0x112233445566778,
            z: 3.5,
        }
    }

    #[test]
    fn position_bound_escape_and_rename_have_exact_widths_and_dd04_boundaries() {
        let info = position_info();
        let escape = SlaveEscape {
            obj_id: info.obj_id,
            x: info.x,
            y: info.y,
            z: info.z,
            yaw: 0.5,
        };
        let mut pos = Writer::new();
        pos.bc(info.obj_id)
            .u16(info.tl_id)
            .string("Boat")
            .u32(68)
            .i32(-12)
            .i32(200)
            .i64(info.x)
            .i64(info.y)
            .f32(info.z);
        let mut esc = Writer::new();
        esc.bc(info.obj_id)
            .i64(info.x)
            .i64(info.y)
            .f32(info.z)
            .f32(0.5);
        let mut name = Writer::new();
        name.bc(info.obj_id).string("NewBoat");
        let packets = vec![
            (
                0x64,
                vec![0x98, 0xba, 0xdc, 0xfe, 0xcc, 0xbb, 0xaa],
                ServerMsg::Slave(SlaveEvent::Bound {
                    master_id: 0xfedcba98,
                    obj_id: info.obj_id,
                }),
            ),
            (
                0x66,
                pos.finish(),
                ServerMsg::Slave(SlaveEvent::Position(info.clone())),
            ),
            (
                0x67,
                esc.finish(),
                ServerMsg::Slave(SlaveEvent::Escaped(escape)),
            ),
            (
                0x70,
                name.finish(),
                ServerMsg::UnitNameChanged {
                    obj_id: info.obj_id,
                    name: "NewBoat".into(),
                },
            ),
        ];
        let mut inner = Writer::new();
        let mut expected = Vec::new();
        for (opcode, raw, message) in packets {
            let mut frame = Writer::new();
            frame.raw(&[0xdd, 1]).u16(0).u16(opcode).raw(&raw);
            assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(), message);
            for n in 0..raw.len() {
                let mut frame = Writer::new();
                frame.raw(&[0xdd, 1]).u16(0).u16(opcode).raw(&raw[..n]);
                assert!(
                    ServerMsg::decode(&frame.finish()).is_err(),
                    "op {opcode:x} cut {n}"
                );
            }
            inner.u16(0).u16(opcode).raw(&raw);
            expected.push(message);
        }
        inner.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        expected.push(ServerMsg::Snow(true));
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut outer = Writer::new();
        outer
            .raw(&[0xdd, 4])
            .u16(5)
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
    fn reports_require_owned_timeline_while_only_real_escape_and_rename_change_existing_units() {
        let mut state = crate::state::WorldState::default();
        state.apply(&player());
        let info = position_info();
        assert!(
            !state
                .slaves
                .apply(&Event::Slave(SlaveEvent::Position(info.clone())))
        );
        state.apply(&Event::Slave(SlaveEvent::Bound {
            master_id: 42,
            obj_id: info.obj_id,
        }));
        assert!(
            state.slaves.own.is_none(),
            "driver identity is not summoner ownership"
        );
        state.apply(&Event::Slave(SlaveEvent::Created(fixture())));
        let mismatch = SlavePositionInfo {
            tl_id: 1,
            ..info.clone()
        };
        assert!(
            !state
                .slaves
                .apply(&Event::Slave(SlaveEvent::Position(mismatch)))
        );
        state.apply(&Event::Slave(SlaveEvent::Position(info.clone())));
        assert_eq!(state.slaves.position, Some(info.clone()));
        assert!(state.units.by_id.is_empty());
        let escape = SlaveEscape {
            obj_id: info.obj_id,
            x: info.x,
            y: info.y,
            z: info.z,
            yaw: 0.5,
        };
        let mut non_slave = crate::state::Units::default();
        non_slave.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(
            info.obj_id,
        ))));
        let unchanged = non_slave.by_id[&info.obj_id].pos;
        assert!(!non_slave.apply(&Event::Slave(SlaveEvent::Escaped(escape.clone()))));
        assert_eq!(non_slave.by_id[&info.obj_id].pos, unchanged);
        state.apply(&Event::Slave(SlaveEvent::Escaped(escape.clone())));
        assert!(state.units.by_id.is_empty());
        let mut unit = crate::state::tests::unit(info.obj_id);
        unit.kind = UnitKind::Slave {
            id: 1,
            tl_id: info.tl_id,
            template_id: 68,
            summoner_id: 42,
        };
        let before = unit.pos;
        state.apply(&Event::UnitSpawned(Box::new(unit)));
        state.apply(&Event::Slave(SlaveEvent::Position(info.clone())));
        assert_eq!(
            state.units.by_id[&info.obj_id].pos, before,
            "map report is not unit movement"
        );
        state.apply(&Event::Slave(SlaveEvent::Escaped(escape)));
        assert_eq!(state.units.by_id[&info.obj_id].pos, info.position());
        assert_eq!(state.units.by_id[&info.obj_id].rotation[2], 0.5);
        state.apply(&Event::UnitNameChanged {
            obj_id: info.obj_id,
            name: "NewBoat".into(),
        });
        assert_eq!(state.units.by_id[&info.obj_id].name, "NewBoat");
        assert_eq!(
            state.slaves.own.as_ref().unwrap().creator_name,
            fixture().creator_name
        );
        state.apply(&Event::UnitNameChanged {
            obj_id: 99,
            name: "Unknown".into(),
        });
        assert!(!state.units.by_id.contains_key(&99));
        state.apply(&Event::Slave(SlaveEvent::Despawn {
            obj_id: info.obj_id,
        }));
        assert_eq!(state.slaves.position, Some(info.clone()));
        state.apply(&Event::Slave(SlaveEvent::Removed {
            obj_id: fixture().owner_obj_id,
            tl_id: info.tl_id,
        }));
        assert_eq!(state.slaves.position, None);
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
