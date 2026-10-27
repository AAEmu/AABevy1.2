//! Opt-in mailbox flow using two fresh accounts and only their own starter items/copper.
//! cargo test -p aa-net --test live_mail -- --ignored --nocapture

use aa_net::interaction::Interact;
use aa_net::model::{gender, race};
use aa_net::shop::{SellEntry, ShopCommand};
use aa_net::{
    Client, Command, CreateCharacter, Event, LeaveTarget, MailCommand, MailEvent, MailSlot,
    MoveCommand, Pos, SendMail, WorldState,
};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Peer {
    client: Client,
    state: WorldState,
    user: String,
}

impl Peer {
    fn send(&self, command: MailCommand) {
        assert!(self.client.send(Command::Mail(command)));
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
                    Event::Mail(MailEvent::RequestRejected { message, .. }) => {
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
            format!("netmail{stamp}{suffix}"),
            Some(format!("M{suffix}{stamp}")),
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

async fn list(peer: &mut Peer) -> Result<()> {
    peer.send(MailCommand::List);
    peer.until("mail list", |e, _| {
        matches!(e, Event::Mail(MailEvent::ListEnd { .. }))
    })
    .await
}

async fn read(peer: &mut Peer, is_sent: bool, id: i64) -> Result<()> {
    peer.send(MailCommand::Read {
        is_sent,
        mail_id: id,
    });
    peer.until("mail body",|e,_|matches!(e,Event::Mail(MailEvent::Body{is_sent:s,body,..}) if *s==is_sent && body.mail_id==id)).await
}

async fn exercise(a: &mut Peer, b: &mut Peer, db: &Connection) -> Result<()> {
    list(a).await?;
    list(b).await?;
    ensure!(a.state.mailbox.received.is_empty() && a.state.mailbox.sent.is_empty());
    ensure!(b.state.mailbox.received.is_empty() && b.state.mailbox.sent.is_empty());
    let name = b.state.stats.name.clone();
    let area = Pos::new(14366.27, 15504.10, 152.86171);
    a.client.send(Command::Move(MoveCommand {
        pos: area,
        ..MoveCommand::default()
    }));
    a.until("merchant/mailbox area", |_, s| {
        s.units
            .by_id
            .values()
            .any(|u| u.kind.template_id() == 12547)
            && s.doodads.by_id.values().any(|d| d.template_id == 320)
    })
    .await?;
    let npc = a
        .state
        .units
        .by_id
        .values()
        .find(|u| u.kind.template_id() == 12547)
        .context("merchant")?
        .clone();
    let mailbox = a
        .state
        .doodads
        .by_id
        .values()
        .filter(|d| d.template_id == 320)
        .min_by(|x, y| {
            ((x.pos.x - area.x).powi(2) + (x.pos.y - area.y).powi(2))
                .total_cmp(&((y.pos.x - area.x).powi(2) + (y.pos.y - area.y).powi(2)))
        })
        .context("mailbox")?
        .clone();
    a.client.send(Command::Move(MoveCommand {
        pos: npc.pos,
        ..MoveCommand::default()
    }));
    a.client.send(Command::Interact(Interact::Npc {
        obj_id: npc.obj_id,
        target_changed: true,
    }));
    let mut equipment: Vec<_> = a
        .state
        .inventory
        .equipment
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, i)| i.as_ref().map(|i| (slot, i.clone())))
        .filter(|(_, i)| {
            i.id != 0
                && i.flags & 1 == 0
                && db
                    .query_row(
                        "SELECT sellable='t' FROM items WHERE id=?",
                        [i.template_id],
                        |r| r.get::<_, bool>(0),
                    )
                    .unwrap_or(false)
        })
        .collect();
    equipment.sort_by_key(|(_, i)| {
        db.query_row(
            "SELECT refund FROM items WHERE id=?",
            [i.template_id],
            |r| r.get::<_, i32>(0),
        )
        .unwrap_or(0)
    });
    ensure!(equipment.len() >= 3, "not enough own unbound starter gear");
    let reserved: Vec<_> = equipment[..2].iter().map(|(_, i)| i.id).collect();
    let sell: Vec<_> = equipment[2..]
        .iter()
        .map(|(slot, i)| SellEntry {
            container: 1,
            slot: *slot as u8,
            item_id: i.id,
        })
        .collect();
    let old_money = a.state.stats.money;
    a.client.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc.obj_id,
        items: sell.clone(),
    }));
    a.until("fund postage by own starter sales", |_, s| {
        s.stats.money > old_money
            && sell
                .iter()
                .all(|i| s.inventory.find_by_id(i.item_id).is_none())
    })
    .await?;
    ensure!(
        a.state.stats.money >= 135,
        "own sale funds {} below 135c postage+gift",
        a.state.stats.money
    );
    println!(
        "own starter sales funded {}c; mailbox template320 obj={} at {:?}",
        a.state.stats.money, mailbox.obj_id, mailbox.pos
    );
    for id in &reserved {
        let (container, slot, _) = a.state.inventory.find_by_id(*id).context("reserved item")?;
        let bag = a
            .state
            .inventory
            .first_free_bag_slot()
            .context("bag full")?;
        let plan = a
            .state
            .inventory
            .plan_move((container, slot), (2, bag))
            .context("unequip")?;
        a.client.send(Command::SwapItems(plan));
        a.until("own attachment in bag", |_, s| {
            s.inventory.find_by_id(*id).is_some_and(|(c, _, _)| c == 2)
        })
        .await?;
    }
    a.client.send(Command::Move(MoveCommand {
        pos: mailbox.pos,
        ..MoveCommand::default()
    }));
    let request = SendMail {
        mail_type: 3,
        receiver: name.clone(),
        title: "Owned mailbox probe".into(),
        text: "Fresh isolated sender and recipient.".into(),
        doodad_obj_id: mailbox.obj_id,
        ..SendMail::default()
    };
    let before = a.state.stats.money;
    let mut invalid = request.clone();
    invalid.receiver = "NoSuchMailCharacter".into();
    a.send(MailCommand::Send(invalid));
    a.until("unknown recipient failure", |e, _| {
        matches!(e, Event::Mail(MailEvent::Failed { code: 15, .. }))
    })
    .await?;
    ensure!(a.state.stats.money == before && a.state.mailbox.sent.is_empty());
    let mut invalid = request.clone();
    invalid.money = i32::MAX;
    a.send(MailCommand::Send(invalid));
    a.until("unaffordable mail failure", |e, _| {
        matches!(e, Event::Mail(MailEvent::Failed { code: 1, .. }))
    })
    .await?;
    ensure!(a.state.stats.money == before);
    println!(
        "recipient-not-found15 and insufficient-copper1 rejected without changing money or mail"
    );

    let mut money = request.clone();
    money.money = 5;
    a.send(MailCommand::Send(money));
    a.until("money mail sent", |e, _| {
        matches!(e, Event::Mail(MailEvent::Sent { .. }))
    })
    .await?;
    let money_id = a
        .state
        .mailbox
        .sent
        .last()
        .context("sent money header")?
        .mail_id;
    a.until("money postage", |_, s| s.stats.money == before - 55)
        .await?;
    b.until("incoming money mail", |_, s| {
        s.mailbox.header(false, money_id).is_some()
    })
    .await?;
    ensure!(b.state.mailbox.unread.received == 1);
    let mut items = request;
    items.title = "Owned equipment attachments".into();
    items.items = reserved
        .iter()
        .map(|id| {
            let (c, s, _) = a.state.inventory.find_by_id(*id).unwrap();
            MailSlot {
                container: c,
                slot: s as u8,
            }
        })
        .collect();
    a.send(MailCommand::Send(items));
    a.until(
        "item mail sent",
        |e, _| matches!(e,Event::Mail(MailEvent::Sent{header,..}) if header.mail_id!=money_id),
    )
    .await?;
    let item_id = a
        .state
        .mailbox
        .sent
        .last()
        .context("sent item header")?
        .mail_id;
    a.until("item postage and escrow", |_, s| {
        s.stats.money == before - 135
            && reserved
                .iter()
                .all(|id| s.inventory.find_by_id(*id).is_none())
    })
    .await?;
    b.until("incoming item mail", |_, s| {
        s.mailbox.header(false, item_id).is_some()
    })
    .await?;
    ensure!(b.state.mailbox.unread.received == 2);
    println!(
        "registered mail delivered immediately: money id={money_id}, equipment id={item_id}; exact135c charge"
    );

    // Reconnect only the recipient we own; both unread mails and escrow must persist.
    b.logout().await?;
    b.reconnect().await?;
    list(b).await?;
    ensure!(b.state.mailbox.received.len() == 2 && b.state.mailbox.unread.received == 2);
    list(a).await?;
    ensure!(a.state.mailbox.sent.len() == 2);
    read(a, true, item_id).await?;
    ensure!(
        a.state
            .mailbox
            .body(true, item_id)
            .unwrap()
            .items
            .iter()
            .flatten()
            .count()
            == 2
    );
    read(b, false, money_id).await?;
    b.until("money mail read status", |_, s| {
        s.mailbox
            .header(false, money_id)
            .is_some_and(|h| h.status == 1)
            && s.mailbox.unread.received == 1
    })
    .await?;
    let b_money = b.state.stats.money;
    b.send(MailCommand::TakeMoney { mail_id: money_id });
    b.until("money claim", |_, s| {
        s.stats.money == b_money + 5
            && s.mailbox
                .body(false, money_id)
                .is_some_and(|body| body.money == 0)
    })
    .await?;
    b.send(MailCommand::Delete {
        is_sent: false,
        mail_id: money_id,
    });
    b.until("delete empty money mail", |_, s| {
        s.mailbox.header(false, money_id).is_none()
    })
    .await?;
    read(b, false, item_id).await?;
    let attachment = b
        .state
        .mailbox
        .body(false, item_id)
        .unwrap()
        .items
        .iter()
        .flatten()
        .next()
        .context("attachment")?
        .clone();
    let first = attachment.item.id;
    b.send(MailCommand::TakeItem {
        mail_id: item_id,
        attachment,
    });
    b.until("native full equipment item claim", |_, s| {
        s.inventory.find_by_id(first).is_some()
            && s.mailbox
                .body(false, item_id)
                .is_some_and(|body| body.items.iter().flatten().count() == 1)
    })
    .await?;
    b.send(MailCommand::TakeAll { mail_id: item_id });
    b.until("remaining attachments claim", |_, s| {
        reserved
            .iter()
            .all(|id| s.inventory.find_by_id(*id).is_some())
            && s.mailbox
                .header(false, item_id)
                .is_some_and(|h| h.attachments == 0)
            && s.mailbox.pending.is_empty()
    })
    .await?;
    ensure!(
        b.state.mailbox.header(false, item_id).is_some(),
        "claim-only unexpectedly deleted mail"
    );
    b.send(MailCommand::TakeAllAndDelete { mail_id: item_id });
    b.until("claim-and-delete acknowledgement", |_, s| {
        s.mailbox.header(false, item_id).is_none()
    })
    .await?;
    list(b).await?;
    ensure!(b.state.mailbox.received.is_empty() && b.state.mailbox.unread.received == 0);
    b.logout().await?;
    b.reconnect().await?;
    list(b).await?;
    ensure!(b.state.mailbox.received.is_empty());
    ensure!(
        reserved
            .iter()
            .all(|id| b.state.inventory.find_by_id(*id).is_some())
    );
    ensure!(b.state.stats.money == b_money + 5);
    println!(
        "read/unread, 5c claim, native equipment claim, sequential claim-only, direct and claim-and-delete, reconnect persistence all confirmed"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires bevy lane and read-only game DB; creates two isolated accounts"]
async fn mail_lists_send_read_claim_delete_and_persistence() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::fresh(stamp, "a").await?;
    let mut b = match Peer::fresh(stamp, "b").await {
        Ok(b) => b,
        Err(e) => {
            let _ = a.logout().await;
            return Err(e);
        }
    };
    let db = Connection::open_with_flags(
        std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into()),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let result = exercise(&mut a, &mut b, &db).await;
    let (out_a, out_b) = tokio::join!(a.logout(), b.logout());
    result?;
    out_a?;
    out_b?;
    println!("both isolated mail clients logged out cleanly");
    Ok(())
}
