//! Fresh paired whole-stack trade protocol; no other accounts or item grants.
//! cargo test -p aa-net --test live_trade -- --ignored --nocapture
use aa_net::interaction::Interact;
use aa_net::model::{gender, race};
use aa_net::shop::{BuyEntry, SellEntry, ShopCommand};
use aa_net::{
    Client, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, Pos, TradeCommand,
    TradeEvent, WorldState,
};
use anyhow::{Context, Result, bail, ensure};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
struct Peer {
    client: Client,
    state: WorldState,
    user: String,
}

impl Peer {
    fn send(&self, command: TradeCommand) {
        assert!(self.client.send(Command::Trade(command)));
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
                    Event::Trade(TradeEvent::RequestRejected { message, .. }) => {
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
            format!("nettrade{stamp}{suffix}"),
            Some(format!("T{suffix}{stamp}")),
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

async fn fund(peer: &mut Peer) -> Result<()> {
    let pos = Pos::new(14366.27, 15504.10, 152.86171);
    peer.client.send(Command::Move(MoveCommand {
        pos,
        ..MoveCommand::default()
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
        ..MoveCommand::default()
    }));
    peer.client.send(Command::Interact(Interact::Npc {
        obj_id: npc.obj_id,
        target_changed: true,
    }));
    let sales: Vec<_> = peer
        .state
        .inventory
        .equipment
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, i)| i.as_ref().map(|i| (slot, i)))
        .filter(|(_, i)| i.id != 0 && matches!(i.template_id, 23387 | 23388))
        .map(|(slot, i)| SellEntry {
            container: 1,
            slot: slot as u8,
            item_id: i.id,
        })
        .collect();
    ensure!(sales.len() == 2, "missing own starter shirt/pants");
    peer.client.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc.obj_id,
        items: sales.clone(),
    }));
    peer.wait_state("own starter gear funds", |s| {
        s.stats.money >= 50
            && sales
                .iter()
                .all(|i| s.inventory.find_by_id(i.item_id).is_none())
    })
    .await?;
    let money = peer.state.stats.money;
    let count = peer.state.inventory.count_of(15659);
    // Normal gold merchant12547/pack171 sells this unbound stackable supply for25c.
    peer.client.send(Command::Shop(ShopCommand::Buy {
        npc_obj_id: npc.obj_id,
        doodad_obj_id: 0,
        buy: vec![BuyEntry {
            item_id: 15659,
            grade: 0,
            count: 2,
            currency: 0,
        }],
        buy_back: vec![],
        use_aa_point: false,
    }));
    peer.wait_state("purchase own two-item trade stack", |s| {
        s.stats.money == money - 50 && s.inventory.count_of(15659) == count + 2
    })
    .await?;
    println!(
        "{} funded {}c after own starter gear sale and two25c supply purchases",
        peer.state.stats.name, peer.state.stats.money
    );
    Ok(())
}

async fn start(a: &mut Peer, b: &mut Peer) -> Result<()> {
    a.send(TradeCommand::Invite {
        obj_id: b.state.stats.obj_id,
    });
    b.until("trade invitation", |e, _| {
        matches!(e, Event::Trade(TradeEvent::Invited { .. }))
    })
    .await?;
    ensure!(b.state.trade.incoming_invite == Some(a.state.stats.obj_id));
    b.send(TradeCommand::Accept {
        obj_id: a.state.stats.obj_id,
    });
    b.wait_state("accepted trade", |s| s.trade.active).await?;
    a.wait_state("started trade", |s| s.trade.active).await?;
    ensure!(
        a.state.trade.partner_obj_id == b.state.stats.obj_id
            && b.state.trade.partner_obj_id == a.state.stats.obj_id
    );
    Ok(())
}

