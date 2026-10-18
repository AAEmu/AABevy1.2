//! Received conflict-zone state. State bytes and timestamp bits remain authoritative wire data.

use std::collections::HashMap;

use crate::session::Event;
use crate::wire::{Reader, Result};

/// `SCConflictZoneState` (0x0ee): exactly u16, u8, u64. No object/instance ID is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConflictZoneState {
    pub zone_group_id: u16,
    /// Native raw state byte; unknown values are preserved.
    pub state: u8,
    /// Original native unsigned 64-bit field, without guessed timestamp conversion.
    pub end_time: u64,
}

impl ConflictZoneState {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_group_id: r.u16()?,
            state: r.u8()?,
            end_time: r.u64()?,
        })
    }

    /// The lane writes this field as signed Unix seconds; zero means no scheduled transition.
    /// This interpretation does not expire the report or advance its state.
    pub fn lane_end_time_seconds(&self) -> i64 {
        self.end_time as i64
    }
}

/// Latest received report for each group in the selected world, including pre-spawn reports.
/// The u16 group key bounds this map to the protocol's 65536 possible keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct ConflictZones {
    pub by_group: HashMap<u16, ConflictZoneState>,
}

impl ConflictZones {
    pub fn get(&self, zone_group_id: u16) -> Option<&ConflictZoneState> {
        self.by_group.get(&zone_group_id)
    }

