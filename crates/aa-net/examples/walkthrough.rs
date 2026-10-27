//! Walks the whole client flow against a local AAEmu server: log in, world list, character list
//! (creating a character when there is none), enter the world, watch the surroundings for a few
//! seconds, move, chat and log out.
//!
//! ```text
//! cargo run -p aa-net --example walkthrough -- [--login 127.0.0.1:1237] [--user bevytest]
//!     [--password-file PATH] [--world ID] [--character NAME] [--watch-secs 10] [--roundtrip] [--delete-test]
//!     [--move-secs N] [--move-hz 10] [--scenario circle|far|underground|jump|fall] [--no-ping] [--ping-interval-ms 3000] [--first-ping-delay-ms N]
//!     [--cmd "/add_labor 10"]... [--target-nearest] [--skill ID]... [--skill-target self|nearest|hostile|OBJ]
//!     [--item TEMPLATE:SKILL]... [--resurrect in-place|shrine]
//! ```
//!
//! `--roundtrip` additionally goes back to the character list, then to the world list, and enters
//! the game a second time before logging out.
//!
//! `--cmd TEXT` (repeatable) sends the text as chat once the player is in the world (server GM
//! commands such as `/add_labor 10` or `/add_xp 500` start with a slash) and prints the events
//! that follow, then the player state again.
//!
//! `--target-nearest` selects the nearest unit in view (`Command::Target`) and prints what the
//! server answers.
//!
//! `--skill ID` (repeatable) casts a skill the character knows at `--skill-target` (default the
//! nearest unit) and prints every event of the next 3 s; `--item TEMPLATE:SKILL` uses the bag
//! item of that template (`SKILL` is `items.use_skill_id`).
//!
//! `--delete-test` creates a second character, deletes it and shows what the server answers.
//!
//! `--move-secs N` walks a circle for N seconds, sending `--move-hz` movement packets per second
//! (default 10), and reports when the server drops the connection (soak test for the keep-alive).
//! `--no-ping` and `--first-ping-delay-ms` change the keep-alive to show what the server does to a
//! client that pings late or never (it drops the connection at its next 30 s sweep).
//!
//! The password comes from `--password-file`, else `$AA_PASSWORD_FILE`, else the lane's shared test
//! password file when it exists, else the user name. It is never printed. With the server's
//! `AutoAccount` an unknown account is created on its first login.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use aa_net::login::congestion;
use aa_net::model::{gender, race, slot, UnitKind};
use aa_net::{Client, ClientOptions, Command, CreateCharacter, Event, LeaveTarget, MoveCommand, MoveState, Pos, WorldState};
use anyhow::{anyhow, bail, Context, Result};

