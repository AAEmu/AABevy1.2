//! Functional skill test against the lane server: logs in a GM character, learns the skills of an ability tree, casts them
//! one by one at a training doll and records every packet that comes back (one JSON line per skill).
//!
//! ```text
//! MSYS_NO_PATHCONV=1 cargo run -p aa-net --example skilltest -- --user bevyskilltest --character Skilltestr \
//!     [--trees 1,2,...] [--skills ID,ID] [--defaults] [--out data/extract/skilltest/run.jsonl] [--keep-cooldowns]
//! ```
//!
//! * `--trees N,...`  for each ability tree: swap it in, `/level`, learn its 11 shown skills, cast each.
//! * `--skills ID,..` cast these skills (learned first when they belong to a tree: the tree is swapped in).
//! * `--defaults`     the default skills (melee attack, Shoot Arrow, Return, kick, sprint, instruments, dance).
//! * `--keep-cooldowns` leave the server's cooldowns on (default: `/ignoreskillcds true`), the same skill is cast twice.
//!
//! GM rights (`access_level` 100 of the account) are needed for `/level`, `/spawn` and `/ignoreskillcds`.
//! Names, target types and the effect kinds each skill is expected to have come from the server's `compact.sqlite3`.

#![allow(dead_code, clippy::collapsible_if)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::time::{Duration, Instant};

use aa_net::model::{gender, race, UnitKind};
use aa_net::{Client, ClientOptions, Command, CreateCharacter, Event, MoveCommand, Pos, WorldState};
use anyhow::{anyhow, bail, Context, Result};

const DUMMY_TEMPLATE: u32 = 13013;
const DEFAULT_SKILLS: &[u32] = &[2, 14495, 16064, 16287, 14900, 14901, 14902, 14152, 15229, 14451, 10594];

struct Args {
    login: String,
    user: String,
    character: String,
    trees: Vec<u8>,
    skills: Vec<u32>,
    defaults: bool,
    out: String,
    level: u8,
    keep_cooldowns: bool,
    target: String,
    window_ms: u64,
    npc: u32,
    at: Option<Pos>,
    repeat: bool,
    no_learn: bool,
    corpse: bool,
    learn_only: bool,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        login: aa_net::default_login(),
        user: "bevyskilltest".into(),
        character: "Skilltestr".into(),
        trees: vec![],
        skills: vec![],
        defaults: false,
        out: "data/extract/skilltest/run.jsonl".into(),
        level: 50,
        keep_cooldowns: false,
        target: "dummy".into(),
        window_ms: 0,
        npc: DUMMY_TEMPLATE,
        at: None,
        repeat: false,
        no_learn: false,
        corpse: false,
        learn_only: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--login" => a.login = value()?,
            "--user" => a.user = value()?,
            "--character" => a.character = value()?,
            "--trees" => a.trees = value()?.split(',').map(|s| s.parse()).collect::<Result<_, _>>()?,
            "--skills" => a.skills = value()?.split(',').map(|s| s.parse()).collect::<Result<_, _>>()?,
            "--defaults" => a.defaults = true,
            "--out" => a.out = value()?,
            "--level" => a.level = value()?.parse()?,
            "--keep-cooldowns" => a.keep_cooldowns = true,
            "--target" => a.target = value()?,
            "--window-ms" => a.window_ms = value()?.parse()?,
            "--npc" => a.npc = value()?.parse()?,
            "--repeat" => a.repeat = true,
            "--no-learn" => a.no_learn = true,
            "--learn-only" => a.learn_only = true,
            "--corpse" => a.corpse = true,
            "--at" => {
                let v: Vec<f32> = value()?.split(',').map(|x| x.parse()).collect::<Result<_, _>>()?;
                a.at = Some(Pos::new(v[0], v[1], v[2]));
            }
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

fn password(user: &str) -> String {
    match std::env::var("AA_PASSWORD_FILE").and_then(|p| std::fs::read_to_string(p).map_err(|_| std::env::VarError::NotPresent)) {
        Ok(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => user.to_string(),
    }
}

// ---- the database -----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct SkillInfo {
    id: u32,
    name: String,
    ability: u8,
    ability_level: i64,
    target_type: i64,
    target_selection: i64,
    target_relation: i64,
    cast_ms: i64,
    chan_ms: i64,
    cooldown_ms: i64,
    cooldown_tag: i64,
    mana: i64,
    min_range: i64,
    max_range: i64,
    plot: bool,
    /// Effect kinds the skill (and its plot) applies: Damage, Heal, Buff, ...
    effects: Vec<String>,
    buffs: Vec<i64>,
}

fn english_names() -> BTreeMap<u32, String> {
    let mut m = BTreeMap::new();
    for path in ["data/extract/combat/skill_coverage.tsv", "data/extract/aa-client-combat/skill_coverage.tsv"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines().skip(1) {
                let mut f = line.split('\t');
                if let (Some(id), Some(name)) = (f.next(), f.next()) {
                    if let Ok(id) = id.parse() {
                        m.entry(id).or_insert_with(|| name.to_string());
                    }
                }
            }
        }
    }
    m
}

