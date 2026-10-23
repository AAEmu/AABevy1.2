//! The in-game cash shop (ICS) and premium service answers.
//!
//! The server sends these only as replies to the shop's requests (`CSICSMenuList`, `CSICSGoodList`,
//! `CSICSGoodDetail`, `CSICSBuyGood`, `CSPremiumServiceList`), which this client does not send yet;
//! they are decoded so a reply never ends a batch or shows as unhandled. The layouts are the lane
//! server's (`systems/economy/cash_shop.rs`):
//!
//! | packet | body |
//! |---|---|
//! | `SCICSMenuList` 0x1cf | `bool enabled`, 6 x (`u8 main tab`, 7 x `u8 sub tab or 0`) |
//! | `SCICSGoodList` 0x1d0 | `bool page end, u16 pages`, [`CashShopGood`] |
//! | `SCICSGoodDetail` 0x1d1 | `bool page end`, [`CashShopSku`] |
//! | `SCICSCheckTime` 0x1d2 | empty (the client asks for the list again) |
//! | `SCICSBuyResult` 0x1d3 | `bool success, u8 mode, str receiver, i32 charged AA points` |
//! | `SCICSSyncGood` 0x1d4 | `i32 shop id, i32 remaining` |
//! | `SCPremiumServiceList` 0x1d8 | `bool end, u8 size`, [`PremiumService`], `i32 exchange ratio` |
use crate::wire::{Reader, Result, WireError};

pub mod op {
    pub const MENU_LIST: u16 = 0x1cf;
    pub const GOOD_LIST: u16 = 0x1d0;
    pub const GOOD_DETAIL: u16 = 0x1d1;
    pub const CHECK_TIME: u16 = 0x1d2;
    pub const BUY_RESULT: u16 = 0x1d3;
    pub const SYNC_GOOD: u16 = 0x1d4;
    pub const PREMIUM_SERVICE_LIST: u16 = 0x1d8;
}

/// A shop entry of a tab page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CashShopGood {
    pub shop_id: u32,
    pub name: String,
    pub main_tab: u8,
    pub sub_tab: u8,
    pub level_min: u8,
    pub level_max: u8,
    pub display_item_id: u32,
    pub on_sale: bool,
    pub hidden: bool,
    pub limited_type: u8,
    pub limited_stock_max: u16,
    pub buy_restrict_type: u8,
    pub buy_restrict_id: u32,
    /// Unix times of the sale window.
    pub sale_start: i64,
    pub sale_end: i64,
    pub currency: u8,
    pub price: u32,
    pub remaining: i32,
    pub bonus_item_id: u32,
    pub bonus_item_count: u32,
    pub buttons: u8,
}

/// One purchasable variant (SKU) of a shop entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CashShopSku {
    pub shop_id: u32,
    pub sku: u32,
    pub item_id: u32,
    pub item_count: u32,
    pub select_type: u8,
    pub default: bool,
    pub event_type: u8,
    pub event_end: i64,
    pub currency: u8,
    pub price: u32,
    pub discount_price: u32,
    pub bonus_item_id: u32,
    pub bonus_item_count: u32,
}

/// A premium service offer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PremiumService {
    pub cash_id: i32,
    pub name: String,
    pub premium_id: u16,
    pub on_sale: u8,
    pub hidden: u8,
    /// Hours.
    pub time: i32,
    pub kind: u8,
    pub price: i32,
    pub id: u32,
    pub bonus_count: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CashShopEvent {
    /// Whether the shop is open and which sub tabs (1..=7) of the six main tabs have goods.
    MenuList { enabled: bool, tabs: [[bool; 7]; 6] },
    Good { page_end: bool, pages: u16, good: CashShopGood },
    Sku { page_end: bool, sku: CashShopSku },
    CheckTime,
    BuyResult { success: bool, mode: u8, receiver: String, charged_aa_points: i32 },
    Remaining { shop_id: i32, remaining: i32 },
    PremiumService { end: bool, size: u8, service: PremiumService, exchange_ratio: i32 },
}