struct Args {
    login: String,
    user: String,
    password_file: Option<String>,
    world: Option<u8>,
    character: String,
    watch_secs: u64,
    roundtrip: bool,
    delete_test: bool,
    move_secs: u64,
    move_hz: u32,
    scenario: String,
    ping: bool,
    ping_interval_ms: u64,
    first_ping_delay_ms: u64,
    cmds: Vec<String>,
    target_nearest: bool,
    skills: Vec<u32>,
    skill_target: String,
    items: Vec<(u32, u32)>,
    resurrect: Option<bool>,
    cmd_wait_ms: u64,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        login: aa_net::default_login(),
        user: "bevytest".into(),
        password_file: std::env::var("AA_PASSWORD_FILE").ok(),
        world: None,
        character: "Bevytest".into(),
        watch_secs: 10,
        roundtrip: false,
        delete_test: false,
        move_secs: 0,
        move_hz: 10,
        scenario: "circle".into(),
        ping: true,
        ping_interval_ms: 3000,
        first_ping_delay_ms: 0,
        cmds: Vec::new(),
        target_nearest: false,
        skills: Vec::new(),
        skill_target: "nearest".into(),
        items: Vec::new(),
        resurrect: None,
        cmd_wait_ms: 2500,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--login" => a.login = value()?,
            "--user" => a.user = value()?,
            "--password-file" => a.password_file = Some(value()?),
            "--world" => a.world = Some(value()?.parse()?),
            "--character" => a.character = value()?,
            "--watch-secs" => a.watch_secs = value()?.parse()?,
            "--roundtrip" => a.roundtrip = true,
            "--delete-test" => a.delete_test = true,
            "--move-secs" => a.move_secs = value()?.parse()?,
            "--move-hz" => a.move_hz = value()?.parse()?,
            "--scenario" => a.scenario = value()?,
            "--no-ping" => a.ping = false,
            "--ping-interval-ms" => a.ping_interval_ms = value()?.parse()?,
            "--first-ping-delay-ms" => a.first_ping_delay_ms = value()?.parse()?,
            "--cmd" => a.cmds.push(value()?),
            "--target-nearest" => a.target_nearest = true,
            "--skill" => a.skills.push(value()?.parse()?),
            "--skill-target" => a.skill_target = value()?,
            "--cmd-wait-ms" => a.cmd_wait_ms = value()?.parse()?,
            "--resurrect" => a.resurrect = Some(value()? == "in-place"),
            "--item" => {
                let v = value()?;
                let (t, s) = v.split_once(':').context("--item needs TEMPLATE:SKILL")?;
                a.items.push((t.parse()?, s.parse()?));
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

fn password(args: &Args) -> String {
    let path = args.password_file.clone().or_else(|| std::env::var("AA_PASSWORD_FILE").ok());
    match path.and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => args.user.clone(),
    }
}

/// Waits for an event `pick` accepts, folding every event into `state`. Failures end the wait.
async fn until<T>(
    client: &mut Client,
    state: &mut WorldState,
    what: &str,
    timeout: Duration,
    mut pick: impl FnMut(&Event) -> Option<T>,
) -> Result<T> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = tokio::time::timeout(left, client.recv())
            .await
            .map_err(|_| anyhow!("timed out waiting for {what}"))?
            .ok_or_else(|| anyhow!("the session ended while waiting for {what}"))?;
        state.apply(&event);
        match &event {
            Event::LoginFailed { message } | Event::ConnectFailed { message } => {
                bail!("{what}: {message}")
            }
            Event::Disconnected { message } => bail!("{} {what}: disconnected ({message})", stamp()),
            Event::RequestTimedOut { what: w } => bail!("{what}: timed out waiting for {w}"),
            Event::EnterWorldDenied { reason } => bail!("{what}: enter world denied ({reason})"),
            Event::CharacterCreateFailed { reason } => bail!("{what}: character creation failed ({reason})"),
            Event::ServerError { code } => println!("  server error message {code}"),
            Event::Kicked { reason, message } => bail!("{what}: kicked ({reason}) {message}"),
            _ => {}
        }
        if let Some(v) = pick(&event) {
            return Ok(v);
        }
    }
}

/// UTC wall clock, `HH:MM:SS.mmm`, to line events up with the server log.
fn stamp() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let day = ms % 86_400_000;
    format!("{:02}:{:02}:{:02}.{:03}Z", day / 3_600_000, day / 60_000 % 60, day / 1000 % 60, day % 1000)
}

