//! A fresh, headless account for original social-window QA. Whispers only the named test peer.
//! cargo run -p aa-client --example social_peer -- --account Fresh --name Freshpeer --whisper Tester --seconds 90
use std::time::{Duration, Instant};

use aa_net::{Client, Command, CreateCharacter, Event, LeaveTarget, WorldState};
use anyhow::{Context, Result, bail, ensure};

fn wait(client: &mut Client, state: &mut WorldState, pick: impl Fn(&Event) -> bool) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(25);
    while Instant::now() < end {
        while let Some(event) = client.try_recv() {
            state.apply(&event);
            if pick(&event) {
                return Ok(());
            }
            if matches!(
                event,
                Event::Disconnected { .. } | Event::LoginFailed { .. }
            ) {
                bail!("peer connection ended: {event:?}");
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    bail!("timed out waiting for the lane server")
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let option = |key: &str| args.windows(2).find(|v| v[0] == key).map(|v| v[1].clone());
    let account = option("--account").context("--account must name a fresh isolated QA account")?;
    let name = option("--name").context("--name is required")?;
    let target = option("--whisper").context("--whisper must name our QA client character")?;
    let resume = args.iter().any(|a| a == "--resume");
    ensure!(
        !resume || account.starts_with("solsocialpeer"),
        "--resume is limited to this fixture's isolated account prefix"
    );
    ensure!(
        !account.is_empty()
            && account.len() <= 64
            && account.bytes().all(|b| b.is_ascii_alphanumeric()),
        "invalid account"
    );
    for value in [&name, &target] {
        ensure!(
            (2..=18).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphabetic()),
            "test names must be ASCII letters"
        );
    }
    let seconds = option("--seconds")
        .map(|v| v.parse::<u64>())
        .transpose()?
        .unwrap_or(90)
        .clamp(10, 300);
    let mut client = Client::spawn_thread()?;
    let mut state = WorldState::default();
    client.send(Command::Login {
        addr: aa_net::default_login(),
        user: account.clone(),
        password: account.into(),
    });
    wait(&mut client, &mut state, |e| {
        matches!(e, Event::WorldList(_))
    })?;
    let world_id = state
        .worlds
        .list
        .worlds
        .iter()
        .find(|w| w.online)
        .context("no online world")?
        .id;
    client.send(Command::EnterWorld { world_id });
    wait(&mut client, &mut state, |e| {
        matches!(e, Event::ReadyToConnect { .. })
    })?;
    client.send(Command::ConnectToWorld);
    wait(&mut client, &mut state, |e| {
        matches!(e, Event::CharacterList(_))
    })?;
    if resume {
        ensure!(
            state.characters.list.len() == 1 && state.characters.list[0].name == name,
            "resume requires exactly the named fixture character"
        );
    } else {
        ensure!(
            state.characters.list.is_empty(),
            "account has existing characters; use a fresh QA account"
        );
        client.send(Command::CreateCharacter(CreateCharacter::new(
            &name,
            aa_net::model::race::NUIAN,
            aa_net::model::gender::MALE,
        )));
        wait(
            &mut client,
            &mut state,
            |e| matches!(e,Event::CharacterList(list) if !list.is_empty()),
        )?;
    }
    client.send(Command::EnterGame {
        character_id: state.characters.list[0].id,
    });
    wait(&mut client, &mut state, |e| {
        matches!(e, Event::EnteredWorld)
    })?;
    println!(
        "READY character={} id={} target={} lifetime={}s",
        state.stats.name, state.stats.character_id, target, seconds
    );
    let start = Instant::now();
    let mut next = Duration::from_secs(5);
    let mut sequence = 0;
    while start.elapsed() < Duration::from_secs(seconds) {
        while let Some(event) = client.try_recv() {
            state.apply(&event);
            match event {
                Event::Chat(ref m) => println!(
                    "CHAT type={} sender={} text={}",
                    m.chat_type, m.sender, m.message
                ),
                Event::Friend(ref e) => println!("FRIEND {e:?}"),
                Event::Disconnected { .. } => bail!("peer disconnected: {event:?}"),
                _ => {}
            }
        }
        if start.elapsed() >= next {
            sequence += 1;
            client.send(Command::Chat {
                chat_type: aa_net::model::chat_type::WHISPER,
                target: target.clone(),
                message: format!("social QA whisper {sequence}"),
            });
            println!("SENT whisper {sequence}");
            next += Duration::from_secs(5);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    client.send(Command::LeaveWorld(LeaveTarget::Quit));
    wait(&mut client, &mut state, |e| {
        matches!(e, Event::LeftWorld { .. })
    })?;
    println!("DONE peer logged out");
    Ok(())
}