fn load_skill(db: &rusqlite::Connection, names: &BTreeMap<u32, String>, id: u32) -> Result<SkillInfo> {
    let mut s = db
        .query_row(
            "SELECT name, ability_id, ability_level, target_type_id, target_selection_id, target_relation_id, casting_time, channeling_time, \
             cooldown_time, mana_cost, min_range, max_range, plot_id, cooldown_tag_id FROM skills WHERE id = ?1",
            [id],
            |r| {
                Ok(SkillInfo {
                    id,
                    name: r.get::<_, String>(0)?,
                    ability: r.get::<_, Option<i64>>(1)?.unwrap_or(0) as u8,
                    ability_level: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    target_type: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    target_selection: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    target_relation: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    cast_ms: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    chan_ms: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    cooldown_ms: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                    mana: r.get::<_, Option<i64>>(9)?.unwrap_or(0),
                    min_range: r.get::<_, Option<i64>>(10)?.unwrap_or(0),
                    max_range: r.get::<_, Option<i64>>(11)?.unwrap_or(0),
                    plot: r.get::<_, Option<i64>>(12)?.is_some(),
                    cooldown_tag: r.get::<_, Option<i64>>(13)?.unwrap_or(0),
                    ..Default::default()
                })
            },
        )
        .with_context(|| format!("skill {id} is not in the database"))?;
    if let Some(n) = names.get(&id) {
        s.name = n.clone();
    }
    // Effects: the skill's own and those of its plot steps.
    let mut kinds: Vec<String> = Vec::new();
    let mut buffs: Vec<i64> = Vec::new();
    let add = |actual_id: i64, actual_type: &str, kinds: &mut Vec<String>, buffs: &mut Vec<i64>| {
        if actual_type == "BuffEffect" {
            if let Ok(b) = db.query_row("SELECT buff_id FROM buff_effects WHERE id = ?1", [actual_id], |r| r.get::<_, i64>(0)) {
                if !buffs.contains(&b) {
                    buffs.push(b);
                }
            }
        }
        let k = actual_type.trim_end_matches("Effect").to_string();
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    };
    let mut q = db.prepare("SELECT e.actual_id, e.actual_type FROM skill_effects se JOIN effects e ON e.id = se.effect_id WHERE se.skill_id = ?1")?;
    for row in q.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (a, t) = row?;
        add(a, &t, &mut kinds, &mut buffs);
    }
    let mut q = db.prepare(
        "SELECT e.actual_id, e.actual_type FROM skills s JOIN plot_events pe ON pe.plot_id = s.plot_id \
         JOIN plot_effects pf ON pf.event_id = pe.id JOIN effects e ON e.id = pf.actual_id AND pf.actual_type = 'Effect' WHERE s.id = ?1",
    )?;
    for row in q.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (a, t) = row?;
        add(a, &t, &mut kinds, &mut buffs);
    }
    kinds.retain(|k| k != "Special" || true);
    s.effects = kinds;
    s.buffs = buffs;
    Ok(s)
}