fn congestion_name(c: u8) -> &'static str {
    match c {
        congestion::LOW => "low",
        congestion::MIDDLE => "middle",
        congestion::HIGH => "high",
        congestion::FULL => "full",
        congestion::PRE_SELECT_RACE_FULL => "pre-select full",
        congestion::CHECK => "check",
        _ => "?",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args = parse_args()?;
    let options = ClientOptions {
        ping_interval: args.ping.then_some(Duration::from_millis(args.ping_interval_ms)),
        first_ping_delay: Duration::from_millis(args.first_ping_delay_ms),
        item_recharge_catalog: None,
    };
    let mut client = Client::spawn_on_with(&tokio::runtime::Handle::current(), options);
    let mut state = WorldState::default();
    let step = Duration::from_secs(20);

    // 1. Login and the world list.
    println!("== login as {} at {}", args.user, args.login);
    client.send(Command::Login {
        addr: args.login.clone(),
        user: args.user.clone(),
        password: password(&args).into(),
    });
    let account = until(&mut client, &mut state, "login", step, |e| match e {
        Event::LoginSucceeded(a) => Some(*a),
        _ => None,
    })
    .await?;
    println!(
        "  account {} (max {} characters, {} extra per world, pre-select period: {})",
        account.account_id, account.max_characters, account.extra_characters_per_world, account.pre_select_period
    );
    let worlds = until(&mut client, &mut state, "the world list", step, |e| match e {
        Event::WorldList(l) => Some(l.clone()),
        _ => None,
    })
    .await?;
    println!("== worlds");
    for w in &worlds.worlds {
        println!(
            "  [{}] {:<12} online={} state={} characters={} race congestion={:?}",
            w.id,
            w.name,
            w.online,
            congestion_name(w.congestion),
            worlds.character_count(w.id),
            &w.race_congestion[1..]
        );
    }
    let world = match args.world {
        Some(id) => worlds.world(id),
        None => worlds.worlds.iter().find(|w| w.online),
    }
    .filter(|w| w.online)
    .context("no online world to enter")?;

    // 2. Enter the world: the login server hands over the game server address.
    println!("== entering world {} ({})", world.id, world.name);
    client.send(Command::EnterWorld { world_id: world.id });
    let cookie = until(&mut client, &mut state, "the world cookie", Duration::from_secs(120), |e| match e {
        Event::WorldQueue(q) => {
            println!("  queued: place {} of {}", q.my_turn, q.normal_length);
            None
        }
        Event::ReadyToConnect { cookie, .. } => Some(*cookie),
        _ => None,
    })
    .await?;
    println!("  game server {} (cookie {})", cookie.addr, cookie.cookie);
    println!("  {} connecting to the game server", stamp());
    client.send(Command::ConnectToWorld);
    let mut characters = until(&mut client, &mut state, "the character list", step, |e| match e {
        Event::CharacterList(l) => Some(l.clone()),
        _ => None,
    })
    .await?;
    println!("  race congestion for creation: {:?}", &state.characters.race_congestion[1..]);

    // 3. Characters; create one if there is none.
    println!("== characters ({})", characters.len());
    if characters.is_empty() {
        println!("  none, creating {}", args.character);
        client.send(Command::CreateCharacter(CreateCharacter::new(
            args.character.clone(),
            race::NUIAN,
            gender::MALE,
        )));
        let created = until(&mut client, &mut state, "the new character", step, |e| match e {
            Event::CharacterCreated(c) => Some((**c).clone()),
            _ => None,
        })
        .await?;
        println!("  created {} (id {})", created.name, created.id);
        characters = until(&mut client, &mut state, "the refreshed list", step, |e| match e {
            Event::CharacterList(l) => Some(l.clone()),
            _ => None,
        })
        .await?;
    }
    for c in &characters {
        let gear: Vec<String> = c
            .equipment
            .slots
            .iter()
            .enumerate()
            .filter(|(i, s)| s.is_some() && !slot::is_body_part(*i))
            .map(|(i, s)| format!("{i}:{}", s.as_ref().map_or(0, |s| s.template_id)))
            .collect();
        println!(
            "  id {} {:<12} {} {} lvl {} zone {} at ({:.1}, {:.1}, {:.1}) abilities {:?} gear [{}] body {:?}",
            c.id,
            c.name,
            aa_net::model::race::name(c.race),
            if c.gender == gender::MALE { "male" } else { "female" },
            c.level,
            c.zone_id,
            c.pos.x,
            c.pos.y,
            c.pos.z,
            c.abilities,
            gear.join(" "),
            &c.equipment.template_ids()[slot::FACE..=slot::BEARD]
        );
    }
    if args.delete_test {
        delete_test(&mut client, &mut state).await?;
        characters = state.characters.list.clone();
    }
    let chosen = characters
        .iter()
        .find(|c| !c.delete_requested())
        .context("every character is pending deletion")?
        .clone();

    // 4. Enter the game.
    println!("== entering the game as {}", chosen.name);
    client.send(Command::EnterGame { character_id: chosen.id });
    let spawn = until(&mut client, &mut state, "the player spawn", step, |e| match e {
        Event::PlayerSpawn(s) => Some((**s).clone()),
        _ => None,
    })
    .await?;
    let u = &spawn.unit;
    println!(
        "  obj {} zone {} instance {} at ({:.2}, {:.2}, {:.2}) yaw {:.2} model {} level {}",
        spawn.obj_id,
        spawn.zone_id(),
        spawn.state.instance_id,
        spawn.position().x,
        spawn.position().y,
        spawn.position().z,
        spawn.yaw(),
        u.model_id,
        u.level
    );
    println!(
        "  appearance: kind {} hair {} skin {} model {}, face {}; equipment templates {:?}",
        u.model_params.kind,
        u.model_params.hair_color,
        u.model_params.skin_color,
        u.model_params.model_id,
        if u.model_params.face.is_some() { "yes" } else { "no" },
        u.equipment.template_ids()
    );
    until(&mut client, &mut state, "entered world", step, |e| matches!(e, Event::EnteredWorld).then_some(())).await?;
    println!("  {} in the world", stamp());

    // 5. Watch what appears around us.
    println!("== watching {} s", args.watch_secs);
    let end = Instant::now() + Duration::from_secs(args.watch_secs);
    let (mut spawned, mut moved, mut removed) = (0, 0, 0);
    while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
        state.apply(&e);
        match e {
            Event::UnitSpawned(_) => spawned += 1,
            Event::UnitMoved { .. } => moved += 1,
            Event::UnitsRemoved(ids) => removed += ids.len(),
            Event::Chat(c) => println!("  chat [{}/{}] {}: {}", c.chat_type, c.sub_type, c.sender, c.message),
            Event::ChatChannelJoined { .. }
            | Event::ChatChannelLeft { .. }
            | Event::TimeOfDay { .. }
            | Event::Snow(_)
            | Event::TargetChanged { .. } => println!("  {e:?}"),
            Event::Disconnected { message } => bail!("{} disconnected while watching: {message}", stamp()),
            _ => {}
        }
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for u in state.units.iter() {
        *kinds
            .entry(match u.kind {
                UnitKind::Character { .. } => "characters",
                UnitKind::Npc { .. } => "npcs",
                UnitKind::Slave { .. } => "slaves",
                UnitKind::Housing { .. } => "houses",
                UnitKind::Transfer { .. } => "transfers",
                UnitKind::Mate { .. } => "mates",
                UnitKind::Shipyard { .. } => "shipyards",
            })
            .or_default() += 1;
    }
    println!(
        "  units in view: {} {:?} (spawn events {spawned}, movement events {moved}, removed {removed}), doodads: {}",
        state.units.by_id.len(),
        kinds,
        state.doodads.by_id.len()
    );
    let mut here = spawn.position();
    let mut nearest: Vec<_> = state.units.iter().collect();
    nearest.sort_by(|a, b| a.pos.distance(here).total_cmp(&b.pos.distance(here)));
    for u in nearest.iter().take(5) {
        println!(
            "  {:>6.1} m  {:<10} {:?} lvl {} model {} hp {} at ({:.1}, {:.1}, {:.1}) obj {}",
            u.pos.distance(here),
            if u.name.is_empty() { "(unnamed)" } else { &u.name },
            u.kind,
            u.level,
            u.model_id,
            u.hp,
            u.pos.x,
            u.pos.y,
            u.pos.z,
            u.obj_id
        );
    }

    print_player_state(&state);
    if let Some(in_place) = args.resurrect {
        println!("== resurrect (in place: {in_place}), dead: {}", state.stats.dead);
        client.send(Command::Resurrect { in_place });
        let end = Instant::now() + Duration::from_millis(3000);
        while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
            state.apply(&e);
            if let Event::Teleported { pos, .. } = &e {
                here = *pos;
            }
            if matches!(e, Event::UnitMoved { .. } | Event::UnitSpawned(_) | Event::DoodadsSpawned(_) | Event::UnitsRemoved(_) | Event::TargetChanged { .. }) {
                continue;
            }
            let line = format!("{e:?}");
            println!("  event {}", if line.len() > 300 { format!("{}...", &line[..300]) } else { line });
        }
        println!("  hp {}/{} dead {}", state.stats.hp, state.stats.max_hp, state.stats.dead);
    }
    if !args.cmds.is_empty() {
        for text in &args.cmds {
            println!("== chat: {text}");
            client.send(Command::Chat {
                chat_type: aa_net::model::chat_type::SAY,
                target: String::new(),
                message: text.clone(),
            });
            let mut teleported = false;
            let end = Instant::now() + Duration::from_millis(args.cmd_wait_ms);
            while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
                state.apply(&e);
                if let Event::Teleported { pos, .. } = &e {
                    here = *pos;
                    teleported = true;
                }
                if matches!(e, Event::UnitMoved { .. } | Event::UnitSpawned(_) | Event::DoodadsSpawned(_)) {
                    continue;
                }
                let line = format!("{e:?}");
                println!("  event {}", if line.len() > 240 { format!("{}...", &line[..240]) } else { line });
            }
            if teleported {
                // The server streams the new surroundings once the client moves.
                println!("  moving to wake up the area at ({:.0}, {:.0})", here.x, here.y);
                for _ in 0..3 {
                    client.send(Command::Move(MoveCommand::new(here, 0.0)));
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                let end = Instant::now() + Duration::from_millis(args.cmd_wait_ms);
                while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
                    state.apply(&e);
                }
                println!("  {} units, {} doodads in view", state.units.by_id.len(), state.doodads.by_id.len());
            }
        }
        print_player_state(&state);
    }

    if args.target_nearest {
        println!("== target");
        let own = state.stats.obj_id;
        let mut nearest: Vec<_> = state.units.iter().collect();
        nearest.sort_by(|a, b| a.pos.distance(here).total_cmp(&b.pos.distance(here)));
        println!("  {} units in view", nearest.len());
        match nearest.first() {
            Some(u) => {
                println!("  selecting {} (obj {}) {:?}", u.name, u.obj_id, u.kind);
                client.send(Command::Target { obj_id: u.obj_id });
            }
            None => println!("  no unit in view; clearing the target"),
        }
        let end = Instant::now() + Duration::from_millis(1500);
        while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
            state.apply(&e);
            if let Event::TargetChanged { obj_id, target_obj_id } = e {
                println!("  TargetChanged obj {obj_id} (own {own}) -> {target_obj_id}");
            }
        }
        print_world_state(&state);
        client.send(Command::Target { obj_id: 0 });
        let end = Instant::now() + Duration::from_millis(1000);
        while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
            state.apply(&e);
        }
        println!("  after clearing: target {:?}", state.target.obj_id);
    }

    if !args.skills.is_empty() || !args.items.is_empty() {
        println!("== skills");
        let mut uses: Vec<Command> = Vec::new();
        let target_obj = match args.skill_target.as_str() {
            "self" => 0,
            // The nearest monster (faction 115): walk next to it first.
            "hostile" => {
                let mob = state
                    .units
                    .iter()
                    .filter(|u| u.faction_id == 115)
                    .min_by(|a, b| a.pos.distance(here).total_cmp(&b.pos.distance(here)))
                    .map(|u| (u.obj_id, u.pos, u.name.clone(), u.level, u.hp));
                match mob {
                    Some((id, pos, name, level, hp)) => {
                        println!("  hostile unit {id} {name:?} lvl {level} hp {hp} at {:.1} m", pos.distance(here));
                        let near = Pos::new(pos.x - 1.5, pos.y, pos.z);
                        for _ in 0..3 {
                            client.send(Command::Move(MoveCommand::new(near, 0.0)));
                            tokio::time::sleep(Duration::from_millis(150)).await;
                        }
                        here = near;
                        id
                    }
                    None => 0,
                }
            }
            "nearest" => state
                .units
                .iter()
                .min_by(|a, b| a.pos.distance(here).total_cmp(&b.pos.distance(here)))
                .map_or(0, |u| u.obj_id),
            n => n.parse().context("--skill-target is self, nearest or an object id")?,
        };
        println!("  target object {target_obj} (0 = self); known skills {:?}", state.skills.known);
        for &skill_id in &args.skills {
            uses.push(Command::UseSkill { skill_id, target_obj_id: target_obj });
        }
        for &(template_id, skill_id) in &args.items {
            match state.inventory.find_template(template_id) {
                Some((slot, item)) => {
                    println!("  item template {template_id} found in bag slot {slot} id {} x{}", item.id, item.count);
                    uses.push(Command::UseItem { item_id: item.id, template_id, skill_id, target_obj_id: 0 });
                }
                None => println!("  no item of template {template_id} in the bag"),
            }
        }
        if target_obj != 0 {
            client.send(Command::Target { obj_id: target_obj });
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        for cmd in uses {
            println!("  send {cmd:?}");
            client.send(cmd);
            let sent = Instant::now();
            let end = Instant::now() + Duration::from_millis(3000);
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
                state.apply(&e);
                let line = format!("{e:?}");
                let name: String = line.chars().take_while(|c| c.is_alphanumeric()).collect();
                *counts.entry(name).or_default() += 1;
                if matches!(e, Event::UnitMoved { .. } | Event::UnitSpawned(_) | Event::DoodadsSpawned(_) | Event::TargetChanged { .. }) {
                    continue;
                }
                println!("  event +{:>4} ms {}", sent.elapsed().as_millis(), if line.len() > 300 { format!("{}...", &line[..300]) } else { line });
            }
            println!("  event counts: {counts:?}");
            let s = &state.skills;
            println!(
                "  skills: casting {:?} last_used {:?} failure {:?}",
                s.casting_of(state.stats.obj_id).map(|c| (c.skill_id, c.tl, c.cast_ms)),
                s.last_used.iter().map(|(k, v)| (*k, v.elapsed().as_millis())).collect::<Vec<_>>(),
                s.last_failure.as_ref().map(|f| (f.skill_id, f.result, aa_net::skill::result::name(f.result)))
            );
        }
        print_player_state(&state);
    }

    // 6. Move and chat.
    println!("== move and chat");
    let mut pos = here;
    for i in 1..=3 {
        pos = Pos::new(here.x + 0.5 * i as f32, here.y + 0.5 * i as f32, here.z);
        client.send(Command::Move(MoveCommand::new(pos, 0.0)));
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("  moved to ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z);
    client.send(Command::Chat {
        chat_type: aa_net::model::chat_type::SAY,
        target: String::new(),
        message: "hello from the aa-net walkthrough".into(),
    });
    let echo = until(&mut client, &mut state, "the chat echo", step, |e| match e {
        Event::Chat(c) if c.message.contains("walkthrough") => Some(c.clone()),
        Event::Chat(c) => {
            println!("  chat [{}] {}: {}", c.chat_type, c.sender, c.message);
            None
        }
        _ => None,
    })
    .await?;
    println!("  echoed: {}: {}", echo.sender, echo.message);
    // Keep listening a moment: after a teleport the units of the new area arrive with the first moves.
    {
        let end = Instant::now() + Duration::from_millis(args.cmd_wait_ms);
        while let Ok(Some(e)) = tokio::time::timeout(end.saturating_duration_since(Instant::now()), client.recv()).await {
            state.apply(&e);
        }
        println!("  after moving: {} units, {} doodads in view", state.units.by_id.len(), state.doodads.by_id.len());
        print_units_summary(&state);
        print_world_state(&state);
    }

    if args.move_secs > 0 {
        walk_circle(&mut client, &mut state, here, args.move_secs, args.move_hz, &args.scenario).await?;
    }

    if args.roundtrip {
        roundtrip(&mut client, &mut state, world.id, chosen.id).await?;
    }

    // 7. Log out.
    println!("== logging out");
    client.send(Command::LeaveWorld(LeaveTarget::Quit));
    let wait = until(&mut client, &mut state, "the logout countdown", step, |e| match e {
        Event::LeavePending { time_ms, .. } => Some(*time_ms),
        _ => None,
    })
    .await?;
    println!("  server logs us out in {} ms", wait);
    until(&mut client, &mut state, "the logout", Duration::from_millis(wait.max(0) as u64 + 15_000), |e| {
        matches!(e, Event::LeftWorld { .. }).then_some(())
    })
    .await?;
    println!("  left the world; done");
    Ok(())
}

/// Units in view by faction.
fn print_units_summary(state: &WorldState) {
    let mut factions: BTreeMap<u32, (usize, u8, u8)> = BTreeMap::new();
    for u in state.units.iter() {
        let e = factions.entry(u.faction_id).or_insert((0, u.level, u.level));
        e.0 += 1;
        e.1 = e.1.min(u.level);
        e.2 = e.2.max(u.level);
    }
    println!("  units by faction (count, min level, max level): {factions:?}");
}

/// Zone, clock, chat channels and target.
fn print_world_state(state: &WorldState) {
    let w = &state.world;
    println!(
        "  zone key {} group {:?} instance {} time {:?} (now {:?}) snow {}",
        w.zone_key,
        w.zone_group_id,
        w.instance_id,
        w.time.map(|t| (t.hour, t.speed)),
        w.hour_now(),
        w.snow
    );
    let channels: Vec<String> = state
        .chat
        .channels
        .iter()
        .map(|c| format!("{}({}/{} faction {} '{}')", aa_net::model::chat_type::name(c.chat_type), c.chat_type, c.sub_type, c.faction_id, c.name))
        .collect();
    println!("  chat channels: {}; log {} lines", channels.join(", "), state.chat.log.len());
    match state.target.obj_id {
        Some(id) => match state.target.unit(&state.units) {
            Some(u) => println!("  target {id}: {} {:?} lvl {} hp {}/{} faction {}", u.name, u.kind, u.level, u.hp, u.max_hp, u.faction_id),
            None => println!("  target {id} (not in view)"),
        },
        None => println!("  no target"),
    }
}

/// What the HUD would read from the state tables.
fn print_player_state(state: &WorldState) {
    let s = &state.stats;
    println!("== player state");
    print_world_state(state);
    let (earned, needed) = s.exp_progress();
    let (gold, silver, copper) = s.money_parts();
    println!(
        "  {} lvl {} hp {}/{} mp {}/{} exp {earned}/{needed} (total {}, recoverable {}) labor {}/{} money {gold}g {silver}s {copper}c bank {} loyalty {} honor {} vocation {}",
        s.name, s.level, s.hp, s.max_hp, s.mp, s.max_mp, s.exp, s.recoverable_exp, s.labor,
        aa_net::state::LABOR_MAX, s.bank_money, s.loyalty, s.honor, s.vocation
    );
    let levels: Vec<u8> = s.active_abilities.iter().map(|&a| s.ability_level(a)).collect();
    println!(
        "  abilities {:?} (levels {:?}, exp {:?}) combat {} dead {}",
        s.active_abilities, levels, s.ability_exp, s.in_combat, s.dead
    );
    let own = state.buffs.of(s.obj_id);
    println!("  own buffs: {}", own.len());
    for b in own {
        println!(
            "    buff {:>8} index {} {:?} stacks {} duration {} ms left {:?} caster {:?} lvl {}",
            b.buff.buff_id, b.buff.index, b.buff.kind, b.buff.stacks, b.buff.duration_ms, b.remaining_ms(),
            b.buff.caster, b.buff.caster_level
        );
    }
    if let Some(sp) = &state.player.spawn {
        let u = &sp.unit;
        println!(
            "  unit: faction {} expedition {} appellation {} skills {:?} passive buffs {:?}",
            u.faction_id, u.expedition_id, u.appellation, u.skills, u.passive_buffs
        );
    }
    let bar: Vec<String> = state
        .actions
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, s)| match state.actions.item_count(i, &state.inventory) {
            Some(n) => format!("{}:{:?}({})x{n}", i + 1, s.kind, s.id),
            None => format!("{}:{:?}({})", i + 1, s.kind, s.id),
        })
        .collect();
    println!("  action bar ({} slots): {}", state.actions.slots.len(), bar.join(" "));
    let inv = &state.inventory;
    println!(
        "  bag {} slots ({} free), bank {} slots; items:",
        inv.bag_slots,
        inv.free_bag_slots(),
        inv.bank_slots
    );
    for (i, it) in inv.bag_items() {
        println!("    bag[{i}] template {} id {} x{} grade {} {:?}", it.template_id, it.id, it.count, it.grade, it.detail);
    }
    let worn: Vec<String> = inv
        .equipment
        .slots
        .iter()
        .enumerate()
        .filter(|(i, s)| s.is_some() && !slot::is_body_part(*i))
        .map(|(i, s)| format!("{i}:{}", s.as_ref().map_or(0, |s| s.template_id)))
        .collect();
    println!("  worn: {}", worn.join(" "));
    let with_buffs = state.units.iter().filter(|u| !state.buffs.of(u.obj_id).is_empty()).count();
    let with_faction = state.units.iter().filter(|u| u.faction_id != 0).count();
    println!("  units with buffs {with_buffs}, with a faction {with_faction} of {}", state.units.by_id.len());
    print_units_summary(state);
    let mut sample: Vec<_> = state.units.iter().filter(|u| !u.buffs.is_empty()).collect();
    sample.sort_by_key(|u| u.obj_id);
    for u in sample.iter().take(4) {
        let ids: Vec<String> = u
            .buffs
            .iter()
            .map(|b| format!("{}({:?} {} ms{})", b.buff_id, b.kind, b.duration_ms, if b.stacks > 1 { format!(" x{}", b.stacks) } else { String::new() }))
            .collect();
        println!("    npc obj {} faction {} lvl {} hp {}/{} buffs {}", u.obj_id, u.faction_id, u.level, u.hp, u.max_hp, ids.join(" "));
    }
}

