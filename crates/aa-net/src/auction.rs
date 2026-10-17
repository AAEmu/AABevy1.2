//! Auction queries and requests. Item/money settlement remains authoritative inventory/mail data.
use crate::mail::{MailBody, MailEvent};
use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};

pub const PAGE_SIZE: usize = 9;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuctioneerIds {
    pub obj_id: u32,
    pub obj_id2: u32,
}
impl AuctioneerIds {
    fn valid(&self) -> bool {
        self.obj_id <= 0xff_ffff && self.obj_id2 <= 0xff_ffff
    }
    fn write(&self, w: &mut Writer) {
        w.bc(self.obj_id).bc(self.obj_id2);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuctionSearch {
    pub keyword: String,
    pub exact_match: bool,
    /// Raw item grade;0 is the lane's wildcard, so it cannot filter only raw Basic0.
    pub grade: u8,
    pub category_a: u8,
    pub category_b: u8,
    pub category_c: u8,
    /// Zero-based page. client_id filters the stored seller character ID (0 means all).
    pub page: i32,
    pub client_id: u32,
    pub filter: i32,
    pub world_id: u8,
    pub min_item_level: u8,
    pub max_item_level: u8,
    /// 0 default,1 evaluated,2 name,3 level,4 expiry,5 bid,6 buyout. Order0 asc/1 desc.
    pub sort_kind: u8,
    pub sort_order: u8,
}
impl AuctionSearch {
    fn write(&self, w: &mut Writer) {
        w.string(&self.keyword)
            .bool(self.exact_match)
            .u8(self.grade)
            .u8(self.category_a)
            .u8(self.category_b)
            .u8(self.category_c)
            .i32(self.page)
            .u32(self.client_id)
            .i32(self.filter)
            .u8(self.world_id)
            .u8(self.min_item_level)
            .u8(self.max_item_level)
            .u8(self.sort_kind)
            .u8(self.sort_order);
    }
    fn valid(&self) -> bool {
        self.keyword.len() <= i16::MAX as usize
            && self.page >= 0
            && self.grade <= 11
            && self.sort_kind <= 6
            && self.sort_order <= 1
            && (self.max_item_level == 0 || self.min_item_level <= self.max_item_level)
    }
}

#[derive(Clone, PartialEq)]
pub struct AuctionLot {
    pub id: u64,
    pub duration: u8,
    pub item: Option<Item>,
    pub world_id: u8,
    pub client_id: u32,
    pub client_name: String,
    pub start_price: i32,
    pub buyout_price: i32,
    pub post_date: i64,
    pub bid_world_id: u8,
    pub bidder_id: u32,
    pub bidder_name: String,
    pub bid_price: i32,
    pub extra: i32,
    item_wire: Vec<u8>,
}
impl std::fmt::Debug for AuctionLot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuctionLot")
            .field("id", &self.id)
            .field("item", &self.item)
            .field("client_id", &self.client_id)
            .field("duration", &self.duration)
            .field("post_date", &self.post_date)
            .field("start_price", &self.start_price)
            .field("buyout_price", &self.buyout_price)
            .field("bidder_id", &self.bidder_id)
            .field("bid_price", &self.bid_price)
            .finish_non_exhaustive()
    }
}
impl AuctionLot {
    fn read(r: &mut Reader) -> Result<Self> {
        let id = r.u64()?;
        let duration = r.u8()?;
        let before = r.rest();
        let item = Item::read(r)?;
        let item_wire = before[..before.len() - r.remaining()].to_vec();
        Ok(Self {
            id,
            duration,
            item,
            item_wire,
            world_id: r.u8()?,
            client_id: r.u32()?,
            client_name: r.string()?,
            start_price: r.i32()?,
            buyout_price: r.i32()?,
            post_date: r.i64()?,
            bid_world_id: r.u8()?,
            bidder_id: r.u32()?,
            bidder_name: r.string()?,
            bid_price: r.i32()?,
            extra: r.i32()?,
        })
    }
    /// Complete native AuctionLot.Write, retaining the original full item description for echo.
    pub fn from_wire(wire: &[u8]) -> Result<Self> {
        let mut r = Reader::new(wire);
        let lot = Self::read(&mut r)?;
        if !r.is_empty() {
            return Err(WireError::Invalid("trailing auction lot bytes".into()));
        }
        Ok(lot)
    }
    fn write(&self, w: &mut Writer) {
        w.u64(self.id)
            .u8(self.duration)
            .raw(&self.item_wire)
            .u8(self.world_id)
            .u32(self.client_id)
            .string(&self.client_name)
            .i32(self.start_price)
            .i32(self.buyout_price)
            .i64(self.post_date)
            .u8(self.bid_world_id)
            .u32(self.bidder_id)
            .string(&self.bidder_name)
            .i32(self.bid_price)
            .i32(self.extra);
    }
    fn valid(&self) -> bool {
        let mut r = Reader::new(&self.item_wire);
        self.id > 0
            && self.duration <= 3
            && self.client_name.len() <= i16::MAX as usize
            && self.bidder_name.len() <= i16::MAX as usize
            && Item::read(&mut r).is_ok_and(|item| item == self.item && r.is_empty())
    }
    pub fn item_id(&self) -> u64 {
        self.item.as_ref().map_or(0, |i| i.id)
    }
    pub fn end_time(&self) -> Option<i64> {
        let hours = match self.duration {
            0 => 6,
            1 => 12,
            2 => 24,
            3 => 48,
            _ => return None,
        };
        self.post_date.checked_add(hours * 3600)
    }
    pub fn bid(
        &self,
        amount: i32,
        world_id: u8,
        bidder_id: u32,
        bidder_name: impl Into<String>,
    ) -> AuctionBid {
        AuctionBid {
            lot_id: self.id,
            world_id,
            bidder_id,
            bidder_name: bidder_name.into(),
            amount,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuctionBid {
    pub lot_id: u64,
    pub world_id: u8,
    pub bidder_id: u32,
    pub bidder_name: String,
    pub amount: i32,
}
impl AuctionBid {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            lot_id: r.u64()?,
            world_id: r.u8()?,
            bidder_id: r.u32()?,
            bidder_name: r.string()?,
            amount: r.i32()?,
        })
    }
    fn write(&self, w: &mut Writer) {
        w.u64(self.lot_id)
            .u8(self.world_id)
            .u32(self.bidder_id)
            .string(&self.bidder_name)
            .i32(self.amount);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuctionCommand {
    Search {
        auctioneer: AuctioneerIds,
        query: AuctionSearch,
    },
    Post {
        auctioneer: AuctioneerIds,
        item_id: u64,
        start_price: i32,
        buyout_price: i32,
        duration: u8,
    },
    Bid {
        auctioneer: AuctioneerIds,
        lot: AuctionLot,
        bid: AuctionBid,
    },
    Cancel {
        auctioneer: AuctioneerIds,
        lot: AuctionLot,
    },
    MyBids {
        auctioneer: AuctioneerIds,
        page: i32,
    },
    LowestPrice {
        auctioneer: AuctioneerIds,
        item_template_id: u32,
        grade: u8,
    },
}
impl AuctionCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        let context = match self {
            Self::Search { auctioneer, .. }
            | Self::Post { auctioneer, .. }
            | Self::Bid { auctioneer, .. }
            | Self::Cancel { auctioneer, .. }
            | Self::MyBids { auctioneer, .. }
            | Self::LowestPrice { auctioneer, .. } => auctioneer,
        };
        if !context.valid() {
            return Err("auctioneer ID exceeds24bits");
        }
        match self {
            Self::Search { query, .. } if !query.valid() => return Err("invalid auction search"),
            Self::Post {
                item_id,
                start_price,
                buyout_price,
                duration,
                ..
            } if *item_id == 0
                || *start_price < 0
                || *buyout_price < 0
                || (*buyout_price > 0 && buyout_price < start_price)
                || *duration > 3 =>
            {
                return Err("invalid auction posting");
            }
            Self::Bid { lot, bid, .. } => {
                if !lot.valid()
                    || lot.item_id() == 0
                    || bid.lot_id != lot.id
                    || bid.amount <= 0
                    || bid.amount < lot.start_price
                    || bid.amount <= lot.bid_price
                    || bid.bidder_name.len() > i16::MAX as usize
                {
                    return Err("invalid auction bid");
                }
            }
            Self::Cancel { lot, .. } if !lot.valid() => {
                return Err("invalid auction cancellation lot");
            }
            Self::MyBids { page, .. } if *page < 0 => return Err("negative auction page"),
            Self::LowestPrice {
                item_template_id,
                grade,
                ..
            } if *item_template_id == 0 || *grade > 11 => return Err("invalid auction quote item"),
            _ => {}
        }
        // Native lot copies include strings and full Item.Write; guard the entire frame too.
        let mut w = Writer::new();
        self.write(&mut w);
        if w.finish().len() > u16::MAX as usize - 6 {
            return Err("auction request exceeds frame length");
        }
        Ok(())
    }
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Post { .. } => cs::CSAuctionPostPacket,
            Self::Search { .. } => cs::CSAuctionSearchPacket,
            Self::Bid { .. } => cs::CSBidAuctionPacket,
            Self::Cancel { .. } => cs::CSCancelAuctionPacket,
            Self::MyBids { .. } => cs::CSAuctionMyBidListPacket,
            Self::LowestPrice { .. } => cs::CSAuctionLowestPricePacket,
        }
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Search { auctioneer, query } => {
                auctioneer.write(w);
                query.write(w);
            }
            Self::Post {
                auctioneer,
                item_id,
                start_price,
                buyout_price,
                duration,
            } => {
                auctioneer.write(w);
                w.u64(*item_id)
                    .i32(*start_price)
                    .i32(*buyout_price)
                    .u8(*duration);
            }
            Self::Bid {
                auctioneer,
                lot,
                bid,
            } => {
                auctioneer.write(w);
                lot.write(w);
                bid.write(w);
            }
            Self::Cancel { auctioneer, lot } => {
                auctioneer.write(w);
                lot.write(w);
            }
            Self::MyBids { auctioneer, page } => {
                auctioneer.write(w);
                w.i32(*page);
            }
            Self::LowestPrice {
                auctioneer,
                item_template_id,
                grade,
            } => {
                auctioneer.write(w);
                w.u32(*item_template_id).u8(*grade);
            }
        }
    }
    pub fn is_buyout(&self) -> bool {
        matches!(self,Self::Bid{lot,bid,..} if lot.buyout_price>0 && bid.amount>=lot.buyout_price)
    }
}

