//! Native local Unix clock: raw samples are supplied at actual packet/frame dispatch.
use super::{TrialNotice, Trials};
use crate::Event;
impl Default for Trials {
    fn default() -> Self {
        Self {
            local_obj_id: Default::default(),
            defendant_context: Default::default(),
            trial_id: Default::default(),
            is_jury: Default::default(),
            jury_court: Default::default(),
            jury_number: Default::default(),
            phase: Default::default(),
            remaining_time_ms: Default::default(),
            last_epoch_seconds: Default::default(),
            expires_at_epoch_seconds: Some(0),
            defendant: Default::default(),
            records: Default::default(),
            records_received: Default::default(),
            records_total: Default::default(),
            records_raw_unknown: Default::default(),
            current_jury: Default::default(),
            jury_ok_count: Default::default(),
            jury_ok_total: Default::default(),
            verdict_counts: Default::default(),
            ruling: Default::default(),
            has_ruling: Default::default(),
            history: Default::default(),
            crime_kind_counts: Default::default(),
            waiting_order: Default::default(),
            jury_counts: Default::default(),
            sentence_time_ms: Default::default(),
            last_receipt: Default::default(),
        }
    }
}
impl Trials {
    pub fn apply_with_time(&mut self, event: &Event, epoch_seconds: Option<i64>) -> bool {
        self.fold(event, epoch_seconds, &mut |_, _| {})
    }
    pub fn apply_with_notices_at(&mut self, event: &Event, epoch_seconds: Option<i64>) -> (bool, Vec<TrialNotice>) {
        let mut notices = Vec::new();
        let changed = self.fold(event, epoch_seconds, &mut |notice, _| notices.push(notice));
        (changed, notices)
    }
    pub fn apply_with_dispatch_at(&mut self, event: &Event, epoch_seconds: Option<i64>) -> (bool, Vec<(TrialNotice, Self)>) {
        let mut dispatch = Vec::new();
        let changed = self.fold(event, epoch_seconds, &mut |notice, state| dispatch.push((notice, state.clone())));
        (changed, dispatch)
    }
    /// Called after queued receipts, once per outer frame. No extrapolation or expiry reset.
    pub fn advance_time(&mut self, epoch_seconds: i64) -> Option<TrialNotice> {
        let expiry = self.expires_at_epoch_seconds?;
        let last = self.last_epoch_seconds?;
        // Actual MSVCR100 _difftime64 rejects either negative operand and returns zero.
        if expiry == 0 || epoch_seconds < 0 || last < 0 || epoch_seconds == last {
            return None;
        }
        let remaining_seconds = if expiry < 0 { 0 } else { expiry - epoch_seconds };
        self.last_epoch_seconds = Some(epoch_seconds);
        Some(TrialNotice::Timer {
            phase: self.phase,
            remaining_seconds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrialEvent;
    #[test]
    fn original_frame_clock_executed_oracles_include_integer_precision_and_invalid_crt_domain() {
        for (last, expiry, now, remaining) in [
            (100, 110, 100, None),
            (100, 110, 101, Some(9)),
            (100, 110, 110, Some(0)),
            (100, 110, 111, Some(-1)),
            (100, 110, 99, Some(11)),
            (100, 0, 101, None),
            (100, -1, 101, Some(0)),
            (-1, 110, 101, None),
            (100, 110, -1, None),
            (i64::MAX - 1, i64::MAX, i64::MAX, Some(0)),
            (0, i64::MAX, 1, Some(i64::MAX - 1)),
            (i64::MAX, 1, 0, Some(1)),
            (9007199254740992, 9007199254740999, 9007199254740993, Some(6)),
        ] {
            let mut s = Trials::default();
            s.phase = 5;
            s.last_epoch_seconds = Some(last);
            s.expires_at_epoch_seconds = Some(expiry);
            assert_eq!(
                s.advance_time(now),
                remaining.map(|remaining_seconds| TrialNotice::Timer { phase: 5, remaining_seconds })
            );
            assert_eq!(s.last_epoch_seconds, Some(if remaining.is_some() { now } else { last }));
            assert_eq!(s.expires_at_epoch_seconds, Some(expiry));
            assert!(s.advance_time(now).is_none());
        }
    }
    #[test]
    fn phase_sampling_wrap_floor_unknowns_and_callback_points() {
        for now in [0, 1, -1, i64::MAX, i64::MAX - 1] {
            for ms in [0, 999, 1000, u32::MAX] {
                let mut s = Trials::default();
                s.trial_id = 7;
                s.apply_with_time(
                    &Event::Trial(TrialEvent::PhaseChanged {
                        trial_id: 7,
                        phase: 1,
                        current_jury: 0,
                        remaining_time_ms: ms,
                    }),
                    Some(now),
                );
                assert_eq!(
                    (s.last_epoch_seconds, s.expires_at_epoch_seconds),
                    (Some(now), Some(now.wrapping_add((ms / 1000) as i64)))
                );
            }
        }
        let mut s = Trials::default();
        assert_eq!((s.last_epoch_seconds, s.expires_at_epoch_seconds), (None, Some(0)));
        s.apply(&Event::Trial(TrialEvent::PhaseChanged {
            trial_id: 7,
            phase: 1,
            current_jury: 0,
            remaining_time_ms: 2000,
        }));
        assert_eq!((s.last_epoch_seconds, s.expires_at_epoch_seconds), (None, None));
        assert!(s.advance_time(100).is_none());
        s.apply_with_time(&Event::Trial(TrialEvent::SummonDefendant { trial_id: 7 }), Some(100));
        s.apply_with_time(
            &Event::Trial(TrialEvent::PhaseChanged {
                trial_id: 7,
                phase: 1,
                current_jury: 0,
                remaining_time_ms: 2000,
            }),
            Some(100),
        );
        let (_, ds) = s.apply_with_dispatch_at(
            &Event::Trial(TrialEvent::PhaseChanged {
                trial_id: 8,
                phase: 3,
                current_jury: 0,
                remaining_time_ms: 9000,
            }),
            Some(101),
        );
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].0, TrialNotice::Status { phase: 3, current_jury: 0 });
        assert_eq!(s.expires_at_epoch_seconds, Some(102));
        let (_, ds) = s.apply_with_dispatch_at(&Event::Trial(TrialEvent::Canceled { trial_id: 999 }), Some(200));
        for (_, v) in ds {
            assert_eq!((v.last_epoch_seconds, v.expires_at_epoch_seconds), (Some(200), Some(0)));
        }
        assert!(s.advance_time(201).is_none());
        s.apply_with_time(&Event::CharacterSelected(Box::default()), Some(300));
        assert_eq!(s, Trials::default());
    }
    #[test]
    fn context_adoption_retains_clock_until_actual_phase_or_role_reset() {
        let mut s = Trials::default();
        s.apply_with_time(
            &Event::Trial(TrialEvent::PhaseChanged {
                trial_id: 7,
                phase: 1,
                current_jury: 0,
                remaining_time_ms: 10000,
            }),
            Some(100),
        );
        let (_, ds) = s.apply_with_dispatch_at(&Event::Trial(TrialEvent::SummonDefendant { trial_id: 8 }), Some(101));
        assert_eq!(ds.len(), 1);
        assert_eq!((ds[0].1.last_epoch_seconds, ds[0].1.expires_at_epoch_seconds), (Some(100), Some(110)));
        assert_eq!(
            s.advance_time(111),
            Some(TrialNotice::Timer {
                phase: 1,
                remaining_seconds: -1
            })
        );
        assert_eq!(s.expires_at_epoch_seconds, Some(110));
    }
}
