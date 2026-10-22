use aa_net::wire::Writer;

use super::*;

/// A lot as the server sends it: item `template` x`count` (id 5000 + `id`), `seller`, prices and the current bidder.
fn lot(id: u64, template: u32, count: i32, seller: u32, start: i32, buyout: i32, bidder: u32, bid: i32) -> AuctionLot {
    let mut w = Writer::new();
    w.u64(id).u8(0).u32(template).u64(5000 + id).u8(0).u8(0).i32(count).u8(0).i64(0).i32(0).u32(0).u8(1).i64(0).i64(0);
    w.u8(1).u32(seller).string("Seller").i32(start).i32(buyout).i64(1_000_000);
    w.u8(if bidder == 0 { 255 } else { 1 }).u32(bidder).string(if bidder == 0 { "" } else { "Bidder" }).i32(bid).i32(0);
    AuctionLot::from_wire(w.as_bytes()).unwrap()
}

fn item(template: u32, id: u64, flags: u8) -> Item {
    Item { template_id: template, id, flags, count: 3, ..default() }
}

fn setup() -> (UiRuntime, Shared, State) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let mut game = super::super::data::GameData::default();
    game.items.insert(5, super::super::data::ItemInfo::default());
    assert!(bridge.borrow().game.0.set(game).is_ok());
    {
        let mut d = bridge.borrow_mut();
        d.character_id = 42;
        d.snapshot.name = "Self".into();
        d.snapshot.money = 10_000;
    }
    super::super::api::install(&mut rt, &bridge).unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    (rt, bridge, state)
}

#[test]
fn window_values_become_server_filters() {
    // the grade choice: 1 is "all", then 3 (Uncommon) up to 12 (Mythic)
    assert_eq!([raw_grade(1), raw_grade(3), raw_grade(11), raw_grade(12), raw_grade(99)], [0, 2, 10, 11, 11]);
    assert_eq!(categories(10_203), (1, 2, 3));
    assert_eq!(categories(60_000), (6, 0, 0));
    assert_eq!(categories(-1), (0, 0, 0));
    assert_eq!([server_page(0), server_page(1), server_page(3)], [0, 0, 2]);
    assert_eq!(sort_kind("item_level"), Some(3));
    assert_eq!(sort_kind("evaluated"), Some(1));
    assert_eq!(sort_kind("other"), None);
}

#[test]
fn the_pager_guesses_one_more_page_after_a_full_one() {
    let now = Instant::now();
    let full: Vec<_> = (1..=9).map(|i| lot(i, 5, 1, 7, 10, 0, 0, 0)).collect();
    let mut r = Results::default();
    r.apply(0, full.clone(), 0, now);
    assert_eq!((r.page, r.total_pages()), (0, 2));
    // the page behind the end is empty: the last one stays
    r.apply(1, vec![], 0, now);
    assert_eq!((r.page, r.lots.len(), r.total_pages()), (0, 9, 1));
    // a short page is the last at once; an empty first page is just empty
    r.apply(0, full[..4].to_vec(), 0, now);
    assert_eq!(r.total_pages(), 1);
    r.apply(0, vec![], 0, now);
    assert_eq!((r.lots.len(), r.total_pages()), (0, 1));
}

#[test]
fn time_left_runs_down_from_the_servers_clock() {
    let now = Instant::now();
    let mut r = Results::default();
    let l = lot(1, 5, 1, 7, 10, 0, 0, 0);
    r.apply(0, vec![l.clone()], 1_000_000 + 6 * 3600 - 100, now);
    assert_eq!(r.left(&l, now), 100);
    assert_eq!(r.left(&l, now + Duration::from_secs(30)), 70);
    assert_eq!(r.left(&l, now + Duration::from_secs(500)), 0);
}

