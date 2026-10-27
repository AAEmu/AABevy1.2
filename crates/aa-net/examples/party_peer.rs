//! Headless fresh-account party peer for real HUD QA; never broadcasts area invitations.
//! cargo run -p aa-net --example party_peer -- --accept-from Tester --seconds 120
//! Optional --ping-after 5 --clear-after 8 waits for owner/marked permission and server ACKs.

use aa_net::model::{gender, race};
use aa_net::{
    Client, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, PartyCommand, PartyEvent,
    Pos, WorldState,
};
use anyhow::{Context, Result, bail, ensure};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

const USAGE: &str = "party_peer [--invite Character] [--seconds 90] [--account FreshAccount] [--name LettersOnly] [--accept-from Character] [--position x,y,z] [--role 0..3] [--ping-after Seconds] [--clear-after Seconds] [--ping-position x,y,z] [--ping-instance ID]";

struct Options {
    account: String,
    name: String,
    invite: Option<String>,
    accept_from: Option<String>,
    seconds: u64,
    position: Option<Pos>,
    role: Option<u8>,
    ping_after: Option<Duration>,
    clear_after: Duration,
    ping_position: Option<Pos>,
    ping_instance: Option<u32>,
}

fn position(value: &str) -> Result<Pos> {
    let values = value
        .split(',')
        .map(str::parse::<f32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        values.len() == 3 && values.iter().all(|v| v.is_finite()),
        "position requires three finite comma-separated numbers"
    );
    Ok(Pos::new(values[0], values[1], values[2]))
}

impl Options {
    fn parse() -> Result<Option<Self>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let mut digits = stamp as u64;
        let mut name = "Peer".to_string();
        while digits != 0 {
            name.push((b'a' + (digits % 26) as u8) as char);
            digits /= 26;
        }
        let mut options = Self {
            account: format!("partyhud{stamp}"),
            name,
            invite: None,
            accept_from: None,
            seconds: 90,
            position: None,
            role: None,
            ping_after: None,
            clear_after: Duration::from_secs(8),
            ping_position: None,
            ping_instance: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if matches!(arg.as_str(), "--help" | "-h") {
                println!("{USAGE}");
                return Ok(None);
            }
            let value = args
                .next()
                .with_context(|| format!("{arg} needs a value; {USAGE}"))?;
            match arg.as_str() {
                "--invite" => options.invite = Some(value),
                "--seconds" => options.seconds = value.parse::<u64>()?.clamp(10, 3600),
                "--account" => options.account = value,
                "--name" => options.name = value,
                "--accept-from" => options.accept_from = Some(value),
                "--position" => options.position = Some(position(&value)?),
                "--role" => options.role = Some(value.parse()?),
                "--ping-after" => options.ping_after = Some(Duration::from_secs(value.parse()?)),
                "--clear-after" => options.clear_after = Duration::from_secs(value.parse()?),
                "--ping-position" => options.ping_position = Some(position(&value)?),
                "--ping-instance" => options.ping_instance = Some(value.parse()?),
                _ => bail!("unknown option {arg}; {USAGE}"),
            }
        }
        ensure!(
            (2..=18).contains(&options.name.len())
                && options.name.bytes().all(|b| b.is_ascii_alphabetic()),
            "--name must be2..18 ASCII letters"
        );
        ensure!(
            !options.account.is_empty()
                && options.account.len() <= 64
                && options.account.bytes().all(|b| b.is_ascii_alphanumeric()),
            "--account must be a fresh alphanumeric test account"
        );
        ensure!(options.role.is_none_or(|r| r <= 3), "--role must be0..3");
        Ok(Some(options))
    }
}

