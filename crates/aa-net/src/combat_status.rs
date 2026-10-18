//! Received combat flags and environmental reports. No health or breathing simulation lives here.

use std::collections::{HashMap, hash_map::Entry};

use crate::session::Event;
use crate::wire::{Reader, Result};

/// Native SCUnitState bit3, converted to the same current flag used by the toggle and SC043.
pub const FORCE_ATTACK_FLAG: u16 = 0x0008;

/// `SCEnvDamage`: a presentation report, separate from authoritative HP changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvironmentDamage {
    /// Raw source byte. The lane uses 0 fall, 1 gimmick, 2 drowning, 3 ship collision.
    pub source: u8,
    pub target_obj_id: u32,
    /// Raw unsigned amount. It is not necessarily the actual HP delta (for example fall caps).
    pub amount: u32,
    /// Present exactly when source==1. This is a full u32, not a BC object ID.
    pub gimmick_id: Option<u32>,
}

impl EnvironmentDamage {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let source = r.u8()?;
        Ok(Self {
            source,
            target_obj_id: r.bc()?,
            amount: r.u32()?,
            gimmick_id: if source == 1 { Some(r.u32()?) } else { None },
        })
    }
}

/// Force attack starts from the native snapshot bit and then follows explicit reports.
/// Invisibility is unknown until its separate report arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitCombatFlags {
    pub force_attack: Option<bool>,
    pub invisible: Option<bool>,
}

/// Received-only flags for announced units (including the player), and the latest local breath
/// value. Breath has no object ID or maximum on this wire and is not advanced with elapsed time.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct CombatStatus {
    pub by_unit: HashMap<u32, UnitCombatFlags>,
    pub breath: Option<u32>,
}

impl CombatStatus {
    pub fn get(&self, obj_id: u32) -> Option<&UnitCombatFlags> {
        self.by_unit.get(&obj_id)
    }