/// Lane's capped/truncated f64 listing deposit, charged once before posting is ACKed.
pub fn listing_fee(buyout_price: i32, duration: u8) -> Option<i32> {
    if buyout_price < 0 || duration > 3 {
        return None;
    }
    Some((f64::from(buyout_price) * 0.01 * f64::from(duration + 1)).min(1_000_000.0) as i32)
}

#[derive(Clone, Debug, PartialEq)]
pub enum AuctionEvent {
    Posted(AuctionLot),
    Searched {
        page: i32,
        count: i32,
        lots: Vec<AuctionLot>,
        error: i16,
        server_time: i64,
    },
    LowestPrice {
        item_template_id: u32,
        grade: u8,
        amount: i32,
    },
    Bid {
        bid: AuctionBid,
        is_buyout: bool,
        item_template_id: u32,
    },
    Canceled(AuctionLot),
    Requested(AuctionCommand),
    RequestRejected {
        command: AuctionCommand,
        message: String,
    },
}
impl AuctionEvent {
    pub fn handles(opcode: u16) -> bool {
        (0x12e..=0x132).contains(&opcode)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Posted(_) => "SCAuctionPosted",
            Self::Searched { .. } => "SCAuctionSearched",
            Self::LowestPrice { .. } => "SCAuctionLowestPrice",
            Self::Bid { .. } => "SCAuctionBid",
            Self::Canceled(_) => "SCAuctionCanceled",
            Self::Requested(_) => "AuctionRequested",
            Self::RequestRejected { .. } => "AuctionRequestRejected",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCAuctionPostedPacket => Self::Posted(AuctionLot::read(r)?),
            sc::SCAuctionCanceledPacket => Self::Canceled(AuctionLot::read(r)?),
            sc::SCAuctionSearchedPacket => {
                let page = r.i32()?;
                let count = r.i32()?;
                if !(0..=PAGE_SIZE as i32).contains(&count)
                    || count as usize > r.remaining().saturating_sub(10) / 51
                {
                    return Err(WireError::Invalid(format!(
                        "invalid auction page count {count}"
                    )));
                }
                let mut lots = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    lots.push(AuctionLot::read(r)?);
                }
                Self::Searched {
                    page,
                    count,
                    lots,
                    error: r.i16()?,
                    server_time: r.i64()?,
                }
            }
            sc::SCAuctionLowestPricePacket => Self::LowestPrice {
                item_template_id: r.u32()?,
                grade: r.u8()?,
                amount: r.i32()?,
            },
            sc::SCAuctionBidPacket => Self::Bid {
                bid: AuctionBid::read(r)?,
                is_buyout: r.bool()?,
                item_template_id: r.u32()?,
            },
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown auction opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuctionQuery {
    Search(AuctionSearch),
    MyBids { page: i32 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuctionQuote {
    pub item_template_id: u32,
    pub grade: u8,
    pub amount: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AuctionFailure {
    pub command: Option<AuctionCommand>,
    pub code: Option<i16>,
    pub message: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuctionPurchaseMail {
    pub lot_id: u64,
    pub mail_id: i64,
    pub item_id: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Auction {
    pub page: i32,
    /// Last response's record count0..9, never total matches. lots.len() is the current cache count.
    pub reported_count: i32,
    pub lots: Vec<AuctionLot>,
    pub loaded: bool,
    pub server_time: i64,
    pub query: Option<AuctionQuery>,
    pub dirty: bool,
    pub quotes: Vec<AuctionQuote>,
    pub last_posted: Option<AuctionLot>,
    pub last_bid: Option<AuctionBid>,
    pub last_canceled: Option<AuctionLot>,
    pub last_purchase_mail: Option<AuctionPurchaseMail>,
    pub pending: Vec<AuctionCommand>,
    pub last_error: Option<AuctionFailure>,
}
impl Auction {
    pub fn lot(&self, id: u64) -> Option<&AuctionLot> {
        self.lots.iter().find(|lot| lot.id == id)
    }
    pub fn quote(&self, template: u32, grade: u8) -> Option<&AuctionQuote> {
        self.quotes
            .iter()
            .find(|q| q.item_template_id == template && q.grade == grade)
    }
    pub fn loading(&self) -> bool {
        self.pending.iter().any(|p| {
            matches!(
                p,
                AuctionCommand::Search { .. } | AuctionCommand::MyBids { .. }
            )
        })
    }
    fn delivered(&mut self, body: &MailBody) {
        if body.mail_type != 16 {
            return;
        }
        let mut candidates = self.pending.iter().filter_map(|p| {
            let AuctionCommand::Bid { lot, .. } = p else {
                return None;
            };
            (p.is_buyout()
                && body
                    .items
                    .iter()
                    .flatten()
                    .any(|a| a.item.id == lot.item_id()))
            .then_some((lot.id, lot.item_id()))
        });
        let Some((lot_id, item_id)) = candidates.next() else {
            return;
        };
        // Mail has ItemID but no LotID. A canceled/relisted item can appear in multiple stale
        // pending requests: do not pick one arbitrarily and falsely attribute the win.
        if candidates.any(|candidate| candidate != (lot_id, item_id)) {
            return;
        }
        self.pending.retain(|p| {
            !matches!(p,AuctionCommand::Bid { lot, .. }
            if p.is_buyout() && lot.id == lot_id && lot.item_id() == item_id)
        });
        self.lots.retain(|lot| lot.id != lot_id);
        self.last_purchase_mail = Some(AuctionPurchaseMail {
            lot_id,
            mail_id: body.mail_id,
            item_id,
        });
        self.dirty = true;
    }

    fn complete(&mut self, pick: impl Fn(&AuctionCommand) -> bool) {
        self.pending.retain(|p| !pick(p));
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Auction(event) = event else {
            match event {
                Event::CharacterSelected(_)
                | Event::LeftWorld { .. }
                | Event::Disconnected { .. } => {
                    let changed = *self != Self::default();
                    *self = Self::default();
                    return changed;
                }
                Event::Mail(MailEvent::Got { header, body, .. })
                    if (13..=17).contains(&header.mail_type) =>
                {
                    self.dirty = true;
                    if let Some(body) = body {
                        self.delivered(body);
                    }
                    return true;
                }
                Event::Mail(MailEvent::Body {
                    is_sent: false,
                    body,
                    ..
                }) if (13..=17).contains(&body.mail_type) => {
                    self.dirty = true;
                    self.delivered(body);
                    return true;
                }
                _ => return false,
            }
        };
        match event {
            AuctionEvent::Requested(command) => {
                self.pending.push(command.clone());
                self.last_error = None;
            }
            AuctionEvent::RequestRejected { command, message } => {
                self.last_error = Some(AuctionFailure {
                    command: Some(command.clone()),
                    code: None,
                    message: message.clone(),
                })
            }
            AuctionEvent::Searched {
                page,
                count,
                lots,
                error,
                server_time,
            } => {
                let command = self
                    .pending
                    .iter()
                    .position(|p| {
                        matches!(
                            p,
                            AuctionCommand::Search { .. } | AuctionCommand::MyBids { .. }
                        )
                    })
                    .map(|i| self.pending.remove(i));
                self.query = match &command {
                    Some(AuctionCommand::Search { query, .. }) => {
                        Some(AuctionQuery::Search(query.clone()))
                    }
                    Some(AuctionCommand::MyBids { page, .. }) => {
                        Some(AuctionQuery::MyBids { page: *page })
                    }
                    _ => None,
                };
                self.page = *page;
                self.reported_count = *count;
                self.lots = lots.clone();
                self.loaded = true;
                self.server_time = *server_time;
                self.dirty = false;
                self.last_error = (*error != 0).then(|| AuctionFailure {
                    command,
                    code: Some(*error),
                    message: format!("auction search failed ({error})"),
                });
            }
            AuctionEvent::Posted(lot) => {
                self.last_posted = Some(lot.clone());
                self.dirty = true;
                self.complete(
                    |p| matches!(p,AuctionCommand::Post{item_id,..} if *item_id==lot.item_id()),
                );
                self.last_error = None;
            }
            AuctionEvent::Canceled(lot) => {
                self.last_canceled = Some(lot.clone());
                self.lots.retain(|l| l.id != lot.id);
                self.dirty = true;
                self.complete(|p| matches!(p,AuctionCommand::Cancel{lot:l,..} if l.id==lot.id));
                self.last_error = None;
            }
            AuctionEvent::Bid { bid, is_buyout, .. } => {
                self.last_bid = Some(bid.clone());
                self.dirty = true;
                if *is_buyout {
                    self.lots.retain(|lot| lot.id != bid.lot_id);
                } else if let Some(lot) = self.lots.iter_mut().find(|lot| lot.id == bid.lot_id) {
                    lot.bid_world_id = bid.world_id;
                    lot.bidder_id = bid.bidder_id;
                    lot.bidder_name = bid.bidder_name.clone();
                    lot.bid_price = bid.amount;
                }
                self.complete(|p|matches!(p,AuctionCommand::Bid{bid:b,..} if b.lot_id==bid.lot_id && b.amount==bid.amount));
                self.last_error = None;
            }
            AuctionEvent::LowestPrice {
                item_template_id,
                grade,
                amount,
            } => {
                let quote = AuctionQuote {
                    item_template_id: *item_template_id,
                    grade: *grade,
                    amount: *amount,
                };
                if let Some(old) = self
                    .quotes
                    .iter_mut()
                    .find(|q| q.item_template_id == *item_template_id && q.grade == *grade)
                {
                    *old = quote;
                } else {
                    self.quotes.push(quote);
                }
                self.complete(|p|matches!(p,AuctionCommand::LowestPrice{item_template_id:i,grade:g,..} if i==item_template_id && g==grade));
            }
        }
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::mail::{MailAttachment, MailHeader};
    fn bytes(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        f(&mut w);
        w.finish()
    }
    fn item(w: &mut Writer, detail: u8) {
        w.u32(23388).u64(500).u8(0).u8(0).i32(1).u8(detail);
        match detail {
            1 => {
                w.u32(50).u8(70).u16(0x9876).u32(60).i64(10).u32(70);
                for i in 0..7 {
                    w.u32(i + 80);
                }
                w.u16(100).u16(110);
            }
            4 => {
                w.raw(&[11, 22, 33, 44, 55, 66, 77, 88, 99]);
            }
            _ => {}
        }
        w.i64(1700000000).i32(0).u32(42).u8(1).i64(3).i64(4);
    }
    fn lot_wire(detail: u8) -> Vec<u8> {
        bytes(|w| {
            w.u64(7).u8(2);
            item(w, detail);
            w.u8(1)
                .u32(42)
                .string("Seller")
                .i32(10)
                .i32(100)
                .i64(1700000000)
                .u8(255)
                .u32(0)
                .string("")
                .i32(0)
                .i32(6);
        })
    }
    fn lot() -> AuctionLot {
        AuctionLot::from_wire(&lot_wire(1)).unwrap()
    }
    fn context() -> AuctioneerIds {
        AuctioneerIds {
            obj_id: 0x010203,
            obj_id2: 0x040506,
        }
    }
    fn apply(a: &mut Auction, e: AuctionEvent) {
        a.apply(&Event::Auction(e));
    }
    fn batch(packets: &[(u16, Vec<u8>)]) -> Result<Vec<ServerMsg>> {
        let raw = bytes(|w| {
            for (op, raw) in packets {
                w.u16(0).u16(*op).raw(raw);
            }
            w.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        });
        let len = raw.len() as u16;
        let wire = bytes(|w| {
            w.raw(&[0xdd, 4])
                .u16(packets.len() as u16 + 1)
                .u8(1)
                .u16(len)
                .u16(!len)
                .raw(&raw);
        });
        match ServerMsg::decode(&wire)? {
            ServerMsg::Batch(v) => Ok(v),
            _ => panic!(),
        }
    }
    #[test]
    fn six_requests_match_native_prefix_fields_and_complete_lot_echo() {
        let auctioneer = context();
        let lot = lot();
        let bid = lot.bid(12, 0, 43, "Buyer");
        let mut bid_wire = lot_wire(1);
        bid_wire.extend(bytes(|w| {
            w.u64(7).u8(0).u32(43).string("Buyer").i32(12);
        }));
        let cases = vec![
            (
                AuctionCommand::Post {
                    auctioneer,
                    item_id: 500,
                    start_price: 10,
                    buyout_price: 100,
                    duration: 2,
                },
                0xb7,
                vec![244, 1, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 100, 0, 0, 0, 2],
            ),
            (
                AuctionCommand::Search {
                    auctioneer,
                    query: AuctionSearch {
                        keyword: "Hat".into(),
                        exact_match: true,
                        grade: 3,
                        category_a: 1,
                        category_b: 2,
                        category_c: 3,
                        page: 4,
                        client_id: 42,
                        filter: 5,
                        world_id: 1,
                        min_item_level: 2,
                        max_item_level: 50,
                        sort_kind: 6,
                        sort_order: 1,
                    },
                },
                0xb8,
                vec![
                    3, 0, b'H', b'a', b't', 1, 3, 1, 2, 3, 4, 0, 0, 0, 42, 0, 0, 0, 5, 0, 0, 0, 1,
                    2, 50, 6, 1,
                ],
            ),
            (
                AuctionCommand::Bid {
                    auctioneer,
                    lot: lot.clone(),
                    bid,
                },
                0xb9,
                bid_wire,
            ),
            (
                AuctionCommand::Cancel { auctioneer, lot },
                0xba,
                lot_wire(1),
            ),
            (
                AuctionCommand::MyBids {
                    auctioneer,
                    page: 4,
                },
                0xbb,
                vec![4, 0, 0, 0],
            ),
            (
                AuctionCommand::LowestPrice {
                    auctioneer,
                    item_template_id: 23388,
                    grade: 3,
                },
                0xbc,
                vec![92, 91, 0, 0, 3],
            ),
        ];
        for (command, op, body) in cases {
            assert!(command.validate().is_ok(), "{command:?}");
            let mut expected = vec![3, 2, 1, 6, 5, 4];
            expected.extend(body);
            let packet = ClientMsg::Auction(command);
            assert_eq!((packet.opcode(), packet.encode_body()), (op, expected));
        }
        // Uninterpreted detail and equipment's reserved bytes survive exact native echo.
        for detail in [0, 1, 4] {
            let raw = lot_wire(detail);
            let lot = AuctionLot::from_wire(&raw).unwrap();
            assert!(lot.valid());
            assert_eq!(bytes(|w| lot.write(w)), raw);
        }
    }
    #[test]
    fn five_responses_and_full_items_leave_following_dd04_opcode_aligned() {
        let packets = vec![
            (0x12e, lot_wire(1)),
            (
                0x12f,
                bytes(|w| {
                    w.i32(2)
                        .i32(2)
                        .raw(&lot_wire(1))
                        .raw(&lot_wire(4))
                        .i16(0)
                        .i64(1700000100);
                }),
            ),
            (0x130, vec![92, 91, 0, 0, 3, 100, 0, 0, 0]),
            (
                0x131,
                bytes(|w| {
                    w.u64(7)
                        .u8(0)
                        .u32(43)
                        .string("Buyer")
                        .i32(12)
                        .bool(false)
                        .u32(23388);
                }),
            ),
            (0x132, lot_wire(4)),
        ];
        let messages = batch(&packets).unwrap();
        assert_eq!(messages.len(), 6);
        assert_eq!(messages.last(), Some(&ServerMsg::Snow(true)));
        for (i, (op, raw)) in packets.iter().enumerate() {
            let mut r = Reader::new(raw);
            let expected = AuctionEvent::read(*op, &mut r).unwrap();
            assert!(r.is_empty());
            assert_eq!(messages[i], ServerMsg::Auction(expected));
            for n in 0..raw.len() {
                assert!(
                    AuctionEvent::read(*op, &mut Reader::new(&raw[..n])).is_err(),
                    "op{op:03x} cut{n}"
                );
            }
        }
        let searched = bytes(|w| {
            w.i32(0).i32(9);
            for _ in 0..9 {
                w.raw(&lot_wire(0));
            }
            w.i16(0).i64(1);
        });
        assert!(AuctionEvent::read(0x12f, &mut Reader::new(&searched)).is_ok());
        for count in [-1, 10, i32::MAX] {
            let bad = bytes(|w| {
                w.i32(0).i32(count).i16(0).i64(1);
            });
            assert!(AuctionEvent::read(0x12f, &mut Reader::new(&bad)).is_err());
        }
        assert!(AuctionLot::from_wire(&[lot_wire(1), vec![0]].concat()).is_err());
        assert!(
            !AuctionEvent::handles(0x133),
            "undefined AuctionMessage must not guess a DD04 boundary"
        );
    }
    #[test]
    fn invalid_request_fields_and_mutated_item_echo_are_rejected() {
        let auctioneer = context();
        for command in [
            AuctionCommand::Post {
                auctioneer,
                item_id: 0,
                start_price: 1,
                buyout_price: 2,
                duration: 0,
            },
            AuctionCommand::Post {
                auctioneer,
                item_id: 500,
                start_price: 10,
                buyout_price: 9,
                duration: 0,
            },
            AuctionCommand::Post {
                auctioneer,
                item_id: 500,
                start_price: 0,
                buyout_price: 0,
                duration: 4,
            },
            AuctionCommand::MyBids {
                auctioneer,
                page: -1,
            },
            AuctionCommand::Search {
                auctioneer,
                query: AuctionSearch {
                    keyword: "a".repeat(32768),
                    ..Default::default()
                },
            },
            AuctionCommand::Search {
                auctioneer: AuctioneerIds {
                    obj_id: 0x1000000,
                    obj_id2: 0,
                },
                query: Default::default(),
            },
        ] {
            assert!(command.validate().is_err());
        }
        let mut changed = lot();
        changed.item.as_mut().unwrap().count = 5;
        assert!(
            AuctionCommand::Cancel {
                auctioneer,
                lot: changed
            }
            .validate()
            .is_err()
        );
        let current = lot();
        let bid = current.bid(9, 0, 43, "Buyer");
        assert!(
            AuctionCommand::Bid {
                auctioneer,
                lot: current,
                bid
            }
            .validate()
            .is_err()
        );
        assert_eq!(listing_fee(199, 0), Some(1));
        assert_eq!(listing_fee(199, 3), Some(7));
        assert_eq!(listing_fee(i32::MAX, 3), Some(1_000_000));
        assert_eq!(listing_fee(-1, 0), None);
        assert_eq!(listing_fee(1, 4), None);
        assert_eq!(lot().end_time(), Some(1700000000 + 24 * 3600));
    }
    #[test]
    fn query_fifo_keeps_record_count_distinct_from_page_and_does_not_invent_totals() {
        let mut a = Auction::default();
        let request = AuctionCommand::Search {
            auctioneer: context(),
            query: AuctionSearch {
                page: 7,
                client_id: 42,
                ..Default::default()
            },
        };
        apply(&mut a, AuctionEvent::Requested(request.clone()));
        apply(&mut a, AuctionEvent::Requested(request));
        assert!(a.loading());
        assert!(!a.loaded);
        apply(
            &mut a,
            AuctionEvent::Searched {
                page: 0,
                count: 0,
                lots: vec![],
                error: 0,
                server_time: 11,
            },
        );
        assert_eq!(a.page, 0);
        assert!(matches!(&a.query,Some(AuctionQuery::Search(q)) if q.page==7));
        assert!(a.loading());
        apply(
            &mut a,
            AuctionEvent::Searched {
                page: 7,
                count: 1,
                lots: vec![lot()],
                error: 0,
                server_time: 12,
            },
        );
        assert_eq!(a.reported_count, 1);
        assert_eq!(a.lots.len(), 1);
        assert!(!a.loading());
        let request = AuctionCommand::MyBids {
            auctioneer: context(),
            page: 2,
        };
        apply(&mut a, AuctionEvent::Requested(request.clone()));
        apply(
            &mut a,
            AuctionEvent::Searched {
                page: 2,
                count: 0,
                lots: vec![],
                error: 5,
                server_time: 13,
            },
        );
        assert_eq!(a.query, Some(AuctionQuery::MyBids { page: 2 }));
        assert!(a.lots.is_empty());
        assert_eq!(a.last_error.as_ref().unwrap().command, Some(request));
        assert_eq!(a.last_error.as_ref().unwrap().code, Some(5));
    }
    #[test]
    fn win_mail_without_lot_id_cannot_arbitrarily_resolve_relisted_item_requests() {
        let mut a = Auction::default();
        for id in [7, 8] {
            let mut current = lot();
            current.id = id;
            let bid = current.bid(100, 0, 43, "Buyer");
            apply(
                &mut a,
                AuctionEvent::Requested(AuctionCommand::Bid {
                    auctioneer: context(),
                    lot: current,
                    bid,
                }),
            );
        }
        let attachment = MailAttachment::from_wire(&bytes(|w| item(w, 1))).unwrap();
        let body = MailBody {
            mail_id: 801,
            mail_type: 16,
            items: std::array::from_fn(|i| (i == 0).then(|| attachment.clone())),
            ..Default::default()
        };
        a.apply(&Event::Mail(MailEvent::Body {
            is_prepare: false,
            is_sent: false,
            body,
            open_date_modified: false,
            unread: Default::default(),
        }));
        assert!(a.last_purchase_mail.is_none() && a.pending.len() == 2 && a.dirty);
    }
    #[test]
    fn posting_refusals_and_unrelated_mail_never_fabricate_escrow_or_purchase() {
        let mut a = Auction::default();
        let current = lot();
        let post = AuctionCommand::Post {
            auctioneer: context(),
            item_id: 500,
            start_price: 10,
            buyout_price: 100,
            duration: 2,
        };
        apply(&mut a, AuctionEvent::Requested(post.clone()));
        assert!(a.last_posted.is_none() && a.lots.is_empty());
        apply(&mut a, AuctionEvent::Posted(current.clone()));
        assert!(a.pending.is_empty() && a.lots.is_empty() && a.dirty);
        assert_eq!(a.last_posted.as_ref().map(AuctionLot::item_id), Some(500));
        apply(
            &mut a,
            AuctionEvent::RequestRejected {
                command: post,
                message: "invalid posting".into(),
            },
        );
        assert!(a.last_error.as_ref().is_some_and(|f| f.code.is_none()));
        assert_eq!(a.last_posted, Some(current.clone()));
        let buy = AuctionCommand::Bid {
            auctioneer: context(),
            lot: current.clone(),
            bid: current.bid(100, 0, 43, "Buyer"),
        };
        apply(&mut a, AuctionEvent::Requested(buy.clone()));
        a.apply(&Event::ServerError { code: 10 });
        assert!(
            a.pending.contains(&buy),
            "an unrelated general error cannot resolve this request"
        );
        let mut raw = bytes(|w| item(w, 1));
        raw[4..12].copy_from_slice(&501u64.to_le_bytes());
        let attachment = MailAttachment::from_wire(&raw).unwrap();
        let body = MailBody {
            mail_id: 801,
            mail_type: 16,
            items: std::array::from_fn(|i| (i == 0).then(|| attachment.clone())),
            ..Default::default()
        };
        a.apply(&Event::Mail(MailEvent::Body {
            is_prepare: false,
            is_sent: false,
            body,
            open_date_modified: false,
            unread: Default::default(),
        }));
        assert!(a.pending.contains(&buy) && a.last_purchase_mail.is_none());
        a.apply(&Event::LeftWorld { target: 1 });
        assert_eq!(a, Auction::default());
    }
    #[test]
    fn auction_ack_changes_only_cache_and_received_win_body_proves_exact_buyout_identity() {
        let mut state = crate::state::WorldState::default();
        let lot = lot();
        let bid = lot.bid(100, 0, 43, "Buyer");
        state.auction.lots.push(lot.clone());
        state.stats.money = 200;
        let command = AuctionCommand::Bid {
            auctioneer: context(),
            lot: lot.clone(),
            bid,
        };
        state.apply(&Event::Auction(AuctionEvent::Requested(command.clone())));
        assert_eq!(state.stats.money, 200);
        assert!(state.inventory.find_by_id(500).is_none());
        assert!(state.auction.lot(7).is_some());
        let header = MailHeader {
            mail_id: 800,
            mail_type: 16,
            ..Default::default()
        };
        state.apply(&Event::Mail(MailEvent::Got {
            header,
            body: None,
            is_cancel: false,
            unread: Default::default(),
        }));
        assert!(state.auction.last_purchase_mail.is_none());
        assert_eq!(state.auction.pending.len(), 1);
        let attachment = MailAttachment::from_wire(&bytes(|w| item(w, 1))).unwrap();
        let body = MailBody {
            mail_id: 800,
            mail_type: 16,
            items: std::array::from_fn(|i| (i == 0).then(|| attachment.clone())),
            ..Default::default()
        };
        state.apply(&Event::Mail(MailEvent::Body {
            is_prepare: false,
            is_sent: true,
            body: body.clone(),
            open_date_modified: false,
            unread: Default::default(),
        }));
        assert!(state.auction.last_purchase_mail.is_none());
        state.apply(&Event::Mail(MailEvent::Body {
            is_prepare: false,
            is_sent: false,
            body,
            open_date_modified: false,
            unread: Default::default(),
        }));
        assert_eq!(
            state.auction.last_purchase_mail,
            Some(AuctionPurchaseMail {
                lot_id: 7,
                mail_id: 800,
                item_id: 500
            })
        );
        assert!(state.auction.pending.is_empty());
        assert!(state.auction.lot(7).is_none());
        assert_eq!(state.stats.money, 200);
        // Normal bid acknowledgements canonicalize bidder metadata but cannot settle inventory.
        let mut a = Auction::default();
        a.lots.push(lot.clone());
        let bid = lot.bid(12, 0, 43, "Buyer");
        apply(
            &mut a,
            AuctionEvent::Requested(AuctionCommand::Bid {
                auctioneer: context(),
                lot: lot.clone(),
                bid: bid.clone(),
            }),
        );
        apply(
            &mut a,
            AuctionEvent::Bid {
                bid,
                is_buyout: false,
                item_template_id: 23388,
            },
        );
        assert_eq!(a.lot(7).unwrap().bidder_id, 43);
        assert_eq!(a.lot(7).unwrap().bid_price, 12);
        assert!(a.pending.is_empty());
        apply(&mut a, AuctionEvent::Canceled(lot));
        assert!(a.lots.is_empty() && a.dirty);
        a.apply(&Event::Disconnected {
            message: "test".into(),
        });
        assert_eq!(a, Auction::default());
    }
}
