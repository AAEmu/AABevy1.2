//! The original auction house window (`x2ui/auction`) over aa-net's auction protocol: `X2Auction`, the events the
//! window listens to and the NPC service that opens it. See `docs/aa-client-auction.md`.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use aa_net::auction::{AuctionLot, AuctionSearch, AuctioneerIds, PAGE_SIZE, listing_fee};
use aa_net::interaction::InteractEvent;
use aa_net::model::Item;
use aa_net::state::{Inventory, Units};
use aa_net::{AuctionCommand, AuctionEvent, Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, ObjectLike, Table, Value};

use super::api::{HudBridge, Shared};
use super::store::ui_grade;
use crate::Screen;

/// `SkillsEnum.UseAuctioneer`: the service skill an auctioneer NPC lists when talked to.
const USE_AUCTIONEER: u32 = 12083;
/// The window closes when the auctioneer is further away (m); the server serves within 5.
const REACH: f32 = 4.0;
/// A search the server never answered (out of its range: it only sends an error) frees the controls after this.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(4);
/// The listing durations, as the window's radio buttons number them (1..=4): hours.
const DURATIONS: usize = 4;
/// `CURRENCY_GOLD`: bids and fees are paid in copper, not AA points.
const CURRENCY_GOLD: i64 = 0;

/// What the result list shows and what repeats it (after a bid, a cancellation, a refresh or another page).
#[derive(Clone, Debug, PartialEq)]
enum Query {
    /// The search tab, or the listing tab (`client_id` is the seller).
    Search(AuctionSearch),
    /// The items the player bid on.
    Bids(i32),
}

impl Query {
    fn at_page(&self, page: i32) -> Query {
        match self {
            Query::Search(s) => Query::Search(AuctionSearch { page, ..s.clone() }),
            Query::Bids(_) => Query::Bids(page),
        }
    }
}

/// The page on screen. The server pages by 9, counting from 0, and never says how many there are.
#[derive(Default)]
struct Results {
    lots: Vec<AuctionLot>,
    page: i32,
    /// The page after this one came back empty (or this one is short): there is no next page.
    last: bool,
    server_time: i64,
    received: Option<Instant>,
}

impl Results {
    /// A page arrived. The empty page behind the last one (the pager can only guess there is one more after a
    /// full page) keeps showing the last one.
    fn apply(&mut self, page: i32, lots: Vec<AuctionLot>, server_time: i64, now: Instant) {
        self.server_time = server_time;
        self.received = Some(now);
        if lots.is_empty() && page > 0 && page == self.page + 1 && !self.lots.is_empty() {
            self.last = true;
            return;
        }
        self.last = lots.len() < PAGE_SIZE;
        self.lots = lots;
        self.page = page.max(0);
    }

    /// The pager's "pages there are": every page until a short one.
    fn total_pages(&self) -> i32 {
        self.page + 1 + i32::from(!self.last)
    }

    /// Seconds a lot has left, by the server's clock at the time of the page and the time that went by since.
    fn left(&self, lot: &AuctionLot, now: Instant) -> i64 {
        let passed = self.received.map_or(0, |at| now.saturating_duration_since(at).as_secs() as i64);
        lot.end_time().map_or(0, |end| end - self.server_time - passed).max(0)
    }
}

/// The item in the listing form: the bag slot (0 based) it lies in, and what was there.
#[derive(Clone)]
struct Attached {
    slot: u8,
    item: Item,
}

/// What the window is told next (raised once the Lua state is free of borrows).
#[derive(Debug, PartialEq)]
enum Signal {
    Attached(bool),
    Searching,
    Searched,
    /// The item (template) is on offer now.
    PutUp(u32),
    Bidded(u32, i32),
    Bought(u32, i32),
    Canceled(u32),
    Lowest(u32, u8, i32),
}