    fn announce(&mut self, unit: &crate::model::Unit) -> bool {
        if let Entry::Vacant(entry) = self.by_unit.entry(unit.obj_id) {
            entry.insert(UnitCombatFlags {
                force_attack: Some(unit.flags & FORCE_ATTACK_FLAG != 0),
                invisible: None,
            });
            true
        } else {
            false
        }
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::UnitSpawned(unit) => self.announce(unit),
            Event::PlayerSpawn(player) => self.announce(&player.unit),
            Event::ForceAttackSet { obj_id, on } => {
                let Some(flags) = self.by_unit.get_mut(obj_id) else {
                    return false;
                };
                if flags.force_attack == Some(*on) {
                    return false;
                }
                flags.force_attack = Some(*on);
                true
            }
            Event::UnitInvisible { obj_id, invisible } => {
                let Some(flags) = self.by_unit.get_mut(obj_id) else {
                    return false;
                };
                if flags.invisible == Some(*invisible) {
                    return false;
                }
                flags.invisible = Some(*invisible);
                true
            }
            Event::SetBreath { breath } => {
                if self.breath == Some(*breath) {
                    return false;
                }
                self.breath = Some(*breath);
                true
            }
            Event::UnitsRemoved(ids) => {
                let mut changed = false;
                for id in ids {
                    changed |= self.by_unit.remove(id).is_some();
                }
                changed
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.by_unit.is_empty() || self.breath.is_some();
                *self = Self::default();
                changed
            }
            // Damage reports must not subtract HP, invent combat engagement/death, or spawn units.
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

    fn dd01(opcode: u16, body: &[u8]) -> Result<ServerMsg> {
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(body);
        ServerMsg::decode(&frame.finish())
    }

    fn dd04(packets: &[(u16, Vec<u8>)]) -> Result<ServerMsg> {
        let mut inner = Writer::new();
        for (opcode, body) in packets {
            inner.u16(0).u16(*opcode).raw(body);
        }
        let inner = inner.finish();
        let length = inner.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(packets.len() as u16)
            .u8(1)
            .u16(length)
            .u16(!length)
            .raw(&inner);
        ServerMsg::decode(&frame.finish())
    }

    #[test]
    fn force_attack_request_is_exactly_one_bool_and_initial_snapshot_uses_only_bit_three() {
        for on in [false, true] {
            let message = crate::game::ClientMsg::SetForceAttack { on };
            assert_eq!(message.encode(7), [0xdd, 1, 0, 7, 0x4f, 0, u8::from(on)]);
            assert_eq!(message.name(), "CSSetForceAttackPacket");
            let body = message.encode_body();
            let mut r = Reader::new(&body);
            assert_eq!(r.bool().unwrap(), on);
            assert!(
                r.is_empty(),
                "lane reader consumes exactly one bool with no object ID"
            );
        }
        for flags in [0, 1, 2, 4, 8, 16, 0x2000, 0xffff, 0xfff7] {
            let mut unit = crate::state::tests::unit(7);
            unit.flags = flags;
            let mut cache = CombatStatus::default();
            assert!(cache.apply(&Event::UnitSpawned(Box::new(unit.clone()))));
            assert_eq!(cache.get(7).unwrap().force_attack, Some(flags & 8 != 0));
            assert_eq!(cache.get(7).unwrap().invisible, None);
            assert!(cache.apply(&Event::ForceAttackSet {
                obj_id: 7,
                on: flags & 8 == 0
            }));
            assert!(!cache.apply(&Event::UnitSpawned(Box::new(unit))));
            assert_eq!(
                cache.get(7).unwrap().force_attack,
                Some(flags & 8 == 0),
                "duplicate announcement must not undo a later explicit report"
            );
        }
    }

    #[test]
    fn combat_status_receive_goldens_preserve_widths_and_all_truncations_fail() {
        let cases = [
            (
                sc::SCEnvDamagePacket,
                vec![1, 0x56, 0x34, 0x12, 0xff, 0xff, 0xff, 0xff, 4, 3, 2, 0xf1],
                ServerMsg::EnvDamage(EnvironmentDamage {
                    source: 1,
                    target_obj_id: 0x123456,
                    amount: u32::MAX,
                    gimmick_id: Some(0xf1020304),
                }),
            ),
            (
                sc::SCEnvDamagePacket,
                vec![3, 0xff, 0xff, 0xff, 4, 3, 2, 0xf1],
                ServerMsg::EnvDamage(EnvironmentDamage {
                    source: 3,
                    target_obj_id: 0xffffff,
                    amount: 0xf1020304,
                    gimmick_id: None,
                }),
            ),
            (
                sc::SCSetBreathPacket,
                vec![4, 3, 2, 0xf1],
                ServerMsg::SetBreath { breath: 0xf1020304 },
            ),
            (
                sc::SCForceAttackSetPacket,
                vec![0xff, 0xff, 0xff, 1],
                ServerMsg::ForceAttackSet {
                    obj_id: 0xffffff,
                    on: true,
                },
            ),
            (
                sc::SCUnitInvisiblePacket,
                vec![0x56, 0x34, 0x12, 0],
                ServerMsg::UnitInvisible {
                    obj_id: 0x123456,
                    invisible: false,
                },
            ),
        ];
        for (opcode, body, expected) in cases {
            assert_eq!(dd01(opcode, &body).unwrap(), expected);
            for cut in 0..body.len() {
                assert!(dd01(opcode, &body[..cut]).is_err(), "{opcode:x} cut {cut}");
            }
            assert_eq!(
                dd04(&[(opcode, body), (sc::SCOnOffSnowPacket, vec![1])]).unwrap(),
                ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
            );
        }
    }

    #[test]
    fn combat_status_environment_source_conditional_has_one_exact_branch_and_keeps_dd04_tail() {
        let mut packets = Vec::new();
        let mut expected = Vec::new();
        for source in 0..=u8::MAX {
            let mut body = Writer::new();
            body.u8(source).bc(0xffffff).u32(u32::MAX);
            if source == 1 {
                body.u32(0xf1020304);
            }
            let body = body.finish();
            assert_eq!(body.len(), if source == 1 { 12 } else { 8 });
            let mut with_tail = body.clone();
            with_tail.extend([0x99; 4]);
            let mut r = Reader::new(&with_tail);
            let value = EnvironmentDamage::read(&mut r).unwrap();
            assert_eq!(r.rest(), &[0x99; 4]);
            assert_eq!(value.source, source);
            assert_eq!(value.gimmick_id, (source == 1).then_some(0xf1020304));
            packets.push((sc::SCEnvDamagePacket, body));
            expected.push(ServerMsg::EnvDamage(value));
        }
        packets.extend([
            (sc::SCForceAttackSetPacket, vec![7, 0, 0, 0]),
            (sc::SCUnitInvisiblePacket, vec![7, 0, 0, 1]),
            (sc::SCSetBreathPacket, vec![0; 4]),
            (sc::SCOnOffSnowPacket, vec![1]),
        ]);
        expected.extend([
            ServerMsg::ForceAttackSet {
                obj_id: 7,
                on: false,
            },
            ServerMsg::UnitInvisible {
                obj_id: 7,
                invisible: true,
            },
            ServerMsg::SetBreath { breath: 0 },
            ServerMsg::Snow(true),
        ]);
        assert_eq!(dd04(&packets).unwrap(), ServerMsg::Batch(expected));
    }

    #[test]
    fn combat_status_tracks_only_received_flags_of_announced_units_and_exact_breath() {
        let mut state = crate::state::WorldState::default();
        let flag = Event::ForceAttackSet {
            obj_id: 7,
            on: false,
        };
        let hidden = Event::UnitInvisible {
            obj_id: 7,
            invisible: true,
        };
        assert!(!state.combat_status.apply(&flag));
        assert!(!state.combat_status.apply(&hidden));
        assert!(state.combat_status.by_unit.is_empty());
        let mut unit = crate::state::tests::unit(7);
        unit.flags = FORCE_ATTACK_FLAG;
        let spawn = Event::UnitSpawned(Box::new(unit));
        state.apply(&spawn);
        assert_eq!(
            state.combat_status.get(7),
            Some(&UnitCombatFlags {
                force_attack: Some(true),
                invisible: None
            })
        );
        assert!(!state.combat_status.apply(&spawn));
        assert!(state.combat_status.apply(&flag));
        assert!(!state.combat_status.apply(&flag));
        assert!(state.combat_status.apply(&hidden));
        assert_eq!(
            state.combat_status.get(7),
            Some(&UnitCombatFlags {
                force_attack: Some(false),
                invisible: Some(true)
            })
        );
        assert!(
            state.units.get(7).is_some(),
            "invisibility must not delete the unit"
        );
        assert!(state.combat_status.apply(&Event::UnitInvisible {
            obj_id: 7,
            invisible: false
        }));
        assert_eq!(state.combat_status.get(7).unwrap().invisible, Some(false));
        for breath in [u32::MAX, 1, 0, 1000] {
            assert!(state.combat_status.apply(&Event::SetBreath { breath }));
            assert_eq!(state.combat_status.breath, Some(breath));
            assert!(!state.combat_status.apply(&Event::SetBreath { breath }));
        }
        state.apply(&Event::UnitsRemoved(vec![7]));
        assert!(state.combat_status.get(7).is_none());
        assert!(!state.combat_status.apply(&flag));
        state.apply(&spawn);
        assert_eq!(
            state.combat_status.get(7),
            Some(&UnitCombatFlags {
                force_attack: Some(true),
                invisible: None
            }),
            "a reused ID must not inherit stale flags"
        );
        for lifecycle in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 1 },
            Event::Disconnected {
                message: "end".into(),
            },
        ] {
            state.apply(&spawn);
            state.apply(&Event::SetBreath { breath: 0 });
            state.apply(&lifecycle);
            assert!(state.combat_status.by_unit.is_empty());
            assert_eq!(state.combat_status.breath, None);
        }
    }

    #[test]
    fn combat_status_damage_reports_never_double_subtract_hp_or_invent_player_progression() {
        let mut state = crate::state::WorldState::default();
        let mut character = crate::model::CharacterState::default();
        character.character.id = 17;
        state.apply(&Event::CharacterSelected(Box::new(character.clone())));
        state.apply(&Event::PlayerSpawn(Box::new(crate::session::PlayerSpawn {
            obj_id: 7,
            state: character,
            unit: crate::state::tests::unit(7),
        })));
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(8))));
        assert!(state.combat_status.get(7).is_some());
        state.apply(&Event::ForceAttackSet {
            obj_id: 7,
            on: true,
        });
        assert_eq!(state.combat_status.get(7).unwrap().force_attack, Some(true));
        for target_obj_id in [7, 8, 99] {
            for source in 0..=3 {
                state.apply(&Event::EnvDamage(EnvironmentDamage {
                    source,
                    target_obj_id,
                    amount: u32::MAX,
                    gimmick_id: (source == 1).then_some(u32::MAX),
                }));
            }
        }
        assert_eq!(
            (state.stats.hp, state.stats.dead, state.stats.in_combat),
            (10, false, false)
        );
        assert_eq!(state.units.get(8).unwrap().hp, 10);
        assert!(state.units.get(99).is_none());
        assert!(state.combat_status.get(99).is_none());
        assert_eq!(state.combat_status.breath, None);
        state.apply(&Event::UnitPoints {
            obj_id: 7,
            hp: 5,
            mp: 2,
        });
        state.apply(&Event::UnitPoints {
            obj_id: 8,
            hp: 4,
            mp: 3,
        });
        assert_eq!(state.stats.hp, 5);
        assert_eq!(state.units.get(8).unwrap().hp, 4);
    }
}
