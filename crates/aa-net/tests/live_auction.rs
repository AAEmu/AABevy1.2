//! Fresh paired auctions: exact escrow identities, cancellation, bids, buyout and mail settlement.
//! Set AA_NET_RUN_AUCTION_MONEY_CLAIMS=1 only after reviewing the documented lane stall.
//! cargo test -p aa-net --test live_auction -- --ignored --nocapture
use aa_net::interaction::Interact;
use aa_net::model::{gender, race};
use aa_net::shop::{SellEntry, ShopCommand};
use aa_net::{
    AuctionCommand, AuctionEvent, AuctionLot, AuctionSearch, AuctioneerIds, Client, Command,
    CreateCharacter, Event, LeaveTarget, MailCommand, MailEvent, MoveCommand, Pos, WorldState,
};
use anyhow::{Context, Result, bail, ensure};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
struct Peer {
    client: Client,
    state: WorldState,
    user: String,
}

impl Peer {
    fn send(&self, command: AuctionCommand) {
        assert!(self.client.send(Command::Auction(command)));
    }

    async fn until(
        &mut self,
        what: &str,
        pick: impl Fn(&Event, &WorldState) -> bool,
    ) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(25), async {
            while let Some(event) = self.client.recv().await {
                self.state.apply(&event);
                match &event {
                    Event::LoginFailed { message }
                    | Event::ConnectFailed { message }
                    | Event::Disconnected { message } => bail!("{what}: {message}"),
                    Event::ServerError { code } if *code != 155 => {
                        bail!("{what}: server error {code}")
                    }
                    Event::CharacterCreateFailed { reason } => {
                        bail!("{what}: character create {reason}")
                    }
                    Event::EnterWorldDenied { reason } => bail!("{what}: world denied {reason}"),
                    Event::Auction(AuctionEvent::RequestRejected { message, .. }) => {
                        bail!("{what}: {message}")
                    }
                    _ => {}
                }
                if pick(&event, &self.state) {
                    return Ok(());
                }
            }
            bail!("{what}: session ended")
        })
        .await
        .with_context(|| format!("{what}: timed out"))?
    }

    async fn wait_state(&mut self, what: &str, pick: impl Fn(&WorldState) -> bool) -> Result<()> {
        if pick(&self.state) {
            return Ok(());
        }
        self.until(what, |_, state| pick(state)).await
    }

    async fn connect(user: String, create: Option<String>) -> Result<Self> {
        let mut peer = Self {
            client: Client::spawn_on(&tokio::runtime::Handle::current()),
            state: WorldState::default(),
            user: user.clone(),
        };
        peer.client.send(Command::Login {
            addr: aa_net::default_login(),
            user: user.clone(),
            password: user.into(),
        });
        peer.until("world list", |e, _| matches!(e, Event::WorldList(_)))
            .await?;
        let world_id = peer
            .state
            .worlds
            .list
            .worlds
            .iter()
            .find(|w| w.online)
            .context("no online world")?
            .id;
        peer.client.send(Command::EnterWorld { world_id });
        peer.until("cookie", |e, _| matches!(e, Event::ReadyToConnect { .. }))
            .await?;
        peer.client.send(Command::ConnectToWorld);
        peer.until("characters", |e, _| matches!(e, Event::CharacterList(_)))
            .await?;
        if let Some(name) = create {
            peer.client
                .send(Command::CreateCharacter(CreateCharacter::new(
                    name,
                    race::NUIAN,
                    gender::MALE,
                )));
            peer.until(
                "created",
                |e, _| matches!(e, Event::CharacterList(list) if !list.is_empty()),
            )
            .await?;
        }
        let id = peer
            .state
            .characters
            .list
            .first()
            .context("missing own character")?
            .id;
        peer.client.send(Command::EnterGame { character_id: id });
        peer.until("spawn", |e, _| matches!(e, Event::EnteredWorld))
            .await?;
        println!("online {} id={id}", peer.state.stats.name);
        Ok(peer)
    }

    async fn fresh(stamp: u128, suffix: &str) -> Result<Self> {
        Self::connect(
            format!("netauction{stamp}{suffix}"),
            Some(format!("A{suffix}{stamp}")),
        )
        .await
    }

    async fn logout(&mut self) -> Result<()> {
        if self.state.player.spawn.is_none() {
            return Ok(());
        }
        self.client.send(Command::LeaveWorld(LeaveTarget::Quit));
        self.until("logout", |e, _| matches!(e, Event::LeftWorld { .. }))
            .await
    }

    async fn reconnect(&mut self) -> Result<()> {
        ensure!(
            self.state.player.spawn.is_none(),
            "must log out own session before reconnecting"
        );
        *self = Self::connect(self.user.clone(), None).await?;
        Ok(())
    }
}

