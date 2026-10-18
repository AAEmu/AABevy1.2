//! NPC equipment repair. Item details/money change only through authoritative item tasks.

use crate::items::{ItemTask, container};
use crate::model::ItemDetail;
use crate::opcodes::cs;
use crate::session::Event;
use crate::wire::Writer;

pub const REPAIR_TASK: u8 = 3;
pub const DURABILITY_LOSS_TASK: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairCommand {
    Single {
        container: u8,
        slot: u8,
        auto_use_aa_point: bool,
    },
    /// Equipped items only, in server equipment order. Unaffordable items can be skipped.
    All {
        auto_use_aa_point: bool,
    },
}

impl RepairCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Single { container, .. } if !matches!(*container, container::EQUIPMENT | container::INVENTORY) => {
                Err("repair target must be in equipment or bag")
            }
            _ => Ok(()),
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Single { .. } => cs::CSRepairSingleEquipmentPacket,
            Self::All { .. } => cs::CSRepairAllEquipmentsPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Single { container, slot, auto_use_aa_point } => {
                w.u8(*container).u8(*slot).bool(*auto_use_aa_point);
            }
            Self::All { auto_use_aa_point } => {
                w.bool(*auto_use_aa_point);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepairEvent {
    /// Local notification that a request was sent; not a successful repair.
    Requested(RepairCommand),
    /// Local validation failed, without sending a packet.
    RequestRejected { command: RepairCommand, message: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepairedItem {
    pub container: u8,
    pub slot: u8,
    pub item_id: u64,
    pub durability: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// An empty report is an acknowledgement with no repairs, not a fabricated success.
    pub items: Vec<RepairedItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Repair {
    pub pending: Vec<RepairCommand>,
    pub last_report: Option<RepairReport>,
    pub last_error: Option<String>,
}

impl Repair {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Repair(RepairEvent::Requested(command)) => {
                self.pending.push(*command);
                self.last_report = None;
                self.last_error = None;
            }
            Event::Repair(RepairEvent::RequestRejected { message, .. }) => self.last_error = Some(message.clone()),
            Event::ItemsChanged(tasks) if tasks.reason == REPAIR_TASK && !tasks.truncated => {
                let items = tasks
                    .tasks
                    .iter()
                    .filter_map(|task| {
                        let ItemTask::Update { container, slot, item_id, detail: ItemDetail::Equip(detail), .. } = task
                        else {
                            return None;
                        };
                        Some(RepairedItem {
                            container: *container,
                            slot: *slot,
                            item_id: *item_id,
                            durability: detail.durability,
                        })
                    })
                    .collect();
                self.last_report = Some(RepairReport { items });
                if !self.pending.is_empty() {
                    self.pending.remove(0);
                }
                self.last_error = None;
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if *self == Self::default() {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

/// The lane's f32/ceil price formula. Supply raw-grade refund multiplier and the maximum
/// durability computed from client data; no maximum durability or repair quote is on the wire.
pub fn repair_cost(price: i32, refund_multiplier: i32, factor: f32, current: u8, maximum: u8) -> Option<i32> {
    if maximum == 0 || price < 0 || refund_multiplier < 0 || !factor.is_finite() || factor < 0.0 {
        return None;
    }
    let ratio = 1.0f32 - f32::from(current) / f32::from(maximum);
    let cost = factor * 0.01f32 * ratio * price as f32;
    let cost = cost * refund_multiplier as f32 * 0.01f32;
    let cost = f64::from(cost).ceil() as f32;
    Some(if cost < 0.0 || cost > i32::MAX as f32 { 0 } else { cost as i32 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::items::{ItemTasks, action};
    use crate::model::{EquipDetail, Item};
    use crate::opcodes::sc;
    use crate::state::WorldState;

    #[test]
    fn repair_requests_and_price_match_lane() {
        let packet = ClientMsg::Repair(RepairCommand::Single { container: 2, slot: 7, auto_use_aa_point: true });
        assert_eq!((packet.opcode(), packet.encode_body()), (0x03c, vec![2, 7, 1]));
        let packet = ClientMsg::Repair(RepairCommand::All { auto_use_aa_point: false });
        assert_eq!((packet.opcode(), packet.encode_body()), (0x03d, vec![0]));
        assert!(RepairCommand::Single { container: 3, slot: 0, auto_use_aa_point: false }.validate().is_err());
        assert_eq!(repair_cost(1000, 100, 25.0, 10, 10), Some(0));
        assert_eq!(repair_cost(1000, 100, 25.0, 4, 10), Some(150));
        assert_eq!(repair_cost(1000, 100, 25.0, 0, 10), Some(250));
        assert_eq!(repair_cost(1000, 100, 25.0, 0, 0), None);
    }

    #[test]
    fn repair_detail_and_separate_money_tasks_preserve_dd04_boundary_and_item_identity() {
        let mut raw = Writer::new();
        raw.u16(0).u16(sc::SCItemTaskSuccessPacket);
        raw.u8(83).u8(1).u8(action::CHANGE_MONEY_AMOUNT).i32(-150).u8(0).u32(0);
        raw.u16(0).u16(sc::SCItemTaskSuccessPacket).u8(REPAIR_TASK).u8(1);
        raw.u8(action::UPDATE_DETAIL).u8(1).u8(3).u64(555).i16(128);
        raw.u8(1).u32(777).u8(10).u16(0).u32(4).i64(5).u32(6);
        for gem in 10..17 {
            raw.u32(gem);
        }
        raw.u16(20).u16(30).raw(&[0; 72]).u8(0).u32(0);
        raw.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 4]).u16(3).u8(1).u16(len).u16(!len).raw(&raw);
        let ServerMsg::Batch(messages) = ServerMsg::decode(&frame.finish()).unwrap() else { panic!() };
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2], ServerMsg::Snow(true));
        let mut state = WorldState::default();
        state.stats.money = 1000;
        state.inventory.equipment.slots[3] = Some(Item {
            id: 555,
            template_id: 8001,
            count: 1,
            grade: 2,
            detail: ItemDetail::Equip(EquipDetail { durability: 4, ..EquipDetail::default() }),
            ..Item::default()
        });
        state.apply(&Event::Repair(RepairEvent::Requested(RepairCommand::All { auto_use_aa_point: false })));
        for message in &messages[..2] {
            let ServerMsg::ItemTasks(tasks) = message else { panic!() };
            state.apply(&Event::ItemsChanged(tasks.clone()));
        }
        let item = state.inventory.equipment.slots[3].as_ref().unwrap();
        assert_eq!((item.id, item.template_id, item.grade), (555, 8001, 2));
        let ItemDetail::Equip(detail) = &item.detail else { panic!() };
        assert_eq!((detail.durability, detail.image_item_template_id, detail.rune_id), (10, 777, 4));
        assert_eq!(
            (detail.gem_ids, detail.temper_physical, detail.temper_magical),
            ([10, 11, 12, 13, 14, 15, 16], 20, 30)
        );
        assert_eq!(state.stats.money, 850);
        assert!(state.repair.pending.is_empty());
        assert_eq!(
            state.repair.last_report.unwrap().items,
            vec![RepairedItem { container: 1, slot: 3, item_id: 555, durability: 10 }]
        );
    }

    #[test]
    fn empty_ack_is_distinct_from_no_response_and_wear_does_not_complete_requests() {
        let mut repair = Repair::default();
        let command = RepairCommand::All { auto_use_aa_point: false };
        repair.apply(&Event::Repair(RepairEvent::Requested(command)));
        assert!(repair.last_report.is_none());
        assert!(
            !repair.apply(&Event::ItemsChanged(ItemTasks { reason: DURABILITY_LOSS_TASK, ..ItemTasks::default() }))
        );
        assert_eq!(repair.pending, vec![command]);
        repair.apply(&Event::ItemsChanged(ItemTasks { reason: REPAIR_TASK, ..ItemTasks::default() }));
        assert!(repair.pending.is_empty() && repair.last_report.as_ref().unwrap().items.is_empty());
        repair.apply(&Event::Disconnected { message: "test".into() });
        assert_eq!(repair, Repair::default());
    }

    #[test]
    fn only_complete_repair_reports_finish_requests_and_every_empty_ack_cut_fails() {
        let mut repair = Repair::default();
        let command = RepairCommand::Single {
            container: 2,
            slot: 7,
            auto_use_aa_point: false,
        };
        repair.apply(&Event::Repair(RepairEvent::Requested(command)));
        assert!(!repair.apply(&Event::ItemsChanged(ItemTasks {
            reason: REPAIR_TASK,
            truncated: true,
            ..Default::default()
        })));
        assert_eq!(repair.pending, vec![command]);
        assert!(repair.last_report.is_none());
        let body = [REPAIR_TASK, 0, 0, 0, 0, 0, 0]; // reason, tasks, forced IDs, final u32 type
        for n in 0..body.len() {
            let mut frame = Writer::new();
            frame
                .raw(&[0xdd, 1, 0, 0])
                .u16(sc::SCItemTaskSuccessPacket)
                .raw(&body[..n]);
            assert!(
                ServerMsg::decode(&frame.finish()).is_err(),
                "empty repair ack cut {n}"
            );
        }
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 1, 0, 0])
            .u16(sc::SCItemTaskSuccessPacket)
            .raw(&body);
        let ServerMsg::ItemTasks(tasks) = ServerMsg::decode(&frame.finish()).unwrap() else {
            panic!()
        };
        assert!(!tasks.truncated);
        assert!(repair.apply(&Event::ItemsChanged(tasks)));
        assert!(repair.pending.is_empty());
        assert_eq!(repair.last_report, Some(RepairReport { items: vec![] }));
    }

}
