# aa-client: the auction house

The original window (`x2ui/auction`: Bid, List Item and Bid History tabs, unchanged Lua) runs on aa-net's auction
protocol. Code: `crates/aa-client/src/hud/auction.rs` (the module, `X2Auction`, the events), tests in
`hud/auction/tests.rs`; the NPC service lives in `hud/store.rs` (`Tables::auctioneers`).

## Opening it

The 1.2 client has no auction key: you talk to an auctioneer. `dialog::begin_talk` sends `CSStartInteraction` to an NPC
with a service (`stores.has_service`, now including `npcs.auctioneer`); the server answers `SCNpcInteractionSkillList`
with skill 12083 (`SkillsEnum.UseAuctioneer`). `auction::sync` then raises `NPC_INTERACTION_START("auction", "", npc)`
and the original `npc_interaction.lua` calls `ToggleAuctionWindow`. Like the store, the window closes with
`NPC_INTERACTION_END` when the player walks more than 4 m away or closes it. The HUD's remote auction button
(`featureSet.hudAuctionButton`) is off: the server only serves the house at an auctioneer in range (it checks the
packet's NPC ids and the NPC being interacted with).

Requests carry the auctioneer's object id (`AuctioneerIds { obj_id, obj_id2: 0 }`).

## `X2Auction`

| Group | Methods | Notes |
|---|---|---|
| Search | `SearchAuctionArticle(page, minLevel, maxLevel, grade, category, exact, name)`, `SearchAuctionArticleByPage`, `SearchRefresh`, `SearchedListSort`, `ClearSearchCondition`, `SearchDeclareSiege` (no-op) | `View::query` remembers the last search; by page, refresh, sort and the refresh after every change repeat it |
| My lots, bids | `SearchMyAuctionArticles(page)` (a search with `client_id` = the character), `SearchMyBidList(page)` (`MyBids`) | |
| Results | `GetSearchedItemCount/Page/TotalCount/SortInfo`, `GetSearchedItemInfo`, `GetSearchedItemPrice`, `GetSearchedItemArticleId`, `IsMyPutupArticle`, `GetLinkText` | `GetSearchedItemInfo` is the item's tooltip table (as the mail window's) plus `articleId`, `stackCount`, `seller`, `bidder`, `bidWorldId`, `leftTime`, `bidPriceStr`, `directPriceStr`, `extra`, `depositStr` |
| Trade | `BidAuctionArticle(id, money)`, `DirectPurchaseAuctionArticle(id)`, `CancelAuctionArticle(id)` | a buy-out is a bid of the buy-out price (what the server treats as one) |
| Listing | `AttachItemFromBag(slot)`, `AttachItemFromPick`, `DetachItem`, `GetAttachedItemExist/Info`, `SetPrice`, `SetDuration`, `CalcDeposit`, `AskAuctionArticle`, `GetLowestPrice` | the item's bag slot is locked while it is in the form |
| State | `SetOpen`, `SetCurTab`, `HasPostAuthority` (true), `RequirePostAuthority` (false), `IsAuctionNpc`, `GetCurrencyForBid/Fee` (gold) | `X2:GetCurrentWorldId` is a constant; a bid of the player's own character reports that world |