#[test]
fn searches_ask_the_server_with_the_windows_conditions() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    state.0.borrow_mut().npc = Some((77, 1));
    rt.lua.load("X2Auction:SearchAuctionArticle(3, 10, 20, 4, 10203, false, 'Sword')").exec()?;
    let v = state.0.borrow();
    let AuctionCommand::Search { auctioneer, query } = &v.commands[0] else { panic!("{:?}", v.commands) };
    assert_eq!(auctioneer.obj_id, 77);
    assert_eq!(
        (query.keyword.as_str(), query.page, query.min_item_level, query.max_item_level, query.grade, (query.category_a, query.category_b, query.category_c)),
        ("Sword", 2, 10, 20, 3, (1, 2, 3))
    );
    assert_eq!(v.signals, [Signal::Searching]);
    Ok(())
}

#[test]
fn pages_sorting_and_refresh_repeat_the_last_search() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    rt.lua.load("X2Auction:SearchAuctionArticleByPage(2); X2Auction:SearchRefresh()").exec()?;
    assert!(state.0.borrow().commands.is_empty(), "nothing to repeat yet");
    rt.lua
        .load(
            "X2Auction:SearchAuctionArticle(1, 0, 0, 1, 0, false, 'Ore'); X2Auction:SearchAuctionArticleByPage(2);
             X2Auction:SearchedListSort('item_name'); X2Auction:SearchedListSort('item_name'); X2Auction:SearchRefresh()",
        )
        .exec()?;
    let v = state.0.borrow();
    let searches: Vec<_> = v
        .commands
        .iter()
        .map(|c| match c {
            AuctionCommand::Search { query, .. } => (query.keyword.clone(), query.page, query.sort_kind, query.sort_order),
            other => panic!("{other:?}"),
        })
        .collect();
    // by page keeps the conditions; a click on the column starts over, the second click turns the order
    assert_eq!(
        searches,
        [("Ore".into(), 0, 0, 0), ("Ore".into(), 1, 0, 0), ("Ore".into(), 0, 2, 0), ("Ore".into(), 0, 2, 1), ("Ore".into(), 0, 2, 1)]
    );
    assert_eq!(v.sort, (2, 1));
    Ok(())
}

#[test]
fn my_listings_are_the_players_lots_and_bids_a_page_of_their_own() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    rt.lua.load("X2Auction:SearchMyAuctionArticles(0); X2Auction:SearchMyBidList(1); X2Auction:SearchMyBidList(2)").exec()?;
    let v = state.0.borrow();
    assert!(matches!(&v.commands[0], AuctionCommand::Search { query, .. } if query.client_id == 42 && query.page == 0));
    assert!(matches!(&v.commands[1], AuctionCommand::MyBids { page: 0, .. }));
    assert!(matches!(&v.commands[2], AuctionCommand::MyBids { page: 1, .. }));
    Ok(())
}

#[test]
fn results_reach_the_window_as_item_tables() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    {
        let mut v = state.0.borrow_mut();
        v.on_event(
            &AuctionEvent::Searched {
                page: 0,
                count: 2,
                lots: vec![lot(1, 5, 4, 7, 100, 900, 0, 0), lot(2, 5, 1, 42, 50, 0, 42, 70)],
                error: 0,
                server_time: 1_000_000,
            },
            Instant::now(),
        );
        assert_eq!(v.signals, [Signal::Searched]);
    }
    rt.lua
        .load(
            "assert(X2Auction:GetSearchedItemCount()==2 and X2Auction:GetSearchedItemPage()==1 and X2Auction:GetSearchedItemTotalCount()==1)
             local a=X2Auction:GetSearchedItemInfo(1)
             assert(a.articleId=='1' and a.stackCount==4 and a.seller=='Seller' and a.bidder=='' and a.bidPriceStr=='100' and a.directPriceStr=='900' and a.itemType==5)
             assert(a.leftTime>6*3600-300 and a.depositStr=='9' and not X2Auction:IsMyPutupArticle(1) and a.level_requirement==nil)
             local b=X2Auction:GetSearchedItemInfo(2)
             assert(b.bidder=='Bidder' and b.bidPriceStr=='70' and b.extra=='70' and b.bidWorldId==X2:GetCurrentWorldId() and X2Auction:IsMyPutupArticle(2))
             local bid,direct=X2Auction:GetSearchedItemPrice(1); assert(bid=='100' and direct=='900')
             assert(X2Auction:GetSearchedItemArticleId(2)=='2' and X2Auction:GetSearchedItemInfo(3)==nil and X2Auction:GetSearchedItemArticleId(3)==nil)",
        )
        .exec()
}

