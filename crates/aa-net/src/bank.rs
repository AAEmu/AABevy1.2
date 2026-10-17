//! Bank money/capacity requests. The bank's slots/items are already in `Inventory` and its
//! balance in `PlayerStats`; bank item moves use the existing `SwapItems` / `SplitItem` requests.
//! Opening the original bank window is a local service interaction (banker skill 13238).

use crate::opcodes::cs;
use crate::wire::Writer;

/// Read-only `bag_expands` row supplied by the application's game data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpansionRule {
    pub is_bank: bool,
    pub step: i32,
    pub price: i32,
    pub item_id: u32,
    pub item_count: i32,
    pub currency_id: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpansionQuote {
    pub container: u8,
    pub current_slots: u32,
    pub new_slots: u32,
    pub rule: ExpansionRule,
    pub money: i64,
    /// Certificates in the bag, including when expanding the bank.
    pub owned_certificates: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpansionFailure {
    InvalidContainer(u8),
    InvalidCapacity(u32),
    NoRule { is_bank: bool, step: i32 },
    InvalidRule,
    NotEnoughMoney { required: i32, available: i64 },
    NotEnoughCertificates { item_id: u32, required: i32, available: i64 },
}

impl ExpansionQuote {
    /// Local preflight only. The acknowledgement remains `InventoryExpanded`; missing
    /// requirements are silently ignored by the lane, without a rejection packet.
    pub fn check(&self) -> std::result::Result<(), ExpansionFailure> {
        if self.rule.price != 0 && self.money < i64::from(self.rule.price) {
            return Err(ExpansionFailure::NotEnoughMoney { required: self.rule.price, available: self.money });
        }
        if self.rule.item_id != 0
            && self.rule.item_count != 0
            && self.owned_certificates < i64::from(self.rule.item_count)
        {
            return Err(ExpansionFailure::NotEnoughCertificates {
                item_id: self.rule.item_id,
                required: self.rule.item_count,
                available: self.owned_certificates,
            });
        }
        Ok(())
    }
}

/// Select exactly the lane's step `(slots-50)/10` and first matching bank/bag row.
/// Returns costs even when unaffordable so the original confirmation dialog can show them.
pub fn expansion_quote(
    inventory: &crate::state::Inventory,
    container: u8,
    money: i64,
    rules: &[ExpansionRule],
) -> std::result::Result<ExpansionQuote, ExpansionFailure> {
    let (is_bank, current_slots) = match container {
        2 => (false, inventory.bag_slots),
        3 => (true, inventory.bank_slots),
        other => return Err(ExpansionFailure::InvalidContainer(other)),
    };
    if !(50..=245).contains(&current_slots) || (current_slots - 50) % 10 != 0 {
        return Err(ExpansionFailure::InvalidCapacity(current_slots));
    }
    let step = ((current_slots - 50) / 10) as i32;
    let rule = *rules
        .iter()
        .find(|r| r.is_bank == is_bank && r.step == step)
        .ok_or(ExpansionFailure::NoRule { is_bank, step })?;
    if rule.price < 0 || rule.item_count < 0 {
        return Err(ExpansionFailure::InvalidRule);
    }
    Ok(ExpansionQuote {
        container,
        current_slots,
        new_slots: current_slots + 10,
        rule,
        money,
        owned_certificates: inventory.count_of(rule.item_id),
    })
}

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
    fn expansion_preflight_exposes_certificate_money_and_current_capacity() {
        use crate::model::Item;
        use crate::state::Inventory;
        let mut inv = Inventory::default();
        inv.bag_slots = 50;
        inv.bank_slots = 60;
        inv.bag = vec![Some(Item { id: 555, template_id: 8000025, count: 2, ..Item::default() })];
        let rules = [
            ExpansionRule { is_bank: false, step: 0, price: 10, item_id: 8000025, item_count: 1, currency_id: 0 },
            ExpansionRule { is_bank: true, step: 1, price: 20, item_id: 8000025, item_count: 3, currency_id: 0 },
        ];
        let quote = expansion_quote(&inv, 2, 9, &rules).unwrap();
        assert_eq!((quote.current_slots, quote.new_slots, quote.owned_certificates), (50, 60, 2));
        assert_eq!(quote.check(), Err(ExpansionFailure::NotEnoughMoney { required: 10, available: 9 }));
        let quote = expansion_quote(&inv, 3, 20, &rules).unwrap();
        assert_eq!(
            quote.check(),
            Err(ExpansionFailure::NotEnoughCertificates { item_id: 8000025, required: 3, available: 2 })
        );
        assert!(expansion_quote(&inv, 2, 10, &rules).unwrap().check().is_ok());
        assert_eq!((inv.bag_slots, inv.bank_slots, inv.bag[0].as_ref().unwrap().count), (50, 60, 2));
        assert_eq!(expansion_quote(&inv, 1, 20, &rules), Err(ExpansionFailure::InvalidContainer(1)));
        inv.bank_slots = 130;
        assert_eq!(expansion_quote(&inv, 3, 20, &rules), Err(ExpansionFailure::NoRule { is_bank: true, step: 8 }));
        inv.bag_slots = 51;
        assert_eq!(expansion_quote(&inv, 2, 20, &rules), Err(ExpansionFailure::InvalidCapacity(51)));
    }

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
