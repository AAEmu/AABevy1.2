//! Coffers (the safes of a house): opening one, its contents and moving items in and out.
//!
//! A coffer is a doodad with a container of its own. The player opens it with `CSCofferInteraction` (the server answers with
//! `SCCofferContentsUpdate`, 30 slots a packet, and refuses with the "in use" message when somebody else has it open) and
//! closes it the same way. Moves between the bag and the coffer are `CSSwapCofferItems` / `CSSplitCofferItem`: the item move
//! block of the bag's moves, with the coffer as container [`container::TRADE`] and the coffer's container id after it. The
//! item tasks that follow name the coffer container the same way, so [`Coffer`] (kept in the player's
//! [`Inventory`](crate::state::Inventory)) is just one more list of slots.

use crate::items::{ItemMove, container};
use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};

/// One `SCCofferContentsUpdate`: the slots from the first one of the packet.
#[derive(Clone, Debug, PartialEq)]
pub struct CofferContents {
    /// The coffer doodad.
    pub obj_id: u32,
    /// The coffer's item container (the id the moves name).
    pub container_id: u64,
    pub slots: Vec<(u8, Option<Item>)>,
}

impl CofferContents {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let obj_id = r.bc()?;
        let container_id = r.u64()?;
        let count = r.u8()?;
        let mut slots = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let slot = r.u8()?;
            slots.push((slot, Item::read(r)?));
        }
        Ok(Self {
            obj_id,
            container_id,
            slots,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CofferEvent {
    Contents(CofferContents),
}

impl CofferEvent {
    pub fn handles(opcode: u16) -> bool {
        opcode == sc::SCCofferContentsUpdatePacket
    }

    pub fn name(&self) -> &'static str {
        "SCCofferContentsUpdate"
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        match opcode {
            sc::SCCofferContentsUpdatePacket => Ok(Self::Contents(CofferContents::read(r)?)),
            _ => Err(WireError::Invalid(format!("coffer opcode {opcode:#x}"))),
        }
    }
}

/// What the player asks of a coffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CofferCommand {
    /// `CSCofferInteraction`: open (`true`) or close the coffer doodad `obj_id`.
    Interact { obj_id: u32, open: bool },
    /// `CSSwapCofferItems`: move or swap two slots, one of them in the coffer `container_id`.
    Swap { mv: ItemMove, container_id: u64 },
    /// `CSSplitCofferItem`: move `count` of a stack.
    Split {
        mv: ItemMove,
        count: i32,
        container_id: u64,
    },
}

impl CofferCommand {
    /// Check only the wire identity and move shape; permissions, range and item restrictions
    /// remain server decisions. This does not predict a successful interaction or transfer.
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Interact { obj_id, .. } => {
                if *obj_id == 0 || *obj_id > 0x00ff_ffff {
                    return Err("coffer interaction needs a nonzero 24-bit doodad ID");
                }
            }
            Self::Swap { mv, container_id }
            | Self::Split {
                mv, container_id, ..
            } => {
                if *container_id == 0 {
                    return Err("coffer move needs a container ID");
                }
                let known = |kind| {
                    matches!(
                        kind,
                        container::EQUIPMENT
                            | container::INVENTORY
                            | container::BANK
                            | container::TRADE
                    )
                };
                if !known(mv.from_container)
                    || !known(mv.to_container)
                    || mv.from_container != container::TRADE && mv.to_container != container::TRADE
                {
                    return Err("coffer moves require known inventory slots and a coffer endpoint");
                }
                if mv.from_container == mv.to_container && mv.from_slot == mv.to_slot {
                    return Err("coffer move has identical endpoints");
                }
                if mv.from_item_id == 0 && mv.to_item_id == 0 {
                    return Err("coffer move has no item");
                }
                if let Self::Split { count, .. } = self {
                    if *count <= 0 || mv.from_item_id == 0 {
                        return Err("coffer split needs a positive count and source item");
                    }
                }
            }
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Interact { .. } => cs::CSCofferInteractionPacket,
            Self::Swap { .. } => cs::CSSwapCofferItemsPacket,
            Self::Split { .. } => cs::CSSplitCofferItemPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Interact { obj_id, open } => {
                w.bc(*obj_id).bool(*open);
            }
            Self::Swap { mv, container_id } => {
                mv.write(w);
                w.u64(*container_id);
            }
            Self::Split {
                mv,
                count,
                container_id,
            } => {
                w.i32(*count);
                mv.write(w);
                w.u64(*container_id);
            }
        }
    }

    /// The command for a move `mv` between the bag and the coffer `container_id`: `count` > 0 moves part of a stack.
    pub fn moving(mv: ItemMove, count: i32, container_id: u64) -> Self {
        if count > 0 {
            Self::Split {
                mv,
                count,
                container_id,
            }
        } else {
            Self::Swap { mv, container_id }
        }
    }
}

