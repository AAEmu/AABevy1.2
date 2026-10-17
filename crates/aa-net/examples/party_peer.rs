//! Short-lived headless party peer for HUD checks, using only a fresh throw-away account.
//! cargo run -p aa-net --example party_peer -- --invite Solhudtest --seconds 90

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::model::{gender, race};
use aa_net::{Client, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, PartyCommand, PartyEvent, WorldState};
use anyhow::{Context, Result, bail};

async fn until(client: &mut Client, state: &mut WorldState, pick: impl Fn(&Event) -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        while let Some(event) = client.recv().await {
            state.apply(&event);
            match &event {
                Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => {
                    bail!("{message}")
                }
                Event::CharacterCreateFailed { reason } => bail!("character creation failed {reason}"),
                Event::ServerError { code } => bail!("server error {code}"),
                _ => {}
            }
            if pick(&event) {
                return Ok(());
            }
        }
        bail!("session ended")
    })
    .await
    .context("waiting for server")?
}

async fn setup(client: &mut Client, state: &mut WorldState) -> Result<String> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let user = format!("partyhud{stamp}");
    // Character names use letters only, to match the original create-screen name policy.
    let mut digits = stamp as u64;
    let mut name = "Peer".to_string();
    while digits != 0 {
        name.push((b'a' + (digits % 26) as u8) as char);
        digits /= 26;
    }
    client.send(Command::Login { addr: "127.0.0.1:27237".into(), user: user.clone(), password: user.clone().into() });
    until(client, state, |e| matches!(e, Event::WorldList(_))).await?;
    let world_id = state.worlds.list.worlds.iter().find(|w| w.online).context("no online world")?.id;
    client.send(Command::EnterWorld { world_id });
    until(client, state, |e| matches!(e, Event::ReadyToConnect { .. })).await?;
    client.send(Command::ConnectToWorld);
    until(client, state, |e| matches!(e, Event::CharacterList(_))).await?;
    client.send(Command::CreateCharacter(CreateCharacter::new(name, race::NUIAN, gender::MALE)));
    until(client, state, |e| matches!(e, Event::CharacterList(list) if !list.is_empty())).await?;
    client.send(Command::EnterGame { character_id: state.characters.list[0].id });
    until(client, state, |e| matches!(e, Event::EnteredWorld)).await?;
    println!(
        "READY account={user} character={} id={} obj={}",
        state.stats.name, state.stats.character_id, state.stats.obj_id
    );
    Ok(user)
}

async fn run_peer(client: &mut Client, state: &mut WorldState, target: Option<String>, seconds: u64) -> Result<()> {
    if let Some(target_name) = target {
        println!("INVITING {target_name}");
        client.send(Command::Party(PartyCommand::Invite { team_id: 0, is_party: true, target_name }));
    }
    let deadline = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(deadline);
    let mut movement = tokio::time::interval(Duration::from_secs(2));
    loop {
        tokio::select! {
            _ = &mut deadline => return Ok(()),
            _ = movement.tick() => {
                if let Some(spawn) = &state.player.spawn {
                    client.send(Command::Move(MoveCommand { pos: spawn.unit.pos, ..MoveCommand::default() }));
                }
            }
            event = client.recv() => {
                let Some(event) = event else { bail!("peer session ended") };
                state.apply(&event);
                match event {
                    Event::Party(PartyEvent::Invited(invite)) => {
                        println!("ACCEPTING from {}", invite.sender_name);
                        client.send(Command::Party(PartyCommand::Reply { invite, accept: true }));
                    }
                    Event::Party(event) => println!("PARTY {event:?}"),
                    Event::Disconnected { message } => bail!("peer disconnected: {message}"),
                    Event::ServerError { code } => eprintln!("server error {code}"),
                    _ => {}
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut target, mut seconds) = (None, 90);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--invite" => target = Some(args.next().context("--invite needs a character name")?),
            "--seconds" => seconds = args.next().context("--seconds needs a number")?.parse::<u64>()?.clamp(10, 600),
            _ => bail!("usage: party_peer [--invite Character] [--seconds 90]"),
        }
    }
    let mut client = Client::spawn_on(&tokio::runtime::Handle::current());
    let mut state = WorldState::default();
    let setup = setup(&mut client, &mut state).await;
    let result = match setup {
        Ok(_) => run_peer(&mut client, &mut state, target, seconds).await,
        Err(e) => Err(e),
    };
    if state.party.id != 0 {
        client.send(Command::Party(PartyCommand::Leave {
            team_id: state.party.id,
            character_id: state.stats.character_id,
        }));
    }
    let logout = if state.player.spawn.is_some() {
        client.send(Command::LeaveWorld(LeaveTarget::Quit));
        until(&mut client, &mut state, |e| matches!(e, Event::LeftWorld { .. })).await
    } else {
        client.send(Command::Disconnect);
        Ok(())
    };
    result?;
    logout?;
    println!("DONE peer logged out");
    Ok(())
}
