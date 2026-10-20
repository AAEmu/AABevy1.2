//! Public farms and doodad planting: received `SCResponseCommonFarmList`,
//! `SCShowCommonFarm`, `SCIsUnitInFarm`, and the `CSCreateDoodad` /
//! `CSRequestCommonFarmList` requests.
//!
//! Layouts follow the AAEmu server (`systems/npcs/public_farm.rs`,
//! `systems/npcs/doodad/packets.rs`): farm entries carry the 9-byte packed world
//! position and a `i64` plant time; `SCShowCommonFarm` carries one `i64`/`i64`/`f32`
//! position.

use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{coord_to_long, long_to_coord, Pos, Reader, Result, WireError, Writer};

/// One planted doodad of `SCResponseCommonFarmListPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommonFarmEntry {
    pub farm_type: u32,
    pub template_id: u32,
    pub time_left: u32,
    pub func_group_id: u32,
    pub pos: Pos,
    /// `PlantTime` (unix seconds).
    pub plant_time: i64,
}

/// `SCResponseCommonFarmListPacket`: the leading count (the response bound) and the
/// caller's planted crops. The server writes the same value twice.
#[derive(Clone, Debug, PartialEq)]
pub struct CommonFarmList {
    pub max_count: i32,
    pub entries: Vec<CommonFarmEntry>,
}

impl CommonFarmList {
    pub const MAX_ENTRIES: i32 = 4096;

    pub fn read(r: &mut Reader) -> Result<Self> {
        let max_count = r.i32()?;
        let count = r.i32()?;
        if !(0..=Self::MAX_ENTRIES).contains(&count) || max_count < 0 {
            return Err(WireError::Invalid(format!(
                "common farm list count {count} (max {max_count})"
            )));
        }
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            entries.push(CommonFarmEntry {
                farm_type: r.u32()?,
                template_id: r.u32()?,
                time_left: r.u32()?,
                func_group_id: r.u32()?,
                pos: r.position()?,
                plant_time: r.i64()?,
            });
        }
        Ok(Self { max_count, entries })
    }
}

/// `SCShowCommonFarmPacket(farmType, count, x, y, z)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShowCommonFarm {
    pub farm_type: i32,
    pub count: i32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
}

impl ShowCommonFarm {
    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FarmEvent {
    List(CommonFarmList),
    Show(ShowCommonFarm),
    /// `SCIsUnitInFarmPacket(type)`: the unit's raw farm-group type.
    IsUnitInFarm(i32),
}

impl FarmEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCResponseCommonFarmListPacket | sc::SCShowCommonFarmPacket | sc::SCIsUnitInFarmPacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::List(_) => "SCResponseCommonFarmList",
            Self::Show(_) => "SCShowCommonFarm",
            Self::IsUnitInFarm(_) => "SCIsUnitInFarm",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCResponseCommonFarmListPacket => Self::List(CommonFarmList::read(r)?),
            sc::SCShowCommonFarmPacket => Self::Show(ShowCommonFarm {
                farm_type: r.i32()?,
                count: r.i32()?,
                x: r.i64()?,
                y: r.i64()?,
                z: r.f32()?,
            }),
            sc::SCIsUnitInFarmPacket => Self::IsUnitInFarm(r.i32()?),
            _ => return Err(WireError::Invalid(format!("farm opcode {opcode:#x}"))),
        })
    }
}

/// `CSCreateDoodadPacket`: place doodad template `id` with the planting item `item_id`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CreateDoodad {
    pub id: u32,
    pub pos: Pos,
    pub z_rot: f32,
    pub scale: f32,
    pub item_id: u64,
}

impl CreateDoodad {
    pub fn write(&self, w: &mut Writer) {
        w.u32(self.id)
            .i64(coord_to_long(self.pos.x))
            .i64(coord_to_long(self.pos.y))
            .f32(self.pos.z)
            .f32(self.z_rot)
            .f32(self.scale)
            .u64(self.item_id);
    }
}

