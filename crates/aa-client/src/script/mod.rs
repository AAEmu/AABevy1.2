//! `--script file.txt`: a line-based script that drives the client for automated tests (see `docs/aa-client-script.md`).
//!
//! The runner starts once the character is in the world and the HUD is up (`--script` implies `--enter-world`, with
//! `--account`, `--login`, `--password-file`, `--character`). It runs one command at a time; the commands that take
//! time (walking, waiting for an event, a talk) keep it busy until they finish or time out. The run exits non-zero
//! when a command fails or times out, an `expect` fails, a Lua error of the UI scripts is logged (unless
//! `--allow-lua-error TEXT`), a packet fails to decode, a compressed batch loses its tail, the connection drops, or
//! the process panics; it always prints a summary.
//!
//! Scripted world clicks use the mouse's own path: [`ScriptPick`] hands the pick (a unit's object id) to the systems
//! that otherwise ray-cast the cursor (`hud::targeting::click_target`, `hud::dialog::talk_input`), so a headless
//! `interact` does exactly what a right click on the NPC does after the ray hit it.

mod parse;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::num::NonZeroU8;

use aa_net::model::UnitKind;
use aa_net::state::{Chat, Doodads, Inventory, Player, Quests, Target, Units};
use aa_net::{Command, Event, NetCommand, NetEvent};
use bevy::app::AppExit;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use mlua::ObjectLike;

use crate::Screen;
use crate::hud::script_api::{self, HudBridge, ScriptedKeys, ScriptedSession};
use crate::ingame::DevOptions;
use crate::ingame::camera::OrbitCamera;
use crate::ingame::player::{LocalPlayer, Mover};
use parse::{Cmd, Cond, CondKind, Line, Spec};

/// A pick the script made, waiting for the system that consumes the mouse's pick, and the results
/// of the last talk and slave interaction.
#[derive(Resource, Default)]
pub struct ScriptPick {
    left: Option<u32>,
    right: Option<u32>,
    talk: Option<(u32, Option<&'static str>)>,
    interacted: Option<u32>,
}

impl ScriptPick {
    /// The unit a scripted left click picked (select it).
    pub fn take_left(&mut self) -> Option<u32> {
        self.left.take()
    }

    /// The unit a scripted right click picked (talk to it).
    pub fn take_right(&mut self) -> Option<u32> {
        self.right.take()
    }

    /// The talk with `npc` began; `what` is what it does for the player (`None`: nothing, the talk ended at once).
    pub fn talked(&mut self, npc: u32, what: Option<&'static str>) {
        self.talk = Some((npc, what));
    }

    /// The scripted right click reached `hud::slave::interact` for `npc`: a slave
    /// (cart, ship) never starts a talk, so this is its completion.
    pub fn interacted(&mut self, npc: u32) {
        self.interacted = Some(npc);
    }

    /// Whether the slave interaction for `npc` ran; consumed once so a later `interact` on the
    /// same unit needs a new click.
    fn take_interacted(&mut self, npc: u32) -> bool {
        if self.interacted != Some(npc) {
            return false;
        }
        self.interacted = None;
        true
    }
}

/// Registers [`ScriptPick`] always and the runner when `--script` is given.
pub struct ScriptPlugin;

impl Plugin for ScriptPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptPick>();
        let args: Vec<String> = aa_dev::args();
        let Some(file) = crate::ingame::enter::flag(&args, "--script") else { return };
        let source = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(e) => {
                println!("SCRIPT RESULT: ERROR cannot read {file}: {e}");
                std::process::exit(2);
            }
        };
        let lines = match parse::parse(&source, &file) {
            Ok(l) => l,
            Err(e) => {
                println!("SCRIPT RESULT: ERROR {e}");
                std::process::exit(2);
            }
        };
        let wanted = lines
            .iter()
            .filter_map(|l| match &l.cmd {
                Cmd::Await { event, .. } => Some(event.to_ascii_lowercase()),
                Cmd::Expect(Cond { kind: CondKind::Event(e), .. }) | Cmd::WaitFor(Cond { kind: CondKind::Event(e), .. }, _) => Some(e.to_ascii_lowercase()),
                _ => None,
            })
            .collect();
        let num = |name: &str, default: f32| crate::ingame::enter::flag(&args, name).and_then(|v| v.parse().ok()).unwrap_or(default);
        aa_dev::SCRIPTED.store(true, std::sync::atomic::Ordering::Relaxed);
        // a panic is a failed run with a summary line (the default hook still prints the message)
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            println!("SCRIPT RESULT: FAIL panic: {info}");
            default_hook(info);
        }));
        println!("SCRIPT: {file}: {} commands", lines.len());
        app.insert_resource(Runner::new(file, lines, wanted, num("--script-enter-timeout", 180.0), num("--script-timeout", 900.0)))
            .add_systems(Update, run_script)
            .add_systems(Last, unexpected_exit);
    }
}

/// What a busy command is waiting for.
enum Active {
    Wait { until: f32 },
    Hold { until: f32 },
    Walk { goal: Goal, radius: f32, start: f32, deadline: f32, best: f32, best_t: f32 },
    Await { event: String, deadline: f32, must: bool },
    WaitFor { cond: Cond, deadline: f32 },
    Target { obj_id: u32, deadline: f32 },
    Interact { npc: u32, deadline: f32 },
    Cast { skill: u32, deadline: f32, turned: bool },
    Fight { skill: Option<u32>, slot: usize, target: u32, next: f32, deadline: f32, casts: u32, duels: usize },
    Loot { deadline: f32, sent: bool, must: bool },
    Shot { path: String, deadline: f32 },
    Quit { deadline: f32, sent: bool },
}

#[derive(Clone)]
enum Goal {
    Point(Vec2),
    Unit(u32),
}

#[derive(Resource)]
struct Runner {
    file: String,
    lines: Vec<Line>,
    pc: usize,
    active: Option<Active>,
    started: Option<f32>,
    enter_timeout: f32,
    max_total: f32,
    mark: Vec3,
    wanted: Vec<String>,
    queue: Vec<String>,
    seen: Vec<String>,
    problems: Vec<String>,
    done_lines: usize,
    lua_seen: usize,
    unhandled: BTreeMap<(u8, u16), usize>,
    server_errors: BTreeMap<i16, usize>,
    disconnected: Option<String>,
    quitting: bool,
    last_cast: Option<(u32, u8, u32)>,
    /// The last own `SkillFired` or `PlotEvent`: an instant skill (no cast time) never sends `SkillStarted`.
    last_fired: Option<u32>,
    loot_seen: bool,
    quest_started: Vec<u32>,
    target_hp0: BTreeMap<u32, i32>,
    finish: Option<(u8, u32)>,
    t0: web_time::Instant,
    events_total: u64,
}

