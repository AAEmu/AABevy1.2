//! Opt-in economy checks on a fresh account; uses only its own starter items and copper.
//! cargo test -p aa-net --test live_shop_bank -- --ignored --nocapture

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::bank::BankCommand;
use aa_net::interaction::Interact;
use aa_net::items::container;
use aa_net::model::{gender, race};
use aa_net::shop::{BuyEntry, SellEntry, ShopCommand, ShopEvent};
use aa_net::{Client, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, Pos, WorldState};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags};

async fn until(
    c: &mut Client,
    s: &mut WorldState,
    what: &str,
    pick: impl Fn(&Event, &WorldState) -> bool,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        while let Some(e) = c.recv().await {
            s.apply(&e);
            match &e {
                Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                    bail!("{what}: {message}")
                }
                Event::ServerError { code } => bail!("{what}: server error {code}"),
                Event::Shop(ShopEvent::RequestRejected(message)) => bail!("{what}: {message}"),
                Event::CharacterCreateFailed { reason } => bail!("{what}: create failed {reason}"),
                _ => {}
            }
            if pick(&e, s) {
                return Ok(());
            }
        }
        bail!("{what}: session ended")
    })
    .await
    .with_context(|| format!("{what}: timed out"))?
}

async fn sold(c: &mut Client, s: &mut WorldState, npc: u32) -> Result<()> {
    c.send(Command::Shop(ShopCommand::ListSold { npc_obj_id: npc }));
    until(c, s, "sold list", |e, _| matches!(e, Event::Shop(ShopEvent::SoldItems(_)))).await
}