/// Walks a circle around `centre` at about 4 m/s for `secs` seconds, one movement packet every
/// `1 / hz` seconds, while the session keeps delivering events. Fails when the connection is lost.
async fn walk_circle(client: &mut Client, state: &mut WorldState, centre: Pos, secs: u64, hz: u32, scenario: &str) -> Result<()> {
    const RADIUS: f32 = 4.0;
    const SPEED: f32 = 4.0;
    println!("== walking a circle for {secs} s at {hz} Hz (scenario {scenario})");
    let centre = match scenario {
        "far" => Pos::new(centre.x + 300.0, centre.y, centre.z),
        "underground" => Pos::new(centre.x, centre.y, centre.z - 150.0),
        "circle" | "jump" | "fall" => centre,
        other => bail!("unknown scenario {other}"),
    };
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / f64::from(hz.max(1))));
    let (mut sent, mut events) = (0u32, 0u32);
    let mut last_report = 0;
    while started.elapsed() < Duration::from_secs(secs) {
        tokio::select! {
            _ = tick.tick() => {
                let t = started.elapsed().as_secs_f32();
                let angle = t * SPEED / RADIUS;
                let pos = Pos::new(centre.x + RADIUS * angle.cos() - RADIUS, centre.y + RADIUS * angle.sin(), centre.z);
                // tangent of the circle: the walking direction, as a yaw (0 faces +y, positive turns left)
                let (vx, vy) = (-SPEED * angle.sin(), SPEED * angle.cos());
                let yaw = (-vx).atan2(vy);
                let mut m = MoveCommand::running(pos, yaw, SPEED);
                // jump: 0.6 s up and down every 2 s; fall: every 3 s drop 3 m and land at 25 m/s
                let phase = t % 2.0;
                match scenario {
                    "jump" if phase < 0.6 => {
                        let h = 1.2 * (1.0 - ((phase - 0.3) / 0.3).powi(2));
                        m.pos.z += h;
                        m.state = if phase < 0.3 { MoveState::Jump } else { MoveState::Fall };
                        m.velocity[2] = ((0.3 - phase) * 13.0 * 1024.0) as i16;
                    }
                    "jump" if phase < 0.7 => m = m.landing(5.0),
                    "fall" if t % 3.0 < 0.5 => {
                        m.pos.z += 3.0 * (1.0 - (t % 3.0) / 0.5);
                        m.state = MoveState::Fall;
                        m.velocity[2] = -(6.0 * 1024.0) as i16;
                    }
                    "fall" if t % 3.0 < 0.6 => m = m.landing(25.0),
                    _ => {}
                }
                client.send(Command::Move(m));
                sent += 1;
            }
            e = client.recv() => {
                let Some(e) = e else { bail!("the session ended after {:.1} s ({sent} moves sent)", started.elapsed().as_secs_f32()) };
                state.apply(&e);
                events += 1;
                match e {
                    Event::Disconnected { message } => bail!(
                        "{} disconnected after {:.1} s of walking, {sent} moves sent: {message}",
                        stamp(),
                        started.elapsed().as_secs_f32()
                    ),
                    Event::Kicked { reason, message } => bail!("kicked after {:.1} s: ({reason}) {message}", started.elapsed().as_secs_f32()),
                    Event::Teleported { pos, .. } => println!("  teleported to ({:.1}, {:.1}, {:.1}) after {:.1} s", pos.x, pos.y, pos.z, started.elapsed().as_secs_f32()),
                    _ => {}
                }
            }
        }
        let s = started.elapsed().as_secs();
        if s >= last_report + 10 {
            last_report = s;
            println!("  {s:>4} s: {sent} moves sent, {events} events received");
        }
    }
    println!("  walked {secs} s: {sent} moves sent, {events} events received, connection still up");
    Ok(())
}