/// The coffer the player has open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coffer {
    /// The coffer doodad (0: none open).
    pub obj_id: u32,
    pub container_id: u64,
    /// The slots by index (`None` = empty); as many as the packets so far named.
    pub slots: Vec<Option<Item>>,
}

impl Coffer {
    pub fn is_open(&self) -> bool {
        self.obj_id != 0
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn count(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Takes `e`'s contents in; `true` when something changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        if matches!(
            e,
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. }
        ) || matches!(e, Event::DoodadsRemoved(ids) if ids.contains(&self.obj_id))
        {
            let changed = *self != Self::default();
            self.close();
            return changed;
        }
        let Event::Coffer(CofferEvent::Contents(c)) = e else {
            return false;
        };
        let mut changed = false;
        // another coffer than the one open: its slots replace the old ones
        if self.obj_id != c.obj_id || self.container_id != c.container_id {
            *self = Self {
                obj_id: c.obj_id,
                container_id: c.container_id,
                slots: Vec::new(),
            };
            changed = true;
        }
        for (slot, item) in &c.slots {
            let slot = usize::from(*slot);
            if self.slots.len() <= slot {
                self.slots.resize(slot + 1, None);
                changed = true;
            }
            if self.slots[slot] != *item {
                self.slots[slot] = item.clone();
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64) -> Item {
        Item {
            template_id: 5,
            id,
            count: 1,
            grade: 1,
            ..Item::default()
        }
    }

    #[test]
    fn contents_come_in_pages() {
        let mut coffer = Coffer::default();
        let page = |first: u8, ids: &[u64]| {
            Event::Coffer(CofferEvent::Contents(CofferContents {
                obj_id: 7,
                container_id: 99,
                slots: ids
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (first + i as u8, (*id != 0).then(|| item(*id))))
                    .collect(),
            }))
        };
        assert!(coffer.apply(&page(0, &[1, 0, 3])));
        assert!(coffer.apply(&page(30, &[4])));
        assert_eq!(
            (
                coffer.obj_id,
                coffer.container_id,
                coffer.slots.len(),
                coffer.count()
            ),
            (7, 99, 31, 3)
        );
        assert_eq!(coffer.slots[2].as_ref().map(|i| i.id), Some(3));
        // a later page of the same coffer keeps the rest; another coffer starts over
        assert!(coffer.apply(&page(0, &[0])));
        assert_eq!(coffer.count(), 2);
        let other = Event::Coffer(CofferEvent::Contents(CofferContents {
            obj_id: 8,
            container_id: 100,
            slots: vec![(0, Some(item(9)))],
        }));
        assert!(coffer.apply(&other));
        assert_eq!((coffer.obj_id, coffer.slots.len()), (8, 1));
        coffer.close();
        assert!(!coffer.is_open());
    }

    #[test]
    fn moves_name_the_coffer_container() {
        let mv = ItemMove {
            from_item_id: 1,
            to_item_id: 0,
            from_container: container::INVENTORY,
            from_slot: 2,
            to_container: container::TRADE,
            to_slot: 3,
        };
        let mut w = Writer::new();
        CofferCommand::moving(mv, 0, 0x1122).write(&mut w);
        let swap = w.finish();
        // two item ids, four bytes of slots, the container id
        assert_eq!(swap.len(), 8 + 8 + 4 + 8);
        assert_eq!(&swap[swap.len() - 8..], &0x1122u64.to_le_bytes());
        let mut w = Writer::new();
        CofferCommand::moving(mv, 4, 0x1122).write(&mut w);
        let split = w.finish();
        assert_eq!(split.len(), 4 + 8 + 8 + 4 + 8);
        assert_eq!(&split[..4], &4i32.to_le_bytes());
        assert_eq!(
            CofferCommand::Interact {
                obj_id: 1,
                open: true
            }
            .opcode(),
            cs::CSCofferInteractionPacket
        );
    }