#[derive(Default)]
struct View {
    open: bool,
    /// The auctioneer talked to: object id and template.
    npc: Option<(u32, u32)>,
    tab: u8,
    inventory: Inventory,
    results: Results,
    query: Option<Query>,
    /// The sort of the results: the column's number (1 value, 2 name, 3 level, 4 time; 0 none), 0 ascending.
    sort: (u8, u8),
    attached: Option<Attached>,
    /// The listing form: start and buyout price (copper), duration (1..=4).
    price: (i64, i64),
    duration: usize,
    quotes: Vec<(u32, i32)>,
    searching: Option<Instant>,
    commands: Vec<AuctionCommand>,
    signals: Vec<Signal>,
    /// `ERROR_MSG` names of refusals to show.
    notices: Vec<&'static str>,
    /// Unlock this bag slot (1 based).
    unlock: Vec<usize>,
}

#[derive(Clone, Default)]
struct State(Rc<RefCell<View>>);

/// The window's grade choice (1 all, then 3 for Uncommon ... 12 for Mythic) as the item grade the server filters by
/// (0 any).
fn raw_grade(window: i64) -> u8 {
    (window - 1).clamp(0, 11) as u8
}

/// The category tree's value, `AABBCC`, as the three category levels.
fn categories(value: i64) -> (u8, u8, u8) {
    let value = value.clamp(0, 999_999);
    ((value / 10_000) as u8, (value / 100 % 100) as u8, (value % 100) as u8)
}

/// The window counts pages from 1 (0 is the first too); the server from 0.
fn server_page(page: i64) -> i32 {
    (page - 1).clamp(0, i64::from(i32::MAX)) as i32
}

