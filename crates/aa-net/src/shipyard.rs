//! Received shipyard state (`SCShipyardStatePacket` 0x1aa).
//!
//! The body is `ShipyardData` followed by the current build `step` (`i32`), exactly as the AAEmu
//! server writes it (`packets/world_objects.rs::SCShipyardStatePacket::write`,
//! `models/shipyard.rs::ShipyardData::write`). The 1.2 client reads the same fields in the same
//! order (`FUN_397afbf0` -> `FUN_3961ef40`, then the `i32` at `+0xe8`; the registration
//! `FUN_391eb9d0` stores the handler at `this + 4 + 4 * 0x1aa`).
//!
//! `step` 1000 is the launch ceremony; `0 <= step < steps.len()` is a construction stage; the
//! server also sends `steps.len()` once the last stage is done (`sync_build_state`). The unit's
//! model and the stage models come from the client's `shipyards` / `shipyard_steps` rows.

use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, long_to_coord};
use std::collections::HashMap;

/// `ShipyardData` plus the `step` the packet appends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShipyardState {
    /// `ShipyardData.Id` (the `shipyards` runtime row, not the object id).
    pub id: u64,
    pub template_id: u32,
    /// `convert_long` of the world X.
    pub x: i64,
    /// `convert_long` of the world Y.
    pub y: i64,
    pub z: f32,
    pub z_rot: f32,
    pub money_amount: i32,
    /// Build actions done in the whole construction.
    pub actions: i32,
    pub kind: u32,
    /// The owner's character id.
    pub kind2: u32,
    pub owner_name: String,
    /// `Type3`: the owner faction.
    pub kind3: u32,
    /// Unix seconds.
    pub spawned: i64,
    pub obj_id: u32,
    pub hp: i32,
    /// 1000 is the launch ceremony; `-1` never reaches the wire (the server sends `steps.len()`).
    pub step: i32,
}

impl ShipyardState {
    /// `ShipyardData.Write` + `i32 step`.
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u64()?,
            template_id: r.u32()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            z_rot: r.f32()?,
            money_amount: r.i32()?,
            actions: r.i32()?,
            kind: r.u32()?,
            kind2: r.u32()?,
            owner_name: r.string()?,
            kind3: r.u32()?,
            spawned: r.i64()?,
            obj_id: r.bc()?,
            hp: r.i32()?,
            step: r.i32()?,
        })
    }

    /// The shipyard's world position (the `x` / `y` fields are `convert_long`).
    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

/// A received `SCShipyardState`.
#[derive(Clone, Debug, PartialEq)]
pub enum ShipyardEvent {
    /// One full shipyard record (the packet has no partial form).
    State(ShipyardState),
}

impl ShipyardEvent {
    pub fn handles(opcode: u16) -> bool {
        opcode == sc::SCShipyardStatePacket
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::State(_) => "SCShipyardState",
        }
    }

    pub fn read(_opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(Self::State(ShipyardState::read(r)?))
    }
}

/// The shipyards the server announced, keyed by `ShipyardData.Id`.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Shipyards {
    pub by_id: HashMap<u64, ShipyardState>,
}

impl Shipyards {
    pub fn of(&self, id: u64) -> Option<&ShipyardState> {
        self.by_id.get(&id)
    }