fn tree_skills(db: &rusqlite::Connection, ability: u8) -> Result<Vec<u32>> {
    let mut q = db.prepare("SELECT id FROM skills WHERE ability_id = ?1 AND SHOW = 't' ORDER BY ability_level, id")?;
    let v = q.query_map([ability], |r| r.get::<_, u32>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

// ---- recording --------------------------------------------------------------------------------------------------------

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn short(e: &Event) -> String {
    let line = format!("{e:?}");
    if line.len() > 260 {
        format!("{}...", &line[..line.char_indices().nth(260).map_or(line.len(), |(i, _)| i)])
    } else {
        line
    }
}

fn is_noise(e: &Event) -> bool {
    matches!(
        e,
        Event::UnitMoved { .. }
            | Event::UnitSpawned(_)
            | Event::UnitPosture { .. }
            | Event::ClockSample(_)
            | Event::Pong(_)
            | Event::TimeOfDay { .. }
            | Event::DoodadsSpawned(_)
            | Event::TargetChanged { .. }
            | Event::Chat(_)
    )
}

/// What one cast produced.
#[derive(Default)]
struct Trace {
    sent_at: Option<Instant>,
    lines: Vec<String>,
    started: Vec<(u32, u16, u8, u32, u32)>, // skill, tl, result, cast_ms, caster
    fired: Vec<(u32, u16, i32, i32, u32)>,  // skill, tl, delay, chan, fire anim
    plot_events: Vec<(u32, u32)>,           // skill, event
    ended: usize,
    stopped: usize,
    damage: Vec<(u32, u32, i32, u8, u8)>, // target, skill, amount, hit type, action kind
    heals: Vec<(u32, u32, i32)>,
    buffs_created: Vec<(u32, u32)>, // obj, buff id
    buffs_removed: usize,
    cooldown_events: usize,
    resets: usize,
    unhandled: Vec<(u8, u16, usize)>,
    decode_failed: Vec<String>,
    server_errors: Vec<i16>,
    deaths: usize,
    other: BTreeMap<String, usize>,
    first_ms: Option<u128>,
    last_ms: u128,
}

fn variant(e: &Event) -> String {
    let line = format!("{e:?}");
    line.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
}

impl Trace {
    fn record(&mut self, e: &Event, own: u32) {
        let ms = self.sent_at.map_or(0, |t| t.elapsed().as_millis());
        if is_noise(e) {
            return;
        }
        self.first_ms.get_or_insert(ms);
        self.last_ms = ms;
        match e {
            Event::SkillStarted(s) => self.started.push((s.skill_id, s.tl, s.result, s.cast_ms, s.caster.obj_id)),
            Event::SkillFired(f) => self.fired.push((f.skill_id, f.tl, f.delay_ms, f.channeling_ms, f.fire_anim_id)),
            Event::PlotEvent(p) => self.plot_events.push((p.skill_id, p.event_id)),
            Event::SkillEnded { .. } | Event::PlotEnded { .. } => self.ended += 1,
            Event::SkillStopped { .. } | Event::CastingStopped { .. } | Event::PlotStopped { .. } => self.stopped += 1,
            Event::UnitDamaged(d) => self.damage.push((d.target_obj_id, d.action.skill_id, d.damage, d.hit_type, d.action.kind)),
            Event::UnitHealed(h) => self.heals.push((h.target_obj_id, h.action.skill_id, h.value)),
            Event::BuffCreated { obj_id, buff } => self.buffs_created.push((*obj_id, buff.buff_id)),
            Event::BuffRemoved { .. } => self.buffs_removed += 1,
            Event::Cooldowns(_) => self.cooldown_events += 1,
            Event::SkillCooldownReset { .. } => self.resets += 1,
            Event::UnhandledGamePacket { level, opcode, len } => {
                self.unhandled.push((*level, *opcode, *len));
                self.lines.push(format!("+{ms} UNHANDLED level {level} opcode {opcode:#05x} {} len {len}", aa_net::opcodes::sc::name(*opcode)));
                return;
            }
            Event::GameDecodeFailed { message } => {
                self.decode_failed.push(message.clone());
                self.lines.push(format!("+{ms} DECODE FAILED {message}"));
                return;
            }
            Event::ServerError { code } => self.server_errors.push(*code),
            Event::UnitDeath { .. } => self.deaths += 1,
            _ => {}
        }
        let _ = own;
        let n = self.other.entry(variant(e)).or_default();
        *n += 1;
        if matches!(e, Event::UnitPoints { obj_id, .. } if *obj_id == own) {
            return;
        }
        if *n <= 14 && self.lines.len() < 160 {
            self.lines.push(format!("+{ms} {}", short(e)));
        }
    }
}

// ---- the session ------------------------------------------------------------------------------------------------------

struct Session {
    client: Client,
    state: WorldState,
    own: u32,
    here: Pos,
    yaw: f32,
    log: Vec<String>,
    npc: u32,
    /// Keep a dead dummy as the target (Revive).
    corpse: bool,
}

impl Session {
    /// Folds events for `dur`; `trace` records them. Ends the program on a lost connection.
    async fn pump(&mut self, dur: Duration, mut trace: Option<&mut Trace>) -> Result<()> {
        let end = Instant::now() + dur;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            match tokio::time::timeout(left, self.client.recv()).await {
                Err(_) => return Ok(()),
                Ok(None) => bail!("the session ended"),
                Ok(Some(e)) => {
                    self.state.apply(&e);
                    if let Event::Disconnected { message } = &e {
                        bail!("disconnected: {message}");
                    }
                    if let Event::Chat(c) = &e {
                        if c.sender_obj_id != self.own || c.message.starts_with("[") {
                            println!("  chat: {}", c.message);
                        }
                    }
                    if let Some(t) = trace.as_deref_mut() {
                        t.record(&e, self.own);
                    }
                }
            }
        }
    }

    async fn gm(&mut self, text: &str) -> Result<()> {
        println!("  gm: {text}");
        self.client.send(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message: text.into() });
        self.pump(Duration::from_millis(900), None).await
    }

    async fn face(&mut self, towards: Pos) -> Result<()> {
        let (dx, dy) = (towards.x - self.here.x, towards.y - self.here.y);
        if dx.abs() + dy.abs() > 0.01 {
            self.yaw = (-dx).atan2(dy);
        }
        for _ in 0..2 {
            self.client.send(Command::Move(MoveCommand::stopped(self.here, self.yaw)));
            self.pump(Duration::from_millis(120), None).await?;
        }
        Ok(())
    }

    fn dummy(&self) -> Option<(u32, Pos)> {
        self.state
            .units
            .iter()
            .filter(|u| matches!(&u.kind, UnitKind::Npc { template_id, .. } if *template_id == self.npc) && (self.corpse || !u.dead))
            .min_by(|a, b| a.pos.distance(self.here).total_cmp(&b.pos.distance(self.here)))
            .filter(|u| u.pos.distance(self.here) < 4.5)
            .map(|u| (u.obj_id, u.pos))
    }

    async fn ensure_dummy(&mut self) -> Result<(u32, Pos)> {
        if self.npc == 0 {
            // `--npc 0`: the nearest living monster of faction 115 in view; the caster walks up to it.
            let mob = self
                .state
                .units
                .iter()
                .filter(|u| u.faction_id == 115 && !u.dead && u.hp > 0)
                .min_by(|a, b| a.pos.distance(self.here).total_cmp(&b.pos.distance(self.here)))
                .map(|u| (u.obj_id, u.pos, u.name.clone()));
            let (id, pos, _) = mob.context("no hostile monster in view")?;
            if pos.distance(self.here) > 3.5 {
                let near = Pos::new(pos.x - 2.0, pos.y, pos.z);
                for _ in 0..3 {
                    self.client.send(Command::Move(MoveCommand::new(near, 0.0)));
                    self.pump(Duration::from_millis(200), None).await?;
                }
                self.here = near;
            }
            return Ok((id, pos));
        }
        if let Some(d) = self.dummy() {
            return Ok(d);
        }
        self.gm(&format!("/spawn npc {}", self.npc)).await?;
        self.pump(Duration::from_millis(2500), None).await?;
        self.dummy().context("the training dummy did not spawn (GM rights?)")
    }

    async fn refill(&mut self) -> Result<()> {
        let name = self.state.stats.name.clone();
        self.client.send(Command::Target { obj_id: 0 });
        self.pump(Duration::from_millis(150), None).await?;
        self.client.send(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message: format!("/heal {name}") });
        self.pump(Duration::from_millis(350), None).await
    }

    async fn swap_in(&mut self, tree: u8) -> Result<()> {
        // Give the skill points of every other tree back: a level 50 character cannot learn three full trees.
        let actives = self.state.stats.active_abilities;
        for a in actives {
            if a != tree && a != 11 {
                self.client.send(Command::ResetSkills { ability: a });
                self.pump(Duration::from_millis(300), None).await?;
            }
        }
        if self.state.stats.active_abilities.contains(&tree) {
            return Ok(());
        }
        let active = self.state.stats.active_abilities;
        let old = active.iter().copied().find(|&a| a == 11).unwrap_or(active[0]);
        println!("  swap ability {old} -> {tree}");
        self.client.send(Command::SwapAbility { old, ability: tree });
        self.pump(Duration::from_millis(1200), None).await?;
        if !self.state.stats.active_abilities.contains(&tree) {
            bail!("ability {tree} did not become active ({:?})", self.state.stats.active_abilities);
        }
        Ok(())
    }

    async fn learn(&mut self, ids: &[u32]) -> Result<Vec<u32>> {
        let mut learned = vec![];
        for &id in ids {
            if self.state.skills.knows(id) {
                learned.push(id);
                continue;
            }
            self.client.send(Command::LearnSkill { skill_id: id });
            self.pump(Duration::from_millis(250), None).await?;
            if self.state.skills.knows(id) {
                learned.push(id);
            } else {
                println!("  skill {id} was not learned");
            }
        }
        Ok(learned)
    }

    /// Whether a (non noise) event is a sign the cast is still going.
    fn casting(&self) -> bool {
        self.state.skills.casting_of(self.own).is_some()
    }

    async fn cast(&mut self, info: &SkillInfo, target_override: Option<&str>) -> Result<Trace> {
        let (dummy, dpos) = self.ensure_dummy().await?;
        self.refill().await?;
        let ground_type = matches!(info.target_type, 6 | 11 | 15 | 18);
        let mode = match (target_override, info.target_type) {
            (_, _) if ground_type => "ground".to_string(),
            (Some("selected"), _) => "dummy".to_string(),
            (Some(m), _) => m.to_string(),
            (None, 0 | 1 | 5) => "self".to_string(),
            (None, _) => "dummy".to_string(),
        };
        // The selected target is the dummy (as a player would have it), except for plain self skills.
        self.client.send(Command::Target { obj_id: dummy });
        self.pump(Duration::from_millis(200), None).await?;
        self.face(dpos).await?;
        let mut trace;
        let mut tries = 0;
        let mut notes: Vec<String> = Vec::new();
        let cmd = loop {
            let cmd = match mode.as_str() {
                "self" => Command::UseSkill { skill_id: info.id, target_obj_id: 0 },
                "ground" => {
                    // A spot in front of the caster at the dummy's side; other spots when the server calls it an obstacle.
                    const SPOTS: [(f32, f32); 6] = [(0.0, 7.0), (0.5, 8.0), (-0.5, 8.0), (1.0, 6.0), (-1.0, 6.0), (std::f32::consts::PI, 8.0)];
                    let (da, dist) = SPOTS[(tries as usize).min(SPOTS.len() - 1)];
                    let dist = if info.min_range > 0 { dist.max(info.min_range as f32 + 2.0) } else { dist };
                    let a = self.yaw + da;
                    Command::UseSkillAt { skill_id: info.id, pos: Pos::new(self.here.x - a.sin() * dist, self.here.y + a.cos() * dist, self.here.z) }
                }
                _ => Command::UseSkill { skill_id: info.id, target_obj_id: dummy },
            };
            trace = Trace { sent_at: Some(Instant::now()), ..Trace::default() };
            self.client.send(cmd.clone());
            let window = if self.state_window_ms(info) > 0 { self.state_window_ms(info) } else { 1500 };
            let deadline = Instant::now() + Duration::from_millis(window);
            let mut quiet_since = Instant::now();
            let mut seen = 0usize;
            loop {
                self.pump(Duration::from_millis(150), Some(&mut trace)).await?;
                let n = trace.lines.len() + trace.other.values().sum::<usize>();
                if n != seen {
                    seen = n;
                    quiet_since = Instant::now();
                }
                let waited = trace.sent_at.unwrap().elapsed();
                if Instant::now() >= deadline {
                    break;
                }
                if waited > Duration::from_millis(900) && quiet_since.elapsed() > Duration::from_millis(1400) && !self.casting() {
                    break;
                }
            }
            // A refusal that is the test's fault is retried the way the client does.
            let refusal = trace.started.iter().find(|s| s.4 == self.own || s.1 == 0).map(|s| s.2).unwrap_or(0);
            tries += 1;
            if refusal == aa_net::skill::result::OUTOF_ANGLE && tries < 3 {
                self.face(dpos).await?;
                notes.push("(driver: turned to the target and asked again)".to_string());
                continue;
            }
            if matches!(refusal, aa_net::skill::result::OBSTACLE | aa_net::skill::result::INVALID_LOCATION | aa_net::skill::result::TOO_CLOSE_RANGE | aa_net::skill::result::TOO_FAR_RANGE)
                && mode == "ground"
                && tries < 6
            {
                notes.push(format!("(driver: spot refused with {}; tried another spot)", aa_net::skill::result::name(refusal)));
                continue;
            }
            if refusal == aa_net::skill::result::LACK_MANA && tries < 3 {
                notes.push("(driver: refilled mana and asked again)".to_string());
                self.refill().await?;
                continue;
            }
            if refusal == aa_net::skill::result::CANNOT_CAST_IN_STUN && tries < 3 {
                notes.push("(driver: stunned by an earlier skill; waited 5 s and asked again)".to_string());
                self.pump(Duration::from_secs(5), None).await?;
                continue;
            }
            if refusal == aa_net::skill::result::CANNOT_CAST_IN_COMBAT && tries < 4 {
                notes.push("(driver: in combat; waited 6 s and asked again)".to_string());
                self.client.send(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message: "/clearcombat".into() });
                self.pump(Duration::from_secs(6), None).await?;
                continue;
            }
            break cmd;
        };
        // Cancel what is left of a channel so the next skill starts clean.
        if let Some(c) = self.state.skills.casting_of(self.own).map(|c| (c.tl, c.skill_id)) {
            trace.lines.push(format!("(driver: still casting tl {} skill {} after the window; sent CSStopCasting)", c.0, c.1));
            trace.other.insert("DRIVER_STILL_CASTING".into(), 1);
            self.client.send(if info.plot { Command::StopCasting { tl: 0, plot_tl: c.0 } } else { Command::StopCasting { tl: c.0, plot_tl: 0 } });
            self.pump(Duration::from_millis(800), Some(&mut trace)).await?;
        }
        trace.lines.insert(0, format!("sent {cmd:?} (mode {mode}){}", if notes.is_empty() { String::new() } else { format!(" {}", notes.join(" ")) }));
        // Drop the target: that ends the auto attack the plots start.
        self.client.send(Command::Target { obj_id: 0 });
        self.pump(Duration::from_millis(1300), None).await?;
        Ok(trace)
    }

    fn state_window_ms(&self, info: &SkillInfo) -> u64 {
        (info.cast_ms + info.chan_ms.clamp(0, 6000)).max(0) as u64 + 5000
    }
}

