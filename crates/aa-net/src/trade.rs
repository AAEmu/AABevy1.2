//! Player-to-player trade. Offers are whole bag stacks; balances/items change only at settlement.

use crate::items::ItemTasks;
use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::state::Inventory;
use crate::wire::{Reader, Result, WireError, Writer};

pub const MAX_OFFERS: usize = 10;
pub const TRADE_TASK: u8 = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeCommand {
    Invite {
        obj_id: u32,
    },
    Accept {
        obj_id: u32,
    },
    Decline {
        obj_id: u32,
        reason: i32,
    },
    Cancel {
        reason: i32,
    },
    /// Whole physical bag stack only. Split before offering a partial stack.
    OfferItem {
        slot: u8,
    },
    RemoveItem {
        slot: u8,
    },
    OfferMoney {
        amount: i32,
    },
    Lock {
        locked: bool,
    },
    Confirm,
}

impl TradeCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Invite { obj_id } | Self::Accept { obj_id } | Self::Decline { obj_id, .. }
                if *obj_id == 0 || *obj_id > 0xff_ffff =>
            {
                Err("invalid trade character object id")
            }
            Self::OfferMoney { amount } if *amount < 0 => Err("negative trade money"),
            _ => Ok(()),
        }
    }
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Invite { .. } => cs::CSCanStartTradePacket,
            Self::Accept { .. } => cs::CSStartTradePacket,
            Self::Decline { .. } => cs::CSCannotStartTradePacket,
            Self::Cancel { .. } => cs::CSCancelTradePacket,
            Self::OfferItem { .. } => cs::CSPutupTradeItemPacket,
            Self::RemoveItem { .. } => cs::CSTakedownTradeItemPacket,
            Self::OfferMoney { .. } => cs::CSPutupTradeMoneyPacket,
            Self::Lock { .. } => cs::CSTradeLockPacket,
            Self::Confirm => cs::CSTradeOkPacket,
        }
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Invite { obj_id } | Self::Accept { obj_id } => {
                w.bc(*obj_id);
            }
            Self::Decline { obj_id, reason } => {
                w.bc(*obj_id).i32(*reason);
            }
            Self::Cancel { reason } => {
                w.i32(*reason);
            }
            Self::OfferItem { slot } => {
                w.u8(2).u8(*slot).i32(0);
            }
            Self::RemoveItem { slot } => {
                w.u8(2).u8(*slot);
            }
            Self::OfferMoney { amount } => {
                w.i32(*amount);
            }
            Self::Lock { locked } => {
                w.bool(*locked);
            }
            Self::Confirm => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TradeEvent {
    Invited {
        obj_id: u32,
    },
    Declined {
        obj_id: u32,
        reason: i32,
    },
    Started {
        obj_id: u32,
    },
    Canceled {
        reason: i32,
        caused_by_me: bool,
    },
    ItemOffered {
        container: u8,
        slot: u8,
        amount: i32,
    },
    OtherItemOffered(Item),
    MoneyOffered(i32),
    OtherMoneyOffered(i32),
    ItemRemoved {
        container: u8,
        slot: u8,
    },
    OtherItemRemoved(Item),
    Confirmed {
        mine: bool,
        other: bool,
    },
    Locked {
        mine: bool,
        other: bool,
    },
    /// The session emits ItemsChanged with these tasks before emitting Made.
    Made(ItemTasks),
    Requested(TradeCommand),
    RequestRejected {
        command: TradeCommand,
        message: String,
    },
}