#[test]
fn bids_and_buyouts_are_checked_before_they_are_sent() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    let lots = vec![lot(1, 5, 1, 7, 100, 900, 0, 0), lot(2, 5, 1, 7, 50, 0, 8, 60), lot(3, 5, 1, 42, 50, 0, 0, 0)];
    state.0.borrow_mut().results.apply(0, lots, 0, Instant::now());
    rt.lua
        .load(
            "X2Auction:BidAuctionArticle('1','99')        -- under the start price
             X2Auction:BidAuctionArticle('2','60')        -- not above the top bid
             X2Auction:BidAuctionArticle('3','60')        -- own lot
             X2Auction:BidAuctionArticle('2','90000')     -- more than the player has
             X2Auction:DirectPurchaseAuctionArticle('2')  -- not for sale outright
             X2Auction:BidAuctionArticle('1','20000')     -- over the buy-out price: the buy-out is paid
             X2Auction:DirectPurchaseAuctionArticle('1')",
        )
        .exec()?;
    let v = state.0.borrow();
    assert_eq!(
        v.notices,
        ["AUC_BID_MONEY_UNDER_START_MONEY", "AUC_BID_MONEY_UNDER_TOP_MOST", "AUC_BID_SELF", "NOT_ENOUGH_MONEY", "AUC_WRONG_DIRECT_MONEY"]
    );
    // the player has 10000: a bid of 20000 on a lot that buys out at 900 is a buy-out, as the direct purchase is
    assert_eq!(v.commands.len(), 2);
    for command in &v.commands {
        assert!(matches!(command, AuctionCommand::Bid { lot, bid, .. } if lot.id == 1 && bid.lot_id == 1));
    }
    assert!(matches!(&v.commands[1], AuctionCommand::Bid { bid, .. } if bid.amount == 900));
    assert!(v.commands[1].is_buyout());
    Ok(())
}

#[test]
fn only_unbid_own_lots_are_cancelled() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    let lots = vec![lot(1, 5, 1, 42, 100, 0, 0, 0), lot(2, 5, 1, 42, 50, 0, 8, 60), lot(3, 5, 1, 7, 50, 0, 0, 0)];
    state.0.borrow_mut().results.apply(0, lots, 0, Instant::now());
    rt.lua.load("X2Auction:CancelAuctionArticle('1'); X2Auction:CancelAuctionArticle('2'); X2Auction:CancelAuctionArticle('3')").exec()?;
    let v = state.0.borrow();
    assert_eq!(v.notices, ["AUC_CANNOT_CANCEL_IF_BID", "AUC_PERMISSION_DENY"]);
    assert!(matches!(&v.commands[..], [AuctionCommand::Cancel { lot, .. }] if lot.id == 1));
    Ok(())
}