impl Runner {
    fn new(file: String, lines: Vec<Line>, mut wanted: Vec<String>, enter_timeout: f32, max_total: f32) -> Runner {
        // Multiple await commands subscribe to one stream, not extra copies of
        // every receipt. Each matching packet may satisfy only one queued wait.
        wanted.sort();
        wanted.dedup();
        Runner {
            file,
            lines,
            pc: 0,
            active: None,
            started: None,
            enter_timeout,
            max_total,
            mark: Vec3::ZERO,
            wanted,
            queue: Vec::new(),
            seen: Vec::new(),
            problems: Vec::new(),
            done_lines: 0,
            lua_seen: 0,
            unhandled: BTreeMap::new(),
            server_errors: BTreeMap::new(),
            disconnected: None,
            quitting: false,
            last_cast: None,
            last_fired: None,
            loot_seen: false,
            quest_started: Vec::new(),
            target_hp0: BTreeMap::new(),
            finish: None,
            t0: web_time::Instant::now(),
            events_total: 0,
        }
    }

    fn queue_event(&mut self, event: &Event) {
        if self.wanted.is_empty() { return; }
        let name = event_name(event);
        let hits: Vec<String> = self.wanted.iter().filter(|p| name_matches(&name, p)).cloned().collect();
        for p in hits {
            if !self.seen.contains(&p) { self.seen.push(p.clone()); }
            if self.queue.len() < 1000 { self.queue.push(p); }
        }
    }

    fn fail(&mut self, message: String) {
        println!("SCRIPT FAIL line {}: {message}", self.current_no());
        self.problems.push(format!("line {}: {message}", self.current_no()));
        self.stop(1);
    }

    fn current_no(&self) -> usize {
        self.lines.get(self.pc.saturating_sub(usize::from(self.pc > 0))).map_or(0, |l| l.no)
    }

    /// Ends the run after a few frames (the disconnect goes out first).
    fn stop(&mut self, code: u8) {
        if self.finish.is_none() {
            self.finish = Some((code, 3));
        }
    }
}

/// The event's variant name, `Interaction.SkillList` for a nested one: the text up to the first character that is not a
/// letter, digit or `_` (`(` becomes `.`). Cheap on big events, `Debug` stops being written at the first space.
fn event_name(e: &Event) -> String {
    struct Prefix(String);
    impl std::fmt::Write for Prefix {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for ch in s.chars() {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    self.0.push(ch);
                } else if ch == '(' {
                    self.0.push('.');
                } else {
                    return Err(std::fmt::Error);
                }
            }
            Ok(())
        }
    }
    let mut p = Prefix(String::new());
    let _ = write!(p, "{e:?}");
    p.0.trim_end_matches('.').to_string()
}

fn name_matches(name: &str, pattern: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == pattern || (n.starts_with(pattern) && n.as_bytes().get(pattern.len()) == Some(&b'.'))
}

#[derive(SystemParam)]
struct Ctx<'w, 's> {
    time: Res<'w, Time>,
    screen: Res<'w, State<Screen>>,
    built: Res<'w, crate::hud::HudBuilt>,
    player: Res<'w, Player>,
    units: Res<'w, Units>,
    stats: Option<Res<'w, aa_net::state::PlayerStats>>,
    doodads: Res<'w, Doodads>,
    target: Res<'w, Target>,
    quests: Res<'w, Quests>,
    inventory: Res<'w, Inventory>,
    chat: Res<'w, Chat>,
    skills: Res<'w, aa_net::state::Skills>,
    bridge: NonSend<'w, HudBridge>,
    rt: Option<NonSend<'w, aa_ui::UiRuntime>>,
    dev: ResMut<'w, DevOptions>,
    keys: ResMut<'w, ScriptedKeys>,
    session: ResMut<'w, ScriptedSession>,
    pick: ResMut<'w, ScriptPick>,
    cameras: Query<'w, 's, &'static mut OrbitCamera>,
    movers: Query<'w, 's, &'static mut Mover, With<LocalPlayer>>,
    net: MessageWriter<'w, NetCommand>,
    commands: Commands<'w, 's>,
    offscreen: Option<Res<'w, aa_dev::OffscreenTarget>>,
    exit: MessageWriter<'w, AppExit>,
}

/// A unit or doodad a spec named.
#[derive(Clone, Debug)]
struct Found {
    obj_id: u32,
    template: u32,
    name: String,
    pos: Vec3,
    dead: bool,
}

impl Ctx<'_, '_> {
    fn own(&self) -> u32 {
        self.player.spawn.as_ref().map_or(0, |s| s.obj_id)
    }

    fn me(&self) -> Option<Vec3> {
        self.movers.single().ok().map(|m| m.pos)
    }

    fn resolve(&self, spec: &Spec) -> Option<Found> {
        let own = self.own();
        let me = self.me()?;
        let faction = self.player.spawn.as_ref().map_or(0, |s| s.character().faction_id);
        let found = |u: &aa_net::Unit| Found {
            obj_id: u.obj_id,
            template: u.kind.template_id(),
            name: u.name.clone(),
            pos: Vec3::new(u.pos.x, u.pos.y, u.pos.z),
            dead: u.dead,
        };
        let dist = |u: &&aa_net::Unit| me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
        let nearest = |it: &mut dyn Iterator<Item = &aa_net::Unit>| it.min_by(|a, b| dist(a).total_cmp(&dist(b))).map(found);
        let others = || self.units.iter().filter(|u| u.obj_id != own);
        match spec {
            // the local player is in `Player`, not in `Units`
            Spec::SelfUnit => self.player.spawn.as_ref().map(|s| Found {
                obj_id: s.obj_id,
                template: 0,
                name: s.name().to_string(),
                pos: me,
                dead: self.own_dead(),
            }),
            Spec::Current => self.target.obj_id.and_then(|t| self.units.get(t)).map(found),
            Spec::Id(id) => self.units.get(*id).map(found),
            Spec::Nearest => nearest(&mut others().filter(|u| !u.dead)),
            Spec::Hostile => nearest(&mut others().filter(|u| !u.dead && script_api::relation(&self.bridge, faction, u) == "hostile")),
            Spec::Friendly => nearest(&mut others().filter(|u| {
                !u.dead && matches!(u.kind, UnitKind::Npc { .. }) && script_api::relation(&self.bridge, faction, u) != "hostile"
            })),
            Spec::Corpse => nearest(&mut others().filter(|u| u.dead)),
            Spec::Template(t) => {
                let of = |alive: bool| others().filter(move |u| matches!(u.kind, UnitKind::Npc { template_id, .. } if template_id == *t) && (!alive || !u.dead));
                nearest(&mut of(true)).or_else(|| nearest(&mut of(false)))
            }
            Spec::Name(n) => {
                let n = n.to_ascii_lowercase();
                nearest(&mut others().filter(|u| u.name.to_ascii_lowercase().contains(&n)))
            }
        }
    }

    fn resolve_doodad(&self, spec: &Spec) -> Option<Found> {
        let me = self.me()?;
        let pos = |d: &aa_net::model::Doodad| Vec3::new(d.pos.x, d.pos.y, d.pos.z);
        let make = |d: &aa_net::model::Doodad| Found { obj_id: d.obj_id, template: d.template_id, name: String::new(), pos: pos(d), dead: false };
        let all = self.doodads.by_id.values().filter(|d| match spec {
            Spec::Id(id) => d.obj_id == *id,
            Spec::Template(t) => d.template_id == *t,
            _ => true,
        });
        all.min_by(|a, b| me.distance(pos(a)).total_cmp(&me.distance(pos(b)))).map(make)
    }

