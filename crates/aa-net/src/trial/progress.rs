//! Received trial counters/history. These are displayed data, never accepted jury choices.
use crate::wire::{Reader, Result};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialHistory {
    /// Raw packet identity; native3907bb20 ignores it when writing the shared trial cache.
    pub player_id: u32,
    pub crime_point: i32,
    pub arrest: i32,
    pub accept_guilty: i32,
    pub accept_trial: i32,
    pub not_guilty: i32,
    pub guilty: i32,
    pub total_seconds: i32,
    pub current_seconds: i32,
    pub crime_record_count: i32,
    pub bot_report: i32,
}
impl TrialHistory {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            player_id: r.u32()?,
            crime_point: r.i32()?,
            arrest: r.i32()?,
            accept_guilty: r.i32()?,
            accept_trial: r.i32()?,
            not_guilty: r.i32()?,
            guilty: r.i32()?,
            total_seconds: r.i32()?,
            current_seconds: r.i32()?,
            crime_record_count: r.i32()?,
            bot_report: r.i32()?,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrialRulingStatus {
    pub count: i32,
    pub total: i32,
    pub sentence_type: u8,
    pub sentence_time_ms: i32,
}
impl TrialRulingStatus {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            count: r.i32()?,
            total: r.i32()?,
            sentence_type: r.u8()?,
            sentence_time_ms: r.i32()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, TrialEvent, TrialNotice, Trials, game::ServerMsg, wire::Writer};
    fn apply(s: &mut Trials, e: TrialEvent) -> Vec<TrialNotice> {
        s.apply_with_notices(&Event::Trial(e)).1
    }
    fn history() -> TrialHistory {
        TrialHistory {
            player_id: 999,
            crime_point: i32::MIN,
            arrest: i32::MAX,
            accept_guilty: i32::MIN,
            accept_trial: i32::MAX,
            not_guilty: i32::MIN,
            guilty: i32::MAX,
            total_seconds: i32::MIN,
            current_seconds: i32::MAX,
            crime_record_count: i32::MIN,
            bot_report: i32::MAX,
        }
    }
    #[test]
    fn crime_kind_counts_follow_details_receipt_not_record_append() {
        let mut s = Trials::default();
        assert_eq!(s.crime_kind_counts, None);
        apply(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        assert_eq!(s.crime_kind_counts, Some([0; 4]));
        let record = |crime_kind| crate::TrialCrimeRecord {
            id: 0,
            victim_id: 0,
            victim_name: vec![],
            reporter_id: 0,
            reporter_name: vec![],
            victim_faction_id: 0,
            victim_expedition_id: 0,
            crime_kind,
            doodad_template_id: 0,
            used_skill_id: 0,
            position_x: 0,
            position_y: 0,
            position_z_bits: 0,
            message: vec![],
            report_time: 0,
        };
        apply(
            &mut s,
            TrialEvent::Records(crate::TrialRecords {
                trial_id: 7,
                raw_unknown: 0,
                total_count: 4,
                count: 4,
                records: vec![record(3), record(2), record(4), record(255)],
            }),
        );
        assert_eq!(s.crime_kind_counts, Some([0; 4]));
        let details = crate::TrialDefendantData {
            defendant_id: 1,
            name: vec![],
            race: 1,
            faction_id: 148,
            trial_id: 999,
            sentence_time_ms: 1,
            judge_obj_id: 0,
        };
        apply(&mut s, TrialEvent::DefendantData(details.clone()));
        assert_eq!(s.crime_kind_counts, Some([1; 4]));
        apply(
            &mut s,
            TrialEvent::Records(crate::TrialRecords {
                trial_id: 7,
                raw_unknown: 0,
                total_count: 5,
                count: 1,
                records: vec![record(3)],
            }),
        );
        assert_eq!(s.crime_kind_counts, Some([1; 4]));
        apply(&mut s, TrialEvent::DefendantData(details));
        assert_eq!(s.crime_kind_counts, Some([2, 1, 1, 1]));
    }

    #[test]
    fn exact_native_lane_progress_bodies_all_truncations_and_dd04_tail() {
        let mut a = Writer::new();
        a.i32(i32::MIN).i32(i32::MAX);
        let mut b = Writer::new();
        b.i32(-1).i32(i32::MIN);
        let mut c = Writer::new();
        c.i32(1).i32(2).u8(255).i32(-1);
        let mut d = Writer::new();
        d.u32(999);
        for _ in 0..5 {
            d.i32(i32::MIN).i32(i32::MAX);
        }
        let rows = [
            (
                0x179,
                a.finish(),
                TrialEvent::JuryOkCount {
                    count: i32::MIN,
                    total: i32::MAX,
                },
            ),
            (0x17a, b.finish(), TrialEvent::VerdictCount { count: -1, total: i32::MIN }),
            (
                0x17d,
                c.finish(),
                TrialEvent::RulingStatus(TrialRulingStatus {
                    count: 1,
                    total: 2,
                    sentence_type: 255,
                    sentence_time_ms: -1,
                }),
            ),
            (0x181, d.finish(), TrialEvent::History(history())),
        ];
        let mut batch = Writer::new();
        for (op, body, expected) in rows {
            let mut r = Reader::new(&body);
            assert_eq!(TrialEvent::read(op, &mut r).unwrap(), expected);
            assert!(r.is_empty());
            for n in 0..body.len() {
                assert!(TrialEvent::read(op, &mut Reader::new(&body[..n])).is_err(), "{op:x}/{n}");
            }
            batch.u16(0).u16(op).raw(&body);
        }
        batch.u16(0).u16(0x1bc).u32(123);
        use std::io::Write;
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&batch.finish()).unwrap();
        let mut packet = Writer::new();
        packet.u8(0xdd).u8(crate::game::LEVEL_COMPRESSED).u16(5).raw(&z.finish().unwrap());
        let ServerMsg::Batch(rows) = ServerMsg::decode(&packet.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[4], ServerMsg::SetBreath { breath: 123 }));
    }
    #[test]
    fn original_x86_progress_cache_before_duplicate_notices_and_no_history_identity_guard() {
        let mut s = Trials::default();
        assert_eq!((s.jury_ok_count, s.jury_ok_total, s.verdict_counts, s.has_ruling), (None, None, None, None));
        for _ in 0..2 {
            assert_eq!(
                apply(
                    &mut s,
                    TrialEvent::JuryOkCount {
                        count: i32::MIN,
                        total: i32::MAX
                    }
                ),
                vec![TrialNotice::JuryOkCount {
                    count: i32::MIN,
                    total: i32::MAX
                }]
            );
            assert_eq!((s.jury_ok_count, s.jury_ok_total), (Some(i32::MIN), Some(i32::MAX)));
        }
        assert_eq!((i32::MIN as f32, i32::MAX as f32), (-2147483648.0, 2147483648.0));
        apply(&mut s, TrialEvent::VerdictCount { count: -1, total: i32::MIN });
        assert_eq!(s.verdict_counts, Some([-1, i32::MIN]));
        let ruling = TrialRulingStatus {
            count: 1,
            total: 2,
            sentence_type: 255,
            sentence_time_ms: -1,
        };
        for _ in 0..2 {
            assert_eq!(apply(&mut s, TrialEvent::RulingStatus(ruling)), vec![TrialNotice::RulingStatus(ruling)]);
            assert_eq!(s.has_ruling, Some(true));
            assert_eq!(s.ruling, Some(ruling));
            assert_eq!(s.verdict_counts, Some([1, 2]));
        }
        assert_eq!((ruling.sentence_type as f32, (ruling.sentence_time_ms as u32) as f32), (255.0, 4294967296.0));
        assert!(apply(&mut s, TrialEvent::History(history())).is_empty());
        assert_eq!(s.history, Some(history()));
        assert_eq!(s.trial_id, 0);
        apply(&mut s, TrialEvent::VerdictCount { count: 7, total: 8 });
        assert_eq!(s.ruling.unwrap().count, 7);
        assert_eq!(s.ruling.unwrap().total, 8);
    }
    #[test]
    fn changed_context_resets_only_native_known_fields_and_session_isolates_history() {
        let mut s = Trials::default();
        apply(&mut s, TrialEvent::JuryOkCount { count: 1, total: 9 });
        apply(&mut s, TrialEvent::History(history()));
        let ruling = TrialRulingStatus {
            count: 3,
            total: 4,
            sentence_type: 2,
            sentence_time_ms: 123,
        };
        apply(&mut s, TrialEvent::RulingStatus(ruling));
        apply(&mut s, TrialEvent::SummonDefendant { trial_id: 7 });
        assert_eq!(
            (s.jury_ok_count, s.jury_ok_total, s.verdict_counts, s.has_ruling),
            (Some(0), Some(9), Some([0, 0]), Some(false))
        );
        assert_eq!(s.ruling.unwrap(), TrialRulingStatus { count: 0, total: 0, ..ruling });
        assert_eq!(s.history, Some(history()));
        apply(&mut s, TrialEvent::RulingStatus(ruling));
        apply(&mut s, TrialEvent::Canceled { trial_id: 999 });
        assert_eq!(s.has_ruling, Some(true));
        assert_eq!(s.ruling, Some(ruling));
        assert_eq!(s.history, Some(history()));
        s.apply(&Event::Disconnected { message: String::new() });
        assert_eq!(s, Trials::default());
    }
}
