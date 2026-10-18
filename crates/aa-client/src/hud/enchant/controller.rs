//! Source-qualified result/input rendezvous. Receipt handlers own admission.
use super::{Mode, records::RowOutput, selection::Prepared};
use aa_net::item_effect::ItemEffectRequest;

#[derive(Clone, Copy)]
pub(in crate::hud) enum ResultKind {
    Grade,
    Socket,
    Magical,
}
impl ResultKind {
    fn matches(self, mode: Mode) -> bool {
        matches!(
            (self, mode),
            (Self::Grade, Mode::Grade) | (Self::Socket, Mode::Socket | Mode::Unsocket) | (Self::Magical, Mode::Gem)
        )
    }
}

pub(super) struct Operation {
    pub character_id: u32,
    pub request: ItemEffectRequest,
    written: bool,
    input_ready: bool,
    result_seen: bool,
}
impl Operation {
    pub fn new(prepared: Prepared) -> Self {
        Self {
            character_id: prepared.character_id,
            request: prepared.request,
            written: false,
            input_ready: prepared.input_ready,
            result_seen: false,
        }
    }
    pub fn matches(&self, character_id: u32, request: &ItemEffectRequest) -> bool {
        self.character_id == character_id && self.request == *request
    }
    pub fn result_seen(&self) -> bool {
        self.result_seen
    }
    pub fn requested(&mut self, character_id: u32, request: &ItemEffectRequest) -> bool {
        if !self.matches(character_id, request) {
            return false;
        }
        self.written = true;
        true
    }
    pub fn result(&mut self, mode: Mode, kind: ResultKind) -> bool {
        // Explicit app transport fence: only an actually written current request
        // arms the native receiver. A queued command is not a write or an ACK.
        if !self.written || !kind.matches(mode) {
            return false;
        }
        //39046430 uses CURRENT mode; the latch means an admitted result receipt,
        // regardless of its raw success/failure value.
        self.result_seen = true;
        self.input_ready
    }
    pub fn row(&mut self, mode: Mode, reason: u8, output: &RowOutput) -> bool {
        if !self.written
            || mode == Mode::Off
            || reason != 40
            || !output.admitted
            || output
                .item
                .as_ref()
                .is_none_or(|item| item.template_id != self.request.source().item.template_id)
        {
            return false;
        }
        // Actual admitted PRE-row-derived output, not persistent ID, packet
        // template or the legacy post-batch inventory, controls this rendezvous.
        self.input_ready = true;
        self.result_seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::{
        item_effect::{CapturedBagItem, ItemEffectMode},
        model::Item,
    };

    fn prepared(unsocket: bool) -> Prepared {
        Prepared {
            character_id: 47,
            input_ready: unsocket,
            request: ItemEffectRequest::new(
                if unsocket {
                    ItemEffectMode::Socket
                } else {
                    ItemEffectMode::Grade { auto_use_aa_point: false }
                },
                if unsocket { 23729 } else { 22522 },
                CapturedBagItem {
                    slot: 1,
                    item: Item {
                        id: 1,
                        template_id: if unsocket { 327 } else { 28299 },
                        count: 3,
                        ..Default::default()
                    },
                },
                CapturedBagItem {
                    slot: 0,
                    item: Item {
                        id: 2,
                        template_id: 31432,
                        count: 1,
                        ..Default::default()
                    },
                },
                None,
            )
            .unwrap(),
        }
    }
    fn written() -> Operation {
        let prepared = prepared(false);
        let request = prepared.request.clone();
        let mut operation = Operation::new(prepared);
        assert!(operation.requested(47, &request));
        operation
    }
    fn output(admitted: bool, template_id: u32) -> RowOutput {
        RowOutput {
            admitted,
            item: Some(Item {
                id: 999,
                template_id,
                count: 0,
                ..Default::default()
            }),
        }
    }
    #[test]
    fn either_order_completes_only_after_actual_write_matching_current_mode_and_admitted_output() {
        for result_first in [false, true] {
            let mut operation = written();
            if result_first {
                assert!(!operation.result(Mode::Grade, ResultKind::Grade));
                assert!(operation.row(Mode::Grade, 40, &output(true, 28299)));
            } else {
                assert!(!operation.row(Mode::Grade, 40, &output(true, 28299)));
                assert!(operation.result(Mode::Grade, ResultKind::Grade));
            }
        }
        let prepared = prepared(false);
        let request = prepared.request.clone();
        let mut operation = Operation::new(prepared);
        assert!(!operation.requested(48, &request));
        assert!(!operation.row(Mode::Grade, 40, &output(true, 28299)));
        assert!(!operation.result(Mode::Grade, ResultKind::Grade));
        assert!(operation.requested(47, &request));
        assert!(!operation.result(Mode::Grade, ResultKind::Grade));
        for (mode, reason, row) in [
            (Mode::Off, 40, output(true, 28299)),
            (Mode::Grade, 39, output(true, 28299)),
            (Mode::Grade, 40, output(false, 28299)),
            (Mode::Grade, 40, output(true, 31003)),
            (
                Mode::Grade,
                40,
                RowOutput {
                    admitted: true,
                    item: None,
                },
            ),
        ] {
            assert!(!operation.row(mode, reason, &row));
        }
        assert!(
            operation.row(Mode::Grade, 40, &output(true, 28299)),
            "different ID and zero count do not replace the native template test"
        );
    }
    #[test]
    fn result_admission_uses_current_mode_and_nonreagent_source_starts_input_ready() {
        let mut operation = written();
        assert!(!operation.result(Mode::Grade, ResultKind::Socket));
        assert!(!operation.result(Mode::Off, ResultKind::Grade));
        assert!(!operation.result(Mode::Gem, ResultKind::Magical));
        assert!(
            operation.row(Mode::Socket, 40, &output(true, 28299)),
            "operation does not freeze its starting mode"
        );
        let prepared = prepared(true);
        let request = prepared.request.clone();
        let mut operation = Operation::new(prepared);
        assert!(!operation.result(Mode::Unsocket, ResultKind::Socket));
        assert!(operation.requested(47, &request));
        assert!(operation.result(Mode::Unsocket, ResultKind::Socket));
    }
}