    fn describe(&self, f: &Found) -> String {
        format!("#{} '{}' (template {}) {:.1} m away{}", f.obj_id, f.name, f.template, self.me().map_or(0.0, |m| m.distance(f.pos)), if f.dead { ", dead" } else { "" })
    }

    /// `(hp, max_hp)`; the local player's from `PlayerStats` (what the HUD shows), the spawn's until it has them.
    fn unit_hp(&self, obj_id: u32) -> Option<(i32, i32)> {
        if obj_id != 0 && obj_id == self.own() {
            return match self.own_stats() {
                Some(s) => Some((s.hp, s.max_hp)),
                None => self.player.spawn.as_ref().map(|s| (s.character().hp, s.character().hp)),
            };
        }
        self.units.get(obj_id).map(|u| (u.hp, u.max_hp))
    }

    fn own_stats(&self) -> Option<&aa_net::state::PlayerStats> {
        self.stats.as_deref().filter(|s| s.obj_id != 0 && s.obj_id == self.own())
    }

    fn own_dead(&self) -> bool {
        self.own_stats().is_some_and(|s| s.dead)
    }
}

fn truthy(v: &mlua::Value) -> bool {
    !matches!(v, mlua::Value::Nil | mlua::Value::Boolean(false))
}

/// The condition now, and a short text of what was seen (for the failure message).
fn eval(r: &Runner, c: &Cond, ctx: &Ctx) -> (bool, String) {
    let own = ctx.own();
    let target = ctx.target.obj_id.and_then(|t| ctx.units.get(t));
    let facts = script_api::facts(&ctx.bridge);
    let (value, seen): (bool, String) = match &c.kind {
        CondKind::InWorld => (ctx.player.spawn.is_some() && *ctx.screen.get() == Screen::InGame, format!("screen {:?}", ctx.screen.get())),
        CondKind::Target => (target.is_some(), format!("target {:?}", ctx.target.obj_id)),
        CondKind::TargetName(n) => (
            target.is_some_and(|u| u.name.to_ascii_lowercase().contains(&n.to_ascii_lowercase())),
            format!("target {:?}", target.map(|u| u.name.clone())),
        ),
        CondKind::TargetTemplate(t) => (target.is_some_and(|u| u.kind.template_id() == *t), format!("target template {:?}", target.map(|u| u.kind.template_id()))),
        CondKind::TargetDead => (target.is_some_and(|u| u.dead), format!("target {:?}", target.map(|u| (u.hp, u.dead)))),
        CondKind::TargetAlive => (target.is_some_and(|u| !u.dead), format!("target {:?}", target.map(|u| (u.hp, u.dead)))),
        CondKind::TargetHurt => {
            let t = ctx.target.obj_id.unwrap_or(0);
            let first = r.target_hp0.get(&t).copied();
            let now = ctx.unit_hp(t);
            (matches!((first, now), (Some(a), Some((b, _))) if b < a) || target.is_some_and(|u| u.dead), format!("hp {first:?} -> {now:?}"))
        }
        CondKind::Dead => (ctx.player.spawn.is_some() && ctx.own_dead(), format!("player hp {:?}", ctx.unit_hp(own))),
        CondKind::Alive => (ctx.player.spawn.is_some() && !ctx.own_dead(), format!("player hp {:?}", ctx.unit_hp(own))),
        CondKind::Dialog => (facts.talking_to.is_some() || facts.directing || ctx.pick.talk.is_some_and(|t| t.1.is_some()), format!("talking_to {:?}, last talk {:?}", facts.talking_to, ctx.pick.talk)),
        CondKind::Quest(q) => (ctx.quests.active_quest(*q).is_some(), format!("active quests {:?}", ctx.quests.active.iter().map(|q| q.template_id).collect::<Vec<_>>())),
        CondKind::QuestDone(q) => (ctx.quests.is_completed(*q), String::new()),
        CondKind::Moved(m) => {
            let d = ctx.me().map_or(0.0, |p| p.truncate().distance(r.mark.truncate()));
            (d >= *m, format!("moved {d:.1} m"))
        }
        CondKind::Near(x, y, rad) => {
            let d = ctx.me().map_or(f32::MAX, |p| p.truncate().distance(Vec2::new(*x, *y)));
            (d <= *rad, format!("{d:.1} m away"))
        }
        CondKind::Item(t, n) => {
            let have = ctx.inventory.count_of(*t);
            (have >= *n, format!("bag holds {have}"))
        }
        CondKind::Lootable => (facts.loot_nearest.is_some(), format!("{:?}", facts.loot_nearest)),
        CondKind::Chat(t) => (ctx.chat.log.iter().any(|m| m.message.contains(t.as_str())), format!("{} chat lines", ctx.chat.log.len())),
        CondKind::Event(e) => (r.seen.iter().any(|s| *s == e.to_ascii_lowercase()), format!("seen {:?}", r.seen)),
        CondKind::NoErrors => (r.problems.is_empty() && aa_dev::capture::lua_errors().is_empty(), format!("{:?}", r.problems)),
        CondKind::Lua(code) => match &ctx.rt {
            Some(rt) => match rt.lua.load(code.as_str()).eval::<mlua::Value>() {
                Ok(v) => (truthy(&v), format!("{v:?}")),
                Err(e) => (false, format!("lua error: {e}")),
            },
            None => (false, "no UI runtime".into()),
        },
        CondKind::Unit(s) => (ctx.resolve(s).is_some(), String::new()),
        CondKind::UnitDead(s) => (ctx.resolve(s).is_some_and(|f| f.dead), String::new()),
        CondKind::HpBelow(p) => {
            let hp = ctx.unit_hp(own);
            (hp.is_some_and(|(h, m)| m > 0 && (h as f32) * 100.0 < *p * m as f32), format!("hp {hp:?}"))
        }
        CondKind::Duel => (facts.duel_active, format!("results {:?}", facts.duel_results)),
        CondKind::DuelChallenge => (facts.duel_challenge.is_some(), format!("challenge {:?}", facts.duel_challenge)),
        CondKind::DuelResult(key) => (facts.duel_results.last().is_some_and(|k| k.strip_prefix("result_") == Some(key.as_str())), format!("results {:?}", facts.duel_results)),
        CondKind::Riding => {
            let seat = ctx.player.spawn.as_ref().and_then(|s| s.unit.attached_to);
            (seat.is_some(), format!("attached to {seat:?}"))
        }
    };
    (value != c.negate, seen)
}

fn say(r: &Runner, text: &str) {
    println!("SCRIPT [{:6.1}s] line {}: {text}", r.t0.elapsed().as_secs_f32(), r.current_no());
}

