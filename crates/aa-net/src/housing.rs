//! Received housing metadata. The active decoder follows the bevy lane writer.
//! Native NetHouse differs at its model field; see `HouseSnapshot::read_native`.

use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, WireError, long_to_coord};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct HouseSnapshot {
    pub tl_id: u16,
    pub db_id: u32,
    pub obj_id: u32,
    pub template_id: u32,
    pub model_id: u32,
    /// Lane PISC's second value; absent in the original native i32 model layout.
    pub lane_model_aux: Option<u32>,
    pub co_owner_id: u32,
    pub owner_id: u32,
    pub owner_name: String,
    pub account_id: u32,
    /// Raw permission enum, without assuming unknown values are public/private.
    pub permission: u8,
    pub all_actions: i32,
    pub current_actions: i32,
    pub tax: i32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub name: String,
    pub allow_recover: bool,
    /// Native signed money field, preserving the lane u32's exact bits.
    pub sell_price: i32,
    pub sell_to_player_id: u32,
    pub sell_to_name: String,
}

impl HouseSnapshot {
    /// Active lane `models/house.rs::House::write`: PISC(model_id, 0).
    pub fn read_lane(r: &mut Reader) -> Result<Self> {
        Self::read(r, true)
    }

    /// Original serializer39897460: a single i32 model field, not PISC.
    /// Explicitly separate from live dispatch; no payload guessing.
    pub fn read_native(r: &mut Reader) -> Result<Self> {
        Self::read(r, false)
    }

