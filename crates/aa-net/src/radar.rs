//! Received ship and public-transport telescope contacts; these never create world Units.
use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, WireError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadarToggle {
    pub on: bool,
    pub range: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferRadarContact {
    pub obj_id: u32,
    pub template_id: u32,
    pub pos: Pos,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShipRadarContact {
    pub obj_id: u32,
    pub summoner_id: u32,
    /// Lane writes Slave=2; unknown received values remain raw.
    pub raw_unit_type: i32,
    pub template_id: u32,
    pub pos: Pos,
    pub faction_id: u32,
    pub expedition_id: u32,
    pub name: String,
    pub summoner_name: String,
    pub hp: i32,
    pub max_hp: i32,
    /// Lane writes zero; no flag meaning is inferred.
    pub raw_tail: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RadarEvent {
    ShipToggled(RadarToggle),
    ShipContacts { last: bool, contacts: Vec<ShipRadarContact> },
    TransferToggled(RadarToggle),
    TransferContacts { last: bool, contacts: Vec<TransferRadarContact> },
}
impl RadarEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCTransferTelescopeToggledPacket | sc::SCTransferTelescopeUnitsPacket | sc::SCTelescopeToggledPacket | sc::SCTelescopeUnitsPacket
        )
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::ShipToggled(_) => "SCTelescopeToggled",
            Self::ShipContacts { .. } => "SCTelescopeUnits",
            Self::TransferToggled(_) => "SCTransferTelescopeToggled",
            Self::TransferContacts { .. } => "SCTransferTelescopeUnits",
        }
    }
    /// Exact bevy-lane count: the native reader caps each packet at ten entries.
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCTelescopeToggledPacket => Self::ShipToggled(RadarToggle {
                on: r.bool()?,
                range: r.f32()?,
            }),
            sc::SCTelescopeUnitsPacket => {
                let last = r.bool()?;
                let count = r.u8()?;
                let mut contacts = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    contacts.push(ShipRadarContact {
                        obj_id: r.bc()?,
                        summoner_id: r.u32()?,
                        raw_unit_type: r.i32()?,
                        template_id: r.u32()?,
                        pos: r.position()?,
                        faction_id: r.u32()?,
                        expedition_id: r.u32()?,
                        name: r.string()?,
                        summoner_name: r.string()?,
                        hp: r.i32()?,
                        max_hp: r.i32()?,
                        raw_tail: r.u32()?,
                    });
                }
                Self::ShipContacts { last, contacts }
            }
            sc::SCTransferTelescopeToggledPacket => Self::TransferToggled(RadarToggle {
                on: r.bool()?,
                range: r.f32()?,
            }),
            sc::SCTransferTelescopeUnitsPacket => {
                let last = r.bool()?;
                let count = r.u8()?;
                let mut contacts = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    contacts.push(TransferRadarContact {
                        obj_id: r.bc()?,
                        template_id: r.u32()?,
                        pos: r.position()?,
                        name: r.string()?,
                    });
                }
                Self::TransferContacts { last, contacts }
            }
            _ => return Err(WireError::Invalid(format!("unknown radar opcode {opcode:03x}"))),
        })
    }
}