#[allow(clippy::too_many_lines)]
fn run_script(mut r: ResMut<Runner>, mut ctx: Ctx, mut events: MessageReader<NetEvent>) {
    let now = ctx.time.elapsed_secs();
    let own = ctx.own();
    // ---- the events
    for NetEvent(e) in events.read() {
        r.events_total += 1;
        match e {
            Event::GameDecodeFailed { message } => {
                let p = format!("packet decode failure: {message}");
                println!("SCRIPT PROBLEM {p}");
                r.problems.push(p);
            }
            Event::UnhandledGamePacket { level, opcode, len } => {
                *r.unhandled.entry((*level, *opcode)).or_default() += 1;
                if *level == aa_net::game::LEVEL_COMPRESSED && *len > 0 {
                    let p = format!("compressed batch lost its tail at opcode {opcode:#05x} ({len} bytes)");
                    println!("SCRIPT PROBLEM {p}");
                    r.problems.push(p);
                }
            }
            Event::ServerError { code } => *r.server_errors.entry(*code).or_default() += 1,
            Event::Disconnected { message } => r.disconnected = Some(message.clone()),
            Event::SkillStarted(s) if s.caster.obj_id == own => r.last_cast = Some((s.skill_id, s.result, s.cast_ms)),
            Event::SkillFired(s) if s.caster.obj_id == own => r.last_fired = Some(s.skill_id),
            // a plot-only skill (Triple Slash, Charge ...) answers with its plot steps, not with SkillStarted / SkillFired
            Event::PlotEvent(p) if p.caster_obj_id == own => r.last_fired = Some(p.skill_id),
            Event::LootItemTook { .. } | Event::LootBag { .. } | Event::LootItemFailed { .. } => r.loot_seen = true,
            Event::QuestStarted(q) => r.quest_started.push(q.template_id),
            _ => {}
        }
        r.queue_event(e);
    }
    // ---- Lua errors
    let lua = aa_dev::capture::lua_errors();
    for message in lua.iter().skip(r.lua_seen) {
        let p = format!("Lua error: {message}");
        println!("SCRIPT PROBLEM {p}");
        r.problems.push(p);
    }
    r.lua_seen = lua.len();

    // ---- finishing
    if let Some((code, frames)) = r.finish {
        if frames == 0 {
            summary(&r, &ctx, code);
            ctx.exit.write(if code == 0 { AppExit::Success } else { AppExit::Error(NonZeroU8::new(code).unwrap_or(NonZeroU8::MIN)) });
            r.finish = Some((code, 0));
        } else {
            if frames == 3 {
                ctx.net.write(NetCommand(Command::Disconnect));
            }
            r.finish = Some((code, frames - 1));
        }
        return;
    }
    if now > r.max_total {
        let max = r.max_total;
        r.fail(format!("the script did not finish in {max:.0} s (--script-timeout)"));
        return;
    }

    // ---- the start: in the world with the HUD up
    let Some(started) = r.started else {
        let ready = *ctx.screen.get() == Screen::InGame && ctx.built.up && ctx.me().is_some() && ctx.own() != 0;
        if ready {
            let began = *r.started.get_or_insert(now);
            r.mark = ctx.me().unwrap_or_default();
            println!("SCRIPT: in the world at {:.1} s as '{}' at ({:.1}, {:.1}, {:.1})", began, ctx.player.spawn.as_ref().map_or("", |s| s.name()), r.mark.x, r.mark.y, r.mark.z);
            r.started = Some(now + 2.0); // a moment for the first units
        } else if let Some(m) = &r.disconnected {
            let m = m.clone();
            r.problems.push(format!("disconnected before the world: {m}"));
            println!("SCRIPT FAIL disconnected before the world: {m}");
            r.stop(1);
        } else if now > r.enter_timeout {
            r.problems.push("never entered the world".into());
            println!("SCRIPT FAIL never entered the world in {:.0} s (--script-enter-timeout)", r.enter_timeout);
            r.stop(1);
        }
        return;
    };
    if now < started {
        return;
    }
    if let Some(m) = r.disconnected.clone()
        && !r.quitting
    {
        r.problems.push(format!("connection lost: {m}"));
        println!("SCRIPT FAIL connection lost: {m}");
        r.stop(1);
        return;
    }

    // ---- the busy command
    if let Some(active) = r.active.take() {
        match tick(&mut r, &mut ctx, active, now) {
            Some(a) => {
                r.active = Some(a);
                return;
            }
            None => {
                if r.finish.is_some() {
                    return;
                }
                r.done_lines += 1;
            }
        }
    }
    // ---- the next commands (the instant ones in a row)
    for _ in 0..64 {
        if r.finish.is_some() || r.active.is_some() {
            return;
        }
        let Some(line) = r.lines.get(r.pc).cloned() else {
            let code = u8::from(!r.problems.is_empty());
            r.stop(code);
            return;
        };
        r.pc += 1;
        println!("SCRIPT [{:6.1}s] > {}", r.t0.elapsed().as_secs_f32(), line.text);
        match start(&mut r, &mut ctx, &line.cmd, now) {
            Some(a) => r.active = Some(a),
            None if r.finish.is_none() => r.done_lines += 1,
            None => {}
        }
    }
}

