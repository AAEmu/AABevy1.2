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
    NoRule {
        is_bank: bool,
        step: i32,
    },
    InvalidRule,
    CapacityChanged {
        expected: u32,
        actual: u32,
    },
    NotEnoughMoney {
        required: i32,
        available: i64,
    },
    NotEnoughCertificates {
        item_id: u32,
        required: i32,
        available: i64,
    },
}

impl ExpansionQuote {
    /// Local preflight only. The acknowledgement remains `InventoryExpanded`; missing
    /// requirements are silently ignored by the lane, without a rejection packet.
    pub fn check(&self) -> std::result::Result<(), ExpansionFailure> {
        self.validate_rule()?;
        if self.rule.price != 0 && self.money < i64::from(self.rule.price) {
            return Err(ExpansionFailure::NotEnoughMoney {
                required: self.rule.price,
                available: self.money,
            });
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

    fn validate_rule(&self) -> std::result::Result<(), ExpansionFailure> {
        let (is_bank, step, new_slots) = expansion_layout(self.container, self.current_slots)?;
        if self.rule.is_bank != is_bank
            || self.rule.step != step
            || self.new_slots != new_slots
            || self.rule.price < 0
            || self.rule.item_count < 0
        {
            return Err(ExpansionFailure::InvalidRule);
        }
        Ok(())
    }

    /// Check the current capacity, money and bag certificates immediately before confirmation.
    /// The wire request does not carry the quoted capacity/step, so a stale quote must not send.
    /// This is local preflight only; it does not reserve requirements or acknowledge expansion.
    pub fn recheck(
        &self,
        inventory: &crate::state::Inventory,
        money: i64,
    ) -> std::result::Result<Self, ExpansionFailure> {
        self.validate_rule()?;
        let actual = if self.container == 3 {
            inventory.bank_slots
        } else {
            inventory.bag_slots
        };
        if actual != self.current_slots {
            return Err(ExpansionFailure::CapacityChanged {
                expected: self.current_slots,
                actual,
            });
        }
        let current = Self {
            money,
            owned_certificates: inventory.count_of(self.rule.item_id),
            ..*self
        };
        current.check()?;
        Ok(current)
    }
}

fn expansion_layout(
    container: u8,
    current_slots: u32,
) -> std::result::Result<(bool, i32, u32), ExpansionFailure> {
    let is_bank = match container {
        2 => false,
        3 => true,
        other => return Err(ExpansionFailure::InvalidContainer(other)),
    };
    if !(50..=245).contains(&current_slots) || (current_slots - 50) % 10 != 0 {
        return Err(ExpansionFailure::InvalidCapacity(current_slots));
    }
    Ok((
        is_bank,
        ((current_slots - 50) / 10) as i32,
        current_slots + 10,
    ))
}

/// Select exactly the lane's step `(slots-50)/10` and first matching bank/bag row.
/// Returns costs even when unaffordable so the original confirmation dialog can show them.
pub fn expansion_quote(
    inventory: &crate::state::Inventory,
    container: u8,
    money: i64,
    rules: &[ExpansionRule],
) -> std::result::Result<ExpansionQuote, ExpansionFailure> {
    let current_slots = match container {
        2 => inventory.bag_slots,
        3 => inventory.bank_slots,
        other => return Err(ExpansionFailure::InvalidContainer(other)),
    };
    let (is_bank, step, new_slots) = expansion_layout(container, current_slots)?;
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
        new_slots,
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
    /// Native expansion with an explicitly unset context ID (zero). Container 2 is bag, 3 bank.
    /// `InventoryExpanded` alone confirms capacity; the lane ignores the native context tail.
    Expand {
        container: u8,
        auto_use_aa_point: bool,
    },
    /// Native's additional BC context field. Its navigation-context meaning stays uninterpreted.
    ExpandWithContext {
        container: u8,
        auto_use_aa_point: bool,
        raw_context_obj_id: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BankEvent {
    /// Local socket write only, not a capacity/money acknowledgement.
    Requested(BankCommand),
    RequestRejected {
        command: BankCommand,
        message: String,
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
            Self::Expand { container, .. } | Self::ExpandWithContext { container, .. }
                if !matches!(container, 2 | 3) =>
            {
                return Err("invalid expandable container");
            }
            Self::ExpandWithContext {
                raw_context_obj_id, ..
            } if *raw_context_obj_id > 0xffffff => {
                return Err("expansion context must fit the native 24-bit object ID");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::DepositMoney { .. } => cs::CSDepositMoneyPacket,
            Self::WithdrawMoney { .. } => cs::CSWithdrawMoneyPacket,
            Self::Expand { .. } | Self::ExpandWithContext { .. } => cs::CSExpandSlotsPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::DepositMoney { amount, aa_point } | Self::WithdrawMoney { amount, aa_point } => {
                w.i32(*amount).i32(*aa_point);
            }
            Self::Expand {
                container,
                auto_use_aa_point,
            } => {
                w.i32(i32::from(*container)).bool(*auto_use_aa_point).bc(0);
            }
            Self::ExpandWithContext {
                container,
                auto_use_aa_point,
                raw_context_obj_id,
            } => {
                w.i32(i32::from(*container))
                    .bool(*auto_use_aa_point)
                    .bc(*raw_context_obj_id);
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
            (
                BankCommand::DepositMoney {
                    amount: 258,
                    aa_point: 0,
                },
                0x047,
                vec![2, 1, 0, 0, 0, 0, 0, 0],
            ),
            (
                BankCommand::WithdrawMoney {
                    amount: 5,
                    aa_point: 1,
                },
                0x048,
                vec![5, 0, 0, 0, 1, 0, 0, 0],
            ),
            (
                BankCommand::Expand {
                    container: 3,
                    auto_use_aa_point: false,
                },
                0x041,
                vec![3, 0, 0, 0, 0, 0, 0, 0],
            ),
            (
                BankCommand::ExpandWithContext {
                    container: 2,
                    auto_use_aa_point: true,
                    raw_context_obj_id: 0x123456,
                },
                0x041,
                vec![2, 0, 0, 0, 1, 0x56, 0x34, 0x12],
            ),
            (
                BankCommand::ExpandWithContext {
                    container: 3,
                    auto_use_aa_point: false,
                    raw_context_obj_id: 0xffffff,
                },
                0x041,
                vec![3, 0, 0, 0, 0, 0xff, 0xff, 0xff],
            ),
        ];
        for (command, opcode, body) in requests {
            assert!(command.validate().is_ok());
            let packet = ClientMsg::Bank(command);
            assert_eq!(packet.opcode(), opcode);
            assert_eq!(packet.encode_body(), body);
        }
        assert!(
            BankCommand::DepositMoney {
                amount: -1,
                aa_point: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            BankCommand::WithdrawMoney {
                amount: 0,
                aa_point: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            BankCommand::Expand {
                container: 1,
                auto_use_aa_point: false
            }
            .validate()
            .is_err()
        );
        assert!(
            BankCommand::ExpandWithContext {
                container: 3,
                auto_use_aa_point: false,
                raw_context_obj_id: 0x1000000
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn confirmation_rechecks_capacity_and_live_bag_requirements_without_changing_state() {
        use crate::model::Item;
        use crate::state::Inventory;
        let mut inv = Inventory::default();
        inv.bag_slots = 50;
        inv.bank_slots = 50;
        inv.bag = vec![Some(Item {
            id: 7,
            template_id: 8000025,
            count: 2,
            ..Default::default()
        })];
        inv.bank = vec![Some(Item {
            id: 8,
            template_id: 8000025,
            count: 99,
            ..Default::default()
        })];
        let rules = [ExpansionRule {
            is_bank: true,
            step: 0,
            price: 10,
            item_id: 8000025,
            item_count: 2,
            currency_id: 0,
        }];
        let quote = expansion_quote(&inv, 3, 0, &rules).unwrap();
        assert!(matches!(
            quote.check(),
            Err(ExpansionFailure::NotEnoughMoney { .. })
        ));
        let fresh = quote.recheck(&inv, 10).unwrap();
        assert_eq!((fresh.money, fresh.owned_certificates), (10, 2));
        assert_eq!(
            quote.money, 0,
            "confirmation returns fresh data without mutating its old quote"
        );
        inv.bag[0].as_mut().unwrap().count = 1;
        assert_eq!(
            quote.recheck(&inv, 10),
            Err(ExpansionFailure::NotEnoughCertificates {
                item_id: 8000025,
                required: 2,
                available: 1,
            })
        );
        inv.bag[0].as_mut().unwrap().count = 2;
        inv.bank_slots = 60;
        assert_eq!(
            quote.recheck(&inv, 10),
            Err(ExpansionFailure::CapacityChanged {
                expected: 50,
                actual: 60
            })
        );
        inv.bank_slots = 50;
        assert!(quote.recheck(&inv, 9).is_err());
        for mut bad in [fresh; 6].into_iter().enumerate().map(|(n, mut bad)| {
            match n {
                0 => bad.rule.price = -1,
                1 => bad.rule.item_count = -1,
                2 => bad.rule.is_bank = false,
                3 => bad.rule.step = 1,
                4 => bad.new_slots = 70,
                _ => bad.container = 2,
            }
            bad
        }) {
            assert_eq!(bad.check(), Err(ExpansionFailure::InvalidRule));
            assert_eq!(bad.recheck(&inv, 100), Err(ExpansionFailure::InvalidRule));
            bad.container = 1;
            assert_eq!(bad.check(), Err(ExpansionFailure::InvalidContainer(1)));
        }
        let bad = ExpansionQuote {
            current_slots: 49,
            ..fresh
        };
        assert_eq!(bad.check(), Err(ExpansionFailure::InvalidCapacity(49)));
        assert_eq!(
            (
                inv.bank_slots,
                inv.bag[0].as_ref().unwrap().count,
                inv.bank[0].as_ref().unwrap().count
            ),
            (50, 2, 99)
        );
    }

    #[test]
    fn bank_expansion_preserves_items_and_compressed_packet_alignment() {
        use crate::game::ServerMsg;
        use crate::model::Item;
        use crate::opcodes::sc;
        use crate::session::Event;
        use crate::state::Inventory;

        let mut raw = Writer::new();
        raw.u16(0)
            .u16(sc::SCInvenExpandedPacket)
            .u8(3)
            .u8(60)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(2)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
        let ServerMsg::Batch(messages) = ServerMsg::decode(&frame.finish()).unwrap() else {
            panic!()
        };
        assert_eq!(
            messages,
            vec![
                ServerMsg::InvenExpanded {
                    container: 3,
                    slots: 60
                },
                ServerMsg::Snow(true)
            ]
        );

        let mut inventory = Inventory::default();
        inventory.apply(&Event::InventoryInit {
            bag_slots: 50,
            bank_slots: 50,
        });
        inventory.bank[49] = Some(Item {
            id: 555,
            template_id: 8001,
            count: 3,
            ..Item::default()
        });
        inventory.apply(&Event::InventoryExpanded {
            container: 3,
            slots: 60,
        });
        assert_eq!((inventory.bank_slots, inventory.bank.len()), (60, 60));
        assert_eq!(inventory.bank_items().next().unwrap().0, 49);
        assert_eq!(inventory.item_at(3, 49).unwrap().id, 555);
        assert_eq!(inventory.first_free_bank_slot(), Some(0));
        assert!(inventory.plan_move((3, 49), (3, 59)).is_some());
        assert!(inventory.plan_move((3, 49), (3, 60)).is_none());
        // Native SC04d serializer397aeaf0 writes exactly two u8 fields.
        for n in 0..2 {
            let mut frame = Writer::new();
            frame
                .raw(&[0xdd, 1, 0, 0])
                .u16(sc::SCInvenExpandedPacket)
                .raw(&[3, 60][..n]);
            assert!(ServerMsg::decode(&frame.finish()).is_err());
        }
    }
}