/// Character select, world select and back into the game.
async fn roundtrip(client: &mut Client, state: &mut WorldState, world: u8, character: u32) -> Result<()> {
    let step = Duration::from_secs(30);
    println!("== roundtrip: back to the character list");
    client.send(Command::LeaveWorld(LeaveTarget::CharacterSelect));
    until(client, state, "leaving to the character list", step, |e| {
        matches!(e, Event::LeftWorld { target: 1 }).then_some(())
    })
    .await?;
    let list = until(client, state, "the character list", step, |e| match e {
        Event::CharacterList(l) => Some(l.len()),
        _ => None,
    })
    .await?;
    println!("  character list again: {list} character(s); units cleared: {}", state.units.by_id.is_empty());
    println!("== roundtrip: back to the world list");
    client.send(Command::ReturnToWorldSelect);
    let worlds = until(client, state, "the world list", step, |e| match e {
        Event::WorldList(l) => Some(l.clone()),
        _ => None,
    })
    .await?;
    println!("  {} world(s), {} character(s) on {}", worlds.worlds.len(), worlds.character_count(world), world);
    client.send(Command::EnterWorld { world_id: world });
    until(client, state, "the world cookie", step, |e| matches!(e, Event::ReadyToConnect { .. }).then_some(())).await?;
    client.send(Command::ConnectToWorld);
    until(client, state, "the character list", step, |e| matches!(e, Event::CharacterList(_)).then_some(())).await?;
    client.send(Command::EnterGame { character_id: character });
    let spawn = until(client, state, "the player spawn", step, |e| match e {
        Event::PlayerSpawn(s) => Some((**s).clone()),
        _ => None,
    })
    .await?;
    until(client, state, "entered world", step, |e| matches!(e, Event::EnteredWorld).then_some(())).await?;
    let p = spawn.position();
    println!("  back in the world at ({:.1}, {:.1}, {:.1})", p.x, p.y, p.z);
    Ok(())
}

