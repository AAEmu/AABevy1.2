//! Original zero-argument Lua controls capture these fields from the received native cache.
use super::Trials;
use crate::wire::Writer;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrialCommand {
    ConfirmCrimeRecords { trial_id: u32, jury_number: i32 },
    CancelTrial { trial_id: u32 },
    ChooseVerdict { trial_id: u32, jury_number: i32, choice: u8 },
}
impl TrialCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::ConfirmCrimeRecords { .. } => 0x073,
            Self::CancelTrial { .. } => 0x074,
            Self::ChooseVerdict { .. } => 0x075,
        }
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::ConfirmCrimeRecords { trial_id, jury_number } => {
                w.u32(*trial_id).i32(*jury_number);
            }
            Self::CancelTrial { trial_id } => {
                w.u32(*trial_id);
            }
            Self::ChooseVerdict { trial_id, jury_number, choice } => {
                w.u32(*trial_id).i32(*jury_number).u8(*choice);
            }
        }
    }
    /// Native weak gate plus stronger captured-context refusal. No invented phase/defendant gate.
    pub fn validate_received(&self, state: &Trials) -> Result<(), &'static str> {
        let trial_id = match self {
            Self::ConfirmCrimeRecords { trial_id, .. } | Self::CancelTrial { trial_id } | Self::ChooseVerdict { trial_id, .. } => *trial_id,
        };
        if trial_id != state.trial_id {
            return Err("trial request refers to a stale received context");
        }
        match self {
            Self::ConfirmCrimeRecords { jury_number, .. } => {
                if !state.is_jury {
                    return Err("crime records confirmation requires received jury role");
                }
                if state.jury_number != Some(*jury_number) {
                    return Err("crime records confirmation refers to an unknown or stale jury seat");
                }
            }
            Self::ChooseVerdict { jury_number, choice, .. } => {
                if !(1..=6).contains(choice) {
                    return Err("verdict choice must be in the native one through six range");
                }
                if state.jury_number != Some(*jury_number) {
                    return Err("verdict refers to an unknown or stale jury seat");
                }
            }
            Self::CancelTrial { .. } => {
                if state.current_jury != Some(0) {
                    return Err("trial cancellation requires known zero current jury count");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, TrialEvent, game::ClientMsg};
    #[test]
    fn verdict_executed_native_choice_matrix_and_exact_byte_writer() {
        for is_jury in [false, true] {
            let mut state = Trials::default();
            state.is_jury = is_jury;
            state.jury_number = Some(i32::MIN);
            for choice in [0, 1, 2, 5, 6, 7, 255] {
                let command = TrialCommand::ChooseVerdict {
                    trial_id: 0,
                    jury_number: i32::MIN,
                    choice,
                };
                assert_eq!(command.validate_received(&state).is_ok(), (1..=6).contains(&choice));
                assert_eq!(ClientMsg::Trial(command).encode_body(), [0, 0, 0, 0, 0, 0, 0, 128, choice]);
                assert_eq!(command.opcode(), 0x75);
            }
            assert!(
                TrialCommand::ChooseVerdict {
                    trial_id: 1,
                    jury_number: i32::MIN,
                    choice: 1
                }
                .validate_received(&state)
                .is_err()
            );
            assert!(
                TrialCommand::ChooseVerdict {
                    trial_id: 0,
                    jury_number: 0,
                    choice: 1
                }
                .validate_received(&state)
                .is_err()
            );
            state.jury_number = None;
            assert!(
                TrialCommand::ChooseVerdict {
                    trial_id: 0,
                    jury_number: i32::MIN,
                    choice: 1
                }
                .validate_received(&state)
                .is_err()
            );
        }
        assert_eq!(
            ClientMsg::Trial(TrialCommand::ChooseVerdict {
                trial_id: u32::MAX,
                jury_number: -1,
                choice: 6
            })
            .encode(19),
            [0xdd, 1, 0, 19, 0x75, 0, 255, 255, 255, 255, 255, 255, 255, 255, 6]
        );
    }
    #[test]
    fn original_controls_native_lane_exact_payloads_and_raw_extremes() {
        let confirm = TrialCommand::ConfirmCrimeRecords {
            trial_id: u32::MAX,
            jury_number: i32::MIN,
        };
        let cancel = TrialCommand::CancelTrial { trial_id: 0 };
        assert_eq!(confirm.opcode(), 0x73);
        assert_eq!(cancel.opcode(), 0x74);
        assert_eq!(ClientMsg::Trial(confirm).encode_body(), [255, 255, 255, 255, 0, 0, 0, 128]);
        assert_eq!(ClientMsg::Trial(cancel).encode_body(), [0; 4]);
        assert_eq!(
            ClientMsg::Trial(confirm).encode(19),
            [0xdd, 1, 0, 19, 0x73, 0, 255, 255, 255, 255, 0, 0, 0, 128]
        );
    }
    #[test]
    fn executed_native_weak_gates_unknowns_and_captured_context_are_distinct() {
        for is_jury in [false, true] {
            for count in [0, 1, -1] {
                let mut s = Trials::default();
                s.is_jury = is_jury;
                s.jury_number = Some(i32::MIN);
                s.current_jury = Some(count);
                assert_eq!(
                    TrialCommand::ConfirmCrimeRecords {
                        trial_id: 0,
                        jury_number: i32::MIN
                    }
                    .validate_received(&s)
                    .is_ok(),
                    is_jury
                );
                assert_eq!(TrialCommand::CancelTrial { trial_id: 0 }.validate_received(&s).is_ok(), count == 0);
            }
        }
        let mut s = Trials::default();
        assert!(TrialCommand::CancelTrial { trial_id: 0 }.validate_received(&s).is_err());
        s.apply(&Event::Trial(TrialEvent::Waiting { order: 0, sentence_time_ms: 0 }));
        assert!(
            TrialCommand::CancelTrial { trial_id: 0 }.validate_received(&s).is_ok(),
            "no invented defendant/phase/nonzero-trial gate"
        );
        s.apply(&Event::Trial(TrialEvent::SummonJury {
            trial_id: 7,
            court: u32::MAX,
            jury_number: -1,
        }));
        assert!(TrialCommand::ConfirmCrimeRecords { trial_id: 7, jury_number: -1 }.validate_received(&s).is_ok());
        assert!(TrialCommand::ConfirmCrimeRecords { trial_id: 7, jury_number: 0 }.validate_received(&s).is_err());
        assert!(
            TrialCommand::ConfirmCrimeRecords { trial_id: 8, jury_number: -1 }
                .validate_received(&s)
                .is_err()
        );
        assert!(TrialCommand::CancelTrial { trial_id: 8 }.validate_received(&s).is_err());
    }
    #[test]
    fn received_seat_assignment_is_after_native_context_callback_and_retained_on_resets() {
        let mut s = Trials::default();
        assert_eq!((s.jury_court, s.jury_number), (None, None));
        let (_, dispatch) = s.apply_with_dispatch(&Event::Trial(TrialEvent::SummonJury {
            trial_id: 7,
            court: u32::MAX,
            jury_number: -1,
        }));
        assert_eq!((dispatch[0].1.jury_court, dispatch[0].1.jury_number), (None, None));
        assert_eq!((s.jury_court, s.jury_number), (Some(u32::MAX), Some(-1)));
        s.apply(&Event::Trial(TrialEvent::Canceled { trial_id: 999 }));
        assert_eq!((s.jury_court, s.jury_number), (Some(u32::MAX), Some(-1)));
        assert!(!s.is_jury);
        s.apply(&Event::Disconnected { message: String::new() });
        assert_eq!((s.jury_court, s.jury_number), (None, None));
    }
}
