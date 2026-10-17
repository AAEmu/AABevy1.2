//! Bank money/capacity requests. The bank's slots/items are already in `Inventory` and its
//! balance in `PlayerStats`; bank item moves use the existing `SwapItems` / `SplitItem` requests.
//! Opening the original bank window is a local service interaction (banker skill 13238).

use crate::opcodes::cs;
use crate::wire::Writer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BankCommand {
    DepositMoney {
        amount: i32,
        aa_point: i32,
    },
    WithdrawMoney {
        amount: i32,
        aa_point: i32,
    },
    /// Container 2 is the bag, 3 the bank. The server confirms with `InventoryExpanded`.
    Expand {
        container: u8,
        auto_use_aa_point: bool,
    },
}

impl BankCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::DepositMoney { amount, aa_point } | Self::WithdrawMoney { amount, aa_point } => {
                if *amount < 0 || *aa_point < 0 || (*amount == 0 && *aa_point == 0) {
                    return Err("invalid bank amount");
                }
            }
            Self::Expand { container, .. } if !matches!(container, 2 | 3) => {
                return Err("invalid expandable container");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::DepositMoney { .. } => cs::CSDepositMoneyPacket,
            Self::WithdrawMoney { .. } => cs::CSWithdrawMoneyPacket,
            Self::Expand { .. } => cs::CSExpandSlotsPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::DepositMoney { amount, aa_point } | Self::WithdrawMoney { amount, aa_point } => {
                w.i32(*amount).i32(*aa_point);
            }
            Self::Expand { container, auto_use_aa_point } => {
                w.i32(i32::from(*container)).bool(*auto_use_aa_point);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;

    #[test]
    fn bank_requests_match_server_readers_and_reject_invalid_values() {
        let requests = [
            (BankCommand::DepositMoney { amount: 258, aa_point: 0 }, 0x047, vec![2, 1, 0, 0, 0, 0, 0, 0]),
            (BankCommand::WithdrawMoney { amount: 5, aa_point: 1 }, 0x048, vec![5, 0, 0, 0, 1, 0, 0, 0]),
            (BankCommand::Expand { container: 3, auto_use_aa_point: false }, 0x041, vec![3, 0, 0, 0, 0]),
        ];
        for (command, opcode, body) in requests {
            assert!(command.validate().is_ok());
            let packet = ClientMsg::Bank(command);
            assert_eq!(packet.opcode(), opcode);
            assert_eq!(packet.encode_body(), body);
        }
        assert!(BankCommand::DepositMoney { amount: -1, aa_point: 0 }.validate().is_err());
        assert!(BankCommand::WithdrawMoney { amount: 0, aa_point: 0 }.validate().is_err());
        assert!(BankCommand::Expand { container: 1, auto_use_aa_point: false }.validate().is_err());
    }

    #[test]
    fn bank_expansion_preserves_items_and_compressed_packet_alignment() {
        use crate::game::ServerMsg;
        use crate::model::Item;
        use crate::opcodes::sc;
        use crate::session::Event;
        use crate::state::Inventory;

        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCInvenExpandedPacket).u8(3).u8(60).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&raw);
        let ServerMsg::Batch(messages) = ServerMsg::decode(&frame.finish()).unwrap() else { panic!() };
        assert_eq!(messages, vec![ServerMsg::InvenExpanded { container: 3, slots: 60 }, ServerMsg::Snow(true)]);

        let mut inventory = Inventory::default();
        inventory.apply(&Event::InventoryInit { bag_slots: 50, bank_slots: 50 });
        inventory.bank[49] = Some(Item { id: 555, template_id: 8001, count: 3, ..Item::default() });
        inventory.apply(&Event::InventoryExpanded { container: 3, slots: 60 });
        assert_eq!((inventory.bank_slots, inventory.bank.len()), (60, 60));
        assert_eq!(inventory.bank_items().next().unwrap().0, 49);
        assert_eq!(inventory.item_at(3, 49).unwrap().id, 555);
        assert_eq!(inventory.first_free_bank_slot(), Some(0));
        assert!(inventory.plan_move((3, 49), (3, 59)).is_some());
        assert!(inventory.plan_move((3, 49), (3, 60)).is_none());
    }
}
