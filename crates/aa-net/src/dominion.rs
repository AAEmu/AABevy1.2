//! Received Dominion state used by original expedition-owner eligibility.
//! No request, timer estimate, or database row creates received territory state.
use crate::{
    session::Event,
    wire::{Reader, Result, WireError},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DominionPosition {
    pub x: i64,
    pub y: i64,
    pub z: f32,
}
impl DominionPosition {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DominionTerritory {
    pub id: u32,
    pub id2: u32,
    pub max_gates: u8,
    pub max_walls: u8,
    pub radius_declare: i16,
    pub radius_dominion: u16,
    pub radius_offense_hq: i16,
    pub radius_siege: i16,
}
impl DominionTerritory {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            id2: r.u32()?,
            max_gates: r.u8()?,
            max_walls: r.u8()?,
            radius_declare: r.i16()?,
            radius_dominion: r.u16()?,
            radius_offense_hq: r.i16()?,
            radius_siege: r.i16()?,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DominionParticipant {
    /// Native39839600 compares this with the expedition ID.
    pub expedition_id: u32,
    pub obj_id: u32,
    pub position: DominionPosition,
    pub ni: u8,
    pub nr: u8,
    pub limit: u8,
    /// Complete received order and duplicates. Native inline storage only has100 entries.
    pub members: Vec<u32>,
}
impl DominionParticipant {
    fn read(r: &mut Reader) -> Result<Self> {
        let expedition_id = r.u32()?;
        let obj_id = r.bc()?;
        let position = DominionPosition::read(r)?;
        let ni = r.u8()?;
        let nr = r.u8()?;
        let limit = r.u8()?;
        let count = r.u8()?;
        let mut members = Vec::with_capacity(count as usize);
        for _ in 0..count {
            members.push(r.u32()?);
        }
        Ok(Self {
            expedition_id,
            obj_id,
            position,
            ni,
            nr,
            limit,
            members,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DominionSiegeTimers {
    pub durations: [i32; 5],
    pub started: i64,
    pub fixed: i64,
    pub bdm: i32,
    /// Raw byte; native sign-extends it to i32, and the owner blocker checks exactly4.
    pub siege_period: u8,
    pub defender: DominionParticipant,
    pub offense: DominionParticipant,
}
impl DominionSiegeTimers {
    fn read(r: &mut Reader) -> Result<Self> {
        let mut durations = [0; 5];
        for value in &mut durations {
            *value = r.i32()?;
        }
        Ok(Self {
            durations,
            started: r.i64()?,
            fixed: r.i64()?,
            bdm: r.i32()?,
            siege_period: r.u8()?,
            defender: DominionParticipant::read(r)?,
            offense: DominionParticipant::read(r)?,
        })
    }
    fn inline_safe(&self) -> bool {
        self.defender.members.len() <= 100 && self.offense.members.len() <= 100
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DominionData {
    /// Lane calls this zone_id; original lookup397f2180 uses zone_groups IDs.
    pub zone_group_id: u16,
    pub expedition_id: u32,
    pub house: u32,
    pub tax_rate: i32,
    pub position: DominionPosition,
    pub cur_house_tax_money: i32,
    pub cur_hunt_tax_money: i32,
    pub peace_tax_money: i32,
    pub cur_house_tax_aa_point: i32,
    pub peace_tax_aa_point: i32,
    pub last_paid_time: i64,
    pub last_siege_end_time: i64,
    pub reign_start_time: i64,
    pub last_tax_rate_changed_time: i64,
    pub last_national_tax_rate_changed_time: i64,
    pub national_tax_rate: u16,
    pub national_monument_db_id: i64,
    pub national_monument_position: DominionPosition,
    pub obj_id: u32,
    pub territory: DominionTerritory,
    pub siege_timers: DominionSiegeTimers,
    pub non_pvp_start: i64,
    pub non_pvp_duration: u16,
}
impl DominionData {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_group_id: r.u16()?,
            expedition_id: r.u32()?,
            house: r.u32()?,
            tax_rate: r.i32()?,
            position: DominionPosition::read(r)?,
            cur_house_tax_money: r.i32()?,
            cur_hunt_tax_money: r.i32()?,
            peace_tax_money: r.i32()?,
            cur_house_tax_aa_point: r.i32()?,
            peace_tax_aa_point: r.i32()?,
            last_paid_time: r.i64()?,
            last_siege_end_time: r.i64()?,
            reign_start_time: r.i64()?,
            last_tax_rate_changed_time: r.i64()?,
            last_national_tax_rate_changed_time: r.i64()?,
            national_tax_rate: r.u16()?,
            national_monument_db_id: r.i64()?,
            national_monument_position: DominionPosition::read(r)?,
            obj_id: r.bc()?,
            territory: DominionTerritory::read(r)?,
            siege_timers: DominionSiegeTimers::read(r)?,
            non_pvp_start: r.i64()?,
            non_pvp_duration: r.u16()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum DominionEvent {
    Data {
        data: DominionData,
        newly_declared: u8,
        final_data_by_request: u8,
    },
    Deleted {
        id: u16,
    },
    OwnerChanged {
        id: u16,
        expedition_id: u32,
        reign_start_time: i64,
        bestowed: u8,
    },
    SiegeState {
        id: u16,
        territory: DominionTerritory,
        siege_timers: DominionSiegeTimers,
        new_owner: u32,
        reign_start_time: Option<i64>,
    },
    SiegeDeclared {
        id: u16,
        offense: DominionParticipant,
    },
}
impl DominionEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x01d..=0x01f | 0x0e9 | 0x0ea)
    }
    pub fn read_lane(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x01d => Self::Data {
                data: DominionData::read(r)?,
                newly_declared: r.u8()?,
                final_data_by_request: r.u8()?,
            },
            0x01e => Self::Deleted { id: r.u16()? },
            0x01f => Self::OwnerChanged {
                id: r.u16()?,
                expedition_id: r.u32()?,
                reign_start_time: r.i64()?,
                bestowed: r.u8()?,
            },
            0x0e9 => {
                let id = r.u16()?;
                let territory = DominionTerritory::read(r)?;
                let siege_timers = DominionSiegeTimers::read(r)?;
                let new_owner = r.u32()?;
                let reign_start_time = if new_owner != 0 { Some(r.i64()?) } else { None };
                Self::SiegeState {
                    id,
                    territory,
                    siege_timers,
                    new_owner,
                    reign_start_time,
                }
            }
            0x0ea => Self::SiegeDeclared {
                id: r.u16()?,
                offense: DominionParticipant::read(r)?,
            },
            _ => return Err(WireError::Invalid("unknown Dominion opcode".into())),
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Dominion {
    /// Full last-provided row fields. Non-gate tax/member/alert deltas remain unsupported.
    pub rows: BTreeMap<u16, DominionData>,
    pub last_receipt: Option<DominionEvent>,
    /// Factual loss of gate coverage; neither a timer nor an optimistic busy flag.
    pub coverage_gap: bool,
}
impl Dominion {
    /// Native constructor has an empty tree. A lost relevant receipt makes either answer unknown.
    pub fn owner_change_blocked(&self, expedition_id: u32) -> Option<bool> {
        if expedition_id == 0 {
            return Some(false);
        }
        if self.coverage_gap {
            return None;
        }
        Some(
            expedition_id != 0
                && self.rows.values().any(|row| {
                    row.siege_timers.siege_period == 4
                        && (row.siege_timers.defender.expedition_id == expedition_id || row.siege_timers.offense.expedition_id == expedition_id)
                }),
        )
    }
    fn owner_changed(row: &mut DominionData, expedition_id: u32, reign_start_time: i64) {
        row.expedition_id = expedition_id;
        row.siege_timers.defender.expedition_id = expedition_id;
        row.reign_start_time = reign_start_time;
        row.national_tax_rate = 500;
        row.last_tax_rate_changed_time = 0;
        row.last_national_tax_rate_changed_time = 0;
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Dominion(receipt) => {
                match receipt {
                    DominionEvent::Data { data, .. } => {
                        self.coverage_gap |= !data.siege_timers.inline_safe();
                        self.rows.insert(data.zone_group_id, data.clone());
                    }
                    DominionEvent::Deleted { id } => {
                        self.rows.remove(id);
                    }
                    DominionEvent::OwnerChanged {
                        id,
                        expedition_id,
                        reign_start_time,
                        ..
                    } => {
                        if let Some(row) = self.rows.get_mut(id) {
                            Self::owner_changed(row, *expedition_id, *reign_start_time);
                        }
                    }
                    DominionEvent::SiegeState {
                        id,
                        territory,
                        siege_timers,
                        new_owner,
                        reign_start_time,
                    } => {
                        self.coverage_gap |= !siege_timers.inline_safe();
                        if let Some(row) = self.rows.get_mut(id) {
                            // Native only invokes the owner helper on a transition from existing phase4.
                            if row.siege_timers.siege_period == 4 && *new_owner != 0 {
                                if let Some(time) = reign_start_time {
                                    Self::owner_changed(row, *new_owner, *time);
                                }
                            }
                            row.territory = territory.clone();
                            row.siege_timers = siege_timers.clone();
                        }
                    }
                    DominionEvent::SiegeDeclared { id, offense } => {
                        self.coverage_gap |= offense.members.len() > 100;
                        if let Some(row) = self.rows.get_mut(id) {
                            row.siege_timers.offense = offense.clone();
                        }
                    }
                }
                self.last_receipt = Some(receipt.clone());
                true
            }
            Event::GameDecodeFailed { .. } => {
                let changed = !self.coverage_gap;
                self.coverage_gap = true;
                changed
            }
            Event::UnhandledGamePacket { level, opcode, .. } => {
                // DD04 has no per-inner-packet length: an unknown opcode abandons its remaining tail.
                let gap = *level == crate::game::LEVEL_COMPRESSED || (*level == crate::game::LEVEL_GAME && matches!(*opcode, 0x024 | 0x025));
                let changed = gap && !self.coverage_gap;
                self.coverage_gap |= gap;
                changed
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Kicked { .. } | Event::Disconnected { .. } => {
                let changed = !self.rows.is_empty() || self.last_receipt.is_some() || self.coverage_gap;
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
    use crate::wire::Writer;
    fn territory_bytes() -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(1).u32(2).u8(3).u8(4).i16(-5).u16(6).i16(-7).i16(8);
        w.finish()
    }
    fn participant_bytes(id: u32, count: u8) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(id).bc(0x010203).i64(-100).i64(200).f32(3.5).u8(9).u8(10).u8(11).u8(count);
        for _ in 0..count {
            w.u32(0x12345678);
        }
        w.finish()
    }
    fn timer_bytes(phase: u8, defender: u32, offense: u32) -> Vec<u8> {
        let mut w = Writer::new();
        for n in [-1, 2, 3, 4, 5] {
            w.i32(n);
        }
        w.i64(-6)
            .i64(-7)
            .i32(-8)
            .u8(phase)
            .raw(&participant_bytes(defender, 2))
            .raw(&participant_bytes(offense, 2));
        w.finish()
    }
    fn data_bytes() -> Vec<u8> {
        // Independent field sequence from native3983a470 and the active lane DominionData writer.
        let mut w = Writer::new();
        w.u16(25).u32(77).u32(2).i32(-3).i64(-4).i64(5).f32(6.0);
        for n in [7, 8, 9, 10, 11] {
            w.i32(n);
        }
        for n in [21, 22, 23, 24, 25] {
            w.i64(n);
        }
        w.u16(500)
            .i64(-31)
            .i64(32)
            .i64(33)
            .f32(34.0)
            .bc(0x020304)
            .raw(&territory_bytes())
            .raw(&timer_bytes(4, 77, 88))
            .i64(-45)
            .u16(46)
            .u8(2)
            .u8(255);
        w.finish()
    }
    fn cases() -> Vec<(u16, Vec<u8>)> {
        let mut owner = Writer::new();
        owner.u16(25).u32(99).i64(-123).u8(2);
        let mut state0 = Writer::new();
        state0.u16(25).raw(&territory_bytes()).raw(&timer_bytes(3, 77, 88)).u32(0);
        let mut state_owner = Writer::new();
        state_owner.u16(25).raw(&territory_bytes()).raw(&timer_bytes(5, 99, 88)).u32(99).i64(-987);
        let mut declared = Writer::new();
        declared.u16(25).raw(&participant_bytes(99, 2));
        vec![
            (0x1d, data_bytes()),
            (0x1e, vec![25, 0]),
            (0x1f, owner.finish()),
            (0xe9, state0.finish()),
            (0xe9, state_owner.finish()),
            (0xea, declared.finish()),
        ]
    }
    #[test]
    fn five_native_lane_layouts_truncate_strictly_and_preserve_conditional_timestamp_and_raw_values() {
        for (opcode, bytes) in cases() {
            let mut r = Reader::new(&bytes);
            let receipt = DominionEvent::read_lane(opcode, &mut r).unwrap();
            assert!(r.is_empty());
            for n in 0..bytes.len() {
                assert!(DominionEvent::read_lane(opcode, &mut Reader::new(&bytes[..n])).is_err(), "{opcode:x} at{n}");
            }
            match receipt {
                DominionEvent::Data {
                    data,
                    newly_declared,
                    final_data_by_request,
                } => {
                    assert_eq!(bytes.len(), 276);
                    assert_eq!((newly_declared, final_data_by_request), (2, 255));
                    assert_eq!((data.zone_group_id, data.expedition_id, data.house, data.tax_rate), (25, 77, 2, -3));
                    assert_eq!((data.position.x, data.position.y, data.position.z), (-4, 5, 6.0));
                    assert_eq!((data.last_paid_time, data.last_national_tax_rate_changed_time), (21, 25));
                    assert_eq!(
                        (data.national_monument_db_id, data.obj_id, data.non_pvp_start, data.non_pvp_duration),
                        (-31, 0x020304, -45, 46)
                    );
                    assert_eq!(data.territory.radius_declare, -5);
                    assert_eq!(data.siege_timers.durations, [-1, 2, 3, 4, 5]);
                    assert_eq!(data.siege_timers.defender.members, vec![0x12345678; 2]);
                    assert_eq!(data.siege_timers.offense.expedition_id, 88);
                }
                DominionEvent::SiegeState {
                    new_owner, reign_start_time, ..
                } => assert_eq!(reign_start_time, if new_owner == 0 { None } else { Some(-987) }),
                _ => {}
            }
        }
    }
    #[test]
    fn each_layout_leaves_next_dd04_packet_aligned() {
        use std::io::Write;
        for (opcode, bytes) in cases() {
            let mut raw = Writer::new();
            raw.u16(0).u16(opcode).raw(&bytes).u16(0).u16(0x0b).u8(0);
            let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
            compressor.write_all(&raw.finish()).unwrap();
            let mut frame = Writer::new();
            frame.u8(0xdd).u8(4).u16(2).raw(&compressor.finish().unwrap());
            let crate::game::ServerMsg::Batch(list) = crate::game::ServerMsg::decode(&frame.finish()).unwrap() else {
                panic!()
            };
            assert_eq!(list.len(), 2);
            assert!(matches!(&list[0], crate::game::ServerMsg::Dominion(_)));
            assert!(matches!(&list[1],crate::game::ServerMsg::Expedition(crate::ExpeditionEvent::Policies(rows)) if rows.is_empty()));
        }
    }
    fn data() -> DominionEvent {
        DominionEvent::read_lane(0x1d, &mut Reader::new(&data_bytes())).unwrap()
    }
    fn apply(s: &mut Dominion, receipt: DominionEvent) {
        s.apply(&Event::Dominion(receipt));
    }
    #[test]
    fn native_gate_tracks_all_five_mutators_without_creating_missing_rows_or_clock_estimates() {
        let mut state = Dominion::default();
        assert_eq!(state.owner_change_blocked(77), Some(false));
        apply(
            &mut state,
            DominionEvent::OwnerChanged {
                id: 25,
                expedition_id: 77,
                reign_start_time: 1,
                bestowed: 1,
            },
        );
        assert!(state.rows.is_empty());
        apply(&mut state, data());
        assert_eq!(state.owner_change_blocked(77), Some(true));
        assert_eq!(state.owner_change_blocked(88), Some(true));
        assert_eq!(state.owner_change_blocked(0), Some(false));
        assert_eq!(state.owner_change_blocked(99), Some(false));
        apply(
            &mut state,
            DominionEvent::OwnerChanged {
                id: 25,
                expedition_id: 99,
                reign_start_time: -123,
                bestowed: 2,
            },
        );
        assert_eq!(state.owner_change_blocked(77), Some(false));
        assert_eq!(state.owner_change_blocked(99), Some(true));
        assert_eq!(state.rows[&25].reign_start_time, -123);
        assert_eq!(state.rows[&25].last_tax_rate_changed_time, 0);
        let mut offense = state.rows[&25].siege_timers.offense.clone();
        offense.expedition_id = 100;
        apply(&mut state, DominionEvent::SiegeDeclared { id: 25, offense });
        assert_eq!(state.owner_change_blocked(88), Some(false));
        assert_eq!(state.owner_change_blocked(100), Some(true));
        let mut timers = state.rows[&25].siege_timers.clone();
        timers.siege_period = 3;
        apply(
            &mut state,
            DominionEvent::SiegeState {
                id: 25,
                territory: DominionTerritory::default(),
                siege_timers: timers.clone(),
                new_owner: 101,
                reign_start_time: Some(777),
            },
        );
        assert_eq!(
            state.rows[&25].expedition_id, 101,
            "existing phase4 admits owner update before timer replacement"
        );
        assert_eq!(state.owner_change_blocked(100), Some(false));
        timers.siege_period = 4;
        timers.defender.expedition_id = 102;
        apply(
            &mut state,
            DominionEvent::SiegeState {
                id: 25,
                territory: DominionTerritory::default(),
                siege_timers: timers,
                new_owner: 103,
                reign_start_time: Some(888),
            },
        );
        assert_eq!(state.rows[&25].expedition_id, 101, "existing phase3 skips owner helper");
        assert_eq!(state.owner_change_blocked(102), Some(true));
        apply(&mut state, DominionEvent::Deleted { id: 25 });
        assert!(state.rows.is_empty());
        assert_eq!(state.owner_change_blocked(102), Some(false));
        let unknown = DominionEvent::SiegeDeclared {
            id: 26,
            offense: DominionParticipant::default(),
        };
        apply(&mut state, unknown.clone());
        assert!(state.rows.is_empty());
        assert_eq!(state.last_receipt, Some(unknown));
    }
    #[test]
    fn coverage_gaps_are_sticky_and_full_snapshot_flags_never_invent_completeness() {
        let mut state = Dominion::default();
        apply(&mut state, data());
        assert!(!state.apply(&Event::UnhandledGamePacket {
            level: crate::game::LEVEL_PROXY,
            opcode: 0x24,
            len: 0
        }));
        assert!(!state.apply(&Event::UnhandledGamePacket {
            level: crate::game::LEVEL_GAME,
            opcode: 0x3ff,
            len: 0
        }));
        assert!(state.apply(&Event::UnhandledGamePacket {
            level: crate::game::LEVEL_COMPRESSED,
            opcode: 0x3ff,
            len: 42
        }));
        assert_eq!(state.owner_change_blocked(77), None);
        apply(&mut state, data());
        assert_eq!(state.owner_change_blocked(77), None);
        state.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(state, Dominion::default());
        assert!(state.apply(&Event::UnhandledGamePacket {
            level: crate::game::LEVEL_GAME,
            opcode: 0x24,
            len: 2
        }));
        assert_eq!(state.owner_change_blocked(77), None);
        state.apply(&Event::Disconnected { message: String::new() });
        assert_eq!(state.owner_change_blocked(77), Some(false));
        let mut raw = data();
        if let DominionEvent::Data { data, .. } = &mut raw {
            data.siege_timers.offense.members = vec![1; 101];
        }
        apply(&mut state, raw);
        assert_eq!(state.rows[&25].siege_timers.offense.members.len(), 101);
        assert_eq!(state.owner_change_blocked(77), None);
    }
    #[test]
    fn malformed_dd04_tail_reports_real_decode_failure_and_invalidates_gate() {
        use std::io::Write;
        let mut raw = Writer::new();
        raw.u16(0).u16(0x1e).u8(25); // truncated u16 territory ID
        let mut compressor = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        compressor.write_all(&raw.finish()).unwrap();
        let mut frame = Writer::new();
        frame.u8(0xdd).u8(4).u16(1).raw(&compressor.finish().unwrap());
        let crate::game::ServerMsg::Batch(list) = crate::game::ServerMsg::decode(&frame.finish()).unwrap() else {
            panic!()
        };
        let [crate::game::ServerMsg::DecodeFailed { message }] = list.as_slice() else {
            panic!("{list:?}")
        };
        let mut state = Dominion::default();
        assert!(state.apply(&Event::GameDecodeFailed { message: message.clone() }));
        assert_eq!(state.owner_change_blocked(77), None);
        assert!(state.rows.is_empty());
    }
}