/// Begins a command: `Some` when it has to wait, `None` when it is done (or failed: the run is then stopping).
#[allow(clippy::too_many_lines)]
fn start(r: &mut Runner, c: &mut Ctx, cmd: &Cmd, now: f32) -> Option<Active> {
    match cmd {
        Cmd::Wait(s) => Some(Active::Wait { until: now + s }),
        Cmd::Enter => None,
        Cmd::Echo(t) => {
            say(r, t);
            None
        }
        Cmd::Mark => {
            r.mark = c.me().unwrap_or_default();
            None
        }
        Cmd::WalkTo(x, y) => walk(r, c, Goal::Point(Vec2::new(*x, *y)), 1.5, now),
        Cmd::WalkForward(m) => {
            let Some(mover) = c.movers.single().ok().map(|m| (m.pos, m.facing)) else {
                r.fail("no player".into());
                return None;
            };
            let dir = crate::ingame::player::forward(mover.1);
            walk(r, c, Goal::Point(mover.0.truncate() + dir * *m), 1.5, now)
        }
        Cmd::WalkToUnit(spec) => match c.resolve(spec) {
            Some(f) => {
                say(r, &format!("walking to {}", c.describe(&f)));
                walk(r, c, Goal::Unit(f.obj_id), 3.0, now)
            }
            None => {
                r.fail(format!("no unit for {spec:?} in sight"));
                None
            }
        },
        Cmd::Face(deg) => {
            let yaw = deg.to_radians();
            for mut o in &mut c.cameras {
                o.yaw = yaw;
            }
            for mut m in &mut c.movers {
                m.facing = yaw;
            }
            None
        }
        Cmd::Orbit(deg) => {
            let facing = c.movers.single().map_or(0.0, |m| m.facing);
            for mut o in &mut c.cameras {
                o.yaw = facing + deg.to_radians();
            }
            Some(Active::Wait { until: now + 0.3 })
        }
        Cmd::Zoom(notches) => {
            let limit = crate::ingame::camera::mode_max_distance();
            for mut o in &mut c.cameras {
                for _ in 0..notches.abs().round() as u32 {
                    o.wanted = crate::ingame::camera::zoom_notch(o.wanted, notches.signum(), limit);
                }
            }
            Some(Active::Wait { until: now + 0.1 })
        }
        Cmd::Pitch(deg) => {
            for mut o in &mut c.cameras {
                o.pitch = crate::ingame::camera::clamp_pitch(deg.to_radians());
            }
            Some(Active::Wait { until: now + 0.1 })
        }
        Cmd::FaceUnit(spec) => {
            let (Some(f), Some(me)) = (c.resolve(spec), c.me()) else {
                r.fail(format!("no unit for {spec:?} in sight"));
                return None;
            };
            let d = f.pos.truncate() - me.truncate();
            let yaw = (-d.x).atan2(d.y);
            for mut o in &mut c.cameras {
                o.yaw = yaw;
            }
            for mut m in &mut c.movers {
                m.facing = yaw;
            }
            None
        }
        Cmd::Tp(x, y, z) => {
            c.net.write(NetCommand(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message: format!("/move {x} {y} {z}") }));
            Some(Active::Wait { until: now + 3.0 })
        }
        Cmd::Gm(text) => {
            c.net.write(NetCommand(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message: format!("/{}", text.trim_start_matches('/')) }));
            Some(Active::Wait { until: now + 0.6 })
        }
        Cmd::Chat(text) => {
            let message = aa_ui::lua::chat::encode_for_transport(text);
            c.net.write(NetCommand(Command::Chat { chat_type: aa_net::model::chat_type::SAY, target: String::new(), message }));
            Some(Active::Wait { until: now + 0.4 })
        }
        Cmd::Target(spec) | Cmd::Click(spec) => {
            let Some(f) = c.resolve(spec) else {
                r.fail(format!("no unit for {spec:?} in sight"));
                return None;
            };
            say(r, &format!("target {}", c.describe(&f)));
            if let Some((hp, _)) = c.unit_hp(f.obj_id) {
                r.target_hp0.entry(f.obj_id).or_insert(hp);
            }
            if matches!(cmd, Cmd::Click(_)) {
                c.pick.left = Some(f.obj_id);
            } else {
                c.net.write(NetCommand(Command::Target { obj_id: f.obj_id }));
            }
            Some(Active::Target { obj_id: f.obj_id, deadline: now + 5.0 })
        }
        Cmd::Cast(skill, spec) => {
            let target = match spec {
                Some(s) => match c.resolve(s) {
                    Some(f) => {
                        if c.target.obj_id != Some(f.obj_id) {
                            c.net.write(NetCommand(Command::Target { obj_id: f.obj_id }));
                        }
                        if let Some((hp, _)) = c.unit_hp(f.obj_id) {
                            r.target_hp0.entry(f.obj_id).or_insert(hp);
                        }
                        f.obj_id
                    }
                    None => {
                        r.fail(format!("no unit for {s:?} in sight"));
                        return None;
                    }
                },
                None => c.target.obj_id.unwrap_or(0),
            };
            if let Some((hp, _)) = c.unit_hp(target) {
                r.target_hp0.entry(target).or_insert(hp);
            }
            (r.last_cast, r.last_fired) = (None, None);
            c.net.write(NetCommand(Command::UseSkill { skill_id: *skill, target_obj_id: target }));
            Some(Active::Cast { skill: *skill, deadline: now + 5.0, turned: false })
        }
        Cmd::CastSlot(n) => {
            c.keys.push_now(&format!("slot{n}"), false);
            Some(Active::Wait { until: now + 1.0 })
        }
        Cmd::Fight { skill, slot, spec, secs } => {
            let Some(f) = c.resolve(spec) else {
                r.fail(format!("no unit for {spec:?} in sight"));
                return None;
            };
            say(r, &format!("fighting {}", c.describe(&f)));
            if let Some((hp, _)) = c.unit_hp(f.obj_id) {
                r.target_hp0.entry(f.obj_id).or_insert(hp);
            }
            c.net.write(NetCommand(Command::Target { obj_id: f.obj_id }));
            Some(Active::Fight { skill: *skill, slot: *slot, target: f.obj_id, next: now + 0.5, deadline: now + secs, casts: 0, duels: script_api::facts(&c.bridge).duel_results.len() })
        }
        Cmd::StopCast => {
            c.net.write(NetCommand(Command::StopCasting { tl: 0, plot_tl: 0 }));
            None
        }
        Cmd::Interact(spec) => {
            let Some(f) = c.resolve(spec) else {
                r.fail(format!("no unit for {spec:?} in sight"));
                return None;
            };
            say(r, &format!("right click on {}", c.describe(&f)));
            c.pick.talk = None;
            c.pick.interacted = None;
            c.pick.right = Some(f.obj_id);
            Some(Active::Interact { npc: f.obj_id, deadline: now + 4.0 })
        }
        Cmd::InteractDoodad(spec, skill) => {
            let Some(f) = c.resolve_doodad(spec) else {
                r.fail(format!("no doodad for {spec:?} in sight"));
                return None;
            };
            say(r, &format!("doodad #{} (template {}) {:.1} m away: skill {skill}", f.obj_id, f.template, c.me().map_or(0.0, |m| m.distance(f.pos))));
            (r.last_cast, r.last_fired) = (None, None);
            c.net.write(NetCommand(Command::UseDoodadSkill { skill_id: *skill, doodad_obj_id: f.obj_id }));
            Some(Active::Cast { skill: *skill, deadline: now + 5.0, turned: false })
        }
        Cmd::Accept(q) => {
            let facts = script_api::facts(&c.bridge);
            let Some(quest) = q.or_else(|| facts.offers.first().copied()) else {
                r.fail(format!("the dialog offers no quest (talking to {:?})", facts.talking_to));
                return None;
            };
            r.quest_started.retain(|s| *s != quest);
            say(r, &format!("accepting quest {quest}"));
            script_api::accept(&c.bridge, quest);
            Some(Active::Await { event: format!("quest:{quest}"), deadline: now + 8.0, must: true })
        }
        Cmd::Complete(q, reward) => {
            let facts = script_api::facts(&c.bridge);
            let Some(quest) = q.or_else(|| facts.completes.first().copied()) else {
                r.fail(format!("the dialog takes in no quest (talking to {:?})", facts.talking_to));
                return None;
            };
            say(r, &format!("handing in quest {quest}"));
            script_api::complete(&c.bridge, quest, *reward);
            Some(Active::WaitFor { cond: Cond { negate: false, kind: CondKind::QuestDone(quest) }, deadline: now + 8.0 })
        }
        Cmd::EndTalk => {
            script_api::end_talk(&c.bridge);
            None
        }
        Cmd::Loot(spec, must) => {
            if let Some(f) = c.resolve(spec) {
                say(r, &format!("looting {}", c.describe(&f)));
            }
            r.loot_seen = false;
            Some(Active::Loot { deadline: now + 6.0, sent: false, must: *must })
        }
        Cmd::Open(action) => {
            c.session.open_now(action);
            Some(Active::Wait { until: now + 0.5 })
        }
        Cmd::Key(k) => {
            c.keys.push_now(k, false);
            Some(Active::Wait { until: now + 0.3 })
        }
        Cmd::Hold(keys, secs) => {
            if keys.is_empty() || !keys.bytes().all(|b| b.is_ascii_alphabetic()) {
                r.fail(format!("hold needs direction keys, not '{keys}'"));
                return None;
            }
            say(r, &format!("holding {keys} for {secs:.2} s"));
            c.dev.script_keys = keys.clone();
            Some(Active::Hold { until: now + secs })
        }
        Cmd::Keys(keys) => {
            if !keys.is_empty() && !keys.bytes().all(|b| b.is_ascii_alphabetic()) {
                r.fail(format!("keys needs direction keys, not '{keys}'"));
                return None;
            }
            say(r, &format!("keys {keys}"));
            c.dev.script_keys = keys.clone();
            None
        }
        Cmd::Type(t) => {
            c.keys.push_now(t, true);
            Some(Active::Wait { until: now + 0.3 })
        }
        Cmd::HudWidget(expression) => {
            let point = c.rt.as_ref().ok_or_else(|| "no UI runtime".to_string())
                .and_then(|rt| widget_click_point(rt, expression));
            match point {
                Ok(point) => c.session.click_now(point, false),
                Err(error) => r.fail(format!("widget click: {error}")),
            }
            Some(Active::Wait { until: now + 0.5 })
        }
        Cmd::HudClick(x, y, right) => {
            c.session.click_now(Vec2::new(*x, *y), *right);
            Some(Active::Wait { until: now + 0.5 })
        }
        Cmd::HudHover(x, y) => {
            c.session.hover_now(Vec2::new(*x, *y));
            Some(Active::Wait { until: now + 0.5 })
        }
        Cmd::Lua(code) => {
            match &c.rt {
                Some(rt) => {
                    if let Err(e) = rt.exec("@script", code) {
                        r.fail(format!("lua: {e}"));
                    }
                }
                None => r.fail("no UI runtime for lua".into()),
            }
            None
        }
        Cmd::Await { event, secs, must } => Some(Active::Await { event: event.to_ascii_lowercase(), deadline: now + secs, must: *must }),
        Cmd::WaitFor(cond, secs) => Some(Active::WaitFor { cond: cond.clone(), deadline: now + secs }),
        Cmd::Expect(cond) => {
            let (ok, seen) = eval(r, cond, c);
            if ok {
                say(r, &format!("expect ok: {cond:?}"));
            } else {
                r.fail(format!("expect failed: {cond:?} ({seen})"));
            }
            None
        }
        Cmd::Shot(path) => {
            if let Some(dir) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::remove_file(path);
            if c.offscreen.is_some() {
                aa_dev::take_offscreen_shot(&mut c.commands, c.offscreen.as_deref(), path);
            } else if aa_dev::args().into_iter().any(|a| a == "--headless") {
                say(r, "shot skipped: --headless has no renderer (use --offscreen-shot)");
                return None;
            } else {
                c.commands
                    .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                    .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
            }
            Some(Active::Shot { path: path.clone(), deadline: now + 10.0 })
        }
        Cmd::Units(templates) => {
            let me = c.me().unwrap_or_default();
            let mut list: Vec<_> = c
                .units
                .iter()
                .filter(|u| u.obj_id != c.own())
                .filter(|u| templates.is_empty() || templates.contains(&u.kind.template_id()))
                .map(|u| (me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)), u))
                .collect();
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, u) in list.iter().take(25) {
                say(r, &format!("  #{} '{}' template {} level {} hp {}/{} {:.1} m{}", u.obj_id, u.name, u.kind.template_id(), u.level, u.hp, u.max_hp, d, if u.dead { " dead" } else { "" }));
            }
            say(r, &format!("{} units in sight, {} doodads", list.len(), c.doodads.by_id.len()));
            None
        }
        Cmd::Me => {
            if let (Some(p), Some(s)) = (c.me(), c.player.spawn.as_ref()) {
                let hp = c.unit_hp(s.obj_id);
                let worn: Vec<(usize, u32)> = s.unit.equipment.slots.iter().enumerate().filter_map(|(i, it)| it.as_ref().map(|it| (i, it.template_id))).collect();
                say(r, &format!("  skills {:?}", c.skills.known.iter().map(|k| k.id).collect::<Vec<_>>()));
                say(r, &format!("  equipment (slot, template) {worn:?}"));
                say(r, &format!("'{}' obj {} at ({:.1}, {:.1}, {:.1}) hp {hp:?} target {:?}", s.name(), s.obj_id, p.x, p.y, p.z, c.target.obj_id));
            }
            None
        }
        Cmd::Bag => {
            for (slot, it) in c.inventory.bag_items() {
                say(r, &format!("  slot {slot}: template {} x{} grade {} id {}", it.template_id, it.count, it.grade, it.id));
            }
            None
        }
        Cmd::Use(template) => {
            match c.inventory.bag_items().find(|(_, it)| it.template_id == *template) {
                Some((slot, _)) => c.bridge.0.borrow_mut().requests.push(crate::hud::api::Request::UseBag { slot: slot + 1 }),
                None => r.fail(format!("no item {template} in the bag")),
            }
            Some(Active::Wait { until: now + 0.5 })
        }
        Cmd::Quests => {
            for q in &c.quests.active {
                say(r, &format!("  quest {} status {} objectives {:?}", q.template_id, q.status, q.objectives));
            }
            None
        }
        Cmd::Quit(graceful) => {
            r.quitting = true;
            if *graceful {
                c.net.write(NetCommand(Command::LeaveWorld(aa_net::game::LeaveTarget::Quit)));
                Some(Active::Quit { deadline: now + 45.0, sent: true })
            } else {
                let code = u8::from(!r.problems.is_empty());
                r.stop(code);
                None
            }
        }
    }
}