async fn fund(peer: &mut Peer, preserve_pants: bool) -> Result<Option<aa_net::model::Item>> {
    peer.client.send(Command::Move(MoveCommand {
        pos: Pos::new(14366.27, 15504.10, 152.86171),
        ..Default::default()
    }));
    peer.wait_state("merchant visible", |s| {
        s.units
            .by_id
            .values()
            .any(|u| u.kind.template_id() == 12547)
    })
    .await?;
    let npc = peer
        .state
        .units
        .by_id
        .values()
        .find(|u| u.kind.template_id() == 12547)
        .context("merchant")?
        .clone();
    peer.client.send(Command::Move(MoveCommand {
        pos: npc.pos,
        ..Default::default()
    }));
    peer.client.send(Command::Interact(Interact::Npc {
        obj_id: npc.obj_id,
        target_changed: true,
    }));
    let pants = peer
        .state
        .inventory
        .equipment
        .slots
        .iter()
        .flatten()
        .find(|i| i.template_id == 23388 && i.id > 0)
        .context("own starter pants")?
        .clone();
    let sales: Vec<_> = peer
        .state
        .inventory
        .equipment
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| item.as_ref().map(|i| (slot, i)))
        .filter(|(_, i)| i.id > 0 && (!preserve_pants || i.id != pants.id))
        .map(|(slot, i)| SellEntry {
            container: 1,
            slot: slot as u8,
            item_id: i.id,
        })
        .collect();
    ensure!(!sales.is_empty(), "no own starter gear to sell");
    peer.client.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc.obj_id,
        items: sales.clone(),
    }));
    peer.wait_state("starter gear funds", |s| {
        sales
            .iter()
            .all(|i| s.inventory.find_by_id(i.item_id).is_none())
            && s.stats.money >= 110
    })
    .await?;
    if preserve_pants {
        let (container, slot, _) = peer
            .state
            .inventory
            .find_by_id(pants.id)
            .context("preserved pants")?;
        let empty = peer
            .state
            .inventory
            .bag
            .iter()
            .position(Option::is_none)
            .context("empty bag slot")?;
        peer.client
            .send(Command::SwapItems(aa_net::items::ItemMove {
                from_item_id: pants.id,
                to_item_id: 0,
                from_container: container,
                from_slot: slot as u8,
                to_container: 2,
                to_slot: empty as u8,
            }));
        peer.wait_state("unequip owned pants for auction", |s| {
            s.inventory
                .find_by_id(pants.id)
                .is_some_and(|(c, _, _)| c == 2)
        })
        .await?;
    }
    println!(
        "{} funded {}c solely by own starter gear; escrow candidate{}",
        peer.state.stats.name, peer.state.stats.money, pants.id
    );
    Ok(preserve_pants.then_some(pants))
}
fn context() -> AuctioneerIds {
    AuctioneerIds::default()
}
async fn search(peer: &mut Peer, seller: u32) -> Result<()> {
    peer.send(AuctionCommand::Search {
        auctioneer: context(),
        query: AuctionSearch {
            client_id: seller,
            ..Default::default()
        },
    });
    peer.until("own-seller auction query", |e, _| {
        matches!(e, Event::Auction(AuctionEvent::Searched { .. }))
    })
    .await
}
async fn post(peer: &mut Peer, item_id: u64) -> Result<AuctionLot> {
    peer.send(AuctionCommand::Post {
        auctioneer: context(),
        item_id,
        start_price: 10,
        buyout_price: 100,
        duration: 0,
    });
    peer.until(
        "posted with exact escrow item",
        |e, _| matches!(e,Event::Auction(AuctionEvent::Posted(lot)) if lot.item_id()==item_id),
    )
    .await?;
    peer.wait_state("escrow leaves owned visible inventory", |s| {
        s.inventory.find_by_id(item_id).is_none()
    })
    .await?;
    Ok(peer
        .state
        .auction
        .last_posted
        .clone()
        .context("posted lot")?)
}
async fn read(peer: &mut Peer, mail_id: i64) -> Result<()> {
    peer.client.send(Command::Mail(MailCommand::Read {
        is_sent: false,
        mail_id,
    }));
    peer.until("owned auction mail body",|e,_|matches!(e,Event::Mail(MailEvent::Body{is_sent:false,body,..}) if body.mail_id==mail_id)).await
}
async fn mail(peer: &mut Peer, kind: u8) -> Result<i64> {
    peer.wait_state("auction mail delivered", |s| {
        s.mailbox.received.iter().any(|h| h.mail_type == kind)
    })
    .await?;
    let id = peer
        .state
        .mailbox
        .received
        .iter()
        .find(|h| h.mail_type == kind)
        .context("auction mail")?
        .mail_id;
    read(peer, id).await?;
    Ok(id)
}
async fn claim_delete(peer: &mut Peer, id: i64) -> Result<()> {
    peer.client
        .send(Command::Mail(MailCommand::TakeAllAndDelete { mail_id: id }));
    peer.wait_state("auction mail claimed and deleted", |s| {
        s.mailbox.header(false, id).is_none()
    })
    .await
}
async fn exercise(a: &mut Peer, b: &mut Peer) -> Result<()> {
    let item = fund(a, true).await?.context("reserved escrow item")?;
    fund(b, false).await?;
    let a_money = a.state.stats.money;
    let b_money = b.state.stats.money;
    let seller = a.state.stats.character_id;
    let lot = post(a, item.id).await?;
    ensure!(
        lot.item.as_ref() == Some(&item),
        "posted item metadata changed"
    );
    a.wait_state("first listing deposit", |s| s.stats.money == a_money - 1)
        .await?;
    search(a, seller).await?;
    ensure!(a.state.auction.reported_count == 1 && a.state.auction.lot(lot.id).is_some());
    a.send(AuctionCommand::LowestPrice {
        auctioneer: context(),
        item_template_id: item.template_id,
        grade: item.grade,
    });
    a.until("lowest-price quote",|e,_|matches!(e,Event::Auction(AuctionEvent::LowestPrice{item_template_id,..}) if *item_template_id==item.template_id)).await?;
    search(b, seller).await?;
    let cached = b
        .state
        .auction
        .lot(lot.id)
        .context("peer sees exact owned lot")?
        .clone();
    ensure!(cached.item == lot.item);
    a.send(AuctionCommand::Cancel {
        auctioneer: context(),
        lot: a
            .state
            .auction
            .lot(lot.id)
            .context("owner cached lot")?
            .clone(),
    });
    a.until(
        "cancel ACK",
        |e, _| matches!(e,Event::Auction(AuctionEvent::Canceled(l)) if l.id==lot.id),
    )
    .await?;
    let id = mail(a, 13).await?;
    ensure!(
        a.state
            .mailbox
            .body(false, id)
            .unwrap()
            .items
            .iter()
            .flatten()
            .any(|i| i.item == item)
    );
    claim_delete(a, id).await?;
    a.wait_state("cancel returns original escrow identity", |s| {
        s.inventory
            .find_by_id(item.id)
            .is_some_and(|(c, _, i)| c == 2 && *i == item)
    })
    .await?;
    ensure!(
        a.state.stats.money == a_money - 1,
        "cancel refunded listing fee unexpectedly"
    );
    println!(
        "cancel preserved full equipment ID{} and charged exact1c listing fee",
        item.id
    );

    let lot = post(a, item.id).await?;
    a.wait_state("second listing fee", |s| s.stats.money == a_money - 2)
        .await?;
    search(b, seller).await?;
    let lot = b.state.auction.lot(lot.id).context("reposted lot")?.clone();
    let bid = lot.bid(
        10,
        0,
        b.state.stats.character_id,
        b.state.stats.name.clone(),
    );
    b.send(AuctionCommand::Bid {
        auctioneer: context(),
        lot: lot.clone(),
        bid,
    });
    b.until("normal bid authoritative ACK",|e,_|matches!(e,Event::Auction(AuctionEvent::Bid{bid,is_buyout:false,..}) if bid.lot_id==lot.id && bid.amount==10)).await?;
    b.wait_state("full normal bid escrow", |s| s.stats.money == b_money - 10)
        .await?;
    b.send(AuctionCommand::MyBids {
        auctioneer: context(),
        page: 0,
    });
    b.until("my bids snapshot", |e, _| {
        matches!(e, Event::Auction(AuctionEvent::Searched { .. }))
    })
    .await?;
    let cached = b.state.auction.lot(lot.id).context("my bid lot")?.clone();
    ensure!(cached.bidder_id == b.state.stats.character_id && cached.bid_price == 10);
    b.send(AuctionCommand::Bid {
        auctioneer: context(),
        lot: cached.clone(),
        bid: cached.bid(
            100,
            0,
            b.state.stats.character_id,
            b.state.stats.name.clone(),
        ),
    });
    b.wait_state("buyout charges100c in addition to earlier bid", |s| {
        s.stats.money == b_money - 110
    })
    .await?;
    b.wait_state("buyout win header", |s| {
        s.mailbox.received.iter().any(|h| h.mail_type == 16)
    })
    .await?;
    ensure!(
        b.state.auction.last_purchase_mail.is_none(),
        "header alone fabricated settlement proof"
    );
    ensure!(
        b.state.inventory.find_by_id(item.id).is_none(),
        "buyout directly fabricated an inventory item before mail claim"
    );
    let refund = mail(b, 15).await?;
    ensure!(b.state.mailbox.body(false, refund).unwrap().money == 10);
    claim_delete(b, refund).await?;
    b.wait_state("prior bid refund", |s| s.stats.money == b_money - 100)
        .await?;
    let win = mail(b, 16).await?;
    ensure!(
        b.state.auction.last_purchase_mail
            == Some(aa_net::AuctionPurchaseMail {
                lot_id: lot.id,
                mail_id: win,
                item_id: item.id
            })
    );
    ensure!(
        !b.state
            .auction
            .pending
            .iter()
            .any(AuctionCommand::is_buyout)
    );
    ensure!(
        b.state
            .mailbox
            .body(false, win)
            .unwrap()
            .items
            .iter()
            .flatten()
            .any(|a| a.item == item)
    );
    claim_delete(b, win).await?;
    b.wait_state("buyer claims original equipment", |s| {
        s.inventory
            .find_by_id(item.id)
            .is_some_and(|(c, _, i)| c == 2 && *i == item)
    })
    .await?;
    let receipt = mail(a, 14).await?;
    ensure!(a.state.mailbox.body(false, receipt).unwrap().money == 90);
    claim_delete(a, receipt).await?;
    a.wait_state("seller receives90percent without deposit refund", |s| {
        s.stats.money == a_money - 2 + 90
    })
    .await?;
    search(a, seller).await?;
    ensure!(a.state.auction.reported_count == 0 && a.state.auction.lots.is_empty());
    b.send(AuctionCommand::MyBids {
        auctioneer: context(),
        page: 0,
    });
    b.until("my bid disappears after buyout", |e, _| {
        matches!(e, Event::Auction(AuctionEvent::Searched { count: 0, .. }))
    })
    .await?;
    ensure!(a.state.inventory.find_by_id(item.id).is_none());
    println!(
        "normal10c bid,100c buyout,10c own-bid refund,90c seller receipt and exact win ItemID{} all authoritative",
        item.id
    );
    b.logout().await?;
    b.reconnect().await?;
    ensure!(
        b.state.stats.money == b_money - 100
            && b.state
                .inventory
                .find_by_id(item.id)
                .is_some_and(|(_, _, i)| *i == item)
    );
    println!(
        "buyer item identity/full details and exact balance persisted across owned logout/reconnect"
    );
    Ok(())
}
#[tokio::test]
#[ignore = "requires bevy lane; creates two fresh owned accounts and auctions only their starter item"]
async fn isolated_auction_cancel_bid_buyout_and_mail_settlement() -> Result<()> {
    ensure!(
        std::env::var("AA_NET_RUN_AUCTION_MONEY_CLAIMS").as_deref() == Ok("1"),
        "live auction run stalled at money-mail claim; review docs/aa-net.md and set AA_NET_RUN_AUCTION_MONEY_CLAIMS=1 only after that server issue is resolved"
    );
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::fresh(stamp, "a").await?;
    let mut b = match Peer::fresh(stamp, "b").await {
        Ok(p) => p,
        Err(e) => {
            let _ = a.logout().await;
            return Err(e);
        }
    };
    let result = exercise(&mut a, &mut b).await;
    let (out_a, out_b) = tokio::join!(a.logout(), b.logout());
    result?;
    out_a?;
    out_b?;
    println!("both isolated auction peers logged out");
    Ok(())
}