async fn exercise(c: &mut Client, s: &mut WorldState, db: &Connection) -> Result<()> {
    let mut q = db.prepare("SELECT m.npc_id,m.merchant_pack_id FROM merchants m JOIN npcs n ON n.id=m.npc_id JOIN merchant_packs p ON p.id=m.merchant_pack_id WHERE n.merchant='t' AND p.kind_id=0")?;
    let merchants: HashMap<u32, u32> =
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    // The starter clearing has no merchant in its subscribed region. This is the configured
    // gold merchant12547 from the lane's read-only main_world/npc_spawns.json, with 25c goods.
    let spawn = Pos::new(14366.27, 15504.10, 152.86171);
    c.send(Command::Move(MoveCommand { pos: spawn, ..MoveCommand::default() }));
    until(c, s, "merchant in view", |_, s| s.units.by_id.values().any(|u| u.kind.template_id() == 12547)).await?;
    // Consume the initial area announcements before picking the closest merchant.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    while let Ok(Some(e)) = tokio::time::timeout_at(deadline, c.recv()).await {
        s.apply(&e);
    }
    let npc = s
        .units
        .by_id
        .values()
        .filter(|u| u.kind.template_id() == 12547)
        .min_by(|a, b| {
            ((a.pos.x - spawn.x).powi(2) + (a.pos.y - spawn.y).powi(2))
                .total_cmp(&((b.pos.x - spawn.x).powi(2) + (b.pos.y - spawn.y).powi(2)))
        })
        .context("no merchant")?
        .clone();
    let npc_id = npc.obj_id;
    let pack = *merchants.get(&npc.kind.template_id()).context("configured NPC is not a merchant")?;
    let mut near = npc.pos;
    near.x -= 1.0;
    c.send(Command::Move(MoveCommand { pos: near, ..MoveCommand::default() }));
    c.send(Command::Interact(Interact::Npc { obj_id: npc_id, target_changed: true }));
    c.send(Command::Interact(Interact::Start {
        npc_obj_id: npc_id,
        obj_id: s.stats.obj_id,
        extra_info: 0,
        pick_id: 0,
        mouse_button: 1,
        modifier_keys: 0,
    }));
    until(c, s, "store service", |e,_| matches!(e, Event::Interaction(aa_net::interaction::InteractEvent::SkillList { skills, .. }) if skills.contains(&12087))).await?;
    println!(
        "merchant obj={npc_id} template={} pack={pack}; service12087; initial copper={}",
        npc.kind.template_id(),
        s.stats.money
    );
    sold(c, s, npc_id).await?;
    ensure!(s.shop.sold.is_empty());

    // Selling the starter main-hand funds buyback and a small bank round-trip; no GM grants.
    let (slot, item) = s
        .inventory
        .equipment
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, it)| it.as_ref().map(|it| (i, it)))
        .find(|(_, it)| {
            it.id != 0
                && db
                    .query_row("SELECT sellable='t' AND refund>0 FROM items WHERE id=?1", [it.template_id], |r| {
                        r.get::<_, bool>(0)
                    })
                    .unwrap_or(false)
        })
        .context("no sellable starter equipment")?;
    let (item_id, template_id, count) = (item.id, item.template_id, item.count);
    let initial_money = s.stats.money;
    c.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc_id,
        items: vec![SellEntry { container: 1, slot: slot as u8, item_id }],
    }));
    until(c, s, "sale", |_, s| s.inventory.find_by_id(item_id).is_none() && s.stats.money > initial_money).await?;
    let proceeds = s.stats.money - initial_money;
    sold(c, s, npc_id).await?;
    ensure!(s.shop.sold.len() == 1 && s.shop.sold[0].id == item_id && s.shop.sold[0].count == count);
    println!(
        "sold item{template_id}:{item_id}, received{proceeds} copper; authoritative buyback list retains its identity"
    );

    until(c, s, "banker in view", |_, s| s.units.by_id.values().any(|u| u.kind.template_id() == 12544)).await?;
    let banker = s.units.by_id.values().find(|u| u.kind.template_id() == 12544).unwrap().clone();
    let mut bank_near = banker.pos;
    bank_near.x -= 1.0;
    c.send(Command::Move(MoveCommand { pos: bank_near, ..MoveCommand::default() }));
    c.send(Command::Interact(Interact::Npc { obj_id: banker.obj_id, target_changed: true }));
    c.send(Command::Interact(Interact::Start {
        npc_obj_id: banker.obj_id,
        obj_id: s.stats.obj_id,
        extra_info: 0,
        pick_id: 0,
        mouse_button: 1,
        modifier_keys: 0,
    }));
    until(c,s,"bank service",|e,_|matches!(e,Event::Interaction(aa_net::interaction::InteractEvent::SkillList{skills,..}) if skills.contains(&13238))).await?;
    println!("banker obj={} template12544: service13238", banker.obj_id);

    let amount = (s.stats.money.min(5)) as i32;
    let (money, bank) = (s.stats.money, s.stats.bank_money);
    c.send(Command::Bank(BankCommand::DepositMoney { amount, aa_point: 0 }));
    until(c, s, "bank deposit", |_, s| {
        s.stats.money == money - i64::from(amount) && s.stats.bank_money == bank + i64::from(amount)
    })
    .await?;
    c.send(Command::Bank(BankCommand::WithdrawMoney { amount, aa_point: 0 }));
    until(c, s, "bank withdraw", |_, s| s.stats.money == money && s.stats.bank_money == bank).await?;

    c.send(Command::Move(MoveCommand { pos: near, ..MoveCommand::default() }));
    c.send(Command::Interact(Interact::Npc { obj_id: npc_id, target_changed: true }));

    c.send(Command::Shop(ShopCommand::Buy {
        npc_obj_id: npc_id,
        doodad_obj_id: 0,
        buy: vec![],
        buy_back: vec![0],
        use_aa_point: false,
    }));
    until(c, s, "buyback", |_, s| s.inventory.find_by_id(item_id).is_some() && s.stats.money == initial_money).await?;
    sold(c, s, npc_id).await?;
    ensure!(s.shop.sold.is_empty());
    println!("buyback returned same item id/count for exact refund price; deposit/withdraw restored both balances");

    let (bag_slot, _, _) = s.inventory.find_by_id(item_id).context("bought back item missing")?;
    ensure!(bag_slot == container::INVENTORY);
    let (_, bag_index, _) = s.inventory.find_by_id(item_id).unwrap();
    let bank_slot = s.inventory.first_free_bank_slot().context("no bank slot")?;
    let plan = s
        .inventory
        .plan_move((container::INVENTORY, bag_index), (container::BANK, bank_slot))
        .context("bank deposit plan")?;
    c.send(Command::SwapItems(plan));
    until(c, s, "bank item deposit", |_, s| {
        s.inventory.item_at(container::BANK, bank_slot).is_some_and(|it| it.id == item_id)
    })
    .await?;
    let plan = s
        .inventory
        .plan_move((container::BANK, bank_slot), (container::INVENTORY, bag_index))
        .context("bank withdrawal plan")?;
    c.send(Command::SwapItems(plan));
    until(c, s, "bank item withdrawal", |_, s| {
        s.inventory.item_at(container::INVENTORY, bag_index).is_some_and(|it| it.id == item_id)
    })
    .await?;
    println!("bank slots{}: moved item into warehouse and back without changing identity", s.inventory.bank_slots);

    // Fund one normal catalog purchase by selling this throw-away character's weapon again.
    c.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc_id,
        items: vec![SellEntry { container: 2, slot: bag_index as u8, item_id }],
    }));
    until(c, s, "fund catalog purchase", |_, s| {
        s.inventory.find_by_id(item_id).is_none() && s.stats.money > initial_money
    })
    .await?;
    let product:Option<(u32,u8,i64)> = db.query_row("SELECT g.item_id,g.grade_id,i.price FROM merchant_goods g JOIN items i ON i.id=g.item_id WHERE g.merchant_pack_id=?1 AND i.price>=0 AND i.price<=?2 ORDER BY i.price,g.id LIMIT 1",rusqlite::params![pack,s.stats.money],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (product, grade, price) = product.context("merchant has no affordable normal catalog item")?;
    let before_count = s.inventory.count_of(product);
    let money = s.stats.money;
    c.send(Command::Shop(ShopCommand::Buy {
        npc_obj_id: npc_id,
        doodad_obj_id: 0,
        buy: vec![BuyEntry { item_id: product, grade, count: 1, currency: 0 }],
        buy_back: vec![],
        use_aa_point: false,
    }));
    until(c, s, "catalog purchase", |_, s| {
        s.inventory.count_of(product) == before_count + 1 && s.stats.money == money - price
    })
    .await?;
    println!("catalog purchase item{product}: +1 item, -{price} copper (server confirmed)");
    Ok(())
}