/// A price the window passes as a decimal string or a number.
fn amount(value: &Value) -> Option<i64> {
    match value {
        Value::Integer(n) => Some(*n),
        Value::Number(n) if n.fract() == 0.0 => Some(*n as i64),
        Value::String(s) => s.to_str().ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn sort_kind(name: &str) -> Option<u8> {
    ["evaluated", "item_name", "item_level", "expire_date"].iter().position(|n| *n == name).map(|i| i as u8 + 1)
}

impl View {
    fn ids(&self) -> AuctioneerIds {
        AuctioneerIds { obj_id: self.npc.map_or(0, |n| n.0), obj_id2: 0 }
    }

    fn refuse(&mut self, name: &'static str) {
        self.notices.push(name);
    }

    fn send(&mut self, command: AuctionCommand) -> bool {
        if let Err(why) = command.validate() {
            warn!("hud: auction request refused: {why}");
            self.refuse("AUCTION_INVALID_ENTRY");
            return false;
        }
        self.commands.push(command);
        true
    }

    /// Ask for the results of `query` (the pages the window repeats), sorted as the window sorts them.
    fn ask(&mut self, query: Query) {
        let query = match query {
            Query::Search(s) => Query::Search(AuctionSearch { sort_kind: self.sort.0, sort_order: self.sort.1, ..s }),
            bids => bids,
        };
        let auctioneer = self.ids();
        let command = match &query {
            Query::Search(s) => AuctionCommand::Search { auctioneer, query: s.clone() },
            Query::Bids(page) => AuctionCommand::MyBids { auctioneer, page: *page },
        };
        if self.send(command) {
            self.query = Some(query);
            self.searching = Some(Instant::now());
            self.signals.push(Signal::Searching);
        }
    }

    fn lot_by_id(&self, id: &str) -> Option<&AuctionLot> {
        let id: u64 = id.parse().ok()?;
        self.results.lots.iter().find(|l| l.id == id)
    }

    fn lot(&self, index: usize) -> Option<&AuctionLot> {
        self.results.lots.get(index.checked_sub(1)?)
    }

    fn quote(&self, template: u32) -> Option<i32> {
        self.quotes.iter().find(|q| q.0 == template).map(|q| q.1)
    }

    /// Put the bag item in the listing form.
    fn attach(&mut self, slot: usize) -> bool {
        let Some(item) = slot.checked_sub(1).and_then(|i| self.inventory.bag.get(i)).and_then(Option::as_ref).cloned() else {
            return false;
        };
        if item.id == 0 || item.flags & 1 != 0 {
            self.refuse("AUC_SOUL_BOUND_ITEM");
            return false;
        }
        self.detach();
        self.attached = Some(Attached { slot: (slot - 1) as u8, item });
        self.signals.push(Signal::Attached(true));
        true
    }

    fn detach(&mut self) {
        if let Some(a) = self.attached.take() {
            self.unlock.push(usize::from(a.slot) + 1);
            self.signals.push(Signal::Attached(false));
        }
    }

    /// The listing button.
    fn post(&mut self, money: i64) {
        let Some(a) = self.attached.clone() else { return };
        let still_there = self.inventory.bag.get(usize::from(a.slot)).and_then(Option::as_ref).is_some_and(|i| i.id == a.item.id);
        let (start, buyout) = (i32::try_from(self.price.0), i32::try_from(self.price.1));
        let (Ok(start_price), Ok(buyout_price), true) = (start, buyout, still_there) else {
            self.refuse("AUC_INVALID_ENTRY");
            return;
        };
        let duration = self.duration.clamp(1, DURATIONS) as u8 - 1;
        if listing_fee(buyout_price, duration).is_some_and(|fee| i64::from(fee) > money) {
            self.refuse("CAN_NOT_PUTUP_MONEY");
            return;
        }
        self.send(AuctionCommand::Post { auctioneer: self.ids(), item_id: a.item.id, start_price, buyout_price, duration });
    }

    fn bid(&mut self, id: &str, amount: i64, me: (u32, &str), money: i64) {
        let Some(lot) = self.lot_by_id(id).cloned() else { return };
        let Ok(amount) = i32::try_from(amount) else {
            self.refuse("AUCTION_INVALID_BID_PRICE");
            return;
        };
        if lot.client_id == me.0 {
            self.refuse("AUC_BID_SELF");
        } else if amount < lot.start_price {
            self.refuse("AUC_BID_MONEY_UNDER_START_MONEY");
        } else if amount <= lot.bid_price {
            self.refuse("AUC_BID_MONEY_UNDER_TOP_MOST");
        } else if i64::from(if lot.buyout_price > 0 { amount.min(lot.buyout_price) } else { amount }) > money {
            self.refuse("NOT_ENOUGH_MONEY");
        } else {
            // the world id and name are the server's to fill in
            let bid = lot.bid(amount, 0, me.0, me.1);
            self.send(AuctionCommand::Bid { auctioneer: self.ids(), lot, bid });
        }
    }

    fn cancel(&mut self, id: &str, me: u32) {
        let Some(lot) = self.lot_by_id(id).cloned() else { return };
        if lot.client_id != me {
            self.refuse("AUC_PERMISSION_DENY");
        } else if lot.bidder_id != 0 {
            self.refuse("AUC_CANNOT_CANCEL_IF_BID");
        } else {
            self.send(AuctionCommand::Cancel { auctioneer: self.ids(), lot });
        }
    }

    /// The window's click on the column: sort by it, the other way when it already is.
    fn sort_by(&mut self, kind: u8) {
        self.sort = (kind, if self.sort.0 == kind { 1 - self.sort.1 } else { 0 });
        if let Some(query) = self.query.clone() {
            self.ask(query.at_page(0));
        }
    }

    /// After a change to the lots (bid, buy-out, cancellation, listing) show the same list again.
    fn refresh(&mut self) {
        if let Some(query) = self.query.clone() {
            self.ask(query);
        }
    }

    fn on_event(&mut self, event: &AuctionEvent, now: Instant) {
        match event {
            AuctionEvent::Searched { page, lots, server_time, error, .. } => {
                self.searching = None;
                if *error != 0 {
                    warn!("hud: auction search failed ({error})");
                }
                self.results.apply(*page, lots.iter().filter(|l| l.item.is_some()).cloned().collect(), *server_time, now);
                if let Some(query) = self.query.take() {
                    self.query = Some(query.at_page(self.results.page));
                }
                self.signals.push(Signal::Searched);
            }
            AuctionEvent::Posted(lot) => {
                self.signals.push(Signal::PutUp(lot.item.as_ref().map_or(0, |i| i.template_id)));
                if let Some(a) = self.attached.take() {
                    self.unlock.push(usize::from(a.slot) + 1);
                }
                self.refresh();
            }
            AuctionEvent::Bid { bid, is_buyout, item_template_id } => {
                self.signals.push(if *is_buyout { Signal::Bought(*item_template_id, bid.amount) } else { Signal::Bidded(*item_template_id, bid.amount) });
                self.refresh();
            }
            AuctionEvent::Canceled(lot) => {
                self.signals.push(Signal::Canceled(lot.item.as_ref().map_or(0, |i| i.template_id)));
                self.refresh();
            }
            AuctionEvent::LowestPrice { item_template_id, grade, amount } => {
                match self.quotes.iter_mut().find(|q| q.0 == *item_template_id) {
                    Some(q) => q.1 = *amount,
                    None => self.quotes.push((*item_template_id, *amount)),
                }
                self.signals.push(Signal::Lowest(*item_template_id, *grade, *amount));
            }
            AuctionEvent::RequestRejected { message, .. } => {
                warn!("hud: auction request: {message}");
                self.refuse("AUC_SERVICE_UNAVAILABLE");
            }
            AuctionEvent::Requested(_) => {}
        }
    }
}

/// The table `GetSearchedItemInfo` returns: the item's tooltip table with the lot's fields.
fn lot_info(lua: &Lua, bridge: &Shared, view: &View, index: usize, now: Instant) -> mlua::Result<Option<Table>> {
    let Some(lot) = view.lot(index) else { return Ok(None) };
    let Some(item) = &lot.item else { return Ok(None) };
    let Some(t) = super::mail::item_info(lua, bridge, item)? else { return Ok(None) };
    let me = bridge.borrow().character_id;
    t.set("idx", index)?;
    // the level column reads an item without a level requirement as level 1
    if t.get::<i64>("level_requirement").unwrap_or(0) <= 0 && t.get::<i64>("level_limit").unwrap_or(0) == 0 {
        t.set("level_requirement", Value::Nil)?;
    }
    t.set("stackCount", item.count)?;
    t.set("articleId", lot.id.to_string())?;
    t.set("bidder", lot.bidder_name.as_str())?;
    t.set("seller", lot.client_name.as_str())?;
    // the lane has one world: a bid of the player's own character was made in the world the window is in
    t.set("bidWorldId", if lot.bidder_id == me && me != 0 { 0 } else { i64::from(lot.bid_world_id) })?;
    t.set("leftTime", view.results.left(lot, now))?;
    t.set("bidPriceStr", lot.bid_price.max(lot.start_price).to_string())?;
    t.set("directPriceStr", lot.buyout_price.to_string())?;
    // the history lists the player's own bid
    t.set("extra", if lot.bidder_id == me { lot.bid_price } else { lot.extra }.to_string())?;
    t.set("depositStr", listing_fee(lot.buyout_price, lot.duration).unwrap_or(0).to_string())?;
    Ok(Some(t))
}

fn me(bridge: &Shared) -> (u32, String, i64) {
    let d = bridge.borrow();
    (d.character_id, d.snapshot.name.clone(), d.snapshot.money)
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = State::default();
    state.0.borrow_mut().duration = 1;
    lua.set_app_data(state.clone());
    let a: Table = lua.globals().get("X2Auction")?;
    // the bid and the fee are paid in gold
    for name in ["GetCurrencyForBid", "GetCurrencyForFee"] {
        a.set(name, lua.create_function(|_, _: Table| Ok(CURRENCY_GOLD))?)?;
    }
    // everyone may list; no premium benefit is needed
    a.set("HasPostAuthority", lua.create_function(|_, _: Table| Ok(true))?)?;
    a.set("RequirePostAuthority", lua.create_function(|_, _: Table| Ok(false))?)?;
    let s = state.clone();
    a.set("IsAuctionNpc", lua.create_function(move |_, _: Table| Ok(s.0.borrow().npc.is_some()))?)?;
    let x2: Table = lua.globals().get("X2")?;
    x2.set("GetCurrentWorldId", lua.create_function(|_, _: Table| Ok(0))?)?;

    let s = state.clone();
    a.set(
        "SetOpen",
        lua.create_function(move |_, (_, open): (Table, bool)| {
            s.0.borrow_mut().open = open;
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SetCurTab",
        lua.create_function(move |_, (_, tab): (Table, u8)| {
            s.0.borrow_mut().tab = tab;
            Ok(())
        })?,
    )?;
    install_search(lua, &a, bridge, &state)?;
    install_results(lua, &a, bridge, &state)?;
    install_trade(lua, &a, bridge, &state)?;
    install_listing(lua, &a, bridge, &state)
}

fn install_search(lua: &Lua, a: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let s = state.clone();
    a.set(
        "SearchAuctionArticle",
        lua.create_function(
            move |_, (_, page, min, max, grade, category, exact, keyword): (Table, i64, i64, i64, i64, i64, bool, Option<String>)| {
                let (a, b, c) = categories(category);
                s.0.borrow_mut().ask(Query::Search(AuctionSearch {
                    keyword: keyword.unwrap_or_default(),
                    exact_match: exact,
                    grade: raw_grade(grade),
                    category_a: a,
                    category_b: b,
                    category_c: c,
                    page: server_page(page),
                    min_item_level: min.clamp(0, 255) as u8,
                    max_item_level: max.clamp(0, 255) as u8,
                    ..default()
                }));
                Ok(())
            },
        )?,
    )?;
    // the listing tab: the player's own lots
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "SearchMyAuctionArticles",
        lua.create_function(move |_, (_, page): (Table, i64)| {
            let seller = b.borrow().character_id;
            s.0.borrow_mut().ask(Query::Search(AuctionSearch { client_id: seller, page: server_page(page), ..default() }));
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SearchMyBidList",
        lua.create_function(move |_, (_, page): (Table, i64)| {
            s.0.borrow_mut().ask(Query::Bids(server_page(page)));
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SearchAuctionArticleByPage",
        lua.create_function(move |_, (_, page): (Table, i64)| {
            let mut v = s.0.borrow_mut();
            if let Some(query) = v.query.clone() {
                v.ask(query.at_page(server_page(page)));
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SearchRefresh",
        lua.create_function(move |_, _: Table| {
            s.0.borrow_mut().refresh();
            Ok(())
        })?,
    )?;
    // the siege declarations of the first category have no server list
    a.set("SearchDeclareSiege", lua.create_function(|_, _: (Table, Option<i64>)| Ok(()))?)?;
    let s = state.clone();
    a.set(
        "SearchedListSort",
        lua.create_function(move |_, (_, kind): (Table, String)| {
            if let Some(kind) = sort_kind(&kind) {
                s.0.borrow_mut().sort_by(kind);
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "ClearSearchCondition",
        lua.create_function(move |_, _: Table| {
            let mut v = s.0.borrow_mut();
            v.query = None;
            v.sort = (0, 0);
            v.results = Results::default();
            Ok(())
        })?,
    )?;
    Ok(())
}

fn install_results(lua: &Lua, a: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let s = state.clone();
    a.set("GetSearchedItemCount", lua.create_function(move |_, _: Table| Ok(s.0.borrow().results.lots.len()))?)?;
    let s = state.clone();
    a.set("GetSearchedItemPage", lua.create_function(move |_, _: Table| Ok(s.0.borrow().results.page + 1))?)?;
    let s = state.clone();
    a.set("GetSearchedItemTotalCount", lua.create_function(move |_, _: Table| Ok(s.0.borrow().results.total_pages()))?)?;
    let s = state.clone();
    a.set("GetSearchedSortInfo", lua.create_function(move |_, _: Table| Ok(s.0.borrow().sort))?)?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "GetSearchedItemInfo",
        lua.create_function(move |lua, (_, index): (Table, usize)| lot_info(lua, &b, &s.0.borrow(), index, Instant::now()))?,
    )?;
    let s = state.clone();
    a.set(
        "GetSearchedItemPrice",
        lua.create_function(move |_, (_, index): (Table, usize)| {
            Ok(match s.0.borrow().lot(index) {
                Some(l) => (Some(l.bid_price.max(l.start_price).to_string()), Some(l.buyout_price.to_string())),
                None => (None, None),
            })
        })?,
    )?;
    let s = state.clone();
    a.set(
        "GetSearchedItemArticleId",
        lua.create_function(move |_, (_, index): (Table, usize)| Ok(s.0.borrow().lot(index).map(|l| l.id.to_string())))?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "IsMyPutupArticle",
        lua.create_function(move |_, (_, index): (Table, usize)| {
            let me = b.borrow().character_id;
            Ok(s.0.borrow().lot(index).is_some_and(|l| l.client_id == me))
        })?,
    )?;
    let s = state.clone();
    a.set(
        "GetLinkText",
        lua.create_function(move |_, (_, index): (Table, usize)| {
            Ok(s.0.borrow().lot(index).and_then(|l| l.item.as_ref()).and_then(super::itemlinks::link).unwrap_or_default())
        })?,
    )
}

fn install_trade(lua: &Lua, a: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "BidAuctionArticle",
        lua.create_function(move |_, (_, id, money): (Table, String, Value)| {
            let (character, name, gold) = me(&b);
            if let Some(money) = amount(&money) {
                s.0.borrow_mut().bid(&id, money, (character, &name), gold);
            }
            Ok(())
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "DirectPurchaseAuctionArticle",
        lua.create_function(move |_, (_, id): (Table, String)| {
            let (character, name, gold) = me(&b);
            let mut v = s.0.borrow_mut();
            match v.lot_by_id(&id).map(|l| l.buyout_price) {
                Some(price) if price > 0 => v.bid(&id, i64::from(price), (character, &name), gold),
                Some(_) => v.refuse("AUC_WRONG_DIRECT_MONEY"),
                None => {}
            }
            Ok(())
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "CancelAuctionArticle",
        lua.create_function(move |_, (_, id): (Table, String)| {
            s.0.borrow_mut().cancel(&id, b.borrow().character_id);
            Ok(())
        })?,
    )
}

fn install_listing(lua: &Lua, a: &Table, bridge: &Shared, state: &State) -> mlua::Result<()> {
    let s = state.clone();
    a.set(
        "AttachItemFromBag",
        lua.create_function(move |lua, (_, slot): (Table, usize)| {
            let bag: Table = lua.globals().get("X2Bag")?;
            if bag.call_method::<bool>("IsLocked", slot)? {
                return Ok(false);
            }
            let attached = s.0.borrow_mut().attach(slot);
            if attached {
                bag.call_method::<()>("LockSlot", slot)?;
            }
            Ok(attached)
        })?,
    )?;
    // the item held on the cursor, as the bag's item is
    let s = state.clone();
    a.set(
        "AttachItemFromPick",
        lua.create_function(move |lua, _: Table| {
            let held = aa_ui::lua::ui(lua).drag.held.as_ref().map(|h| h.table.clone());
            let Some(h) = held else { return Ok(false) };
            if h.get::<String>("kind").unwrap_or_default() != "bag" || h.get::<bool>("partial").unwrap_or(false) {
                return Ok(false);
            }
            let slot: usize = h.get("slot")?;
            let uid: String = h.get("uid")?;
            let same = s.0.borrow().inventory.bag.get(slot.saturating_sub(1)).and_then(Option::as_ref).is_some_and(|i| i.id.to_string() == uid);
            let bag: Table = lua.globals().get("X2Bag")?;
            if !same || bag.call_method::<bool>("IsLocked", slot)? {
                return Ok(false);
            }
            let attached = s.0.borrow_mut().attach(slot);
            if attached {
                bag.call_method::<()>("LockSlot", slot)?;
                aa_ui::lua::drag::clear(lua);
            }
            Ok(attached)
        })?,
    )?;
    let s = state.clone();
    a.set(
        "DetachItem",
        lua.create_function(move |_, _: Table| {
            s.0.borrow_mut().detach();
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set("GetAttachedItemExist", lua.create_function(move |_, _: Table| Ok(s.0.borrow().attached.is_some()))?)?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "GetAttachedItemInfo",
        lua.create_function(move |lua, _: Table| match s.0.borrow().attached.as_ref() {
            Some(a) => super::mail::item_info(lua, &b, &a.item),
            None => Ok(None),
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SetPrice",
        lua.create_function(move |_, (_, start, buyout): (Table, Value, Value)| {
            s.0.borrow_mut().price = (amount(&start).unwrap_or(0), amount(&buyout).unwrap_or(0));
            Ok(())
        })?,
    )?;
    let s = state.clone();
    a.set(
        "SetDuration",
        lua.create_function(move |_, (_, duration): (Table, usize)| {
            s.0.borrow_mut().duration = duration.clamp(1, DURATIONS);
            Ok(())
        })?,
    )?;
    // the deposit is a share of the buy-out price that grows with the duration (the server's fee)
    a.set(
        "CalcDeposit",
        lua.create_function(|_, (_, _start, buyout, duration): (Table, Value, Value, usize)| {
            let buyout = amount(&buyout).and_then(|n| i32::try_from(n).ok()).unwrap_or(0);
            Ok(listing_fee(buyout, duration.clamp(1, DURATIONS) as u8 - 1).unwrap_or(0))
        })?,
    )?;
    let (s, b) = (state.clone(), bridge.clone());
    a.set(
        "AskAuctionArticle",
        lua.create_function(move |_, _: Table| {
            s.0.borrow_mut().post(b.borrow().snapshot.money);
            Ok(())
        })?,
    )?;
    // the cheapest buy-out price for the item: the answer is the cached one, the event brings the fresh one
    let s = state.clone();
    a.set(
        "GetLowestPrice",
        lua.create_function(move |_, (_, template, grade): (Table, u32, u8)| {
            let mut v = s.0.borrow_mut();
            if template == 0 {
                return Ok(None);
            }
            let ids = v.ids();
            v.send(AuctionCommand::LowestPrice { auctioneer: ids, item_template_id: template, grade: ui_grade(grade) });
            Ok(v.quote(template).map(|n| n.to_string()))
        })?,
    )
}

fn notice(rt: &UiRuntime, name: &str) {
    let text = super::alerts::localized(rt, "ERROR_MSG", name).unwrap_or_else(|| name.to_owned());
    rt.fire_event("SYSMSG", text);
}

fn item_name(bridge: &Shared, template: u32) -> String {
    bridge.borrow().game.get().and_then(|g| g.items.get(&template)).map(|i| i.named.name.clone()).unwrap_or_default()
}

fn fire(rt: &UiRuntime, bridge: &Shared, signal: Signal) {
    match signal {
        Signal::Attached(on) => rt.fire_event("AUCTION_ITEM_ATTACHMENT_STATE_CHANGED", on),
        Signal::Searching => rt.fire_event("AUCTION_ITEM_SEARCH", ()),
        Signal::Searched => rt.fire_event("AUCTION_ITEM_SEARCHED", ()),
        Signal::PutUp(template) => rt.fire_event("AUCTION_ITEM_PUT_UP", item_name(bridge, template)),
        Signal::Bidded(template, money) => rt.fire_event("AUCTION_BIDDED", (item_name(bridge, template), money.to_string())),
        Signal::Bought(template, money) => rt.fire_event("AUCTION_BOUGHT", (item_name(bridge, template), money.to_string())),
        Signal::Canceled(template) => rt.fire_event("AUCTION_CANCELED", item_name(bridge, template)),
        Signal::Lowest(template, grade, money) => rt.fire_event("AUCTION_LOWEST_PRICE", (template, ui_grade(grade), money.to_string())),
    }
}

fn window_open(rt: &UiRuntime) -> bool {
    rt.lua.globals().get::<mlua::Function>("IsActivatedAuction").and_then(|f| f.call::<bool>(())).unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    inventory: Res<Inventory>,
    units: Res<Units>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        events.clear();
        return;
    };
    let now = Instant::now();
    let mut opened = None;
    {
        let mut v = state.0.borrow_mut();
        if inventory.is_changed() || v.inventory.bag.is_empty() {
            v.inventory = inventory.clone();
        }
        for NetEvent(event) in events.read() {
            match event {
                Event::Interaction(InteractEvent::SkillList { npc_obj_id, skills, interactable: true, .. }) if skills.contains(&USE_AUCTIONEER) => {
                    // the NPC the player is talking to, as for the store
                    let npc = bridge.0.borrow().dialog.npc.filter(|n| n.0 == *npc_obj_id);
                    if npc.is_some() {
                        v.npc = npc;
                        opened = Some(*npc_obj_id);
                    }
                }
                Event::Auction(event) => v.on_event(event, now),
                // a refusal where a search was sent: nothing else will come
                Event::ServerError { .. } if v.searching.take().is_some() => v.signals.push(Signal::Searched),
                _ => {}
            }
        }
        if v.searching.is_some_and(|at| now.saturating_duration_since(at) >= SEARCH_TIMEOUT) {
            v.searching = None;
            v.signals.push(Signal::Searched);
        }
    }
    if let Some(npc) = opened {
        rt.fire_event("NPC_INTERACTION_START", ("auction", "", npc.to_string()));
    }
    // the player walked away or closed the window: the talk is over
    let npc = state.0.borrow().npc;
    if let Some(npc) = npc.filter(|_| opened.is_none()) {
        let me = bridge.0.borrow().pos;
        let near = units.get(npc.0).is_some_and(|u| me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) <= REACH);
        if !window_open(&rt) || !near || bridge.0.borrow().dialog.npc != Some(npc) {
            state.0.borrow_mut().npc = None;
            rt.fire_event("NPC_INTERACTION_END", ());
        }
    }
    let (signals, notices, unlock, commands) = {
        let mut v = state.0.borrow_mut();
        (std::mem::take(&mut v.signals), std::mem::take(&mut v.notices), std::mem::take(&mut v.unlock), std::mem::take(&mut v.commands))
    };
    for slot in unlock {
        if let Ok(bag) = rt.lua.globals().get::<Table>("X2Bag") {
            let _ = bag.call_method::<()>("UnlockSlot", slot);
        }
    }
    for signal in signals {
        fire(&rt, &bridge.0, signal);
    }
    for name in notices {
        notice(&rt, name);
    }
    for command in commands {
        net.write(NetCommand(Command::Auction(command)));
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud).before(aa_ui::UiFrame).run_if(in_state(Screen::InGame)).run_if(super::hud_up),
    );
}

#[cfg(test)]
mod tests;