async fn offer(peer: &mut Peer, id: u64) -> Result<()> {
    let (container, slot, _) = peer
        .state
        .inventory
        .find_by_id(id)
        .context("own offered item")?;
    ensure!(container == 2);
    peer.send(
        peer.state
            .trade
            .plan_offer(&peer.state.inventory, slot)
            .map_err(anyhow::Error::msg)?,
    );
    peer.wait_state("own offer acknowledged with identity", |s| {
        s.trade
            .mine
            .iter()
            .any(|o| o.item.as_ref().is_some_and(|i| i.id == id))
    })
    .await
}

async fn money(peer: &mut Peer, amount: i32) -> Result<()> {
    peer.send(TradeCommand::OfferMoney { amount });
    peer.until(
        "money offer acknowledged",
        |e, _| matches!(e,Event::Trade(TradeEvent::MoneyOffered(value)) if *value==amount),
    )
    .await
}

async fn both_locked(a: &mut Peer, b: &mut Peer) -> Result<()> {
    a.send(TradeCommand::Lock { locked: true });
    a.wait_state("my lock", |s| s.trade.my_locked).await?;
    b.wait_state("other lock", |s| s.trade.other_locked).await?;
    b.send(TradeCommand::Lock { locked: true });
    b.wait_state("both B locks", |s| s.trade.can_confirm())
        .await?;
    a.wait_state("both A locks", |s| s.trade.can_confirm())
        .await
}