    fn read(r: &mut Reader, lane: bool) -> Result<Self> {
        let tl_id = r.u16()?;
        let db_id = r.u32()?;
        let obj_id = r.bc()?;
        let template_id = r.u32()?;
        let (model_id, lane_model_aux) = if lane {
            let values = r.pisc(2)?;
            (values[0] as u32, Some(values[1] as u32))
        } else {
            (r.u32()?, None)
        };
        Ok(Self {
            tl_id,
            db_id,
            obj_id,
            template_id,
            model_id,
            lane_model_aux,
            co_owner_id: r.u32()?,
            owner_id: r.u32()?,
            owner_name: house_string(r, lane)?,
            account_id: r.u32()?,
            permission: r.u8()?,
            all_actions: r.i32()?,
            current_actions: r.i32()?,
            tax: r.i32()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            name: house_string(r, lane)?,
            allow_recover: r.bool()?,
            sell_price: r.i32()?,
            sell_to_player_id: r.u32()?,
            sell_to_name: house_string(r, lane)?,
        })
    }

    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

fn house_string(r: &mut Reader, lane: bool) -> Result<String> {
    if lane {
        return r.string();
    }
    let len = r.i16()?;
    if !(0..=128).contains(&len) {
        return Err(WireError::Invalid(format!("native house string length {len}")));
    }
    Ok(String::from_utf8_lossy(r.bytes(len as usize)?).trim_matches('\0').to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub enum HousingEvent {
    State(HouseSnapshot),
    BuildProgress {
        tl_id: u16,
        model_id: u32,
        all_actions: i32,
        current_actions: i32,
    },
    PermissionChanged {
        tl_id: u16,
        permission: u8,
    },
    Demolished {
        tl_id: u16,
    },
}

impl HousingEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCHouseStatePacket | sc::SCHouseBuildProgressPacket | sc::SCHousePermissionChangedPacket | sc::SCHouseDemolishedPacket
        )
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::State(_) => "SCHouseState",
            Self::BuildProgress { .. } => "SCHouseBuildProgress",
            Self::PermissionChanged { .. } => "SCHousePermissionChanged",
            Self::Demolished { .. } => "SCHouseDemolished",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCHouseStatePacket => Self::State(HouseSnapshot::read_lane(r)?),
            sc::SCHouseBuildProgressPacket => Self::BuildProgress {
                tl_id: r.u16()?,
                model_id: r.u32()?,
                all_actions: r.i32()?,
                current_actions: r.i32()?,
            },
            sc::SCHousePermissionChangedPacket => Self::PermissionChanged {
                tl_id: r.u16()?,
                permission: r.u8()?,
            },
            sc::SCHouseDemolishedPacket => Self::Demolished { tl_id: r.u16()? },
            _ => return Err(WireError::Invalid(format!("housing opcode {opcode:#x}"))),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct House {
    pub snapshot: HouseSnapshot,
    /// A received demolition notice, not a locally guessed disappearance.
    pub demolished: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Houses {
    pub by_tl_id: HashMap<u16, House>,
}

impl Houses {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Housing(HousingEvent::State(snapshot)) => {
                let house = House {
                    snapshot: snapshot.clone(),
                    demolished: false,
                };
                if self.by_tl_id.get(&snapshot.tl_id) == Some(&house) {
                    return false;
                }
                // A reused runtime object ID must not keep a stale timeline entry.
                self.by_tl_id.retain(|tl, h| *tl == snapshot.tl_id || h.snapshot.obj_id != snapshot.obj_id);
                self.by_tl_id.insert(snapshot.tl_id, house);
                true
            }
            Event::Housing(HousingEvent::BuildProgress {
                tl_id,
                model_id,
                all_actions,
                current_actions,
            }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                let state = &mut house.snapshot;
                if (state.model_id, state.all_actions, state.current_actions) == (*model_id, *all_actions, *current_actions) {
                    return false;
                }
                state.model_id = *model_id;
                state.all_actions = *all_actions;
                state.current_actions = *current_actions;
                true
            }
            Event::Housing(HousingEvent::PermissionChanged { tl_id, permission }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.snapshot.permission == *permission {
                    return false;
                }
                house.snapshot.permission = *permission;
                true
            }
            Event::Housing(HousingEvent::Demolished { tl_id }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.demolished {
                    return false;
                }
                house.demolished = true;
                true
            }
            Event::UnitsRemoved(ids) => {
                let before = self.by_tl_id.len();
                self.by_tl_id.retain(|_, h| !ids.contains(&h.snapshot.obj_id));
                self.by_tl_id.len() != before
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_tl_id.is_empty() {
                    return false;
                }
                self.by_tl_id.clear();
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::wire::Writer;

    fn hex(raw: &str) -> Vec<u8> {
        raw.split_whitespace()
            .flat_map(|word| (0..word.len()).step_by(2).map(move |i| u8::from_str_radix(&word[i..i + 2], 16).unwrap()))
            .collect()
    }

    // Independent lane House::house_state_layout fixture (not an encoder round trip).
    fn lane_body() -> Vec<u8> {
        hex(
            "0700 0d0c0b0a 030201 2c010000 001000 06000000 05000000 0300 4f776e 09000000 02 28000000 07000000 fa000000 0000000000100000 0000000000200000 00006040 0300 487574 01 e8030000 08000000 0000",
        )
    }

    pub(crate) fn snapshot() -> HouseSnapshot {
        HouseSnapshot::read_lane(&mut Reader::new(&lane_body())).unwrap()
    }

    fn with_tail(opcode: u16, body: &[u8], expected: HousingEvent) {
        let mut plain = Writer::new();
        plain.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(body);
        let expected = ServerMsg::Housing(expected);
        assert_eq!(ServerMsg::decode(&plain.finish()).unwrap(), expected);
        let mut inner = Writer::new();
        inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut compressed = Writer::new();
        compressed.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
        assert_eq!(
            ServerMsg::decode(&compressed.finish()).unwrap(),
            ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
        );
    }

    #[test]
    fn housing_lane_and_native_layouts_are_explicit_and_consume_exact_fields() {
        let body = lane_body();
        let house = snapshot();
        assert_eq!(
            (house.tl_id, house.db_id, house.obj_id, house.template_id, house.model_id, house.lane_model_aux),
            (7, 0x0a0b0c0d, 0x010203, 300, 16, Some(0))
        );
        assert_eq!((house.co_owner_id, house.owner_id, house.account_id, house.permission), (6, 5, 9, 2));
        assert_eq!((&*house.owner_name, &*house.name, &*house.sell_to_name), ("Own", "Hut", ""));
        assert_eq!((house.all_actions, house.current_actions, house.tax), (40, 7, 250));
        assert_eq!(house.position(), Pos::new(1.0, 2.0, 3.5));
        assert!(house.allow_recover);
        assert_eq!((house.sell_price, house.sell_to_player_id), (1000, 8));
        with_tail(sc::SCHouseStatePacket, &body, HousingEvent::State(house.clone()));
        let mut native = body.clone();
        native.splice(13..16, [16, 0, 0, 0]);
        let mut reader = Reader::new(&native);
        let mut expected = house.clone();
        expected.lane_model_aux = None;
        assert_eq!(HouseSnapshot::read_native(&mut reader).unwrap(), expected);
        assert!(reader.is_empty());
        for cut in 0..body.len() {
            assert!(HouseSnapshot::read_lane(&mut Reader::new(&body[..cut])).is_err(), "lane cut {cut}");
        }
        for cut in 0..native.len() {
            assert!(HouseSnapshot::read_native(&mut Reader::new(&native[..cut])).is_err(), "native cut {cut}");
        }
        // Native fixed-buffer string limits are bytes; lane keeps its own string contract.
        for len in [128usize, 129] {
            let mut raw = native.clone();
            raw.splice(25..30, (len as i16).to_le_bytes().into_iter().chain(std::iter::repeat_n(b'x', len)));
            assert_eq!(HouseSnapshot::read_native(&mut Reader::new(&raw)).is_ok(), len == 128);
        }
    }

    #[test]
    fn housing_delta_goldens_unknown_enums_and_batch_boundaries() {
        let cases = [
            (
                sc::SCHouseBuildProgressPacket,
                hex("0700 ffffffff 00000080 ffffff7f"),
                HousingEvent::BuildProgress {
                    tl_id: 7,
                    model_id: u32::MAX,
                    all_actions: i32::MIN,
                    current_actions: i32::MAX,
                },
            ),
            (
                sc::SCHousePermissionChangedPacket,
                hex("0700 fe"),
                HousingEvent::PermissionChanged { tl_id: 7, permission: 254 },
            ),
            (sc::SCHouseDemolishedPacket, hex("0700"), HousingEvent::Demolished { tl_id: 7 }),
        ];
        for (opcode, raw, event) in cases {
            with_tail(opcode, &raw, event);
            for cut in 0..raw.len() {
                assert!(HousingEvent::read(opcode, &mut Reader::new(&raw[..cut])).is_err());
            }
        }
        assert!(HousingEvent::read(0xffff, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn housing_received_state_deltas_removal_replacement_and_lifecycle() {
        let mut world = crate::state::WorldState::default();
        let mut house = snapshot();
        let state = Event::Housing(HousingEvent::State(house.clone()));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::Demolished { tl_id: 7 })));
        world.apply(&state);
        assert!(world.units.by_id.is_empty());
        assert!(!world.houses.apply(&state));
        let delta = Event::Housing(HousingEvent::BuildProgress {
            tl_id: 7,
            model_id: 55,
            all_actions: 80,
            current_actions: 60,
        });
        assert!(world.houses.apply(&delta));
        assert!(!world.houses.apply(&delta));
        assert_eq!(world.houses.by_tl_id[&7].snapshot.current_actions, 60);
        assert!(
            world
                .houses
                .apply(&Event::Housing(HousingEvent::PermissionChanged { tl_id: 7, permission: 255 }))
        );
        assert_eq!(world.houses.by_tl_id[&7].snapshot.permission, 255);
        let demolished = Event::Housing(HousingEvent::Demolished { tl_id: 7 });
        assert!(world.houses.apply(&demolished));
        assert!(!world.houses.apply(&demolished));
        assert!(world.houses.by_tl_id[&7].demolished, "demolition notice does not invent unit removal");
        assert!(!world.houses.apply(&Event::UnitsRemoved(vec![999])));
        assert!(world.houses.apply(&Event::UnitsRemoved(vec![house.obj_id])));
        assert!(!world.houses.apply(&delta));
        world.apply(&state);
        house.tl_id = 8;
        world.apply(&Event::Housing(HousingEvent::State(house)));
        assert_eq!(world.houses.by_tl_id.len(), 1);
        assert!(!world.houses.by_tl_id.contains_key(&7));
        for reset in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
        ] {
            world.apply(&state);
            assert!(world.houses.apply(&reset));
            assert!(!world.houses.apply(&reset));
        }
    }
}