fn walk(r: &mut Runner, c: &mut Ctx, goal: Goal, radius: f32, now: f32) -> Option<Active> {
    let Some(me) = c.me() else {
        r.fail("no player".into());
        return None;
    };
    let to = goal_pos(c, &goal).unwrap_or(me.truncate());
    let dist = me.truncate().distance(to);
    if dist <= radius {
        return None;
    }
    // the character settles on the ground first: the stuck check starts later
    Some(Active::Walk { goal, radius, start: now, deadline: now + 30.0 + dist / 3.0, best: dist, best_t: now + 15.0 })
}

fn goal_pos(c: &Ctx, goal: &Goal) -> Option<Vec2> {
    match goal {
        Goal::Point(p) => Some(*p),
        Goal::Unit(id) => c.units.get(*id).map(|u| Vec2::new(u.pos.x, u.pos.y)),
    }
}

/// One frame of a busy command: `Some` keeps it, `None` ends it (success, or a failure that stops the run).
#[allow(clippy::too_many_lines)]
fn tick(r: &mut Runner, c: &mut Ctx, active: Active, now: f32) -> Option<Active> {
    match active {
        Active::Wait { until } => (now < until).then_some(Active::Wait { until }),
        Active::Hold { until } => {
            if now < until {
                Some(Active::Hold { until })
            } else {
                c.dev.script_keys.clear();
                None
            }
        }
        Active::Walk { goal, radius, start, deadline, mut best, mut best_t } => {
            let Some(me) = c.me() else {
                c.dev.script_keys.clear();
                r.fail("the player is gone".into());
                return None;
            };
            let Some(to) = goal_pos(c, &goal) else {
                c.dev.script_keys.clear();
                r.fail("the unit walked to is gone".into());
                return None;
            };
            let dist = me.truncate().distance(to);
            if dist <= radius {
                c.dev.script_keys.clear();
                say(r, &format!("arrived at ({:.1}, {:.1}) after {:.1} s", me.x, me.y, now - start));
                return None;
            }
            if dist < best - 0.5 {
                best = dist;
                best_t = now;
            }
            if now > deadline || now - best_t > 8.0 {
                c.dev.script_keys.clear();
                let why = if now > deadline { "timed out" } else { "stuck (no progress for 8 s)" };
                r.fail(format!("walk {why} at ({:.1}, {:.1}, {:.1}), {dist:.1} m from ({:.1}, {:.1})", me.x, me.y, me.z, to.x, to.y));
                return None;
            }
            // the camera heading is the walking direction: 0 = north, counter-clockwise
            let d = to - me.truncate();
            let yaw = (-d.x).atan2(d.y);
            for mut o in &mut c.cameras {
                o.yaw = yaw;
            }
            c.dev.script_keys = "W".into();
            Some(Active::Walk { goal, radius, start, deadline, best, best_t })
        }
        Active::Await { event, deadline, must } => {
            // quest:N is the accept button waiting for the server's QuestStarted
            if let Some(q) = event.strip_prefix("quest:").and_then(|q| q.parse::<u32>().ok()) {
                if r.quest_started.contains(&q) || c.quests.active_quest(q).is_some() {
                    say(r, &format!("quest {q} started"));
                    return None;
                }
                if now > deadline {
                    r.fail(format!("quest {q} was not started by the server within 8 s"));
                    return None;
                }
                return Some(Active::Await { event, deadline, must });
            }
            if let Some(i) = r.queue.iter().position(|q| *q == event) {
                r.queue.remove(i);
                return None;
            }
            if now > deadline {
                if must {
                    r.fail(format!("event {event} did not arrive in time"));
                } else {
                    say(r, &format!("event {event}: timeout"));
                }
                return None;
            }
            Some(Active::Await { event, deadline, must })
        }
        Active::WaitFor { cond, deadline } => {
            let (ok, seen) = eval(r, &cond, c);
            if ok {
                return None;
            }
            if now > deadline {
                r.fail(format!("waitfor timed out: {cond:?} ({seen})"));
                return None;
            }
            Some(Active::WaitFor { cond, deadline })
        }
        Active::Target { obj_id, deadline } => {
            if c.target.obj_id == Some(obj_id) || c.target.obj_id.is_some() && obj_id == c.own() {
                return None;
            }
            if now > deadline {
                r.fail(format!("the server did not select target #{obj_id} (target is {:?})", c.target.obj_id));
                return None;
            }
            Some(Active::Target { obj_id, deadline })
        }
        Active::Interact { npc, deadline } => {
            if let Some((id, what)) = c.pick.talk {
                say(r, &format!("talk with #{id}: {}", what.unwrap_or("nothing to say (no quest or service)")));
                return None;
            }
            // A right click on a slave (cart, ship) sends the interaction request and never starts a
            // talk (hud::slave::interact returns before the dialog). Only that path completes the
            // command: a consumed pick alone can mean `talk_input` dropped it (the item-look
            // pointer owned the click) or the directing dialog returned first, and no packet left.
            if c.pick.take_interacted(npc) {
                say(
                    r,
                    &format!("right click on #{npc}: slave interaction request sent"),
                );
                return None;
            }
            if c.pick.right.is_none()
                && c.units
                    .get(npc)
                    .is_some_and(|u| matches!(u.kind, UnitKind::Slave { .. }))
            {
                r.fail(format!(
                    "right click on #{npc} did not reach the slave interaction"
                ));
                return None;
            }
            if now > deadline {
                r.fail(format!("right click on #{npc} started no talk (out of the 6 m reach, hostile or dead?)"));
                return None;
            }
            Some(Active::Interact { npc, deadline })
        }
        Active::Cast { skill, deadline, turned } => {
            if let Some((id, result, cast_ms)) = r.last_cast {
                // not facing the target: the combat code turns the player and asks once more, as the real client does
                if result == aa_net::skill::result::OUTOF_ANGLE && !turned {
                    say(r, &format!("skill {id}: out of angle, the client turns and asks again"));
                    r.last_cast = None;
                    return Some(Active::Cast { skill, deadline, turned: true });
                }
                if result != 0 {
                    r.fail(format!("the server refused skill {id}: result {result}"));
                    return None;
                }
                say(r, &format!("skill {skill} started (cast {cast_ms} ms)"));
                return Some(Active::Wait { until: now + cast_ms as f32 / 1000.0 + 0.6 });
            }
            if r.last_fired == Some(skill) {
                say(r, &format!("skill {skill} fired (instant or plot)"));
                return Some(Active::Wait { until: now + 0.6 });
            }
            if now > deadline {
                r.fail(format!("skill {skill}: no SkillStarted, SkillFired or PlotEvent from the server in 5 s"));
                return None;
            }
            Some(Active::Cast { skill, deadline, turned })
        }
        Active::Fight { skill, slot, target, mut next, deadline, mut casts, duels } => {
            let dead = c.units.get(target).is_none_or(|u| u.dead);
            // A duel never kills: it ends with its result.
            if script_api::facts(&c.bridge).duel_results.len() > duels {
                say(r, &format!("the duel with #{target} ended after {casts} casts"));
                return None;
            }
            if dead {
                say(r, &format!("target #{target} is dead after {casts} casts"));
                return None;
            }
            if now > deadline {
                let hp = c.unit_hp(target);
                r.fail(format!("fight timed out: #{target} still alive, hp {hp:?}, {casts} casts"));
                return None;
            }
            if now >= next {
                match skill {
                    Some(s) => {
                        c.net.write(NetCommand(Command::UseSkill { skill_id: s, target_obj_id: target }));
                    }
                    None => c.keys.push_now(&format!("slot{slot}"), false),
                }
                casts += 1;
                next = now + 1.5;
            }
            Some(Active::Fight { skill, slot, target, next, deadline, casts, duels })
        }
        Active::Loot { deadline, sent, must } => {
            let facts = script_api::facts(&c.bridge);
            if !sent {
                if facts.loot_nearest.is_some() {
                    c.keys.push_now("f", false);
                    return Some(Active::Loot { deadline: now + 6.0, sent: true, must });
                }
                if now > deadline {
                    if must {
                        r.fail("no corpse with loot within reach".into());
                    } else {
                        say(r, "no corpse with loot within reach (tryloot)");
                    }
                    return None;
                }
                return Some(Active::Loot { deadline, sent, must });
            }
            if r.loot_seen {
                say(r, "loot taken");
                return None;
            }
            if now > deadline {
                r.fail("the server answered neither LootItemTook nor LootBag".into());
                return None;
            }
            Some(Active::Loot { deadline, sent, must })
        }
        Active::Shot { path, deadline } => {
            if std::path::Path::new(&path).exists() {
                say(r, &format!("screenshot {path}"));
                return None;
            }
            if now > deadline {
                r.fail(format!("screenshot {path} was not written"));
                return None;
            }
            Some(Active::Shot { path, deadline })
        }
        Active::Quit { deadline, sent } => {
            if r.disconnected.is_some() {
                let code = u8::from(!r.problems.is_empty());
                r.stop(code);
                return None;
            }
            if now > deadline {
                r.problems.push("the server did not close the connection after the logout".into());
                println!("SCRIPT FAIL the server did not close the connection within 45 s of the logout");
                r.stop(1);
                return None;
            }
            Some(Active::Quit { deadline, sent })
        }
    }
}

