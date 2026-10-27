//! Fresh own-character repair probe; no grants or changes to existing users.
//! cargo test -p aa-net --test live_repair -- --ignored --nocapture

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::interaction::Interact;
use aa_net::model::{ItemDetail, gender, race, slot};
use aa_net::shop::{SellEntry, ShopCommand};
use aa_net::{
    Client, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, MoveState, Pos, RepairCommand, RepairEvent,
    WorldState,
};
use anyhow::{Context, Result, bail, ensure};

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

async fn report(c: &mut Client, s: &mut WorldState, command: RepairCommand) -> Result<()> {
    c.send(Command::Repair(command));
    until(c, s, "repair report", |e, _| matches!(e, Event::ItemsChanged(t) if t.reason == 3)).await?;
    ensure!(s.repair.pending.is_empty() && s.repair.last_report.is_some());
    Ok(())
}

fn durability(s: &WorldState, id: u64) -> Option<u8> {
    let (_, _, it) = s.inventory.find_by_id(id)?;
    match &it.detail {
        ItemDetail::Equip(d) => Some(d.durability),
        _ => None,
    }
}

async fn exercise(c: &mut Client, s: &mut WorldState) -> Result<()> {
    let equipped: Vec<_> = s
        .inventory
        .equipment
        .slots
        .iter()
        .flatten()
        .filter_map(|i| match &i.detail {
            ItemDetail::Equip(d) if i.id != 0 => Some((i.id, d.durability)),
            _ => None,
        })
        .collect();
    // The response must remain an empty acknowledgement without an interaction object.
    report(c, s, RepairCommand::All { auto_use_aa_point: false }).await?;
    ensure!(s.repair.last_report.as_ref().unwrap().items.is_empty());

    // Real blacksmith8233, also a merchant: advertises12087 due to lane option precedence.
    let near = Pos::new(13573.98, 15907.689, 175.212);
    c.send(Command::Move(MoveCommand::stopped(near, 0.0)));
    until(c, s, "blacksmith in view", |_, s| s.units.by_id.values().any(|u| u.kind.template_id() == 8233)).await?;
    let npc = s
        .units
        .by_id
        .values()
        .filter(|u| u.kind.template_id() == 8233)
        .min_by(|a, b| {
            ((a.pos.x - near.x).powi(2) + (a.pos.y - near.y).powi(2))
                .total_cmp(&((b.pos.x - near.x).powi(2) + (b.pos.y - near.y).powi(2)))
        })
        .context("missing blacksmith")?
        .clone();
    let mut ground = npc.pos;
    ground.x -= 1.0;
    c.send(Command::Move(MoveCommand::stopped(ground, 0.0)));
    c.send(Command::Interact(Interact::Npc { obj_id: npc.obj_id, target_changed: true }));
    c.send(Command::Interact(Interact::Start {
        npc_obj_id: npc.obj_id,
        obj_id: s.stats.obj_id,
        extra_info: 0,
        pick_id: 0,
        mouse_button: 1,
        modifier_keys: 0,
    }));
    until(c,s,"blacksmith service",|e,_|matches!(e,Event::Interaction(aa_net::interaction::InteractEvent::SkillList{skills,..}) if skills.contains(&12087))).await?;
    report(c, s, RepairCommand::All { auto_use_aa_point: false }).await?;
    ensure!(s.repair.last_report.as_ref().unwrap().items.is_empty());
    println!("blacksmith8233 obj{}: healthy equipment produces an empty repair report", npc.obj_id);

    // Own starter weapon funds repairs. Keep the starter armor for the wear probe.
    let weapon = s.inventory.equipment.slots[slot::MAINHAND].as_ref().context("no starter weapon")?.clone();
    let money = s.stats.money;
    c.send(Command::Shop(ShopCommand::Sell {
        npc_obj_id: npc.obj_id,
        items: vec![SellEntry { container: 1, slot: slot::MAINHAND as u8, item_id: weapon.id }],
    }));
    until(c, s, "repair funds", |_, s| s.inventory.find_by_id(weapon.id).is_none() && s.stats.money > money).await?;
    let owned_obj = s.stats.obj_id;
    // Small nonlethal falls on this throwaway character use ordinary movement/combat wear.
    // Wear is random; stop well before the server's5%-HP floor and never force item state.
    for _ in 0..60 {
        if equipped.iter().any(|(id, d)| durability(s, *id).is_some_and(|now| now < *d))
            || s.stats.hp <= s.stats.max_hp / 5
        {
            break;
        }
        let hp = s.stats.hp;
        let mut air = ground;
        air.z += 24.0;
        c.send(Command::Move(MoveCommand::stopped(air, 0.0).with_state(MoveState::Fall)));
        c.send(Command::Move(MoveCommand::stopped(ground, 0.0).landing(20.5)));
        until(
            c,
            s,
            "own landing",
            |e, _| matches!(e,Event::UnitPoints{obj_id,hp:now,..} if *obj_id==owned_obj && *now<hp),
        )
        .await?;
        let end = tokio::time::Instant::now() + Duration::from_millis(25);
        while let Ok(Some(e)) = tokio::time::timeout_at(end, c.recv()).await {
            s.apply(&e);
        }
    }
    let worn = equipped.iter().find_map(|(id, max)| durability(s, *id).filter(|d| d < max).map(|now| (*id, *max, now)));
    if let Some((id, max, old)) = worn {
        let (container, index, _) = s.inventory.find_by_id(id).context("worn item gone")?;
        let money = s.stats.money;
        report(c, s, RepairCommand::Single { container, slot: index as u8, auto_use_aa_point: false }).await?;
        ensure!(durability(s, id) == Some(max));
        ensure!(s.repair.last_report.as_ref().unwrap().items.iter().any(|i| i.item_id == id && i.durability == max));
        ensure!(s.stats.money <= money);
        println!("single repair item{id}: durability{old}->{max}, copper{money}->{}, server confirmed", s.stats.money);
        report(c, s, RepairCommand::All { auto_use_aa_point: false }).await?;
        ensure!(equipped.iter().filter(|(id, _)| *id != weapon.id).all(|(id, max)| durability(s, *id) == Some(*max)));
        println!("all remaining equipped repairs confirmed");
    } else {
        println!("wear RNG produced no damaged armor; positive paid repair remains unverified (no fixture grants)");
    }
    c.send(Command::Repair(RepairCommand::Single { container: 3, slot: 0, auto_use_aa_point: false }));
    until(c, s, "local bank repair rejection", |e, _| matches!(e, Event::Repair(RepairEvent::RequestRejected { .. })))
        .await?;
    ensure!(s.repair.pending.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires bevy lane; creates a throw-away account and may damage only its own starter armor"]
async fn npc_repair_acknowledgement_and_natural_wear() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let user = format!("netrepair{stamp}");
    let mut c = Client::spawn_on(&tokio::runtime::Handle::current());
    let mut s = WorldState::default();
    c.send(Command::Login { addr: aa_net::default_login(), user: user.clone(), password: user.into() });
    until(&mut c, &mut s, "worlds", |e, _| matches!(e, Event::WorldList(_))).await?;
    let world_id = s.worlds.list.worlds.iter().find(|w| w.online).context("no world")?.id;
    c.send(Command::EnterWorld { world_id });
    until(&mut c, &mut s, "cookie", |e, _| matches!(e, Event::ReadyToConnect { .. })).await?;
    c.send(Command::ConnectToWorld);
    until(&mut c, &mut s, "characters", |e, _| matches!(e, Event::CharacterList(_))).await?;
    c.send(Command::CreateCharacter(CreateCharacter::new(format!("R{stamp}"), race::NUIAN, gender::MALE)));
    until(&mut c, &mut s, "create", |e, _| matches!(e,Event::CharacterList(list) if !list.is_empty())).await?;
    c.send(Command::EnterGame { character_id: s.characters.list[0].id });
    until(&mut c, &mut s, "spawn", |e, _| matches!(e, Event::EnteredWorld)).await?;
    println!("fresh {} id{}", s.stats.name, s.stats.character_id);
    let result = exercise(&mut c, &mut s).await;
    c.send(Command::LeaveWorld(LeaveTarget::Quit));
    let logout = until(&mut c, &mut s, "logout", |e, _| matches!(e, Event::LeftWorld { .. })).await;
    result?;
    logout?;
    println!("repair test account logged out cleanly");
    Ok(())
}