    /// The record of the unit with this object id, when one was received.
    pub fn by_obj_id(&self, obj_id: u32) -> Option<&ShipyardState> {
        self.by_id.values().find(|s| s.obj_id == obj_id)
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Shipyard(ShipyardEvent::State(state)) => {
                if self.by_id.get(&state.id) == Some(state) {
                    return false;
                }
                // A reused runtime object id must not keep a stale timeline entry.
                self.by_id
                    .retain(|id, s| *id == state.id || s.obj_id != state.obj_id);
                self.by_id.insert(state.id, state.clone());
                true
            }
            Event::UnitsRemoved(ids) => {
                let before = self.by_id.len();
                self.by_id.retain(|_, s| !ids.contains(&s.obj_id));
                self.by_id.len() != before
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.by_id.is_empty();
                self.by_id.clear();
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::game::ServerMsg;

    /// `SCShipyardStatePacket` bytes from the server's `shipyard_state_layout` test
    /// (`packets/world_objects.rs`: `encode(..) == frame(0x1aa, &b)`).
    pub(crate) fn native_body() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0x0102030405060708u64.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&(4096i64 << 32).to_le_bytes());
        b.extend_from_slice(&(8192i64 << 32).to_le_bytes());
        b.extend_from_slice(&3.5f32.to_le_bytes());
        b.extend_from_slice(&0.25f32.to_le_bytes());
        b.extend_from_slice(&(-1i32).to_le_bytes());
        b.extend_from_slice(&4i32.to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(&6u32.to_le_bytes());
        b.extend_from_slice(&[3, 0, b'Z', b'e', b'd']);
        b.extend_from_slice(&101u32.to_le_bytes());
        b.extend_from_slice(&1_700_000_000i64.to_le_bytes());
        b.extend_from_slice(&[0x0C, 0x0B, 0x0A]);
        b.extend_from_slice(&1000i32.to_le_bytes());
        b.extend_from_slice(&2i32.to_le_bytes());
        b
    }

    pub(crate) fn snapshot() -> ShipyardState {
        ShipyardState::read(&mut Reader::new(&native_body())).unwrap()
    }

    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut f = vec![crate::game::MAGIC, crate::game::LEVEL_GAME, 0, 1];
        f.extend_from_slice(&opcode.to_le_bytes());
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn shipyard_state_matches_the_server_encoder_bytes() {
        let body = native_body();
        assert_eq!(body.len(), 0x50);
        let expected = ShipyardState {
            id: 0x0102030405060708,
            template_id: 3,
            x: 4096i64 << 32,
            y: 8192i64 << 32,
            z: 3.5,
            z_rot: 0.25,
            money_amount: -1,
            actions: 4,
            kind: 5,
            kind2: 6,
            owner_name: "Zed".into(),
            kind3: 101,
            spawned: 1_700_000_000,
            obj_id: 0x0A0B0C,
            hp: 1000,
            step: 2,
        };
        assert_eq!(
            ServerMsg::decode(&frame(sc::SCShipyardStatePacket, &body)).unwrap(),
            ServerMsg::Shipyard(ShipyardEvent::State(expected.clone()))
        );
        let mut r = Reader::new(&body);
        assert_eq!(ShipyardState::read(&mut r).unwrap(), expected);
        assert!(r.is_empty());
        assert_eq!(expected.position(), Pos::new(1.0, 2.0, 3.5));
    }

    #[test]
    fn every_truncation_of_the_shipyard_body_fails() {
        let body = native_body();
        for cut in 0..body.len() {
            assert!(
                ServerMsg::decode(&frame(sc::SCShipyardStatePacket, &body[..cut])).is_err(),
                "cut {cut}"
            );
        }
    }

    fn batch(packets: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut raw = crate::wire::Writer::new();
        for (opcode, body) in packets {
            raw.u16(0).u16(*opcode).raw(body);
        }
        let raw = raw.finish();
        let size = u16::try_from(raw.len()).unwrap();
        let mut out = vec![crate::game::MAGIC, crate::game::LEVEL_COMPRESSED];
        out.extend((packets.len() as u16).to_le_bytes());
        out.push(1); // one final stored block
        out.extend(size.to_le_bytes());
        out.extend((!size).to_le_bytes());
        out.extend(raw);
        out
    }

    #[test]
    fn a_shipyard_state_beside_another_packet_keeps_both_boundaries() {
        let frame = batch(&[
            (sc::SCShipyardStatePacket, native_body()),
            (sc::SCOnOffSnowPacket, vec![1]),
        ]);
        assert_eq!(
            ServerMsg::decode(&frame).unwrap(),
            ServerMsg::Batch(vec![
                ServerMsg::Shipyard(ShipyardEvent::State(snapshot())),
                ServerMsg::Snow(true),
            ])
        );
    }

    #[test]
    fn shipyards_upsert_and_forget_by_object_id() {
        let mut yards = Shipyards::default();
        let state = snapshot();
        assert!(yards.apply(&Event::Shipyard(ShipyardEvent::State(state.clone()))));
        assert_eq!(yards.of(state.id), Some(&state));
        assert_eq!(yards.by_obj_id(state.obj_id), Some(&state));
        assert!(!yards.apply(&Event::Shipyard(ShipyardEvent::State(state.clone()))));

        let mut moved = state.clone();
        moved.actions = 9;
        assert!(yards.apply(&Event::Shipyard(ShipyardEvent::State(moved.clone()))));
        assert_eq!(yards.of(state.id), Some(&moved));

        // A reused object id replaces the old runtime id.
        let mut reused = moved.clone();
        reused.id = 7;
        assert!(yards.apply(&Event::Shipyard(ShipyardEvent::State(reused.clone()))));
        assert_eq!(yards.of(state.id), None);
        assert_eq!(yards.of(7), Some(&reused));

        assert!(yards.apply(&Event::UnitsRemoved(vec![state.obj_id])));
        assert!(yards.by_id.is_empty());
        assert!(!yards.apply(&Event::UnitsRemoved(vec![state.obj_id])));
    }

    #[test]
    fn shipyards_clear_on_a_new_world_or_a_lost_connection() {
        for end in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected {
                message: String::new(),
            },
        ] {
            let mut yards = Shipyards::default();
            yards.apply(&Event::Shipyard(ShipyardEvent::State(snapshot())));
            assert!(yards.apply(&end), "{end:?}");
            assert!(yards.by_id.is_empty(), "{end:?}");
        }
    }
}