#[test]
fn listing_an_item_from_the_bag() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    {
        let mut v = state.0.borrow_mut();
        v.inventory.bag.resize(8, None);
        v.inventory.bag[2] = Some(item(5, 9_007_199_254_740_993, 0));
        v.inventory.bag[3] = Some(item(5, 8, 1));
        v.npc = Some((77, 1));
    }
    rt.lua.load("HUD.bag[3]={uid='9007199254740993',id=5,count=3}; HUD.bag[4]={uid='8',id=5,count=3}").exec()?;
    rt.lua
        .load(
            "assert(not X2Auction:AttachItemFromBag(4), 'soul bound')
             assert(not X2Auction:AttachItemFromBag(7), 'empty')
             assert(X2Auction:AttachItemFromBag(3), 'attach')
             assert(X2Bag:IsLocked(3), 'locked')
             assert(X2Auction:GetAttachedItemExist())
             local i=X2Auction:GetAttachedItemInfo(); assert(i.itemType==5 and i.stack==3, 'info')
             assert(X2Auction:CalcDeposit('100','1000',1)==10 and X2Auction:CalcDeposit('100','1000',4)==40 and X2Auction:CalcDeposit('100','0',2)==0, 'deposit')
             X2Auction:SetPrice('100', 1000); X2Auction:SetDuration(3); X2Auction:AskAuctionArticle()",
        )
        .exec()?;
    {
        let v = state.0.borrow();
        assert_eq!(v.notices, ["AUC_SOUL_BOUND_ITEM"]);
        assert_eq!(v.signals, [Signal::Attached(true)]);
        assert!(matches!(
            &v.commands[..],
            [AuctionCommand::Post { item_id: 9_007_199_254_740_993, start_price: 100, buyout_price: 1000, duration: 2, auctioneer }] if auctioneer.obj_id == 77
        ));
    }
    // the server accepted it: the form empties and the slot unlocks
    let mut v = state.0.borrow_mut();
    v.signals.clear();
    v.on_event(&AuctionEvent::Posted(lot(9, 5, 3, 42, 100, 1000, 0, 0)), Instant::now());
    assert_eq!(v.signals, [Signal::PutUp(5)]);
    assert_eq!((v.attached.is_none(), v.unlock.clone()), (true, vec![3]));
    Ok(())
}

#[test]
fn a_listing_that_costs_more_than_the_player_has_is_refused() -> mlua::Result<()> {
    let (rt, bridge, state) = setup();
    bridge.borrow_mut().snapshot.money = 5;
    state.0.borrow_mut().inventory.bag.resize(2, None);
    state.0.borrow_mut().inventory.bag[0] = Some(item(5, 8, 0));
    rt.lua.load("X2Auction:AttachItemFromBag(1); X2Auction:SetPrice('0','1000'); X2Auction:SetDuration(1); X2Auction:AskAuctionArticle()").exec()?;
    let v = state.0.borrow();
    assert_eq!(v.notices, ["CAN_NOT_PUTUP_MONEY"]);
    assert!(v.commands.is_empty());
    Ok(())
}

#[test]
fn detaching_unlocks_the_slot_and_tells_the_window() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    state.0.borrow_mut().inventory.bag.resize(2, None);
    state.0.borrow_mut().inventory.bag[1] = Some(item(5, 8, 0));
    rt.lua.load("X2Auction:AttachItemFromBag(2); X2Auction:DetachItem(); X2Auction:DetachItem()").exec()?;
    let v = state.0.borrow();
    assert_eq!(v.signals, [Signal::Attached(true), Signal::Attached(false)]);
    assert_eq!(v.unlock, [2]);
    Ok(())
}

#[test]
fn answers_repeat_the_list_and_cache_the_lowest_price() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    rt.lua.load("X2Auction:SearchMyAuctionArticles(1)").exec()?;
    let mut v = state.0.borrow_mut();
    v.commands.clear();
    v.signals.clear();
    let now = Instant::now();
    v.on_event(&AuctionEvent::Canceled(lot(1, 5, 1, 42, 10, 0, 0, 0)), now);
    let bid = lot(1, 5, 1, 7, 10, 0, 0, 0).bid(15, 1, 42, "Self");
    v.on_event(&AuctionEvent::Bid { bid, is_buyout: false, item_template_id: 5 }, now);
    assert_eq!(v.signals, [Signal::Canceled(5), Signal::Searching, Signal::Bidded(5, 15), Signal::Searching]);
    assert_eq!(v.commands.len(), 2);
    v.on_event(&AuctionEvent::LowestPrice { item_template_id: 5, grade: 0, amount: 120 }, now);
    assert_eq!(v.quote(5), Some(120));
    drop(v);
    rt.lua.load("assert(X2Auction:GetLowestPrice(5, 1)=='120')").exec()?;
    assert!(matches!(state.0.borrow().commands.last(), Some(AuctionCommand::LowestPrice { item_template_id: 5, grade: 0, .. })));
    Ok(())
}