impl CashShopEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, op::MENU_LIST..=op::SYNC_GOOD | op::PREMIUM_SERVICE_LIST)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::MenuList { .. } => "SCICSMenuList",
            Self::Good { .. } => "SCICSGoodList",
            Self::Sku { .. } => "SCICSGoodDetail",
            Self::CheckTime => "SCICSCheckTime",
            Self::BuyResult { .. } => "SCICSBuyResult",
            Self::Remaining { .. } => "SCICSSyncGood",
            Self::PremiumService { .. } => "SCPremiumServiceList",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            op::MENU_LIST => {
                let enabled = r.bool()?;
                let mut tabs = [[false; 7]; 6];
                for tab in &mut tabs {
                    r.u8()?; // the main tab number
                    for sub in tab.iter_mut() {
                        *sub = r.u8()? != 0;
                    }
                }
                Self::MenuList { enabled, tabs }
            }
            op::GOOD_LIST => {
                let (page_end, pages, shop_id, name) = (r.bool()?, r.u16()?, r.u32()?, r.string()?);
                let good = CashShopGood {
                    shop_id,
                    name,
                    main_tab: r.u8()?,
                    sub_tab: r.u8()?,
                    level_min: r.u8()?,
                    level_max: r.u8()?,
                    display_item_id: r.u32()?,
                    on_sale: r.bool()?,
                    hidden: r.bool()?,
                    limited_type: r.u8()?,
                    limited_stock_max: r.u16()?,
                    buy_restrict_type: r.u8()?,
                    buy_restrict_id: r.u32()?,
                    sale_start: r.i64()?,
                    sale_end: r.i64()?,
                    currency: r.u8()?,
                    price: r.u32()?,
                    remaining: r.i32()?,
                    bonus_item_id: r.u32()?,
                    bonus_item_count: r.u32()?,
                    buttons: r.u8()?,
                };
                Self::Good { page_end, pages, good }
            }
            op::GOOD_DETAIL => Self::Sku {
                page_end: r.bool()?,
                sku: CashShopSku {
                    shop_id: r.u32()?,
                    sku: r.u32()?,
                    item_id: r.u32()?,
                    item_count: r.u32()?,
                    select_type: r.u8()?,
                    default: r.bool()?,
                    event_type: r.u8()?,
                    event_end: r.i64()?,
                    currency: r.u8()?,
                    price: r.u32()?,
                    discount_price: r.u32()?,
                    bonus_item_id: r.u32()?,
                    bonus_item_count: r.u32()?,
                },
            },
            op::CHECK_TIME => Self::CheckTime,
            op::BUY_RESULT => Self::BuyResult { success: r.bool()?, mode: r.u8()?, receiver: r.string()?, charged_aa_points: r.i32()? },
            op::SYNC_GOOD => Self::Remaining { shop_id: r.i32()?, remaining: r.i32()? },
            op::PREMIUM_SERVICE_LIST => {
                let (end, size) = (r.bool()?, r.u8()?);
                let service = PremiumService {
                    cash_id: r.i32()?,
                    name: r.string()?,
                    premium_id: r.u16()?,
                    on_sale: r.u8()?,
                    hidden: r.u8()?,
                    time: r.i32()?,
                    kind: r.u8()?,
                    price: r.i32()?,
                    id: r.u32()?,
                    bonus_count: r.i32()?,
                };
                Self::PremiumService { end, size, service, exchange_ratio: r.i32()? }
            }
            opcode => return Err(WireError::Invalid(format!("not a cash shop packet: {opcode:#05x}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    fn read(opcode: u16, w: &Writer) -> CashShopEvent {
        let body = w.as_bytes();
        for n in 0..body.len() {
            assert!(CashShopEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:#x} cut {n}");
        }
        CashShopEvent::read(opcode, &mut Reader::new(body)).unwrap()
    }

    #[test]
    fn shop_replies_decode() {
        let mut w = Writer::new();
        w.bool(true);
        for main in 1..=6u8 {
            w.u8(main).raw(&[if main == 2 { 1 } else { 0 }, 0, 3, 0, 0, 0, 0]);
        }
        let CashShopEvent::MenuList { enabled, tabs } = read(op::MENU_LIST, &w) else { panic!() };
        assert!(enabled && tabs[1][0] && tabs[0][2] && !tabs[0][0]);

        let mut w = Writer::new();
        w.bool(true).u16(1).u32(5).string("Pack").u8(1).u8(2).u8(0).u8(55).u32(30001).bool(true).bool(false);
        w.u8(0).u16(0).u8(0).u32(0).i64(0).i64(0).u8(0).u32(300).i32(-1).u32(0).u32(0).u8(7);
        let CashShopEvent::Good { page_end, pages, good } = read(op::GOOD_LIST, &w) else { panic!() };
        assert!(page_end && pages == 1);
        assert_eq!((good.shop_id, good.name.as_str(), good.price, good.remaining, good.buttons), (5, "Pack", 300, -1, 7));

        let mut w = Writer::new();
        w.bool(true).u32(5).u32(6).u32(30001).u32(2).u8(0).bool(true).u8(0).i64(0).u8(1).u32(300).u32(250).u32(0).u32(0);
        let CashShopEvent::Sku { sku, .. } = read(op::GOOD_DETAIL, &w) else { panic!() };
        assert_eq!((sku.sku, sku.item_count, sku.discount_price), (6, 2, 250));

        let mut w = Writer::new();
        w.bool(true).u8(0).string("Bob").i32(10);
        assert_eq!(read(op::BUY_RESULT, &w), CashShopEvent::BuyResult { success: true, mode: 0, receiver: "Bob".into(), charged_aa_points: 10 });
        assert_eq!(CashShopEvent::read(op::CHECK_TIME, &mut Reader::new(&[])).unwrap(), CashShopEvent::CheckTime);

        let mut w = Writer::new();
        w.bool(true).u8(1).i32(8000001).string("Premium").u16(1).u8(0).u8(0).i32(720).u8(0).i32(300).u32(0).i32(0).i32(0);
        let CashShopEvent::PremiumService { service, .. } = read(op::PREMIUM_SERVICE_LIST, &w) else { panic!() };
        assert_eq!((service.cash_id, service.time, service.price), (8000001, 720, 300));
    }
}