fn json_record(info: &SkillInfo, phase: &str, t: &Trace, s: &Session, learned: bool, dummy: u32) -> String {
    let mut o = String::new();
    let refused: Vec<u8> = t.started.iter().filter(|x| x.2 != 0).map(|x| x.2).collect();
    let _ = write!(
        o,
        "{{\"skill\":{},\"name\":{},\"phase\":{},\"tree\":{},\"learned\":{},\"target_type\":{},\"cast_ms\":{},\"chan_ms\":{},\"cooldown_ms\":{},\"plot\":{},\"effects\":[{}],\"buffs\":{:?},",
        info.id,
        esc(&info.name),
        esc(phase),
        info.ability,
        learned,
        info.target_type,
        info.cast_ms,
        info.chan_ms,
        info.cooldown_ms,
        info.plot,
        info.effects.iter().map(|e| esc(e)).collect::<Vec<_>>().join(","),
        info.buffs
    );
    let _ = write!(
        o,
        "\"started\":{},\"refused\":{:?},\"fired\":{},\"plot_events\":{},\"ended\":{},\"stopped\":{},",
        t.started.iter().filter(|x| x.2 == 0).count(),
        refused,
        t.fired.len(),
        t.plot_events.len(),
        t.ended,
        t.stopped
    );
    let own_dmg = t.damage.iter().filter(|d| d.0 == dummy).count();
    let total: i64 = t.damage.iter().map(|d| i64::from(d.2)).sum();
    let _ = write!(
        o,
        "\"damage_n\":{},\"damage_on_target\":{},\"damage_total\":{},\"heal_n\":{},\"heal_total\":{},\"buffs_created\":[{}],\"buffs_removed\":{},\"cooldown_events\":{},\"resets\":{},",
        t.damage.len(),
        own_dmg,
        total,
        t.heals.len(),
        t.heals.iter().map(|h| i64::from(h.2)).sum::<i64>(),
        t.buffs_created.iter().map(|b| format!("[{},{}]", b.0, b.1)).collect::<Vec<_>>().join(","),
        t.buffs_removed,
        t.cooldown_events,
        t.resets
    );
    let cd_left = s
        .state
        .skills
        .cooldown_with_total(info.id, info.cooldown_tag as u32, info.cooldown_ms.max(0) as u32)
        .map_or(-1, |(left, _)| left.as_millis() as i64);
    let _ = write!(o, "\"cd_left_ms\":{cd_left},");
    let casting_left = s.state.skills.casting_of(s.own).is_some();
    let used = s.state.skills.last_used.contains_key(&info.id);
    let _ = write!(
        o,
        "\"unhandled\":[{}],\"decode_failed\":[{}],\"server_errors\":{:?},\"deaths\":{},\"casting_left\":{},\"last_used_set\":{},\"first_ms\":{},\"last_ms\":{},\"counts\":{{{}}},",
        t.unhandled.iter().map(|u| format!("[{},{},{}]", u.0, u.1, u.2)).collect::<Vec<_>>().join(","),
        t.decode_failed.iter().map(|m| esc(m)).collect::<Vec<_>>().join(","),
        t.server_errors,
        t.deaths,
        casting_left,
        used,
        t.first_ms.map_or(-1, |v| v as i64),
        t.last_ms,
        t.other.iter().map(|(k, v)| format!("{}:{v}", esc(k))).collect::<Vec<_>>().join(",")
    );
    let _ = write!(o, "\"events\":[{}]}}", t.lines.iter().map(|l| esc(l)).collect::<Vec<_>>().join(","));
    o
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")))
        .init();
    let args = parse_args()?;
    let db = rusqlite::Connection::open_with_flags(std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into()), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let names = english_names();
    if let Some(dir) = std::path::Path::new(&args.out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&args.out)?;

    let options = ClientOptions { ping_interval: Some(Duration::from_millis(3000)), first_ping_delay: Duration::ZERO, item_recharge_catalog: None };
    let mut client = Client::spawn_on_with(&tokio::runtime::Handle::current(), options);
    let mut state = WorldState::default();
    macro_rules! until {
        ($what:expr, $secs:expr, $pick:expr) => {{
            let deadline = Instant::now() + Duration::from_secs($secs);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let e = tokio::time::timeout(left, client.recv())
                    .await
                    .map_err(|_| anyhow!("timed out waiting for {}", $what))?
                    .ok_or_else(|| anyhow!("the session ended waiting for {}", $what))?;
                state.apply(&e);
                match &e {
                    Event::LoginFailed { message } | Event::ConnectFailed { message } | Event::Disconnected { message } => bail!("{}: {message}", $what),
                    Event::RequestTimedOut { what: w } => bail!("{}: timed out waiting for {w}", $what),
                    Event::EnterWorldDenied { reason } => bail!("{}: enter world denied ({reason})", $what),
                    _ => {}
                }
                #[allow(clippy::redundant_closure_call)]
                if let Some(v) = ($pick)(&e) {
                    break v;
                }
            }
        }};
    }
    println!("== login as {}", args.user);
    client.send(Command::Login { addr: args.login.clone(), user: args.user.clone(), password: password(&args.user).into() });
    until!("login", 20, |e: &Event| matches!(e, Event::LoginSucceeded(_)).then_some(()));
    let worlds = until!("the world list", 20, |e: &Event| match e {
        Event::WorldList(l) => Some(l.clone()),
        _ => None,
    });
    let world = worlds.worlds.iter().find(|w| w.online).context("no online world")?.id;
    client.send(Command::EnterWorld { world_id: world });
    until!("the world cookie", 120, |e: &Event| matches!(e, Event::ReadyToConnect { .. }).then_some(()));
    client.send(Command::ConnectToWorld);
    let mut characters = until!("the character list", 20, |e: &Event| match e {
        Event::CharacterList(l) => Some(l.clone()),
        _ => None,
    });
    if characters.is_empty() {
        client.send(Command::CreateCharacter(CreateCharacter::new(args.character.clone(), race::NUIAN, gender::MALE)));
        until!("the new character", 20, |e: &Event| matches!(e, Event::CharacterCreated(_)).then_some(()));
        characters = until!("the refreshed list", 20, |e: &Event| match e {
            Event::CharacterList(l) => Some(l.clone()),
            _ => None,
        });
    }
    let chosen = characters.iter().find(|c| c.name == args.character).or_else(|| characters.first()).context("no character")?.clone();
    println!("== entering the game as {}", chosen.name);
    client.send(Command::EnterGame { character_id: chosen.id });
    let spawn = until!("the player spawn", 20, |e: &Event| match e {
        Event::PlayerSpawn(s) => Some((**s).clone()),
        _ => None,
    });
    until!("entered world", 20, |e: &Event| matches!(e, Event::EnteredWorld).then_some(()));
    let mut s = Session { client, state, own: spawn.obj_id, here: spawn.position(), yaw: 0.0, log: vec![], npc: args.npc, corpse: args.corpse };
    s.pump(Duration::from_secs(3), None).await?;
    println!("  obj {} at ({:.1}, {:.1}, {:.1}) level {}", s.own, s.here.x, s.here.y, s.here.z, s.state.stats.level);

    // ---- GM setup
    if let Some(at) = args.at {
        for _ in 0..4 {
            s.client.send(Command::Move(MoveCommand::stopped(at, 0.0)));
            s.pump(Duration::from_millis(250), None).await?;
        }
        s.here = at;
        s.pump(Duration::from_secs(2), None).await?;
        println!("  moved to ({:.1}, {:.1}, {:.1})", at.x, at.y, at.z);
    }
    if s.state.stats.level < args.level {
        s.gm(&format!("/level {}", args.level)).await?;
    }
    if !args.keep_cooldowns {
        s.gm("/ignoreskillcds true").await?;
    }
    if args.corpse {
        let (d, _) = s.ensure_dummy().await?;
        s.client.send(Command::Target { obj_id: d });
        s.pump(Duration::from_millis(300), None).await?;
        s.gm("/kill").await?;
        println!("  killed the dummy {d} (a corpse to cast at)");
    }
    let (dummy, dpos) = s.ensure_dummy().await?;
    println!("  dummy obj {dummy} at ({:.1}, {:.1}) {:.1} m", dpos.x, dpos.y, dpos.distance(s.here));

    // ---- the work list: (phase label, skill ids)
    let mut jobs: Vec<(String, Vec<u32>)> = vec![];
    for &t in &args.trees {
        jobs.push((format!("tree{t}"), tree_skills(&db, t)?));
    }
    if !args.skills.is_empty() {
        jobs.push(("skills".into(), args.skills.clone()));
    }
    if args.defaults {
        jobs.push(("default".into(), DEFAULT_SKILLS.to_vec()));
    }

    for (phase, ids) in jobs {
        println!("== {phase}: {} skills", ids.len());
        let mut infos = vec![];
        for &id in &ids {
            infos.push(load_skill(&db, &names, id)?);
        }
        // Consecutive skills of one tree form a group: the tree is swapped in, its skills learned, then they are cast.
        let mut groups: Vec<Vec<SkillInfo>> = vec![];
        for info in infos {
            match groups.last_mut() {
                Some(g) if g[0].ability == info.ability => g.push(info),
                _ => groups.push(vec![info]),
            }
        }
        for group in groups {
            let tree = group[0].ability;
            if tree > 0 && !args.no_learn {
                s.swap_in(tree).await?;
                s.gm(&format!("/level {}", args.level)).await?;
                println!("  ability {tree} active {:?}, player level {}", s.state.stats.active_abilities, s.state.stats.level);
            }
            let to_learn: Vec<u32> = group.iter().filter(|i| i.ability > 0 && !args.no_learn).map(|i| i.id).collect();
            let mut learned = s.learn(&to_learn).await?;
            learned.extend(group.iter().filter(|i| s.state.skills.knows(i.id)).map(|i| i.id));
            println!("  learned {}/{}", learned.len(), to_learn.len());
            if args.learn_only {
                continue;
            }
            for info in &group {
                let is_learned = info.ability == 0 || learned.contains(&info.id);
                println!("-- {} {} (type {}, cast {} chan {} cd {})", info.id, info.name, info.target_type, info.cast_ms, info.chan_ms, info.cooldown_ms);
                let target = (args.target != "dummy").then_some(args.target.as_str());
                let trace = s.cast(info, target).await?;
                let rec = json_record(info, &phase, &trace, &s, is_learned, dummy);
                writeln!(out, "{rec}")?;
                let refused: Vec<String> = trace.started.iter().filter(|x| x.2 != 0).map(|x| aa_net::skill::result::name(x.2).to_string()).collect();
                println!(
                    "   started {} refused {:?} fired {} plot {} ended {} stopped {} damage {} heal {} buffs+{} unhandled {} decode {}",
                    trace.started.iter().filter(|x| x.2 == 0).count(),
                    refused,
                    trace.fired.len(),
                    trace.plot_events.len(),
                    trace.ended,
                    trace.stopped,
                    trace.damage.len(),
                    trace.heals.len(),
                    trace.buffs_created.len(),
                    trace.unhandled.len(),
                    trace.decode_failed.len()
                );
                if args.repeat && !trace.started.iter().any(|x| x.2 != 0) {
                    // The same skill again at once: the server's cooldown (or the cast in progress) has to refuse it.
                    let again = s.cast(info, target).await?;
                    let rec = json_record(info, &format!("{phase}:repeat"), &again, &s, is_learned, dummy);
                    writeln!(out, "{rec}")?;
                    let refused: Vec<String> = again.started.iter().filter(|x| x.2 != 0).map(|x| aa_net::skill::result::name(x.2).to_string()).collect();
                    println!("   repeat: refused {refused:?} fired {} plot {} cooldown events {}", again.fired.len(), again.plot_events.len(), again.cooldown_events);
                    if info.cooldown_ms > 0 && info.cooldown_ms <= 15_000 {
                        // Once the cooldown is over the skill has to work again.
                        s.pump(Duration::from_millis(info.cooldown_ms as u64 + 800), None).await?;
                        let after = s.cast(info, target).await?;
                        let rec = json_record(info, &format!("{phase}:after-cd"), &after, &s, is_learned, dummy);
                        writeln!(out, "{rec}")?;
                        let refused: Vec<String> = after.started.iter().filter(|x| x.2 != 0).map(|x| aa_net::skill::result::name(x.2).to_string()).collect();
                        println!("   after the cooldown: refused {refused:?} fired {} plot {}", after.fired.len(), after.plot_events.len());
                    }
                }
                if args.window_ms > 0 {
                    s.pump(Duration::from_millis(args.window_ms), None).await?;
                }
            }
        }
    }
    s.gm("/ignoreskillcds false").await?;
    println!("== done");
    s.client.send(Command::LeaveWorld(aa_net::LeaveTarget::Quit));
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        match tokio::time::timeout(Duration::from_millis(500), s.client.recv()).await {
            Ok(Some(Event::LeftWorld { .. })) | Ok(None) => break,
            _ => {}
        }
    }
    Ok(())
}