async fn until(
    client: &mut Client,
    state: &mut WorldState,
    pick: impl Fn(&Event) -> bool,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        while let Some(event) = client.recv().await {
            state.apply(&event);
            match &event {
                Event::LoginFailed { message }
                | Event::ConnectFailed { message }
                | Event::Disconnected { message } => bail!("{message}"),
                Event::CharacterCreateFailed { reason } => {
                    bail!("character creation failed {reason}")
                }
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

async fn setup(client: &mut Client, state: &mut WorldState, options: &Options) -> Result<()> {
    client.send(Command::Login {
        addr: aa_net::default_login(),
        user: options.account.clone(),
        password: options.account.clone().into(),
    });
    until(client, state, |e| matches!(e, Event::WorldList(_))).await?;
    let world_id = state
        .worlds
        .list
        .worlds
        .iter()
        .find(|w| w.online)
        .context("no online world")?
        .id;
    client.send(Command::EnterWorld { world_id });
    until(client, state, |e| matches!(e, Event::ReadyToConnect { .. })).await?;
    client.send(Command::ConnectToWorld);
    until(client, state, |e| matches!(e, Event::CharacterList(_))).await?;
    ensure!(
        state.characters.list.is_empty(),
        "account already has characters; use a fresh isolated account"
    );
    client.send(Command::CreateCharacter(CreateCharacter::new(
        &options.name,
        race::NUIAN,
        gender::MALE,
    )));
    until(
        client,
        state,
        |e| matches!(e,Event::CharacterList(list) if !list.is_empty()),
    )
    .await?;
    client.send(Command::EnterGame {
        character_id: state.characters.list[0].id,
    });
    until(client, state, |e| matches!(e, Event::EnteredWorld)).await?;
    println!(
        "READY account={} character={} id={} obj={} instance={} lifetime={}s",
        options.account,
        state.stats.name,
        state.stats.character_id,
        state.stats.obj_id,
        state.world.instance_id,
        options.seconds
    );
    Ok(())
}

fn request(client: &Client, command: PartyCommand) {
    println!("REQUEST {command:?}");
    client.send(Command::Party(command));
}

fn can_ping(state: &WorldState) -> bool {
    // Exact lane Team.IsMarked: any marker's stored ID matches, regardless of marker type.
    state.party.id != 0
        && (state.party.owner_id == state.stats.character_id
            || state
                .party
                .marks
                .iter()
                .any(|(_, id)| *id == state.stats.character_id))
}

fn log_party(state: &WorldState, event: &PartyEvent) {
    let local = matches!(
        event,
        PartyEvent::Replied { .. } | PartyEvent::RequestRejected(_)
    );
    println!("{} {event:?}", if local { "LOCAL" } else { "ACK" });
    if !matches!(event, PartyEvent::RemoteMembers(_)) {
        let members = state
            .party
            .members
            .iter()
            .map(|m| {
                format!(
                    "{}:{}@slot{}/role{}/online{}",
                    m.id, m.name, m.slot, m.role, m.online
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "STATE team={} owner={} party={} members=[{members}] loot={:?} marks={:?} ping={:?} may_ping={}",
            state.party.id,
            state.party.owner_id,
            state.party.is_party,
            state.party.looting_rule,
            state.party.marks,
            state.party.ping,
            can_ping(state)
        );
    }
}

#[derive(Clone, Copy, Debug)]
enum Ping {
    Waiting,
    AwaitSet {
        team: u32,
        pos: Pos,
        instance: u32,
        sent: Instant,
    },
    Showing {
        team: u32,
        pos: Pos,
        instance: u32,
        clear: Instant,
    },
    AwaitClear {
        sent: Instant,
    },
    Done,
}

impl Ping {
    fn observe(&mut self, event: &PartyEvent, options: &Options) {
        match (*self, event) {
            (
                Self::AwaitSet {
                    team,
                    pos,
                    instance,
                    ..
                },
                PartyEvent::Ping {
                    has_ping: true,
                    pos: received,
                    instance_id,
                },
            ) if pos == *received && instance == *instance_id => {
                println!("PING_CONFIRMED team={team} pos={pos:?} instance={instance}");
                *self = Self::Showing {
                    team,
                    pos,
                    instance,
                    clear: Instant::now() + options.clear_after,
                };
            }
            (
                Self::AwaitClear { .. },
                PartyEvent::Ping {
                    has_ping: false, ..
                },
            ) => {
                println!("PING_CLEAR_CONFIRMED");
                *self = Self::Done;
            }
            _ => {}
        }
    }

    fn tick(
        &mut self,
        client: &Client,
        state: &WorldState,
        options: &Options,
        joined: Option<Instant>,
        warned: &mut bool,
    ) {
        let now = Instant::now();
        match *self {
            Self::Waiting => {
                let (Some(delay), Some(joined)) = (options.ping_after, joined) else {
                    return;
                };
                if now < joined + delay || state.party.id == 0 {
                    return;
                }
                if !can_ping(state) {
                    if !*warned {
                        println!(
                            "PING_WAIT_PERMISSION owner={} self={} (leader must mark this member)",
                            state.party.owner_id, state.stats.character_id
                        );
                        *warned = true;
                    }
                    return;
                }
                let Some(spawn) = &state.player.spawn else {
                    return;
                };
                let pos = options
                    .ping_position
                    .or(options.position)
                    .unwrap_or(spawn.unit.pos);
                let instance = options.ping_instance.unwrap_or(state.world.instance_id);
                let team = state.party.id;
                request(
                    client,
                    PartyCommand::SetPing {
                        team_id: team,
                        has_ping: true,
                        pos,
                        instance_id: instance,
                    },
                );
                *self = Self::AwaitSet {
                    team,
                    pos,
                    instance,
                    sent: now,
                };
            }
            Self::Showing {
                team,
                pos,
                instance,
                clear,
            } if now >= clear => {
                if state.party.id != team
                    || !can_ping(state)
                    || state.party.ping != Some((pos, instance))
                {
                    println!("PING_CLEAR_SKIPPED membership/authority/ping changed");
                    *self = Self::Done;
                    return;
                }
                request(
                    client,
                    PartyCommand::SetPing {
                        team_id: team,
                        has_ping: false,
                        pos,
                        instance_id: instance,
                    },
                );
                *self = Self::AwaitClear { sent: now };
            }
            Self::AwaitSet { sent, .. } | Self::AwaitClear { sent }
                if now >= sent + Duration::from_secs(10) =>
            {
                println!("PING_UNACKNOWLEDGED no matching server response in10s");
                *self = Self::Done;
            }
            _ => {}
        }
    }
}

async fn run_peer(client: &mut Client, state: &mut WorldState, options: &Options) -> Result<()> {
    if let Some(pos) = options.position {
        client.send(Command::Move(MoveCommand {
            pos,
            ..MoveCommand::default()
        }));
    }
    if let Some(target_name) = &options.invite {
        request(
            client,
            PartyCommand::Invite {
                team_id: 0,
                is_party: true,
                target_name: target_name.clone(),
            },
        );
    }
    let deadline = tokio::time::sleep(Duration::from_secs(options.seconds));
    tokio::pin!(deadline);
    let mut pulse = tokio::time::interval(Duration::from_millis(250));
    let mut next_movement = Instant::now();
    let mut joined = None;
    let mut role_sent = false;
    let mut warned = false;
    let mut ping = Ping::Waiting;
    loop {
        tokio::select! {
            _=&mut deadline=>return Ok(()),
            _=pulse.tick()=> {
                if state.party.id!=0 && joined.is_none() {joined=Some(Instant::now());}
                if state.party.id!=0 && !role_sent {
                    if let Some(role)=options.role {request(client,PartyCommand::SetRole{team_id:state.party.id,member_id:state.stats.character_id,role});}
                    role_sent=true;
                }
                ping.tick(client,state,options,joined,&mut warned);
                if Instant::now()>=next_movement {
                    if let Some(spawn)=&state.player.spawn {
                        client.send(Command::Move(MoveCommand{pos:options.position.unwrap_or(spawn.unit.pos),..MoveCommand::default()}));
                    }
                    next_movement=Instant::now()+Duration::from_secs(2);
                }
            }
            event=client.recv()=> {
                let Some(event)=event else {bail!("peer session ended")};
                state.apply(&event);
                match event {
                    Event::Party(event)=> {
                        log_party(state,&event);
                        ping.observe(&event,options);
                        if let PartyEvent::Invited(invite)=event {
                            let accept=!invite.is_area && options.accept_from.as_ref().is_none_or(|name|name.eq_ignore_ascii_case(&invite.sender_name));
                            println!("{} invitation from {}",if accept {"ACCEPTING"}else{"DECLINING"},invite.sender_name);
                            request(client,PartyCommand::Reply{invite,accept});
                        }
                    }
                    Event::Disconnected{message}=>bail!("peer disconnected: {message}"),
                    Event::ServerError{code}=>eprintln!("SERVER_NOTICE code={code}"),
                    _=>{}
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let Some(options) = Options::parse()? else {
        return Ok(());
    };
    let mut client = Client::spawn_on(&tokio::runtime::Handle::current());
    let mut state = WorldState::default();
    let result = match setup(&mut client, &mut state, &options).await {
        Ok(()) => run_peer(&mut client, &mut state, &options).await,
        Err(e) => Err(e),
    };
    if state.party.id != 0 {
        request(
            &client,
            PartyCommand::Leave {
                team_id: state.party.id,
                character_id: state.stats.character_id,
            },
        );
    }
    let logout = if state.player.spawn.is_some() {
        client.send(Command::LeaveWorld(LeaveTarget::Quit));
        until(&mut client, &mut state, |e| {
            matches!(e, Event::LeftWorld { .. })
        })
        .await
    } else {
        client.send(Command::Disconnect);
        Ok(())
    };
    result?;
    logout?;
    println!("DONE peer logged out");
    Ok(())
}