Events raised: `AUCTION_ITEM_SEARCH` (a request left), `AUCTION_ITEM_SEARCHED` (the page arrived), `AUCTION_ITEM_PUT_UP`,
`AUCTION_ITEM_ATTACHMENT_STATE_CHANGED`, `AUCTION_BIDDED`, `AUCTION_BOUGHT` (no 1.2 script listens), `AUCTION_CANCELED`,
`AUCTION_LOWEST_PRICE`. The chat lines ("Auction item listed: ...", "Auction Bid Placed: ...", "Auction listing
canceled: ...") are the original's.

### Mapping the window's values to the server's

* **Pages.** The window counts from 1, the server from 0 (`page - 1`, 0 counts as the first). The server never says how
  many pages there are: after a full page (9 lots) the pager offers one more; when that comes back empty the last page
  stays on screen and the pager stops there.
* **Grade.** The grade combo gives 1 for "All" and 3..12 for Uncommon..Mythic; the server filters by the raw item grade
  (0 = any): `value - 1`.
* **Category.** The tree's value is `AABBCC` (`MakeCategoryID`): the three `auction_*_category` ids the database
  assigns to items and the server compares (`1 weapon / 1 one-handed / 1 dagger` is `010101`).
* **Sort.** Column keys `evaluated`, `item_name`, `item_level`, `expire_date` are the server's sort kinds 1..4; a second
  click on the column turns the order. The server sorts, so a click asks for page 1 again.
* **Time left.** `end_time - (server time of the page + time since it arrived)`.
* **Deposit.** The server's fee (`aa_net::auction::listing_fee`): the buy-out price x 1 % x the duration's step (6 h is
  1, 12 h 2, 24 h 3, 48 h 4), at most 100 g. No buy-out price, no fee.
* **Level column.** An item without a level requirement shows level 1 (the original's `nil -> "1"`).

## What the client refuses before asking

The server drops several bad requests without an answer, so the client says why (`SYSMSG`, the original `ERROR_MSG`
texts): soul-bound item, a listing fee above the money (`CAN_NOT_PUTUP_MONEY`), a bid under the start price or not above
the top bid, a bid on the player's own lot, too little money for a bid or buy-out, a buy-out of a lot without a
buy-out price, cancelling a lot that has a bid. Server refusals (`SCErrorMsg`) already reach the screen through
`alerts.rs`. A search the server never answers (out of range: it only sends an error) frees the window's controls
after 4 s or at the error.

## Live check (lane `bevy`, two accounts, 2026-10-22)

Driven by `aa-client --headless --offscreen-shot --script` with `uiclick` on the original buttons; screenshots in
`shots/aa-client/auction_*.png`.

| Step | Result |
|---|---|
| Talk to auctioneer template 1413 (Caernord) | the original window opens (`auction_open.png`), requests carry NPC 21229 |
| List Item tab, right-click equivalent `AttachItemToAuction(slot)` | item, quantity and suggested start price in the form, slot locked (`auction_attached.png`) |
| buy-out 1 g per unit, List | `AuctionPost`, 40 s deposit paid, "Auction item listed: Iron Ore", the list shows the lot (`auction_listed.png`) |
| second account: name "Iron", Search, click the Name column | the lot appears, sorted (`auction_selected.png`) |
| Bid | original confirm dialog, bid 4 s 10 c taken, "Auction Bid Placed", Bid History "Your Bid" (`auction_bid_history.png`) |
| first account's List Item tab | the lot now reads "Bidding" (the original disables Cancel for it) |
| List two more, cancel the lot without a bid | `AuctionCancel`, "Auction listing canceled", cancel mail to the seller (`auction_cancelled.png`) |
| second account: Buy Now of the ingots | 20 g paid, lot gone, "Succesfull Purchase" mail for the buyer and "Successful Auction Notice" for the seller (`auction_bought.png`) |
| List 5 Copper Ore, 48 h | the form shows "Fee 20 silver" (4 % of the 5 g buy-out), `AuctionPost` duration 3 (`auction_48h.png`) |
| tree "Crafting > Raw Goods > Ore", hover an icon | both ore lots, "Your Bid" on the one bid on, the item tooltip (`auction_tooltip.png`) |
| walk 8 m away with the window open | `NPC_INTERACTION_END`, the window closes |

## Not done

* Chat lines for other players' actions (`AUCTION_BIDDEN`, `AUCTION_BOUGHT_BY_SOMEONE`): the server tells nothing but
  the settlement mails; the mailbox shows those.
* The siege declaration category (`SearchDeclareSiege`) and the AA-point currencies (the lane trades gold only).
* Claiming the settlement mails through the window is the mailbox's job (`mail.rs`) and was not re-tested here.