/// Local memory bound for an unfinished multi-packet snapshot, not a server contact limit.
pub const MAX_RADAR_CONTACTS: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct TransferRadar {
    /// None until a genuine toggle arrives; range retains raw f32 bits.
    pub toggle: Option<RadarToggle>,
    /// None until a complete enabled snapshot arrives. Order and duplicate IDs remain raw.
    pub contacts: Option<Vec<TransferRadarContact>>,
    pending: Vec<TransferRadarContact>,
    discarding: bool,
}
impl TransferRadar {
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn discarding_snapshot(&self) -> bool {
        self.discarding
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                let changed = self.toggle.is_some() || self.contacts.is_some() || !self.pending.is_empty() || self.discarding;
                *self = Self::default();
                changed
            }
            Event::Radar(RadarEvent::TransferToggled(toggle)) => {
                self.toggle = Some(*toggle);
                self.contacts = None;
                self.pending.clear();
                self.discarding = false;
                true
            }
            Event::Radar(RadarEvent::TransferContacts { last, contacts }) => {
                if !self.toggle.is_some_and(|toggle| toggle.on) {
                    return false;
                }
                if self.discarding || self.pending.len().saturating_add(contacts.len()) > MAX_RADAR_CONTACTS {
                    self.pending.clear();
                    self.discarding = !last;
                    return true;
                }
                self.pending.extend_from_slice(contacts);
                if *last {
                    self.contacts = Some(std::mem::take(&mut self.pending));
                }
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct ShipRadar {
    /// None until a genuine toggle arrives; range retains raw f32 bits.
    pub toggle: Option<RadarToggle>,
    /// None until a complete enabled snapshot arrives. Order and duplicate IDs remain raw.
    pub contacts: Option<Vec<ShipRadarContact>>,
    pending: Vec<ShipRadarContact>,
    discarding: bool,
}
impl ShipRadar {
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn discarding_snapshot(&self) -> bool {
        self.discarding
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                let changed = self.toggle.is_some() || self.contacts.is_some() || !self.pending.is_empty() || self.discarding;
                *self = Self::default();
                changed
            }
            Event::Radar(RadarEvent::ShipToggled(toggle)) => {
                self.toggle = Some(*toggle);
                self.contacts = None;
                self.pending.clear();
                self.discarding = false;
                true
            }
            Event::Radar(RadarEvent::ShipContacts { last, contacts }) => {
                if !self.toggle.is_some_and(|toggle| toggle.on) {
                    return false;
                }
                if self.discarding || self.pending.len().saturating_add(contacts.len()) > MAX_RADAR_CONTACTS {
                    self.pending.clear();
                    self.discarding = !last;
                    return true;
                }
                self.pending.extend_from_slice(contacts);
                if *last {
                    self.contacts = Some(std::mem::take(&mut self.pending));
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::wire::Writer;
    fn contact(id: u32) -> TransferRadarContact {
        TransferRadarContact {
            obj_id: id,
            template_id: 77,
            pos: Pos::default(),
            name: "wagon".into(),
        }
    }
    fn check(opcode: u16, body: &[u8], expected: RadarEvent) {
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(body);
        assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(), ServerMsg::Radar(expected.clone()));
        for cut in 0..body.len() {
            assert!(RadarEvent::read(opcode, &mut Reader::new(&body[..cut])).is_err(), "cut {cut}");
        }
        let mut inner = Writer::new();
        inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
        assert_eq!(
            ServerMsg::decode(&frame.finish()).unwrap(),
            ServerMsg::Batch(vec![ServerMsg::Radar(expected), ServerMsg::Snow(true)])
        );
    }
    #[test]
    fn transfer_radar_lane_goldens_truncations_and_chunk_counts_preserve_alignment() {
        check(
            0x1b4,
            &[1, 0, 0, 0x7a, 0x44],
            RadarEvent::TransferToggled(RadarToggle { on: true, range: 1000.0 }),
        );
        let mut body = vec![1, 1, 3, 2, 1, 77, 0, 0, 0];
        body.extend([0; 9]);
        body.extend([5, 0]);
        body.extend(b"wagon");
        let mut expected = contact(0x010203);
        expected.pos = crate::wire::decode_position(&[0; 9]);
        check(
            0x1b5,
            &body,
            RadarEvent::TransferContacts {
                last: true,
                contacts: vec![expected.clone()],
            },
        );
        // The lane writer loops the full u8 count; native caps at ten. Never leave entry11 as an opcode.
        let entry = &body[2..];
        let mut eleven = vec![0, 11];
        for _ in 0..11 {
            eleven.extend(entry);
        }
        check(
            0x1b5,
            &eleven,
            RadarEvent::TransferContacts {
                last: false,
                contacts: vec![expected; 11],
            },
        );
        check(0x1b5, &[1, 0], RadarEvent::TransferContacts { last: true, contacts: vec![] });
    }
    #[test]
    fn transfer_radar_publishes_only_complete_enabled_snapshots_and_resets_on_every_toggle() {
        let mut state = crate::state::WorldState::default();
        let chunk = |last, ids: &[u32]| {
            Event::Radar(RadarEvent::TransferContacts {
                last,
                contacts: ids.iter().map(|id| contact(*id)).collect(),
            })
        };
        state.apply(&chunk(true, &[99]));
        assert!(state.transfer_radar.contacts.is_none());
        state.apply(&Event::Radar(RadarEvent::TransferToggled(RadarToggle { on: true, range: -2.0 })));
        state.apply(&chunk(false, &[1, 1]));
        assert!(state.transfer_radar.contacts.is_none());
        state.apply(&chunk(true, &[2]));
        assert_eq!(
            state.transfer_radar.contacts.as_ref().unwrap().iter().map(|c| c.obj_id).collect::<Vec<_>>(),
            vec![1, 1, 2]
        );
        state.apply(&chunk(false, &[3]));
        assert_eq!(state.transfer_radar.contacts.as_ref().unwrap().len(), 3);
        assert!(state.units.by_id.is_empty());
        assert!(state.slaves.own.is_none());
        state.apply(&Event::Radar(RadarEvent::TransferToggled(RadarToggle { on: true, range: 5.0 })));
        assert!(state.transfer_radar.contacts.is_none());
        assert_eq!(state.transfer_radar.pending_count(), 0);
        state.apply(&Event::Radar(RadarEvent::TransferToggled(RadarToggle {
            on: false,
            range: f32::INFINITY,
        })));
        state.apply(&chunk(true, &[4]));
        assert!(state.transfer_radar.contacts.is_none());
        assert_eq!(state.transfer_radar.toggle.unwrap().range.to_bits(), f32::INFINITY.to_bits());
        let mut selected = state.clone();
        selected.apply(&Event::CharacterSelected(Box::default()));
        assert_eq!(selected.transfer_radar, TransferRadar::default());
        state.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(state.transfer_radar, TransferRadar::default());
    }
    #[test]
    fn transfer_radar_overflow_discards_whole_incomplete_snapshot_then_recovers() {
        let mut radar = TransferRadar::default();
        radar.apply(&Event::Radar(RadarEvent::TransferToggled(RadarToggle { on: true, range: 10.0 })));
        let chunk = |last, n| {
            Event::Radar(RadarEvent::TransferContacts {
                last,
                contacts: vec![contact(1); n],
            })
        };
        radar.apply(&chunk(true, 1));
        radar.apply(&chunk(false, MAX_RADAR_CONTACTS));
        radar.apply(&chunk(false, 1));
        assert!(radar.discarding_snapshot());
        assert_eq!(radar.pending_count(), 0);
        radar.apply(&chunk(true, 1));
        assert!(!radar.discarding_snapshot());
        assert_eq!(radar.contacts.as_ref().unwrap().len(), 1);
        radar.apply(&chunk(true, 0));
        assert_eq!(radar.contacts, Some(vec![]));
        radar.apply(&Event::Disconnected { message: "test".into() });
        assert_eq!(radar, TransferRadar::default());
    }

    fn ship(id: u32) -> ShipRadarContact {
        ShipRadarContact {
            obj_id: id,
            summoner_id: 42,
            raw_unit_type: -7,
            template_id: 100,
            pos: crate::wire::decode_position(&[0; 9]),
            faction_id: 9,
            expedition_id: 10,
            name: "ship".into(),
            summoner_name: "owner".into(),
            hp: -1,
            max_hp: 250,
            raw_tail: 0xaabbccdd,
        }
    }
    #[test]
    fn ship_radar_wire_preserves_all_owner_hp_and_unknown_fields_without_unit_creation() {
        check(0x1b2, &[0, 0, 0, 0, 0x80], RadarEvent::ShipToggled(RadarToggle { on: false, range: -0.0 }));
        // Independent native/lane sequence, including non-Slave kind and nonzero tail.
        let mut body = vec![1, 1, 3, 2, 1, 42, 0, 0, 0, 0xf9, 0xff, 0xff, 0xff, 100, 0, 0, 0];
        body.extend([0; 9]);
        body.extend([9, 0, 0, 0, 10, 0, 0, 0, 4, 0]);
        body.extend(b"ship");
        body.extend([5, 0]);
        body.extend(b"owner");
        body.extend([0xff, 0xff, 0xff, 0xff, 250, 0, 0, 0, 0xdd, 0xcc, 0xbb, 0xaa]);
        check(
            0x1b3,
            &body,
            RadarEvent::ShipContacts {
                last: true,
                contacts: vec![ship(0x010203)],
            },
        );
        let mut eleven = vec![0, 11];
        for _ in 0..11 {
            eleven.extend(&body[2..]);
        }
        check(
            0x1b3,
            &eleven,
            RadarEvent::ShipContacts {
                last: false,
                contacts: vec![ship(0x010203); 11],
            },
        );
    }
    #[test]
    fn ship_radar_chunk_lifecycle_is_independent_from_transport_and_ownership() {
        let mut state = crate::state::WorldState::default();
        let chunk = |last, ids: &[u32]| {
            Event::Radar(RadarEvent::ShipContacts {
                last,
                contacts: ids.iter().map(|id| ship(*id)).collect(),
            })
        };
        state.apply(&chunk(true, &[1]));
        assert!(state.ship_radar.contacts.is_none());
        state.apply(&Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: true, range: 500.0 })));
        state.apply(&chunk(false, &[1, 1]));
        assert!(state.ship_radar.contacts.is_none());
        state.apply(&chunk(true, &[2]));
        assert_eq!(state.ship_radar.contacts.as_ref().unwrap().len(), 3);
        state.apply(&Event::Radar(RadarEvent::TransferToggled(RadarToggle { on: false, range: 0.0 })));
        assert_eq!(state.ship_radar.contacts.as_ref().unwrap().len(), 3);
        assert!(state.units.by_id.is_empty());
        assert!(state.slaves.own.is_none());
        state.apply(&Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: false, range: 55.0 })));
        state.apply(&chunk(true, &[3]));
        assert!(state.ship_radar.contacts.is_none());
        state.apply(&Event::CharacterSelected(Box::default()));
        assert_eq!(state.ship_radar, ShipRadar::default());
        state.apply(&Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: true, range: 10.0 })));
        state.apply(&chunk(true, &[]));
        assert_eq!(state.ship_radar.contacts, Some(vec![]));
        state.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(state.ship_radar, ShipRadar::default());
    }
    #[test]
    fn ship_radar_overflow_retains_complete_snapshot_and_cannot_publish_partial_tail() {
        let mut radar = ShipRadar::default();
        radar.apply(&Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: true, range: 10.0 })));
        let chunk = |last, n| {
            Event::Radar(RadarEvent::ShipContacts {
                last,
                contacts: vec![ship(1); n],
            })
        };
        radar.apply(&chunk(true, 1));
        radar.apply(&chunk(false, MAX_RADAR_CONTACTS));
        radar.apply(&chunk(false, 1));
        assert!(radar.discarding_snapshot());
        radar.apply(&chunk(true, 2));
        assert!(!radar.discarding_snapshot());
        assert_eq!(radar.contacts.as_ref().unwrap().len(), 1);
        radar.apply(&chunk(true, 0));
        assert_eq!(radar.contacts, Some(vec![]));
        radar.apply(&Event::Disconnected { message: "test".into() });
        assert_eq!(radar, ShipRadar::default());
    }
}