    /// Returns true only when a received value or world/account scope changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::ConflictZoneState(state) => {
                if self.get(state.zone_group_id) == Some(state) {
                    return false;
                }
                self.by_group.insert(state.zone_group_id, *state);
                true
            }
            Event::LoginSucceeded(_)
            | Event::ReadyToConnect { .. }
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. } => {
                let changed = !self.by_group.is_empty();
                self.by_group.clear();
                changed
            }
            // Character selection/spawn, map movement and elapsed time cannot clear/advance
            // world-level reports. They may arrive before any local player exists.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::opcodes::sc;
    use crate::wire::Writer;

    fn dd01(body: &[u8]) -> Result<ServerMsg> {
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 1, 0, 0])
            .u16(sc::SCConflictZoneStatePacket)
            .raw(body);
        ServerMsg::decode(&frame.finish())
    }

    fn body(state: ConflictZoneState) -> Vec<u8> {
        let mut w = Writer::new();
        w.u16(state.zone_group_id)
            .u8(state.state)
            .u64(state.end_time);
        w.finish()
    }

    #[test]
    fn native_conflict_golden_retains_unsigned_time_bits_and_exact_tail() {
        // Native397c2cc0: ushort group, byte state, uint64 endTime. Lane uses signed Unix seconds.
        let bytes = [
            0x34, 0x12, 0xfe, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0xf1, 0xab,
        ];
        let value = ConflictZoneState {
            zone_group_id: 0x1234,
            state: 0xfe,
            end_time: 0xf123_4567_89ab_cdef,
        };
        let mut r = Reader::new(&bytes);
        assert_eq!(ConflictZoneState::read(&mut r).unwrap(), value);
        assert_eq!(r.rest(), &[0xab]);
        assert_eq!(
            dd01(&bytes[..11]).unwrap(),
            ServerMsg::ConflictZoneState(value)
        );
        assert_eq!(
            ServerMsg::ConflictZoneState(value).name(),
            "SCConflictZoneState"
        );
        assert_eq!(value.lane_end_time_seconds(), -1_070_935_975_390_360_081);
        for length in 0..11 {
            assert!(dd01(&bytes[..length]).is_err(), "truncated length {length}");
        }
    }

    #[test]
    fn conflict_accepts_all_raw_states_and_timestamp_bit_patterns() {
        for state in 0..=u8::MAX {
            for (zone_group_id, end_time) in [
                (0, 0),
                (u16::MAX, u64::MAX),
                (14, 1 << 63),
                (7, 1_792_300_000),
            ] {
                let expected = ConflictZoneState {
                    zone_group_id,
                    state,
                    end_time,
                };
                assert_eq!(
                    dd01(&body(expected)).unwrap(),
                    ServerMsg::ConflictZoneState(expected)
                );
            }
        }
        assert_eq!(
            ConflictZoneState {
                end_time: 0,
                ..Default::default()
            }
            .lane_end_time_seconds(),
            0
        );
        assert_eq!(
            ConflictZoneState {
                end_time: u64::MAX,
                ..Default::default()
            }
            .lane_end_time_seconds(),
            -1
        );
        assert_eq!(
            ConflictZoneState {
                end_time: 1 << 63,
                ..Default::default()
            }
            .lane_end_time_seconds(),
            i64::MIN
        );
    }

    #[test]
    fn conflict_dd04_preserves_repeated_group_reports_and_following_state() {
        let a = ConflictZoneState {
            zone_group_id: 14,
            state: 6,
            end_time: 1_792_300_000,
        };
        let b = ConflictZoneState {
            state: 0xff,
            end_time: u64::MAX,
            ..a
        };
        let mut inner = Writer::new();
        inner
            .u16(0)
            .u16(sc::SCConflictZoneStatePacket)
            .raw(&body(a));
        inner.u16(0).u16(sc::SCSetBreathPacket).u32(777);
        inner
            .u16(0)
            .u16(sc::SCConflictZoneStatePacket)
            .raw(&body(b));
        inner
            .u16(0)
            .u16(sc::SCUnitPointsPacket)
            .bc(5)
            .i32(100)
            .i32(200);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(4)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&inner);
        let decoded = ServerMsg::decode(&frame.finish()).unwrap();
        assert_eq!(
            decoded,
            ServerMsg::Batch(vec![
                ServerMsg::ConflictZoneState(a),
                ServerMsg::SetBreath { breath: 777 },
                ServerMsg::ConflictZoneState(b),
                ServerMsg::UnitPoints {
                    obj_id: 5,
                    hp: 1,
                    mp: 2
                }
            ])
        );
    }

    #[test]
    fn conflict_cache_updates_only_reported_groups_and_never_expires_or_grants_permission() {
        let mut zones = ConflictZones::default();
        let a = ConflictZoneState {
            zone_group_id: 14,
            state: 6,
            end_time: 1,
        }; // deliberately in the past
        let b = ConflictZoneState {
            zone_group_id: 7,
            state: 255,
            end_time: u64::MAX,
        };
        assert!(zones.apply(&Event::ConflictZoneState(a)));
        assert!(zones.apply(&Event::ConflictZoneState(b)));
        assert!(!zones.apply(&Event::ConflictZoneState(a)));
        assert!(!zones.apply(&Event::TimeOfDay {
            hour: 23.0,
            speed: Some(1.0)
        }));
        assert!(!zones.apply(&Event::ForceAttackSet {
            obj_id: 5,
            on: true
        }));
        assert_eq!(zones.get(14), Some(&a));
        assert_eq!(zones.get(7), Some(&b));
        let updated = ConflictZoneState {
            state: 3,
            end_time: 0,
            ..a
        };
        assert!(zones.apply(&Event::ConflictZoneState(updated)));
        assert_eq!(zones.by_group.len(), 2);
        assert_eq!(zones.get(14), Some(&updated));
        assert_eq!(zones.get(7), Some(&b));
    }

    #[test]
    fn initial_conflict_reports_survive_character_spawn_and_clear_at_world_account_boundaries() {
        let value = ConflictZoneState {
            zone_group_id: 14,
            state: 6,
            end_time: 0,
        };
        let mut world = crate::state::WorldState::default();
        world.apply(&Event::ConflictZoneState(value));
        world.apply(&Event::CharacterSelected(Box::default()));
        world.apply(&Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn {
            obj_id: 5,
            state: Default::default(),
            unit: crate::state::tests::unit(5),
        })));
        world.apply(&Event::EnteredWorld);
        world.apply(&Event::LobbyReady);
        assert_eq!(world.conflict_zones.get(14), Some(&value));
        assert!(world.combat_status.get(5).is_some());
        assert_eq!(
            world.combat_status.get(5).unwrap().force_attack,
            Some(false)
        );
        for event in [
            Event::LoginSucceeded(Default::default()),
            Event::ReadyToConnect {
                world_id: 1,
                cookie: crate::login::WorldCookie {
                    cookie: 0,
                    addr: "127.0.0.1:1".parse().unwrap(),
                },
            },
            Event::LeftWorld { target: 0 },
            Event::Disconnected {
                message: String::new(),
            },
        ] {
            let mut zones = world.conflict_zones.clone();
            assert!(zones.apply(&event));
            assert_eq!(zones, ConflictZones::default());
            assert!(!zones.apply(&event));
        }
    }
}