impl TradeEvent {
    pub fn handles(opcode: u16) -> bool {
        (0x15e..=0x16a).contains(&opcode)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Invited { .. } => "SCCanStartTrade",
            Self::Declined { .. } => "SCCannotStartTrade",
            Self::Started { .. } => "SCTradeStarted",
            Self::Canceled { .. } => "SCTradeCanceled",
            Self::ItemOffered { .. } => "SCTradeItemPutup",
            Self::OtherItemOffered(_) => "SCOtherTradeItemPutup",
            Self::MoneyOffered(_) => "SCTradeMoneyPutup",
            Self::OtherMoneyOffered(_) => "SCOtherTradeMoneyPutup",
            Self::ItemRemoved { .. } => "SCTradeItemTookdown",
            Self::OtherItemRemoved(_) => "SCOtherTradeItemTookdown",
            Self::Confirmed { .. } => "SCTradeOkUpdate",
            Self::Locked { .. } => "SCTradeLockUpdate",
            Self::Made(_) => "SCTradeMade",
            Self::Requested(_) => "TradeRequested",
            Self::RequestRejected { .. } => "TradeRequestRejected",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        let item = |r: &mut Reader| {
            Item::read(r)?.ok_or_else(|| WireError::Invalid("empty trade offer item".into()))
        };
        Ok(match opcode {
            sc::SCCanStartTradePacket => Self::Invited { obj_id: r.bc()? },
            sc::SCCannotStartTradePacket => Self::Declined {
                obj_id: r.bc()?,
                reason: r.i32()?,
            },
            sc::SCTradeStartedPacket => Self::Started { obj_id: r.bc()? },
            sc::SCTradeCanceledPacket => Self::Canceled {
                reason: r.i32()?,
                caused_by_me: r.bool()?,
            },
            sc::SCTradeItemPutupPacket => Self::ItemOffered {
                container: r.u8()?,
                slot: r.u8()?,
                amount: r.i32()?,
            },
            sc::SCOtherTradeItemPutupPacket => Self::OtherItemOffered(item(r)?),
            sc::SCTradeMoneyPutupPacket => Self::MoneyOffered(r.i32()?),
            sc::SCOtherTradeMoneyPutupPacket => Self::OtherMoneyOffered(r.i32()?),
            sc::SCTradeItemTookdownPacket => Self::ItemRemoved {
                container: r.u8()?,
                slot: r.u8()?,
            },
            sc::SCOtherTradeItemTookdownPacket => Self::OtherItemRemoved(item(r)?),
            sc::SCTradeOkUpdatePacket => Self::Confirmed {
                mine: r.bool()?,
                other: r.bool()?,
            },
            sc::SCTradeLockUpdatePacket => Self::Locked {
                mine: r.bool()?,
                other: r.bool()?,
            },
            sc::SCTradeMadePacket => {
                let tasks = ItemTasks::read(r)?;
                if tasks.truncated {
                    return Err(WireError::Invalid(
                        "truncated trade settlement tasks".into(),
                    ));
                }
                r.u32()?;
                Self::Made(tasks)
            }
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown trade opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeOffer {
    pub container: u8,
    pub slot: u8,
    /// Raw own-offer ACK amount; zero means the entire stack. Settlement is always whole-stack.
    pub amount: i32,
    /// Snapshot of exact identity from the authoritative inventory when the offer was ACKed.
    pub item: Option<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeFailure {
    pub command: Option<TradeCommand>,
    pub reason: Option<i32>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Trade {
    pub active: bool,
    pub partner_obj_id: u32,
    pub incoming_invite: Option<u32>,
    pub outgoing_invite: Option<u32>,
    pub mine: Vec<TradeOffer>,
    pub other: Vec<Item>,
    pub my_money: i32,
    pub other_money: i32,
    pub my_locked: bool,
    pub other_locked: bool,
    pub my_ok: bool,
    pub other_ok: bool,
    pub pending: Vec<TradeCommand>,
    pub last_error: Option<TradeFailure>,
    pub last_canceled: Option<(i32, bool)>,
    pub last_settlement: Option<ItemTasks>,
    /// Offer edits made while locked cause the lane to clear OK as well as locks.
    edit_unlock_pending: bool,
}

impl Trade {
    /// Native UI requires both locks, although this lane's old C# guard requires only one.
    pub fn can_confirm(&self) -> bool {
        self.active && self.my_locked && self.other_locked && !self.my_ok
    }

    /// Full-stack offer preflight. Slot is a physical bag index, not a sorted UI index.
    pub fn plan_offer(
        &self,
        inventory: &Inventory,
        slot: usize,
    ) -> std::result::Result<TradeCommand, &'static str> {
        if !self.active {
            return Err("trade is not active");
        }
        if self.my_locked {
            return Err("trade is locked");
        }
        if self.my_ok || self.other_ok {
            return Err("cancel confirmed trade before changing offers");
        }
        let item = inventory.item_at(2, slot).ok_or("empty bag slot")?;
        if item.id == 0 || item.count <= 0 {
            return Err("invalid trade item");
        }
        if item.flags & 1 != 0 {
            return Err("bound item cannot be traded");
        }
        if self.mine.len() >= MAX_OFFERS {
            return Err("trade cart is full");
        }
        if self.mine.iter().any(|offer| {
            offer.item.as_ref().is_some_and(|old| old.id == item.id)
                || (offer.container == 2 && offer.slot as usize == slot)
        }) {
            return Err("item already offered");
        }
        Ok(TradeCommand::OfferItem {
            slot: u8::try_from(slot).map_err(|_| "bag slot exceeds wire index")?,
        })
    }

    /// Own item ACKs carry no item ID. Seed the snapshot from inventory before a later update.
    pub(crate) fn seed_inventory(&mut self, event: &Event, inventory: &Inventory) -> bool {
        let Event::Trade(TradeEvent::ItemOffered {
            container, slot, ..
        }) = event
        else {
            return false;
        };
        let Some(offer) = self.mine.iter_mut().rev().find(|offer| {
            offer.container == *container && offer.slot == *slot && offer.item.is_none()
        }) else {
            return false;
        };
        offer.item = inventory.item_at(*container, *slot as usize).cloned();
        offer.item.is_some()
    }

    fn edited(&mut self) {
        self.edit_unlock_pending |= self.my_locked || self.other_locked;
    }
    fn complete(&mut self, pick: impl Fn(&TradeCommand) -> bool) {
        self.pending.retain(|p| !pick(p));
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Trade(event) = event else {
            if matches!(
                event,
                Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. }
            ) {
                let changed = *self != Self::default();
                *self = Self::default();
                return changed;
            }
            return false;
        };
        match event {
            TradeEvent::Requested(command) => {
                match command {
                    TradeCommand::Invite { obj_id } => self.outgoing_invite = Some(*obj_id),
                    TradeCommand::Decline { .. } => self.incoming_invite = None,
                    _ => {}
                }
                // The declining player has no server reply; Requested is the local send notice.
                if !matches!(command, TradeCommand::Decline { .. })
                    && !self.pending.contains(command)
                {
                    self.pending.push(command.clone());
                }
                self.last_error = None;
            }
            TradeEvent::RequestRejected { command, message } => {
                self.last_error = Some(TradeFailure {
                    command: Some(command.clone()),
                    reason: None,
                    message: message.clone(),
                })
            }
            TradeEvent::Invited { obj_id } => self.incoming_invite = Some(*obj_id),
            TradeEvent::Declined { obj_id, reason } => {
                if self.outgoing_invite == Some(*obj_id) {
                    self.outgoing_invite = None;
                }
                self.complete(|p| matches!(p,TradeCommand::Invite{obj_id:id} if id==obj_id));
                self.last_error = Some(TradeFailure {
                    command: Some(TradeCommand::Invite { obj_id: *obj_id }),
                    reason: Some(*reason),
                    message: format!("trade invitation declined ({reason})"),
                });
            }
            TradeEvent::Started { obj_id } => {
                *self = Self {
                    active: true,
                    partner_obj_id: *obj_id,
                    ..Self::default()
                };
            }
            TradeEvent::Canceled {
                reason,
                caused_by_me,
            } => {
                *self = Self {
                    last_canceled: Some((*reason, *caused_by_me)),
                    ..Self::default()
                };
            }
            TradeEvent::Made(tasks) => {
                *self = Self {
                    last_settlement: Some(tasks.clone()),
                    ..Self::default()
                };
            }
            TradeEvent::ItemOffered {
                container,
                slot,
                amount,
            } if self.active => {
                self.edited();
                self.mine.push(TradeOffer {
                    container: *container,
                    slot: *slot,
                    amount: *amount,
                    item: None,
                });
                self.complete(
                    |p| matches!(p,TradeCommand::OfferItem{slot:s} if *container==2 && s==slot),
                );
            }
            TradeEvent::OtherItemOffered(item) if self.active => {
                self.edited();
                self.other.push(item.clone());
            }
            TradeEvent::ItemRemoved { container, slot } if self.active => {
                self.edited();
                self.mine
                    .retain(|offer| offer.container != *container || offer.slot != *slot);
                self.complete(
                    |p| matches!(p,TradeCommand::RemoveItem{slot:s} if *container==2 && s==slot),
                );
            }
            TradeEvent::OtherItemRemoved(item) if self.active => {
                self.edited();
                self.other.retain(|old| old.id != item.id);
            }
            TradeEvent::MoneyOffered(amount) if self.active => {
                self.edited();
                self.my_money = *amount;
                self.complete(|p| matches!(p,TradeCommand::OfferMoney{amount:a} if a==amount));
            }
            TradeEvent::OtherMoneyOffered(amount) if self.active => {
                self.edited();
                self.other_money = *amount;
            }
            TradeEvent::Locked { mine, other } if self.active => {
                self.my_locked = *mine;
                self.other_locked = *other;
                if !mine && !other && self.edit_unlock_pending {
                    self.my_ok = false;
                    self.other_ok = false;
                    self.edit_unlock_pending = false;
                }
                self.complete(|p| matches!(p,TradeCommand::Lock{locked} if locked==mine));
            }
            TradeEvent::Confirmed { mine, other } if self.active => {
                self.my_ok = *mine;
                self.other_ok = *other;
                if *mine {
                    self.complete(|p| matches!(p, TradeCommand::Confirm));
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::items::{ItemTask, action};
    use crate::state::WorldState;
    fn bytes(write: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        write(&mut w);
        w.finish()
    }
    fn item(w: &mut Writer, id: u64) {
        w.u32(100)
            .u64(id)
            .u8(2)
            .u8(0)
            .i32(7)
            .u8(0)
            .i64(1)
            .i32(0)
            .u32(0)
            .u8(1)
            .i64(0)
            .i64(0);
    }
    fn batch(packets: &[(u16, Vec<u8>)]) -> Result<Vec<ServerMsg>> {
        let mut w = Writer::new();
        for (op, body) in packets {
            w.u16(0).u16(*op).raw(body);
        }
        w.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let raw = w.finish();
        let len = raw.len() as u16;
        let mut w = Writer::new();
        w.raw(&[0xdd, 4])
            .u16(packets.len() as u16 + 1)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
        match ServerMsg::decode(&w.finish())? {
            ServerMsg::Batch(batch) => Ok(batch),
            _ => panic!(),
        }
    }
    #[test]
    fn all_trade_commands_have_exact_wire_shapes_and_whole_stack_sentinel() {
        let cases = vec![
            (
                TradeCommand::Invite { obj_id: 0x010203 },
                0xed,
                vec![3, 2, 1],
            ),
            (
                TradeCommand::Accept { obj_id: 0x010203 },
                0xec,
                vec![3, 2, 1],
            ),
            (
                TradeCommand::Decline {
                    obj_id: 0x010203,
                    reason: 4,
                },
                0xee,
                vec![3, 2, 1, 4, 0, 0, 0],
            ),
            (TradeCommand::Cancel { reason: 5 }, 0xef, vec![5, 0, 0, 0]),
            (
                TradeCommand::OfferItem { slot: 7 },
                0xf0,
                vec![2, 7, 0, 0, 0, 0],
            ),
            (
                TradeCommand::OfferMoney { amount: 9 },
                0xf1,
                vec![9, 0, 0, 0],
            ),
            (TradeCommand::RemoveItem { slot: 7 }, 0xf2, vec![2, 7]),
            (TradeCommand::Lock { locked: true }, 0xf3, vec![1]),
            (TradeCommand::Confirm, 0xf4, vec![]),
        ];
        for (command, op, raw) in cases {
            assert!(command.validate().is_ok());
            let packet = ClientMsg::Trade(command);
            assert_eq!((packet.opcode(), packet.encode_body()), (op, raw));
        }
        assert!(TradeCommand::OfferMoney { amount: -1 }.validate().is_err());
        assert!(
            TradeCommand::Invite { obj_id: 0x1000000 }
                .validate()
                .is_err()
        );
    }
    #[test]
    fn all_trade_packets_leave_the_next_dd04_opcode_aligned() {
        let packets = vec![
            (0x15e, vec![3, 2, 1]),
            (0x15f, vec![3, 2, 1, 5, 0, 0, 0]),
            (0x160, vec![3, 2, 1]),
            (0x161, vec![5, 0, 0, 0, 1]),
            (0x162, vec![2, 7, 0, 0, 0, 0]),
            (0x163, bytes(|w| item(w, 500))),
            (0x164, vec![10, 0, 0, 0]),
            (0x165, vec![20, 0, 0, 0]),
            (0x166, vec![2, 7]),
            (0x167, bytes(|w| item(w, 500))),
            (0x168, vec![1, 0]),
            (0x169, vec![1, 1]),
            (
                0x16a,
                bytes(|w| {
                    w.u8(TRADE_TASK)
                        .u8(2)
                        .u8(action::CHANGE_MONEY_AMOUNT)
                        .i32(5)
                        .u8(action::REMOVE)
                        .u8(2)
                        .u8(3)
                        .u64(500)
                        .u32(100)
                        .u8(1)
                        .u64(600)
                        .u32(0);
                }),
            ),
        ];
        let messages = batch(&packets).unwrap();
        assert_eq!(messages.len(), 14);
        assert_eq!(messages.last(), Some(&ServerMsg::Snow(true)));
        for (i, (op, raw)) in packets.iter().enumerate() {
            let mut r = Reader::new(raw);
            let expected = TradeEvent::read(*op, &mut r).unwrap();
            assert!(r.is_empty());
            assert_eq!(messages[i], ServerMsg::Trade(expected));
        }
        let ServerMsg::Trade(TradeEvent::Made(tasks)) = &messages[12] else {
            panic!()
        };
        assert_eq!(tasks.tasks[0], ItemTask::Money(5));
        assert_eq!(tasks.force_remove, vec![600]);
    }
    #[test]
    fn truncated_or_empty_trade_items_and_settlements_fail_strictly() {
        assert!(TradeEvent::read(0x163, &mut Reader::new(&[0; 4])).is_err());
        let raw = bytes(|w| item(w, 500));
        for n in 0..raw.len() {
            assert!(TradeEvent::read(0x163, &mut Reader::new(&raw[..n])).is_err());
        }
        let raw = bytes(|w| {
            w.u8(TRADE_TASK)
                .u8(1)
                .u8(action::CHANGE_MONEY_AMOUNT)
                .i32(5)
                .u8(0)
                .u32(0);
        });
        for n in 0..raw.len() {
            assert!(
                TradeEvent::read(0x16a, &mut Reader::new(&raw[..n])).is_err(),
                "cut{n}"
            );
        }
        assert!(TradeEvent::read(0x16a, &mut Reader::new(&[48, 255, 255])).is_err());
        assert!(matches!(batch(&[(0x16a, vec![48, 1, 255])]).unwrap().as_slice(),[crate::game::ServerMsg::DecodeFailed{..}, crate::game::ServerMsg::Snow(true)]));
    }
    fn apply(trade: &mut Trade, event: TradeEvent) {
        trade.apply(&Event::Trade(event));
    }
    #[test]
    fn edit_ack_then_unlock_invalidates_consent_but_explicit_unlock_preserves_lane_quirk() {
        let mut trade = Trade::default();
        apply(&mut trade, TradeEvent::Started { obj_id: 3 });
        apply(
            &mut trade,
            TradeEvent::Locked {
                mine: true,
                other: true,
            },
        );
        apply(
            &mut trade,
            TradeEvent::Confirmed {
                mine: true,
                other: false,
            },
        );
        apply(
            &mut trade,
            TradeEvent::Requested(TradeCommand::OfferMoney { amount: 7 }),
        );
        assert!(trade.my_locked && trade.my_ok);
        assert_eq!(trade.my_money, 0);
        apply(&mut trade, TradeEvent::MoneyOffered(7));
        assert!(trade.my_locked && trade.my_ok);
        apply(
            &mut trade,
            TradeEvent::Locked {
                mine: false,
                other: false,
            },
        );
        assert!(!trade.my_ok && !trade.other_ok && !trade.can_confirm());
        apply(
            &mut trade,
            TradeEvent::Locked {
                mine: true,
                other: true,
            },
        );
        assert!(trade.can_confirm());
        apply(
            &mut trade,
            TradeEvent::Confirmed {
                mine: true,
                other: false,
            },
        );
        apply(
            &mut trade,
            TradeEvent::Locked {
                mine: false,
                other: false,
            },
        );
        assert!(trade.my_ok);
        apply(&mut trade, TradeEvent::OtherMoneyOffered(8));
        assert!(
            trade.my_ok,
            "unlocked edit does not emit a reset in this lane"
        );
        apply(
            &mut trade,
            TradeEvent::Canceled {
                reason: 0,
                caused_by_me: false,
            },
        );
        assert!(!trade.active && trade.last_canceled == Some((0, false)));
    }
    #[test]
    fn own_offer_identity_is_seeded_and_full_stack_preflight_prevents_duplicates() {
        let mut state = WorldState::default();
        state.inventory.bag = vec![None; 8];
        state.inventory.bag[7] = Some(Item {
            id: 500,
            template_id: 100,
            count: 7,
            ..Item::default()
        });
        state.apply(&Event::Trade(TradeEvent::Started { obj_id: 3 }));
        assert_eq!(
            state.trade.plan_offer(&state.inventory, 7).unwrap(),
            TradeCommand::OfferItem { slot: 7 }
        );
        state.apply(&Event::Trade(TradeEvent::Requested(
            TradeCommand::OfferItem { slot: 7 },
        )));
        assert!(state.trade.mine.is_empty());
        state.apply(&Event::Trade(TradeEvent::ItemOffered {
            container: 2,
            slot: 7,
            amount: 0,
        }));
        assert_eq!(
            state.trade.mine[0].item.as_ref().map(|i| (i.id, i.count)),
            Some((500, 7))
        );
        assert!(state.trade.plan_offer(&state.inventory, 7).is_err());
        assert_eq!(state.inventory.bag[7].as_ref().unwrap().count, 7);
        state.apply(&Event::Trade(TradeEvent::OtherItemOffered(Item {
            id: 600,
            template_id: 101,
            count: 9,
            ..Item::default()
        })));
        assert_eq!(state.trade.other[0].id, 600);
        state.apply(&Event::Trade(TradeEvent::OtherItemRemoved(Item {
            id: 600,
            ..Item::default()
        })));
        assert!(state.trade.other.is_empty());
        state.apply(&Event::Trade(TradeEvent::ItemRemoved {
            container: 2,
            slot: 7,
        }));
        assert!(state.trade.mine.is_empty());
        state.inventory.bag[7].as_mut().unwrap().flags = 1;
        assert!(state.trade.plan_offer(&state.inventory, 7).is_err());
    }
    #[test]
    fn settlement_and_lifecycle_preserve_exact_authoritative_money_and_item_id() {
        let mut state = WorldState::default();
        state.stats.money = 100;
        state.inventory.bag = vec![None; 8];
        state.inventory.bag[7] = Some(Item {
            id: 500,
            template_id: 100,
            count: 7,
            ..Item::default()
        });
        state.apply(&Event::Trade(TradeEvent::Started { obj_id: 3 }));
        let tasks = ItemTasks {
            reason: 48,
            tasks: vec![
                ItemTask::Money(5),
                ItemTask::Remove {
                    container: 2,
                    slot: 1,
                    item_id: 500,
                    template_id: 100,
                },
                ItemTask::Add { raw_detail_block: None,
                    action: action::CHANGE_OWNER,
                    container: 2,
                    slot: 4,
                    item: Item {
                        id: 600,
                        template_id: 101,
                        count: 9,
                        ..Item::default()
                    },
                },
            ],
            ..ItemTasks::default()
        };
        state.apply(&Event::ItemsChanged(tasks.clone()));
        state.apply(&Event::Trade(TradeEvent::Made(tasks.clone())));
        assert_eq!(state.stats.money, 105);
        assert!(state.inventory.find_by_id(500).is_none());
        assert_eq!(
            state
                .inventory
                .find_by_id(600)
                .map(|(c, s, i)| (c, s, i.count)),
            Some((2, 4, 9))
        );
        assert!(!state.trade.active && state.trade.last_settlement == Some(tasks));
        state.apply(&Event::Disconnected {
            message: "test".into(),
        });
        assert_eq!(state.trade, Trade::default());
    }
}