    #[test]
    fn contents_decode_full_items_empty_slots_and_reject_every_truncation() {
        // systems/npcs/doodad/coffer.rs::SCCofferContentsUpdatePacket::write,
        // models/item.rs::Item::write: an unpadded detail type 0, followed by item timestamps.
        let mut w = Writer::new();
        w.bc(0xabcdef)
            .u64(0x1234)
            .u8(2)
            .u8(29)
            .u32(5)
            .u64(99)
            .u8(2)
            .u8(0x10)
            .i32(3)
            .u8(0)
            .i64(100)
            .i32(20)
            .u32(7)
            .u8(1)
            .i64(200)
            .i64(300)
            .u8(30)
            .u32(0);
        let body = w.finish();
        for cut in 0..body.len() {
            assert!(
                CofferContents::read(&mut Reader::new(&body[..cut])).is_err(),
                "cut {cut}"
            );
        }
        let mut r = Reader::new(&body);
        let c = CofferContents::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(
            (c.obj_id, c.container_id, c.slots[0].0),
            (0xabcdef, 0x1234, 29)
        );
        let item = c.slots[0].1.as_ref().unwrap();
        assert_eq!(
            (
                item.template_id,
                item.id,
                item.grade,
                item.flags,
                item.count
            ),
            (5, 99, 2, 0x10, 3)
        );
        assert_eq!(
            (
                item.create_time,
                item.lifespan_mins,
                item.made_unit_id,
                item.world_id,
                item.unsecure_time,
                item.unpack_time
            ),
            (100, 20, 7, 1, 200, 300)
        );
        assert_eq!(c.slots[1], (30, None));
        let mut empty = Writer::new();
        empty.bc(1).u64(2).u8(0);
        assert!(
            CofferContents::read(&mut Reader::new(empty.as_bytes()))
                .unwrap()
                .slots
                .is_empty()
        );
    }

    #[test]
    fn reused_identity_and_lifecycle_never_keep_old_contents() {
        let mut c = Coffer::default();
        let page = |obj_id, container_id, slot| {
            Event::Coffer(CofferEvent::Contents(CofferContents {
                obj_id,
                container_id,
                slots: vec![(slot, Some(item(99)))],
            }))
        };
        assert!(c.apply(&page(7, 9, 30)));
        assert!(
            !c.apply(&page(7, 9, 30)),
            "duplicate receipt does not mark changed"
        );
        assert!(
            c.apply(&page(7, 10, 0)),
            "runtime doodad ID can be reused for a new container"
        );
        assert_eq!(c.slots.len(), 1);
        assert!(!c.apply(&Event::DoodadsRemoved(vec![8])));
        assert!(c.apply(&Event::DoodadsRemoved(vec![7])));
        assert_eq!(c, Coffer::default());
        for reset in [
            Event::LeftWorld { target: 1 },
            Event::Disconnected {
                message: String::new(),
            },
            Event::CharacterSelected(Box::default()),
        ] {
            assert!(c.apply(&page(7, 9, 0)));
            assert!(c.apply(&reset));
            assert_eq!(c, Coffer::default());
        }
    }

    #[test]
    fn invalid_wire_identities_and_moves_are_rejected() {
        assert!(
            CofferCommand::Interact {
                obj_id: 0xffffff,
                open: true
            }
            .validate()
            .is_ok()
        );
        for obj_id in [0, 0x1000000] {
            assert!(
                CofferCommand::Interact { obj_id, open: true }
                    .validate()
                    .is_err()
            );
        }
        let mv = ItemMove {
            from_item_id: 1,
            from_container: container::INVENTORY,
            to_container: container::TRADE,
            ..Default::default()
        };
        assert!(
            CofferCommand::Swap {
                mv,
                container_id: 1
            }
            .validate()
            .is_ok()
        );
        assert!(
            CofferCommand::Split {
                mv,
                container_id: 1,
                count: 1
            }
            .validate()
            .is_ok()
        );
        assert!(
            CofferCommand::Swap {
                mv,
                container_id: 0
            }
            .validate()
            .is_err()
        );
        for count in [0, -1] {
            assert!(
                CofferCommand::Split {
                    mv,
                    container_id: 1,
                    count
                }
                .validate()
                .is_err()
            );
        }
        for mv in [
            ItemMove {
                from_item_id: 0,
                ..mv
            },
            ItemMove {
                to_container: container::INVENTORY,
                ..mv
            },
            ItemMove {
                from_container: u8::MAX,
                ..mv
            },
            ItemMove {
                from_container: container::TRADE,
                ..mv
            },
        ] {
            assert!(
                CofferCommand::Swap {
                    mv,
                    container_id: 1
                }
                .validate()
                .is_err()
            );
        }
    }
}
