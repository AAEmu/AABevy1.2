//! Original crime reputation and jury-number receipts. No trial choices or optimistic state.
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrimeEvent {
    Changed {
        points: i32,
        crime_points: i16,
        infamy_points: i32,
        crime_state: i16,
    },
    JuryPointChanged {
        jury_point: i32,
    },
    JuryWaitingNumber {
        waiting_number: i32,
    },
}
impl CrimeEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x16f | 0x182 | 0x18a)
    }
    /// Native397b6800/397afa10/397afe10 and active lane writers agree exactly.
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x16f => Self::Changed {
                points: r.i32()?,
                crime_points: r.i16()?,
                infamy_points: r.i32()?,
                crime_state: r.i16()?,
            },
            0x182 => Self::JuryWaitingNumber { waiting_number: r.i32()? },
            0x18a => Self::JuryPointChanged { jury_point: r.i32()? },
            _ => return Err(WireError::Invalid("unknown crime opcode".into())),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrimeReputation {
    pub crime_points: i16,
    pub infamy_points: i32,
    pub crime_state: i16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrimeNotice {
    /// CRIME_REPORTED(crime delta, infamy delta, crime-state delta), after assignment.
    Reported {
        crime_delta: i16,
        infamy_delta: i32,
        crime_state_delta: i16,
    },
    /// PLAYER_JURY_POINT(), after selected-character jury-point assignment.
    JuryPoint,
    /// JURY_WAITING_NUMBER(number). Native does not persist a queue-number getter here.
    JuryWaitingNumber(i32),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Crimes {
    pub character_id: Option<u32>,
    pub reputation: Option<CrimeReputation>,
    pub jury_point: Option<i32>,
    /// Bounded observation only; every duplicate is still forwarded through Event::Crime.
    pub last_receipt: Option<CrimeEvent>,
}
impl Crimes {
    fn hydrate(&mut self, state: &crate::model::CharacterState) {
        self.character_id = Some(state.character.id);
        self.reputation = Some(CrimeReputation {
            crime_points: state.character.crime_point,
            infamy_points: state.character.infamy_point,
            crime_state: state.character.crime_state,
        });
        self.jury_point = Some(state.jury_point);
        self.last_receipt = None;
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_with_notices(event).0
    }
    /// Use instead of apply for ordered Lua callbacks; never fold twice.
    pub fn apply_with_notices(&mut self, event: &Event) -> (bool, Vec<CrimeNotice>) {
        let before = self.clone();
        let mut notices = Vec::new();
        match event {
            Event::CharacterSelected(state) => self.hydrate(state),
            Event::PlayerSpawn(spawn) => {
                if self.character_id != Some(spawn.state.character.id) {
                    self.hydrate(&spawn.state)
                }
            }
            Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => *self = Self::default(),
            Event::Crime(receipt) => {
                self.last_receipt = Some(*receipt);
                match *receipt {
                    CrimeEvent::Changed {
                        crime_points,
                        infamy_points,
                        crime_state,
                        ..
                    } => {
                        if let Some(old) = self.reputation {
                            self.reputation = Some(CrimeReputation {
                                crime_points,
                                infamy_points,
                                crime_state,
                            });
                            notices.push(CrimeNotice::Reported {
                                crime_delta: crime_points.wrapping_sub(old.crime_points),
                                infamy_delta: infamy_points.wrapping_sub(old.infamy_points),
                                crime_state_delta: crime_state.wrapping_sub(old.crime_state),
                            });
                        }
                    }
                    CrimeEvent::JuryPointChanged { jury_point } => {
                        if self.character_id.is_some() {
                            self.jury_point = Some(jury_point);
                            notices.push(CrimeNotice::JuryPoint)
                        }
                    }
                    CrimeEvent::JuryWaitingNumber { waiting_number } => notices.push(CrimeNotice::JuryWaitingNumber(waiting_number)),
                }
            }
            _ => {}
        }
        (*self != before, notices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::ServerMsg, session::PlayerSpawn, wire::Writer};
    fn selected(id: u32) -> crate::model::CharacterState {
        let mut s = crate::model::CharacterState::default();
        s.character.id = id;
        s.character.crime_point = i16::MAX;
        s.character.infamy_point = i32::MIN;
        s.character.crime_state = i16::MIN;
        s.jury_point = 7;
        s
    }
    #[test]
    fn native_and_lane_jury_number_request_has_no_body_or_identity_bytes() {
        let request = crate::game::ClientMsg::RequestJuryWaitingNumber;
        assert_eq!(request.opcode(), 0x79);
        assert!(request.encode_body().is_empty());
    }
    #[test]
    fn native_lane_signed_wire_goldens_truncations_and_dd04_alignment() {
        let receipts = [
            (
                0x16f,
                vec![255, 255, 255, 127, 0, 128, 255, 255, 255, 127, 255, 127],
                CrimeEvent::Changed {
                    points: i32::MAX,
                    crime_points: i16::MIN,
                    infamy_points: i32::MAX,
                    crime_state: i16::MAX,
                },
            ),
            (0x18a, vec![0, 0, 0, 128], CrimeEvent::JuryPointChanged { jury_point: i32::MIN }),
            (0x182, vec![255, 255, 255, 255], CrimeEvent::JuryWaitingNumber { waiting_number: -1 }),
        ];
        let mut batch = Writer::new();
        for (op, body, expected) in &receipts {
            let mut r = Reader::new(body);
            assert_eq!(CrimeEvent::read(*op, &mut r).unwrap(), *expected);
            assert!(r.is_empty());
            for n in 0..body.len() {
                assert!(CrimeEvent::read(*op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
            }
            batch.u16(0).u16(*op).raw(body);
        }
        assert!(CrimeEvent::read(0, &mut Reader::new(&[])).is_err());
        // Same uncompressed content consumed by DD04, followed by a known fixed packet.
        batch.u16(0).u16(0x1bc).u32(17);
        use std::io::Write;
        let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        compressor.write_all(&batch.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(4).raw(&compressor.finish().unwrap());
        let ServerMsg::Batch(decoded) = ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(decoded.len(), 4);
        for (i, (_, _, expected)) in receipts.iter().enumerate() {
            assert!(matches!(&decoded[i],ServerMsg::Crime(actual) if actual==expected));
        }
        assert!(matches!(decoded[3], ServerMsg::SetBreath { breath: 17 }));
    }
    #[test]
    fn actual_identity_hydration_native_wrapping_notices_and_duplicate_delivery() {
        let mut s = Crimes::default();
        let c = selected(77);
        assert!(s.apply_with_notices(&Event::CharacterSelected(Box::new(c.clone()))).1.is_empty());
        // No-hook original391dd1cb..391dd25d oracle: MAX/MIN crossing => (1,-1,-1).
        let e = Event::Crime(CrimeEvent::Changed {
            points: 123,
            crime_points: i16::MIN,
            infamy_points: i32::MAX,
            crime_state: i16::MAX,
        });
        assert_eq!(
            s.apply_with_notices(&e).1,
            vec![CrimeNotice::Reported {
                crime_delta: 1,
                infamy_delta: -1,
                crime_state_delta: -1
            }]
        );
        assert_eq!(
            s.apply_with_notices(&e).1,
            vec![CrimeNotice::Reported {
                crime_delta: 0,
                infamy_delta: 0,
                crime_state_delta: 0
            }]
        );
        for _ in 0..2 {
            assert_eq!(
                s.apply_with_notices(&Event::Crime(CrimeEvent::JuryPointChanged { jury_point: -9 })).1,
                vec![CrimeNotice::JuryPoint]
            );
        }
        s.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: c,
            unit: crate::state::tests::unit(7),
        })));
        assert_eq!(
            s.jury_point,
            Some(-9),
            "matching spawn must not overwrite later header receipts with initial snapshot"
        );
        assert_eq!(
            s.apply_with_notices(&Event::Crime(CrimeEvent::JuryWaitingNumber { waiting_number: i32::MIN }))
                .1,
            vec![CrimeNotice::JuryWaitingNumber(i32::MIN)]
        );
        assert_eq!(s.last_receipt, Some(CrimeEvent::JuryWaitingNumber { waiting_number: i32::MIN }));
        s.apply(&Event::CharacterSelected(Box::new(selected(88))));
        assert_eq!(s.character_id, Some(88));
        assert_eq!(s.jury_point, Some(7));
        assert!(s.last_receipt.is_none());
    }
    #[test]
    fn lifecycle_unknown_header_and_world_player_snapshot_are_authoritative() {
        let mut s = Crimes::default();
        assert!(
            s.apply_with_notices(&Event::Crime(CrimeEvent::JuryPointChanged { jury_point: 99 }))
                .1
                .is_empty()
        );
        assert_eq!(s.jury_point, None);
        assert!(
            s.apply_with_notices(&Event::Crime(CrimeEvent::Changed {
                points: 1,
                crime_points: 2,
                infamy_points: 3,
                crime_state: 4
            }))
            .1
            .is_empty()
        );
        assert_eq!(s.reputation, None);
        let mut world = crate::state::WorldState::default();
        world.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn {
            obj_id: 7,
            state: selected(77),
            unit: crate::state::tests::unit(7),
        })));
        world.apply(&Event::Crime(CrimeEvent::JuryPointChanged { jury_point: -99 }));
        world.apply(&Event::Crime(CrimeEvent::Changed {
            points: 100,
            crime_points: 20,
            infamy_points: 30,
            crime_state: 40,
        }));
        let spawn = world.player.spawn.as_ref().unwrap();
        assert_eq!(spawn.state.jury_point, -99);
        assert_eq!(spawn.state.character.crime_state, 40);
        assert_eq!(
            world.crimes.reputation,
            Some(CrimeReputation {
                crime_points: 20,
                infamy_points: 30,
                crime_state: 40
            })
        );
        for e in [
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
            Event::Kicked {
                reason: 0,
                message: String::new(),
            },
            Event::PhaseChanged(Phase::Connecting),
        ] {
            s.apply(&Event::CharacterSelected(Box::new(selected(77))));
            s.apply(&e);
            assert_eq!(s, Crimes::default());
        }
    }
}
