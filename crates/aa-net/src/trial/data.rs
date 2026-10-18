//! Native trial data/record bodies. The active lane writes complete declared record rows.
use crate::wire::{Reader, Result, WireError};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialDefendantData {
    pub defendant_id: u32,
    pub name: Vec<u8>,
    pub race: u8,
    pub faction_id: u32,
    /// Retained raw; native details callback ignores this field.
    pub trial_id: u32,
    pub sentence_time_ms: i32,
    pub judge_obj_id: u32,
}
impl TrialDefendantData {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            defendant_id: r.u32()?,
            name: r.blob()?.to_vec(),
            race: r.u8()?,
            faction_id: r.u32()?,
            trial_id: r.u32()?,
            sentence_time_ms: r.i32()?,
            judge_obj_id: r.bc()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialCrimeRecord {
    pub id: u32,
    pub victim_id: u32,
    pub victim_name: Vec<u8>,
    pub reporter_id: u32,
    pub reporter_name: Vec<u8>,
    pub victim_faction_id: u32,
    pub victim_expedition_id: u32,
    pub crime_kind: u8,
    pub doodad_template_id: u32,
    pub used_skill_id: u32,
    /// Original fixed-point wire coordinates; no guessed world/zone mapping.
    pub position_x: i64,
    pub position_y: i64,
    pub position_z_bits: u32,
    pub message: Vec<u8>,
    pub report_time: i64,
}
impl TrialCrimeRecord {
    pub fn position_z(&self) -> f32 {
        f32::from_bits(self.position_z_bits)
    }
    pub(super) fn native_safe(&self) -> bool {
        self.victim_name.len() <= 128 && self.reporter_name.len() <= 128 && self.message.len() <= 200
    }
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            id: r.u32()?,
            victim_id: r.u32()?,
            victim_name: r.blob()?.to_vec(),
            reporter_id: r.u32()?,
            reporter_name: r.blob()?.to_vec(),
            victim_faction_id: r.u32()?,
            victim_expedition_id: r.u32()?,
            crime_kind: r.u8()?,
            doodad_template_id: r.u32()?,
            used_skill_id: r.u32()?,
            position_x: r.i64()?,
            position_y: r.i64()?,
            position_z_bits: r.u32()?,
            message: r.blob()?.to_vec(),
            report_time: r.i64()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialRecords {
    pub trial_id: u32,
    pub raw_unknown: u32,
    pub total_count: i32,
    pub count: i32,
    pub records: Vec<TrialCrimeRecord>,
}
impl TrialRecords {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        let trial_id = r.u32()?;
        let raw_unknown = r.u32()?;
        let total_count = r.i32()?;
        let count = r.i32()?;
        let n = count.max(0) as usize;
        if n > r.remaining() / 63 {
            return Err(WireError::Invalid("trial records count exceeds body".into()));
        }
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            records.push(TrialCrimeRecord::read(r)?)
        }
        Ok(Self {
            trial_id,
            raw_unknown,
            total_count,
            count,
            records,
        })
    }
    pub(super) fn native_safe(&self) -> bool {
        self.count <= 5 && self.records.len() == self.count.max(0) as usize && self.records.iter().all(TrialCrimeRecord::native_safe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, TrialEvent, TrialNotice, Trials, game::ServerMsg, wire::Writer};
    fn row(w: &mut Writer) {
        w.u32(1).u32(2).blob(&[b'A', 0, 255]).u32(3).blob(&[b'B', 255]);
        w.u32(148)
            .u32(1000)
            .u8(4)
            .u32(55)
            .u32(66)
            .i64(i64::MIN)
            .i64(i64::MAX)
            .u32(0x7fc01234)
            .blob(&[0, 255])
            .i64(-1);
    }
    fn records(n: i32) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(7).u32(u32::MAX).i32(100).i32(n);
        for _ in 0..n.max(0) {
            row(&mut w)
        }
        w.finish()
    }
    fn fold(s: &mut Trials, e: TrialEvent) -> Vec<TrialNotice> {
        s.apply_with_notices(&Event::Trial(e)).1
    }
    fn decode(op: u16, b: &[u8]) -> TrialEvent {
        TrialEvent::read(op, &mut Reader::new(b)).unwrap()
    }
    #[test]
    fn exact_native_lane_data_records_phase_and_all_truncations() {
        let mut w = Writer::new();
        w.u32(2).blob(&[b'A', 0, 255]).u8(255).u32(148).u32(999).i32(i32::MIN).bc(0x123456);
        let mut phase = Writer::new();
        phase.u32(7).u8(255).i32(i32::MIN).u32(u32::MAX);
        let bodies = [(0x176, w.finish()), (0x177, records(1)), (0x178, phase.finish())];
        let mut batch = Writer::new();
        for (op, b) in &bodies {
            let mut r = Reader::new(b);
            let event = TrialEvent::read(*op, &mut r).unwrap();
            assert!(r.is_empty());
            for n in 0..b.len() {
                assert!(TrialEvent::read(*op, &mut Reader::new(&b[..n])).is_err(), "{op:x}/{n}");
            }
            match event {
                TrialEvent::DefendantData(d) => {
                    assert_eq!((d.trial_id, d.sentence_time_ms, d.judge_obj_id), (999, i32::MIN, 0x123456));
                    assert_eq!(d.name, [b'A', 0, 255]);
                }
                TrialEvent::Records(d) => {
                    let r = &d.records[0];
                    assert_eq!((d.total_count, d.count), (100, 1));
                    assert_eq!(
                        (r.position_x, r.position_y, r.position_z_bits, r.report_time),
                        (i64::MIN, i64::MAX, 0x7fc01234, -1)
                    );
                    assert!(r.position_z().is_nan());
                }
                TrialEvent::PhaseChanged {
                    phase,
                    current_jury,
                    remaining_time_ms,
                    ..
                } => assert_eq!((phase, current_jury, remaining_time_ms), (255, i32::MIN, u32::MAX)),
                _ => panic!(),
            }
            batch.u16(0).u16(*op).raw(b);
        }
        batch.u16(0).u16(0x1bc).u32(99);
        use std::io::Write;
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&batch.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(4).raw(&z.finish().unwrap());
        let ServerMsg::Batch(rows) = ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[3], ServerMsg::SetBreath { breath: 99 }));
    }
    #[test]
    fn raw_lane_six_rows_survive_without_unsafe_native_callback_and_bad_counts_refuse() {
        let event = decode(0x177, &records(6));
        let TrialEvent::Records(raw) = &event else { panic!() };
        assert_eq!(raw.records.len(), 6);
        let mut s = Trials::default();
        fold(&mut s, event.clone());
        assert_eq!(s.last_receipt, Some(event));
        assert_eq!(s.trial_id, 0);
        assert!(s.records.is_empty());
        let mut w = Writer::new();
        w.u32(7).u32(0).i32(0).i32(i32::MAX);
        assert!(TrialRecords::read(&mut Reader::new(&w.finish())).is_err());
        let negative = decode(0x177, &records(-1));
        fold(&mut s, negative);
        assert_eq!(s.records_received, Some(-1));
        assert!(s.records.is_empty());
    }
    #[test]
    fn native_record_adoption_order_duplicates_role_guard_and_coherent_details() {
        let mut s = Trials::default();
        let e = decode(0x177, &records(1));
        assert_eq!(fold(&mut s, e.clone()), vec![TrialNotice::BeautyshopStatus]);
        assert_eq!((s.trial_id, s.records_received, s.records_total), (7, Some(1), Some(100)));
        fold(&mut s, e.clone());
        assert_eq!(s.records.len(), 2);
        assert_eq!(s.records_received, Some(2));
        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        let mut other = records(1);
        other[..4].copy_from_slice(&8u32.to_le_bytes());
        fold(&mut s, decode(0x177, &other));
        assert_eq!(s.trial_id, 7);
        assert_eq!(s.records.len(), 3);
        let data = TrialDefendantData {
            defendant_id: 2,
            name: b"name".to_vec(),
            race: 1,
            faction_id: 148,
            trial_id: 999,
            sentence_time_ms: -1,
            judge_obj_id: 42,
        };
        assert_eq!(
            fold(&mut s, TrialEvent::DefendantData(data.clone())),
            vec![TrialNotice::ShowCrimeRecords { phase: 0 }]
        );
        assert_eq!(s.trial_id, 7);
        assert_eq!(s.sentence_time_ms, Some(-1));
        fold(&mut s, TrialEvent::Canceled { trial_id: 999 });
        assert_eq!(s.defendant.as_ref().unwrap().judge_obj_id, 0);
        assert_eq!(s.records.len(), 3);
        fold(
            &mut s,
            TrialEvent::Records(TrialRecords {
                trial_id: 8,
                raw_unknown: 0,
                total_count: 0,
                count: 0,
                records: vec![],
            }),
        );
        assert!(s.defendant.is_none());
        assert!(s.records.is_empty());
        assert_eq!(s.sentence_time_ms, Some(-1));
        let mut bad = data;
        bad.name = vec![1; 129];
        fold(&mut s, TrialEvent::DefendantData(bad.clone()));
        assert!(s.defendant.is_none());
        assert_eq!(s.last_receipt, Some(TrialEvent::DefendantData(bad)));
    }
    #[test]
    fn forty_two_original_x86_phase_oracle_cases_preserve_notice_order_and_wrong_trial_gate() {
        for role in 0..3 {
            for trial in [7, 8] {
                for phase in 0..7 {
                    let mut s = Trials::default();
                    fold(
                        &mut s,
                        TrialEvent::Records(TrialRecords {
                            trial_id: 7,
                            raw_unknown: 0,
                            total_count: 0,
                            count: 0,
                            records: vec![],
                        }),
                    );
                    fold(
                        &mut s,
                        TrialEvent::JuryWaiting {
                            count: 99,
                            total: 6,
                            sentence_time_ms: -1,
                        },
                    );
                    if role == 1 {
                        fold(
                            &mut s,
                            TrialEvent::SummonJury {
                                trial_id: 7,
                                court: 0,
                                jury_number: 0,
                            },
                        );
                    }
                    if role == 2 {
                        fold(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
                    }
                    s.phase = 1;
                    let actual = fold(
                        &mut s,
                        TrialEvent::PhaseChanged {
                            trial_id: trial,
                            phase,
                            current_jury: i32::MIN,
                            remaining_time_ms: u32::MAX,
                        },
                    );
                    let mut expected = vec![TrialNotice::Status { phase, current_jury: i32::MIN }];
                    if role == 0 || trial == 7 {
                        if trial != 7 {
                            expected.push(TrialNotice::BeautyshopStatus);
                        }
                        if phase == 2 && role == 2 {
                            expected.push(TrialNotice::JuryWaiting {
                                count: i32::MIN,
                                total: 6,
                                sentence_time_ms: -1,
                            });
                        }
                        if (2..=4).contains(&phase) {
                            expected.push(TrialNotice::ShowCrimeRecords { phase });
                        }
                        if phase == 5 && role == 1 {
                            expected.push(TrialNotice::Verdicts {
                                minutes: [14316, 35791, 57266, 71582, 85899],
                            });
                        }
                        assert_eq!((s.phase, s.current_jury, s.remaining_time_ms), (phase, Some(i32::MIN), u32::MAX));
                    } else {
                        assert_eq!((s.phase, s.current_jury, s.remaining_time_ms), (1, Some(99), 0));
                    }
                    assert_eq!(actual, expected, "role{role}/trial{trial}/phase{phase}");
                }
            }
        }
    }
}