async fn exercise(a: &mut Peer, b: &mut Peer) -> Result<()> {
    fund(a).await?;
    fund(b).await?;
    // Use whole unbound starter supply stacks; identity/count must survive the round trip.
    let choose = |s: &WorldState| {
        s.inventory
            .bag_items()
            .filter(|(_, i)| i.id != 0 && i.count > 1 && i.flags & 1 == 0)
            .max_by_key(|(_, i)| i.count)
            .map(|(_, i)| (i.id, i.count))
    };
    let (a_item, a_count) = choose(&a.state).context("no unbound own stacked supply A")?;
    let (b_item, b_count) = choose(&b.state).context("no unbound own stacked supply B")?;
    let (a_money, b_money) = (a.state.stats.money, b.state.stats.money);
    a.send(TradeCommand::Invite {
        obj_id: b.state.stats.obj_id,
    });
    b.wait_state("initial invitation", |s| {
        s.trade.incoming_invite == Some(a.state.stats.obj_id)
    })
    .await?;
    b.send(TradeCommand::Decline {
        obj_id: a.state.stats.obj_id,
        reason: 1,
    });
    a.until("decline acknowledgement", |e, _| {
        matches!(e, Event::Trade(TradeEvent::Declined { reason: 1, .. }))
    })
    .await?;
    ensure!(!a.state.trade.active && !b.state.trade.active);
    start(a, b).await?;
    offer(a, a_item).await?;
    offer(b, b_item).await?;
    a.wait_state("other exact offer A", |s| {
        s.trade
            .other
            .iter()
            .any(|i| i.id == b_item && i.count == b_count)
    })
    .await?;
    b.wait_state("other exact offer B", |s| {
        s.trade
            .other
            .iter()
            .any(|i| i.id == a_item && i.count == a_count)
    })
    .await?;
    money(a, 5).await?;
    money(b, 2).await?;
    a.wait_state("other money2", |s| s.trade.other_money == 2)
        .await?;
    b.wait_state("other money5", |s| s.trade.other_money == 5)
        .await?;
    ensure!(a.state.stats.money == a_money && b.state.stats.money == b_money);
    a.send(TradeCommand::Lock { locked: true });
    a.wait_state("A locked", |s| s.trade.my_locked).await?;
    b.wait_state("B sees A lock", |s| s.trade.other_locked)
        .await?;
    // Native allows B to edit because B is still unlocked. The edit invalidates A's lock.
    money(b, 3).await?;
    for peer in [&mut *a, &mut *b] {
        peer.until("offer edit unlocks both", |e, s| {
            matches!(
                e,
                Event::Trade(TradeEvent::Locked {
                    mine: false,
                    other: false
                })
            ) && !s.trade.my_locked
                && !s.trade.other_locked
                && !s.trade.my_ok
                && !s.trade.other_ok
        })
        .await?;
    }
    let (_, slot, _) = a.state.inventory.find_by_id(a_item).unwrap();
    a.send(TradeCommand::RemoveItem { slot: slot as u8 });
    a.wait_state("remove own offer", |s| s.trade.mine.is_empty())
        .await?;
    b.wait_state("remove remote offer by ID", |s| s.trade.other.is_empty())
        .await?;
    offer(a, a_item).await?;
    a.send(TradeCommand::Cancel { reason: 0 });
    for peer in [&mut *a, &mut *b] {
        peer.until("cancel without settlement", |e, s| {
            matches!(e, Event::Trade(TradeEvent::Canceled { .. })) && !s.trade.active
        })
        .await?;
    }
    ensure!(
        a.state.trade.last_canceled == Some((0, true))
            && b.state.trade.last_canceled == Some((0, false))
    );
    ensure!(
        a.state.inventory.find_by_id(a_item).unwrap().2.count == a_count
            && b.state.inventory.find_by_id(b_item).unwrap().2.count == b_count
    );
    ensure!(a.state.stats.money == a_money && b.state.stats.money == b_money);
    println!(
        "invite/decline, whole-stack offers, remove/reoffer, edit-driven lock invalidation and cancel confirmed"
    );

    start(a, b).await?;
    offer(a, a_item).await?;
    offer(b, b_item).await?;
    money(a, 5).await?;
    money(b, 3).await?;
    both_locked(a, b).await?;
    a.send(TradeCommand::Confirm);
    a.wait_state("first confirmation", |s| s.trade.my_ok && !s.trade.other_ok)
        .await?;
    b.wait_state("other confirmed", |s| s.trade.other_ok && !s.trade.my_ok)
        .await?;
    ensure!(a.state.stats.money == a_money && b.state.stats.money == b_money);
    b.send(TradeCommand::Confirm);
    for peer in [&mut *a, &mut *b] {
        peer.until("authoritative trade settlement", |e, s| {
            matches!(e, Event::Trade(TradeEvent::Made(_)))
                && !s.trade.active
                && s.trade.last_settlement.is_some()
        })
        .await?;
    }
    ensure!(a.state.stats.money == a_money - 2 && b.state.stats.money == b_money + 2);
    ensure!(
        a.state.inventory.find_by_id(a_item).is_none()
            && b.state.inventory.find_by_id(b_item).is_none()
    );
    ensure!(
        a.state
            .inventory
            .find_by_id(b_item)
            .is_some_and(|(c, _, i)| c == 2 && i.count == b_count)
    );
    ensure!(
        b.state
            .inventory
            .find_by_id(a_item)
            .is_some_and(|(c, _, i)| c == 2 && i.count == a_count)
    );
    println!(
        "settlement: exact IDs{a_item}/{b_item}, full counts{a_count}/{b_count}, only final money deltas A-2/B+2"
    );

    start(a, b).await?;
    b.logout().await?;
    a.until("departing partner cancels trade", |e, s| {
        matches!(e, Event::Trade(TradeEvent::Canceled { .. })) && !s.trade.active
    })
    .await?;
    b.reconnect().await?;
    ensure!(
        b.state.stats.money == b_money + 2
            && b.state
                .inventory
                .find_by_id(a_item)
                .is_some_and(|(_, _, i)| i.count == a_count)
    );
    ensure!(!b.state.trade.active && b.state.trade.mine.is_empty());
    println!(
        "disconnect cleanup and recipient saved balance/item identity verified after own reconnect"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires bevy lane; creates two fresh isolated accounts and exchanges only their starter supplies"]
async fn full_stack_trade_locking_cancel_settlement_and_lifecycle() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::fresh(stamp, "a").await?;
    let mut b = match Peer::fresh(stamp, "b").await {
        Ok(b) => b,
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
    println!("both isolated trade peers logged out cleanly");
    Ok(())
}
