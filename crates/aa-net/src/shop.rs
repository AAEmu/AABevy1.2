//! NPC shop requests and the server-confirmed buyback list.
//!
//! The normal catalog is client data (`merchants` / `merchant_goods`), not a server packet.
//! Buying/selling updates [`crate::state::Inventory`] and [`crate::state::PlayerStats`] through
//! item tasks. Re-request the sold list after a transaction; `dirty` records that it may be stale.

use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};

pub mod currency {
    pub const MONEY: u8 = 0;
    pub const HONOR: u8 = 1;
    pub const VOCATION: u8 = 2;
    /// On the wire, but the lane's checkout does not support it yet.
    pub const SIEGE: u8 = 3;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuyEntry {
    pub item_id: u32,
    pub grade: u8,
    pub count: i32,
    pub currency: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SellEntry {
    pub container: u8,
    pub slot: u8,
    pub item_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShopCommand {
    /// Counts are individual items, not stacks. Buyback indices are zero-based sold-list slots.
    Buy {
        npc_obj_id: u32,
        doodad_obj_id: u32,
        buy: Vec<BuyEntry>,
        buy_back: Vec<i32>,
        use_aa_point: bool,
    },
    /// Sells the whole stack in each slot; split first for a partial sale.
    Sell {
        npc_obj_id: u32,
        items: Vec<SellEntry>,
    },
    ListSold {
        npc_obj_id: u32,
    },
}

impl ShopCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Buy { npc_obj_id, doodad_obj_id, buy, buy_back, .. } => {
                if *npc_obj_id == 0 && *doodad_obj_id == 0 {
                    return Err("shop has no NPC or doodad");
                }
                if buy.len() > 255 || buy_back.len() > 255 {
                    return Err("shop cart exceeds the wire count");
                }
                if buy.iter().any(|i| i.item_id == 0 || i.count <= 0 || i.currency > currency::SIEGE) {
                    return Err("invalid shop item or quantity");
                }
                if buy_back.iter().any(|i| !(0..12).contains(i)) {
                    return Err("invalid buyback slot");
                }
                if buy_back.iter().enumerate().any(|(n, slot)| buy_back[..n].contains(slot)) {
                    return Err("duplicate buyback slot");
                }
            }
            Self::Sell { npc_obj_id, items } => {
                if *npc_obj_id == 0 {
                    return Err("sale has no NPC");
                }
                if items.len() > 255 {
                    return Err("sale exceeds the wire count");
                }
                if items.iter().any(|i| !matches!(i.container, 1 | 2) || i.item_id == 0) {
                    return Err("invalid sale item or container");
                }
                if items.iter().enumerate().any(|(n, item)| items[..n].iter().any(|old| old.item_id == item.item_id)) {
                    return Err("duplicate sale item");
                }
            }
            Self::ListSold { npc_obj_id } if *npc_obj_id == 0 => return Err("sold list has no NPC"),
            _ => {}
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Buy { .. } => cs::CSBuyItemsPacket,
            Self::Sell { .. } => cs::CSSellItemsPacket,
            Self::ListSold { .. } => cs::CSListSoldItemPacket,
        }
    }