use rusqlite::OptionalExtension;

#[tokio::test]
#[ignore = "requires bevy lane and read-only game database; creates a throw-away account"]
async fn shop_sale_buyback_catalog_and_bank_roundtrips() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let user = format!("netshop{stamp}");
    let mut c = Client::spawn_on(&tokio::runtime::Handle::current());
    let mut s = WorldState::default();
    c.send(Command::Login { addr: aa_net::default_login(), user: user.clone(), password: user.into() });
    until(&mut c, &mut s, "world list", |e, _| matches!(e, Event::WorldList(_))).await?;
    let world_id = s.worlds.list.worlds.iter().find(|w| w.online).context("no world")?.id;
    c.send(Command::EnterWorld { world_id });
    until(&mut c, &mut s, "cookie", |e, _| matches!(e, Event::ReadyToConnect { .. })).await?;
    c.send(Command::ConnectToWorld);
    until(&mut c, &mut s, "characters", |e, _| matches!(e, Event::CharacterList(_))).await?;
    c.send(Command::CreateCharacter(CreateCharacter::new(format!("Shop{stamp}"), race::NUIAN, gender::MALE)));
    until(&mut c, &mut s, "create", |e, _| matches!(e,Event::CharacterList(v) if !v.is_empty())).await?;
    c.send(Command::EnterGame { character_id: s.characters.list[0].id });
    until(&mut c, &mut s, "spawn", |e, _| matches!(e, Event::EnteredWorld)).await?;
    let db =
        Connection::open_with_flags(std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into()), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let result = exercise(&mut c, &mut s, &db).await;
    c.send(Command::LeaveWorld(LeaveTarget::Quit));
    let logout = until(&mut c, &mut s, "logout", |e, _| matches!(e, Event::LeftWorld { .. })).await;
    result?;
    logout?;
    Ok(())
}