/// Received farm state plus the last doodad-planting request receipt.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Farms {
    pub list: Option<CommonFarmList>,
    pub show: Option<ShowCommonFarm>,
    pub unit_farm_type: Option<i32>,
}

fn replace<T: PartialEq>(slot: &mut Option<T>, value: T) -> bool {
    if slot.as_ref() == Some(&value) {
        return false;
    }
    *slot = Some(value);
    true
}

impl Farms {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Farm(FarmEvent::List(list)) => replace(&mut self.list, list.clone()),
            Event::Farm(FarmEvent::Show(show)) => replace(&mut self.show, *show),
            Event::Farm(FarmEvent::IsUnitInFarm(kind)) => replace(&mut self.unit_farm_type, *kind),
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = self.list.is_some() || self.show.is_some() || self.unit_farm_type.is_some();
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
    use crate::game::ServerMsg;
    use crate::opcodes::cs;

    fn with_tail(opcode: u16, body: &[u8], expected: FarmEvent) {
        let mut plain = Writer::new();
        plain.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(body);
        assert_eq!(ServerMsg::decode(&plain.finish()).unwrap(), ServerMsg::Farm(expected.clone()));

        let mut inner = Writer::new();
        inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut compressed = Writer::new();
        compressed.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
        assert_eq!(
            ServerMsg::decode(&compressed.finish()).unwrap(),
            ServerMsg::Batch(vec![ServerMsg::Farm(expected), ServerMsg::Snow(true)])
        );
    }

    fn list_body() -> Vec<u8> {
        let mut w = Writer::new();
        w.i32(1)
            .i32(1)
            .u32(1)
            .u32(380)
            .u32(5000)
            .u32(77)
            .position(Pos::new(100.0, 200.0, -100.0))
            .i64(1_700_000_000);
        w.finish()
    }

    #[test]
    fn common_farm_list_decodes_exactly_and_empty_is_not_an_error() {
        let body = list_body();
        let expected = CommonFarmList {
            max_count: 1,
            entries: vec![CommonFarmEntry {
                farm_type: 1,
                template_id: 380,
                time_left: 5000,
                func_group_id: 77,
                pos: Pos::new(100.0, 200.0, -100.0),
                plant_time: 1_700_000_000,
            }],
        };
        let mut reader = Reader::new(&body);
        let decoded = CommonFarmList::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        assert_eq!(decoded.max_count, expected.max_count);
        assert_eq!(decoded.entries.len(), 1);
        let entry = &decoded.entries[0];
        let want = &expected.entries[0];
        assert_eq!(
            (entry.farm_type, entry.template_id, entry.time_left, entry.func_group_id, entry.plant_time),
            (want.farm_type, want.template_id, want.time_left, want.func_group_id, want.plant_time)
        );
        assert_eq!((entry.pos.x, entry.pos.y), (want.pos.x, want.pos.y));
        assert!((entry.pos.z - want.pos.z).abs() < 0.01);
        for cut in 0..body.len() {
            assert!(CommonFarmList::read(&mut Reader::new(&body[..cut])).is_err(), "cut {cut}");
        }
        with_tail(sc::SCResponseCommonFarmListPacket, &body, FarmEvent::List(expected));

        let mut empty_writer = Writer::new();
        empty_writer.i32(0).i32(0);
        let empty = empty_writer.finish();
        assert_eq!(
            CommonFarmList::read(&mut Reader::new(&empty)).unwrap(),
            CommonFarmList { max_count: 0, entries: vec![] }
        );
        assert!(CommonFarmList::read(&mut Reader::new(&[(-1i32).to_le_bytes(), 0i32.to_le_bytes()].concat())).is_err());
        assert!(CommonFarmList::read(&mut Reader::new(&[0i32.to_le_bytes(), (-1i32).to_le_bytes()].concat())).is_err());
        assert!(CommonFarmList::read(&mut Reader::new(&[0i32.to_le_bytes(), (CommonFarmList::MAX_ENTRIES + 1).to_le_bytes()].concat())).is_err());
    }