/// Resolve a current widget hit, then use the same physical-pixel pointer queue as
/// uiclick. Never invoke a Lua handler directly or click through an occluding widget.
fn widget_click_point(rt: &aa_ui::UiRuntime, expression: &str) -> Result<Vec2, String> {
    let widget: mlua::Table = rt.lua.load(expression).eval().map_err(|e| e.to_string())?;
    let id = aa_ui::lua::value_id(&rt.lua, &mlua::Value::Table(widget.clone())).ok_or("widget missing")?;
    let (x, y) = widget.call_method::<(f32, f32)>("GetEffectiveOffset", ()).map_err(|e| e.to_string())?;
    let (w, h) = widget.call_method::<(f32, f32)>("GetEffectiveExtent", ()).map_err(|e| e.to_string())?;
    let point = Vec2::new(x + w * 0.5, y + h * 0.5);
    let scale = aa_ui::lua::ui(&rt.lua).ui_scale;
    if !point.is_finite() || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 || !scale.is_finite() || scale <= 0.0 {
        return Err("widget has no finite positive extent".into());
    }
    if aa_ui::input::pick(&rt.lua, point.x / scale, point.y / scale) != Some(id) {
        return Err("widget centre is hidden, disabled, retired or occluded".into());
    }
    Ok(point)
}

