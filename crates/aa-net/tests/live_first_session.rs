//! Opt-in checks against the project's bevy lane. Uses a fresh throw-away account and character.
//! Run: cargo test -p aa-net --test live_first_session -- --ignored --nocapture

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::items::container;
use aa_net::model::{gender, race, slot};
use aa_net::{Client, Command, CreateCharacter, Event, LeaveTarget, WorldState};
use anyhow::{Context, Result, bail, ensure};

async fn until(client: &mut Client, state: &mut WorldState, what: &str, pick: impl Fn(&Event, &WorldState) -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        while let Some(event) = client.recv().await {
            state.apply(&event);
            match &event {
                Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                    bail!("{what}: {message}")
                }
                Event::ServerError { code } => bail!("{what}: server error {code}"),
                Event::CharacterCreateFailed { reason } => bail!("{what}: character creation failed {reason}"),
                Event::EnterWorldDenied { reason } => bail!("{what}: enter world denied {reason}"),
                _ => {}
            }
            if pick(&event, state) {
                return Ok(());
            }
        }
        bail!("{what}: session ended")
    })
    .await
    .with_context(|| format!("{what}: timed out"))?
}

async fn move_item(client: &mut Client, state: &mut WorldState, from: (u8, usize), to: (u8, usize)) -> Result<()> {
    let plan = state.inventory.plan_move(from, to).context("invalid item move")?;
    let (first, second) = (plan.from_item_id, plan.to_item_id);
    client.send(Command::SwapItems(plan));
    until(client, state, "item move", |event, state| {
        matches!(event, Event::ItemsChanged(_))
            && state.inventory.item_at(from.0, from.1).map_or(0, |i| i.id) == second
            && state.inventory.item_at(to.0, to.1).map_or(0, |i| i.id) == first
    })
    .await
}

async fn exercise(client: &mut Client, state: &mut WorldState) -> Result<()> {
    until(client, state, "starter inventory", |_, s| s.inventory.bag_items().next().is_some()).await?;
    let bag = container::INVENTORY;
    let equipment = container::EQUIPMENT;
    let empty = state.inventory.first_free_bag_slot().context("starter bag is full")?;
    let occupied = state.inventory.bag_items().next().context("no starter items")?.0;
    move_item(client, state, (bag, occupied), (bag, empty)).await?;
    move_item(client, state, (bag, empty), (bag, occupied)).await?;
    println!("bag moves: moved to an empty slot and back");

    let weapon = state.inventory.worn(slot::MAINHAND).context("starter weapon missing")?.id;
    ensure!(weapon != 0, "starter weapon has no item id");
    move_item(client, state, (equipment, slot::MAINHAND), (bag, empty)).await?;
    move_item(client, state, (bag, empty), (equipment, slot::MAINHAND)).await?;
    ensure!(state.inventory.worn(slot::MAINHAND).is_some_and(|i| i.id == weapon));
    println!("equipment: unequipped and wore the same weapon again");

    let (stack_slot, original) = state
        .inventory
        .bag_items()
        .find(|(_, i)| i.count > 1)
        .context("starter stack missing")?;
    let (stack_id, count) = (original.id, original.count);
    let plan = state
        .inventory
        .plan_move((bag, stack_slot), (bag, empty))
        .context("invalid split")?;
    client.send(Command::SplitItem { mv: plan, count: 1 });
    until(client, state, "split stack", |event, s| {
        matches!(event, Event::ItemsChanged(_))
            && s.inventory
                .item_at(bag, stack_slot)
                .is_some_and(|i| i.id == stack_id && i.count == count - 1)
            && s.inventory.item_at(bag, empty).is_some_and(|i| i.id != stack_id && i.count == 1)
    })
    .await?;
    let plan = state
        .inventory
        .plan_move((bag, empty), (bag, stack_slot))
        .context("invalid merge")?;
    client.send(Command::SwapItems(plan));
    until(client, state, "merge stack", |event, s| {
        matches!(event, Event::ItemsChanged(_))
            && s.inventory.item_at(bag, empty).is_none()
            && s.inventory
                .item_at(bag, stack_slot)
                .is_some_and(|i| i.id == stack_id && i.count == count)
    })
    .await?;
    println!("stacks: split one item, then merged it back without losing count");

    client.send(Command::ResetSkills { ability: 1 });
    until(
        client,
        state,
        "reset Battlerage",
        |e, s| matches!(e, Event::SkillsReset { obj_id, ability: 1 } if *obj_id == s.stats.obj_id),
    )
    .await?;
    client.send(Command::LearnSkill { skill_id: 18132 });
    until(client, state, "learn Triple Slash", |e, s| {
        matches!(e, Event::SkillLearned { skill_id: 18132, level: 1 }) && s.skills.knows(18132)
    })
    .await?;
    println!("skills: reset Battlerage and learned Triple Slash again");
    Ok(())
}

#[tokio::test]
#[ignore = "requires the local bevy lane and creates a throw-away account"]
async fn first_session_inventory_and_skill_learning() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let user = format!("aanet{stamp}");
    let mut client = Client::spawn_on(&tokio::runtime::Handle::current());
    let mut state = WorldState::default();
    client.send(Command::Login {
        addr: aa_net::default_login(),
        user: user.clone(),
        password: user.into(),
    });
    until(&mut client, &mut state, "world list", |e, _| matches!(e, Event::WorldList(_))).await?;
    let world = state.worlds.list.worlds.iter().find(|w| w.online).context("no online world")?.id;
    client.send(Command::EnterWorld { world_id: world });
    until(&mut client, &mut state, "world cookie", |e, _| {
        matches!(e, Event::ReadyToConnect { .. })
    })
    .await?;
    client.send(Command::ConnectToWorld);
    until(&mut client, &mut state, "character list", |e, _| {
        matches!(e, Event::CharacterList(_))
    })
    .await?;
    // At most 18 letters/digits, globally unique within this test's timestamp range.
    let name = format!("Net{stamp}");
    client.send(Command::CreateCharacter(CreateCharacter::new(name, race::NUIAN, gender::MALE)));
    until(
        &mut client,
        &mut state,
        "created character",
        |e, _| matches!(e, Event::CharacterList(list) if !list.is_empty()),
    )
    .await?;
    client.send(Command::EnterGame {
        character_id: state.characters.list[0].id,
    });
    until(&mut client, &mut state, "entered world", |e, _| matches!(e, Event::EnteredWorld)).await?;
    let result = exercise(&mut client, &mut state).await;
    client.send(Command::LeaveWorld(LeaveTarget::Quit));
    let logout = until(&mut client, &mut state, "clean logout", |e, _| matches!(e, Event::LeftWorld { .. })).await;
    result?;
    logout?;
    Ok(())
}