    #[test]
    fn show_common_farm_and_is_unit_in_farm_preserve_the_server_fields() {
        let mut w = Writer::new();
        w.i32(3)
            .i32(2)
            .i64(coord_to_long(1024.5))
            .i64(coord_to_long(2048.0))
            .f32(12.5);
        let body = w.finish();
        let mut reader = Reader::new(&body);
        let show = FarmEvent::read(sc::SCShowCommonFarmPacket, &mut reader).unwrap();
        assert!(reader.is_empty());
        assert_eq!(
            show,
            FarmEvent::Show(ShowCommonFarm {
                farm_type: 3,
                count: 2,
                x: coord_to_long(1024.5),
                y: coord_to_long(2048.0),
                z: 12.5,
            })
        );
        let FarmEvent::Show(show_fields) = show.clone() else { panic!() };
        assert_eq!(show_fields.position(), Pos::new(1024.5, 2048.0, 12.5));
        with_tail(sc::SCShowCommonFarmPacket, &body, show);
        for cut in 0..body.len() {
            assert!(FarmEvent::read(sc::SCShowCommonFarmPacket, &mut Reader::new(&body[..cut])).is_err());
        }

        let body = 4i32.to_le_bytes();
        with_tail(sc::SCIsUnitInFarmPacket, &body, FarmEvent::IsUnitInFarm(4));
        assert!(FarmEvent::read(sc::SCIsUnitInFarmPacket, &mut Reader::new(&[])).is_err());
        assert!(FarmEvent::read(0xffff, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn create_doodad_body_matches_the_server_reader() {
        let request = CreateDoodad {
            id: 2336,
            pos: Pos::new(1000.5, 2000.25, 100.0),
            z_rot: 1.5,
            scale: 1.0,
            item_id: 77,
        };
        let mut w = Writer::new();
        request.write(&mut w);
        let mut expected = Writer::new();
        expected
            .raw(&2336u32.to_le_bytes())
            .raw(&coord_to_long(1000.5).to_le_bytes())
            .raw(&coord_to_long(2000.25).to_le_bytes())
            .raw(&100.0f32.to_le_bytes())
            .raw(&1.5f32.to_le_bytes())
            .raw(&1.0f32.to_le_bytes())
            .raw(&77u64.to_le_bytes());
        assert_eq!(w.finish(), expected.finish());
        assert_eq!(
            crate::game::ClientMsg::CreateDoodad(request).opcode(),
            cs::CSCreateDoodadPacket
        );
        assert_eq!(cs::CSCreateDoodadPacket, 0x0e6);
        assert_eq!(cs::CSRequestCommonFarmList, 0x10b);
    }

    #[test]
    fn farm_state_hydrates_changes_only_on_receipt_and_resets() {
        let mut state = Farms::default();
        let list = CommonFarmList {
            max_count: 1,
            entries: vec![CommonFarmEntry {
                farm_type: 1,
                template_id: 380,
                time_left: 5000,
                func_group_id: 77,
                pos: Pos::new(1.0, 2.0, 3.0),
                plant_time: 7,
            }],
        };
        let event = Event::Farm(FarmEvent::List(list.clone()));
        assert!(state.apply(&event));
        assert!(!state.apply(&event));
        assert_eq!(state.list.as_ref(), Some(&list));
        assert!(state.apply(&Event::Farm(FarmEvent::Show(ShowCommonFarm {
            farm_type: 1,
            count: 2,
            x: 0,
            y: 0,
            z: 0.0
        }))));
        assert!(!state.apply(&Event::Farm(FarmEvent::Show(ShowCommonFarm {
            farm_type: 1,
            count: 2,
            x: 0,
            y: 0,
            z: 0.0
        }))));
        assert!(state.apply(&Event::Farm(FarmEvent::IsUnitInFarm(3))));
        assert_eq!(state.unit_farm_type, Some(3));
        state = Farms::default();
        for reset in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
        ] {
            assert!(state.apply(&event));
            assert!(state.apply(&reset));
            assert_eq!(state, Farms::default());
            assert!(!state.apply(&reset));
        }
    }
}