/// The client ended itself (a login that failed, no character, ...) before the script did.
fn unexpected_exit(r: Res<Runner>, mut exits: MessageReader<AppExit>) {
    if exits.read().next().is_some() && r.finish.is_none() {
        println!("SCRIPT RESULT: FAIL the client exited by itself at command {} of {} (see the log)", r.pc, r.lines.len());
    }
}

fn summary(r: &Runner, c: &Ctx, code: u8) {
    let (warnings, errors, allowed) = aa_dev::capture::counts();
    println!("SCRIPT SUMMARY {}", r.file);
    println!("  commands:   {} of {} done ({} s wall)", r.done_lines, r.lines.len(), r.t0.elapsed().as_secs());
    println!("  events:     {} received, {} server errors {:?}", r.events_total, r.server_errors.values().sum::<usize>(), r.server_errors);
    println!("  log:        {warnings} warnings, {errors} errors, {allowed} allowed Lua errors");
    if !r.unhandled.is_empty() {
        let list: Vec<String> = r.unhandled.iter().map(|((l, o), n)| format!("level {l} {o:#05x} x{n}")).collect();
        println!("  unhandled:  {}", list.join(", "));
    }
    if let Some(p) = c.me() {
        println!("  player:     ({:.1}, {:.1}, {:.1}), moved {:.1} m since the start", p.x, p.y, p.z, p.truncate().distance(r.mark.truncate()));
    }
    for p in &r.problems {
        println!("  PROBLEM:    {p}");
    }
    println!("SCRIPT RESULT: {}", if code == 0 { "PASS" } else { "FAIL" });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_click_uses_current_scaled_layout_and_rejects_obscured_or_missing_targets() -> Result<(), Box<dyn std::error::Error>> {
        let rt = aa_ui::UiRuntime::new("en_us", 1280., 720.)?;
        aa_ui::lua::ui(&rt.lua).ui_scale = 0.85;
        rt.lua.load("CLICK_TARGET=UIParent:CreateWidget('button','clickTarget','UIParent');CLICK_TARGET:SetExtent(100,40);CLICK_TARGET:AddAnchor('TOPLEFT','UIParent',100,80);CLICK_TARGET:Show(true)").exec()?;
        let original = widget_click_point(&rt, "CLICK_TARGET")?;
        assert!((original - Vec2::new(127.5, 85.)).length() < 0.001);
        rt.lua.load("BLOCKER=UIParent:CreateWidget('button','clickBlocker','UIParent');BLOCKER:SetExtent(100,40);BLOCKER:AddAnchor('TOPLEFT','UIParent',100,80);BLOCKER:Show(true)").exec()?;
        assert!(widget_click_point(&rt, "CLICK_TARGET").is_err());
        rt.lua.load("BLOCKER:Show(false);CLICK_TARGET:RemoveAllAnchors();CLICK_TARGET:AddAnchor('TOPLEFT','UIParent',200,80)").exec()?;
        assert!((widget_click_point(&rt, "CLICK_TARGET")? - original - Vec2::new(85., 0.)).length() < 0.001);
        rt.lua.load("CLICK_TARGET:Show(false)").exec()?;
        assert!(widget_click_point(&rt, "CLICK_TARGET").is_err());
        assert!(widget_click_point(&rt, "nil").is_err());
        Ok(())
    }

    #[test]
    fn repeated_awaits_require_separate_received_packets() {
        let mut runner=Runner::new("receipt-regression".into(),vec![],vec!["notice.usermusic".into(),"notice.usermusic".into(),"notice.usermusicpaused".into()],1.,10.);
        let play=Event::Notice(aa_net::notice::NoticeEvent::UserMusic{obj_id:100,author:"fixture".into(),midi:vec![1,2,3]});
        runner.queue_event(&play);
        assert_eq!(runner.queue,vec!["notice.usermusic"]);
        runner.queue.remove(0);
        assert!(runner.queue.is_empty(),"a second await still needs another packet");
        runner.queue_event(&Event::Notice(aa_net::notice::NoticeEvent::UserMusicPaused{obj_id:100}));
        assert_eq!(runner.queue,vec!["notice.usermusicpaused"]);
        runner.queue.clear();
        runner.queue_event(&play);
        runner.queue_event(&play);
        assert_eq!(runner.queue,vec!["notice.usermusic","notice.usermusic"]);
    }

    #[test]
    fn a_slave_click_only_completes_after_the_interaction_path_ran() {
        let mut pick = ScriptPick {
            right: Some(20),
            ..Default::default()
        };
        // `talk_input` takes the pick before the item-look and directing checks, so consumption
        // alone does not prove that `slave::interact` ran.
        assert_eq!(pick.take_right(), Some(20));
        assert!(!pick.take_interacted(20));
        pick.right = Some(20);
        assert_eq!(pick.take_right(), Some(20));
        pick.interacted(20);
        assert!(pick.take_interacted(20));
        assert!(
            !pick.take_interacted(20),
            "a later interact needs a new click"
        );
    }

    #[test]
    fn event_names_are_prefixes() {
        assert_eq!(event_name(&Event::Snow(true)), "Snow.true");
        assert_eq!(event_name(&Event::UnitDeath { obj_id: 3 }), "UnitDeath");
        assert!(name_matches("Snow.true", "snow"));
        assert!(!name_matches("SnowX", "snow"));
        assert!(name_matches("UnitDeath", "unitdeath"));
        assert_eq!(event_name(&Event::LoginSucceeded(Default::default())), "LoginSucceeded.AccountInfo");
    }
}