    /// Call [`Self::validate`] before encoding caller-provided carts. The session does this.
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Buy { npc_obj_id, doodad_obj_id, buy, buy_back, use_aa_point } => {
                assert!(buy.len() <= 255 && buy_back.len() <= 255, "shop cart exceeds wire count");
                w.bc(*npc_obj_id).bc(*doodad_obj_id).u32(0).u8(buy.len() as u8).u8(buy_back.len() as u8);
                for i in buy {
                    w.u32(i.item_id).u8(i.grade).i32(i.count).u8(i.currency);
                }
                for i in buy_back {
                    w.i32(*i);
                }
                w.bool(*use_aa_point);
            }
            Self::Sell { npc_obj_id, items } => {
                assert!(items.len() <= 255, "sale exceeds wire count");
                w.bc(*npc_obj_id).bc(0).u8(items.len() as u8);
                for i in items {
                    w.u8(i.container).u8(i.slot).u64(i.item_id).u32(0);
                }
            }
            Self::ListSold { npc_obj_id } => {
                w.bc(*npc_obj_id);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShopEvent {
    SoldItems(Vec<Item>),
    /// Local validation rejected the request; no packet was sent and no inventory changed.
    RequestRejected(String),
}

impl ShopEvent {
    pub fn handles(opcode: u16) -> bool {
        opcode == sc::SCSoldItemListPacket
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::SoldItems(_) => "SCSoldItemList",
            Self::RequestRejected(_) => "ShopRequestRejected",
        }
    }
    pub fn read(r: &mut Reader) -> Result<Self> {
        let count = r.i32()?;
        // Item.Write requires at least its template ID; a sold entry must contain a real item.
        if !(0..=12).contains(&count) || count as usize > r.remaining() / 4 {
            return Err(WireError::Invalid(format!("invalid sold item count {count}")));
        }
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            items.push(Item::read(r)?.ok_or_else(|| WireError::Invalid("empty sold item record".into()))?);
        }
        Ok(Self::SoldItems(items))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Shop {
    pub sold: Vec<Item>,
    /// An empty list is meaningful only after the server answered the list request.
    pub loaded: bool,
    pub dirty: bool,
    pub last_error: Option<String>,
}

impl Shop {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Shop(ShopEvent::SoldItems(items)) => {
                self.sold = items.clone();
                self.loaded = true;
                self.dirty = false;
                self.last_error = None;
            }
            Event::Shop(ShopEvent::RequestRejected(message)) => self.last_error = Some(message.clone()),
            Event::ItemsChanged(tasks) if matches!(tasks.reason, 53 | 54) => {
                if self.dirty {
                    return false;
                }
                self.dirty = true;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::items::{ItemTask, ItemTasks};

    fn plain_item(w: &mut Writer) {
        w.u32(8001).u64(555).u8(2).u8(0).i32(3).u8(0).i64(10).i32(0).u32(0).u8(1).i64(0).i64(0);
    }

    fn batch(entries: &[(u16, Vec<u8>)]) -> Vec<ServerMsg> {
        let mut raw = Writer::new();
        for (opcode, body) in entries {
            raw.u16(0).u16(*opcode).raw(body);
        }
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 4]).u16(entries.len() as u16).u8(1).u16(len).u16(!len).raw(&raw);
        let ServerMsg::Batch(messages) = ServerMsg::decode(&frame.finish()).unwrap() else { panic!() };
        messages
    }

    #[test]
    fn shop_requests_match_server_packet_readers() {
        let buy = ShopCommand::Buy {
            npc_obj_id: 0x010203,
            doodad_obj_id: 0,
            buy: vec![BuyEntry { item_id: 8001, grade: 2, count: 3, currency: 1 }],
            buy_back: vec![4],
            use_aa_point: false,
        };
        let packet = ClientMsg::Shop(buy);
        assert_eq!(packet.opcode(), 0x0ae);
        assert_eq!(
            packet.encode_body(),
            [3, 2, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0x41, 0x1f, 0, 0, 2, 3, 0, 0, 0, 1, 4, 0, 0, 0, 0]
        );
        let packet = ClientMsg::Shop(ShopCommand::Sell {
            npc_obj_id: 0x010203,
            items: vec![SellEntry { container: 2, slot: 4, item_id: 555 }],
        });
        assert_eq!(packet.opcode(), 0x0b0);
        assert_eq!(packet.encode_body(), [3, 2, 1, 0, 0, 0, 1, 2, 4, 0x2b, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let packet = ClientMsg::Shop(ShopCommand::ListSold { npc_obj_id: 0x010203 });
        assert_eq!(packet.opcode(), 0x0b1);
        assert_eq!(packet.encode_body(), [3, 2, 1]);
    }

    #[test]
    fn cart_counts_and_duplicates_are_rejected_before_sending() {
        let buy = |count, buy_back| ShopCommand::Buy {
            npc_obj_id: 1,
            doodad_obj_id: 0,
            buy: vec![BuyEntry { item_id: 8001, grade: 0, count, currency: 0 }],
            buy_back,
            use_aa_point: false,
        };
        assert!(buy(1, vec![0]).validate().is_ok());
        assert!(buy(0, vec![]).validate().is_err());
        assert!(buy(-1, vec![]).validate().is_err());
        assert!(buy(1, vec![-1]).validate().is_err());
        assert!(buy(1, vec![12]).validate().is_err());
        assert!(buy(1, vec![0, 0]).validate().is_err());
        let mut too_many = buy(1, vec![]);
        let ShopCommand::Buy { buy, .. } = &mut too_many else { panic!() };
        buy.resize(256, buy[0]);
        assert!(too_many.validate().is_err());
        let sell =
            ShopCommand::Sell { npc_obj_id: 1, items: vec![SellEntry { container: 2, slot: 1, item_id: 555 }; 2] };
        assert!(sell.validate().is_err());
    }

    #[test]
    fn sold_list_and_item_transactions_preserve_compressed_boundaries() {
        let mut sold = Writer::new();
        sold.i32(1);
        plain_item(&mut sold);
        // The exact writer includes its trailing u32 even for a money-only or empty task list.
        let mut money = Writer::new();
        money.u8(73).u8(1).u8(1).i32(-25).u8(0).u32(0);
        let messages = batch(&[
            (sc::SCSoldItemListPacket, sold.finish()),
            (sc::SCItemTaskSuccessPacket, money.finish()),
            (sc::SCOnOffSnowPacket, vec![1]),
        ]);
        assert_eq!(messages.len(), 3);
        let ServerMsg::Shop(ShopEvent::SoldItems(items)) = &messages[0] else { panic!() };
        assert_eq!((items[0].id, items[0].template_id, items[0].count, items[0].grade), (555, 8001, 3, 2));
        assert!(matches!(&messages[1], ServerMsg::ItemTasks(t) if t.tasks == vec![ItemTask::Money(-25)]));
        assert_eq!(messages[2], ServerMsg::Snow(true));
        let messages =
            batch(&[(sc::SCItemTaskSuccessPacket, vec![52, 0, 0, 0, 0, 0, 0]), (sc::SCOnOffSnowPacket, vec![1])]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1], ServerMsg::Snow(true));
    }

    #[test]
    fn inventory_bank_chunks_read_declared_slots_instead_of_following_packets() {
        let mut contents = Writer::new();
        contents.u8(3).u8(2).u8(1);
        for _ in 0..20 {
            contents.u32(0);
        }
        let messages =
            batch(&[(sc::SCCharacterInvenContentsPacket, contents.finish()), (sc::SCOnOffSnowPacket, vec![1])]);
        assert_eq!(messages.len(), 2);
        assert!(
            matches!(&messages[0], ServerMsg::InvenContents(c) if c.container == 3 && c.first_slot() == 10 && c.items.len() == 20)
        );
        assert_eq!(messages[1], ServerMsg::Snow(true));
    }

    #[test]
    fn invalid_lists_and_unknown_tasks_are_skipped_without_applying_partial_transactions() {
        for count in [-1i32, 13, i32::MAX] {
            let data = count.to_le_bytes();
            assert!(ShopEvent::read(&mut Reader::new(&data)).is_err());
        }
        let mut sold = Writer::new();
        sold.i32(1).u32(0);
        assert!(ShopEvent::read(&mut Reader::new(&sold.finish())).is_err());
        let mut tasks = Writer::new();
        tasks.u8(52).u8(2).u8(1).i32(-500).u8(99);
        assert!(matches!(batch(&[(sc::SCItemTaskSuccessPacket, tasks.finish()), (sc::SCOnOffSnowPacket, vec![1])]).as_slice(),[ServerMsg::DecodeFailed{..}]));
    }

    #[test]
    fn buyback_state_waits_for_server_list_and_marks_transactions_dirty() {
        let mut shop = Shop::default();
        shop.apply(&Event::Shop(ShopEvent::SoldItems(vec![Item { id: 555, template_id: 8001, ..Item::default() }])));
        assert!(shop.loaded && !shop.dirty && shop.sold[0].id == 555);
        // STORE_BUY (54) covers both catalog purchases and removing a bought-back item.
        shop.apply(&Event::ItemsChanged(ItemTasks { reason: 54, ..ItemTasks::default() }));
        assert!(shop.dirty && shop.sold[0].id == 555);
        shop.apply(&Event::Shop(ShopEvent::SoldItems(vec![])));
        assert!(!shop.dirty && shop.sold.is_empty());
        shop.apply(&Event::ItemsChanged(ItemTasks { reason: 54, ..ItemTasks::default() }));
        assert!(shop.dirty && shop.sold.is_empty());
        shop.apply(&Event::Shop(ShopEvent::SoldItems(vec![Item { id: 555, ..Item::default() }])));
        // GET_ITEM_BY_COIN (52) is not a transaction against this NPC's buyback list.
        shop.apply(&Event::ItemsChanged(ItemTasks { reason: 52, ..ItemTasks::default() }));
        assert!(!shop.dirty);
        shop.apply(&Event::ItemsChanged(ItemTasks { reason: 53, ..ItemTasks::default() }));
        assert!(shop.dirty && shop.sold.len() == 1);
        shop.apply(&Event::Shop(ShopEvent::SoldItems(vec![])));
        assert!(shop.loaded && !shop.dirty && shop.sold.is_empty());
        shop.apply(&Event::Disconnected { message: "test".into() });
        assert_eq!(shop, Shop::default());
    }
}