/// Create a throw-away character, delete it, and report the server's answers.
async fn delete_test(client: &mut Client, state: &mut WorldState) -> Result<()> {
    let step = Duration::from_secs(20);
    println!("== delete test");
    client.send(Command::CreateCharacter(CreateCharacter::new("Bevytmp", race::DWARF, gender::FEMALE)));
    let id = until(client, state, "the temporary character", step, |e| match e {
        Event::CharacterCreated(c) => Some(c.id),
        _ => None,
    })
    .await?;
    until(client, state, "the refreshed list", step, |e| matches!(e, Event::CharacterList(_)).then_some(())).await?;
    println!("  created character {id}; list has {}", state.characters.list.len());
    client.send(Command::DeleteCharacter { id });
    let (status, requested, delete_time) = until(client, state, "the delete response", step, |e| match e {
        Event::CharacterDeleteResponse { status, delete_request_time, delete_time, .. } => {
            Some((*status, *delete_request_time, *delete_time))
        }
        _ => None,
    })
    .await?;
    println!("  delete response: status {status}, requested {requested}, deletes at {delete_time}");
    let list = until(client, state, "the list after the delete", step, |e| match e {
        Event::CharacterList(l) => Some(l.clone()),
        _ => None,
    })
    .await?;
    match list.iter().find(|c| c.id == id) {
        Some(c) => println!("  still listed, delete requested: {}", c.delete_requested()),
        None => println!("  no longer listed"),
    }
    Ok(())
}
