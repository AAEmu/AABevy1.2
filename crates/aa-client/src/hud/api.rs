//! The game API of the in-game scripts: the `X2*` objects they call, backed by a [`Snapshot`] of
//! aa-net's state that the app pushes into the Lua state (`HUD.state`), and the requests they queue
//! (`HUD.request`).
//!
//! Layers, in the order they are installed into the stage's Lua state:
//!
//! 1. [`crate::login::gameapi::install`]: every `X2*` object as a permissive stub, the native
//!    constants.
//! 2. **FALLBACK**: `aa-ui/examples/fake/fake_ingame.lua`, the fake in-game data of the reference
//!    capture. Everything aa-net does not provide yet is still read from it.
//! 3. `ingame.lua`: the real data. It overrides the fake, function by function, as the data lands.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use aa_net::state::{LABOR_MAX, Player, PlayerStats};
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

use super::data::{SharedGameData, Snapshot};
use super::panels::{self, Panels};
use crate::login::gameapi;

/// What a script asked the app to do (queued by `HUD.request(kind, ...)`).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `HUD.request("chat", channel, message, target)`: a line typed into the chat input.
    Chat { chat_type: i16, message: String, target: String },
}

/// The state shared between the Bevy systems and the Lua closures (single threaded).
pub struct HudData {
    pub snapshot: Snapshot,
    /// What the Lua state has seen last.
    pub pushed: Option<Snapshot>,
    /// The action bars, bag, buffs and target, and what the Lua state has seen of them.
    pub panels: Panels,
    pub panels_pushed: Option<Panels>,
    /// Game position of the target.
    pub target_pos: Option<Vec3>,
    /// The HUD's clock starts here (`HUD.now_ms()`).
    pub start: Instant,
    pub game: SharedGameData,
    pub requests: Vec<Request>,
    /// The local player's entity (found by its name once), its game position and facing.
    pub player_entity: Option<Entity>,
    pub pos: Vec3,
    pub yaw: f32,
}

pub type Shared = Rc<RefCell<HudData>>;

/// The non-send resource holding the shared state.
pub struct HudBridge(pub Shared);

impl Default for HudData {
    fn default() -> Self {
        HudData {
            snapshot: Snapshot::default(),
            pushed: None,
            panels: Panels::default(),
            panels_pushed: None,
            target_pos: None,
            start: Instant::now(),
            game: SharedGameData::default(),
            requests: Vec::new(),
            player_entity: None,
            pos: Vec3::ZERO,
            yaw: 0.0,
        }
    }
}

impl Default for HudBridge {
    fn default() -> Self {
        let data = HudData::default();
        data.game.load_in_background();
        HudBridge(Rc::new(RefCell::new(data)))
    }
}

/// Fake in-game game API of the aa-ui example (**FALLBACK** data).
const FAKE_INGAME: &str = include_str!("../../../aa-ui/examples/fake/fake_ingame.lua");
const INGAME_LUA: &str = include_str!("ingame.lua");

/// Install the game API into a fresh runtime, before the stage's scripts run.
pub fn install(rt: &mut UiRuntime, bridge: &Shared) -> mlua::Result<()> {
    // 1. the X2* objects and constants (the login screens' install, with an empty bridge)
    gameapi::install(rt, &gameapi::new_shared())?;
    let lua = &rt.lua;
    // the table the fake file keeps its helpers in
    let fake = lua.create_table()?;
    fake.raw_set("locale", "en_us")?;
    fake.raw_set("worlds", lua.create_table()?)?;
    fake.raw_set("characters", lua.create_table()?)?;
    fake.raw_set(
        "fire_now",
        lua.create_function(|lua, (name, rest): (String, MultiValue)| {
            aa_ui::lua::fire_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })?,
    )?;
    fake.raw_set(
        "fire_later",
        lua.create_function(|lua, (name, rest): (String, MultiValue)| {
            aa_ui::lua::queue_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })?,
    )?;
    lua.globals().raw_set("FAKE", fake)?;
    // 2. FALLBACK: the fake in-game data
    lua.load(FAKE_INGAME).set_name("@fake_ingame.lua").exec()?;
    // 3. the real data
    let hud = lua.create_table()?;
    install_functions(lua, &hud, bridge)?;
    lua.globals().raw_set("HUD", hud.clone())?;
    push_snapshot(lua, &hud, &bridge.borrow().snapshot)?;
    panels::push(lua, &hud, &bridge.borrow().panels, None)?;
    lua.load(INGAME_LUA).set_name("@ingame.lua").exec()
}

/// The Rust functions of the `HUD` table.
fn install_functions(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    // the player's game position and facing (changes every frame while moving: not in the snapshot)
    let b = bridge.clone();
    hud.raw_set(
        "player_pos",
        lua.create_function(move |_, ()| {
            let d = b.borrow();
            Ok((d.pos.x as f64, d.pos.y as f64, d.pos.z as f64, d.yaw as f64))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "faction_name",
        lua.create_function(move |_, id: u32| Ok(b.borrow().game.get().and_then(|g| g.factions.get(&id).cloned())))?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "request",
        lua.create_function(move |_, args: MultiValue| {
            let text = |i: usize| match args.iter().nth(i) {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => String::new(),
            };
            let num = |i: usize| match args.iter().nth(i) {
                Some(Value::Integer(n)) => *n,
                Some(Value::Number(n)) => *n as i64,
                _ => 0,
            };
            let request = match text(0).as_str() {
                "chat" => Request::Chat { chat_type: num(1) as i16, message: text(2), target: text(3) },
                other => {
                    warn!("hud: unknown request {other:?}");
                    return Ok(());
                }
            };
            b.borrow_mut().requests.push(request);
            Ok(())
        })?,
    )?;
    // the HUD's clock: buff and cooldown end times are on it
    let b = bridge.clone();
    hud.raw_set("now_ms", lua.create_function(move |_, ()| Ok(panels::now_ms(b.borrow().start) as f64))?)?;
    // metres from the player to the target
    let b = bridge.clone();
    hud.raw_set(
        "target_distance",
        lua.create_function(move |_, ()| {
            let d = b.borrow();
            Ok(d.target_pos.map(|t| t.distance(d.pos) as f64))
        })?,
    )?;
    // database lookups for the content hooks: name, icon (and the rest)
    let b = bridge.clone();
    hud.raw_set(
        "skill_info",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let info = d.game.get().and_then(|g| g.skills.get(&id));
            Ok((info.map(|s| s.named.name.clone()), info.map(|s| s.named.icon.clone())))
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "item_info",
        lua.create_function(move |_, id: u32| {
            let d = b.borrow();
            let info = d.game.get().and_then(|g| g.items.get(&id));
            Ok((info.map(|i| i.named.name.clone()), info.map(|i| i.named.icon.clone()), info.map(|i| i.max_stack), info.map(|i| i.fixed_grade)))
        })?,
    )?;
    // the chat window widget (the scripts create it; `GetEntityByName` is a stub)
    hud.raw_set(
        "chat_window",
        lua.create_function(|lua, ()| {
            let id = {
                let u = aa_ui::lua::ui(lua);
                u.tree.objects.iter().enumerate().find(|(_, o)| o.alive && o.widget().is_some_and(|w| w.class == "chatwindow")).map(|(i, _)| i as aa_ui::tree::Id)
            };
            match id {
                Some(id) => aa_ui::lua::object_table(lua, id),
                None => Ok(None),
            }
        })?,
    )?;
    Ok(())
}

/// The snapshot as the Lua table `HUD.state`.
fn push_snapshot(lua: &Lua, hud: &Table, s: &Snapshot) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.raw_set("unit_id", s.unit_id.as_str())?;
    t.raw_set("name", s.name.as_str())?;
    t.raw_set("level", s.level as i64)?;
    t.raw_set("race", s.race as i64)?;
    t.raw_set("gender", s.gender as i64)?;
    t.raw_set("hp", s.hp as i64)?;
    t.raw_set("max_hp", s.max_hp as i64)?;
    t.raw_set("mp", s.mp as i64)?;
    t.raw_set("max_mp", s.max_mp as i64)?;
    t.raw_set("exp_percent", s.exp_percent)?;
    t.raw_set("dead", s.dead)?;
    t.raw_set("in_combat", s.in_combat)?;
    t.raw_set("recoverable_exp", s.recoverable_exp as i64)?;
    t.raw_set("honor", s.honor as i64)?;
    t.raw_set("vocation", s.vocation as i64)?;
    t.raw_set("exp_cur", s.exp_cur.to_string())?;
    t.raw_set("exp_need", s.exp_need.to_string())?;
    t.raw_set("labor", s.labor as i64)?;
    t.raw_set("max_labor", s.max_labor as i64)?;
    t.raw_set("money", s.money.to_string())?;
    t.raw_set("bm_point", s.bm_point.to_string())?;
    t.raw_set("zone", s.zone.as_str())?;
    t.raw_set("zone_group", s.zone_group as i64)?;
    t.raw_set("zone_key_name", s.zone_key_name.as_str())?;
    t.raw_set("zone_rect", lua.create_sequence_from(s.zone_rect.iter().map(|v| *v as f64))?)?;
    t.raw_set("zone_faction", s.zone_faction as i64)?;
    t.raw_set("zone_faction_name", s.zone_faction_name.as_str())?;
    t.raw_set("sub_zone", s.sub_zone.as_str())?;
    t.raw_set("faction", s.faction.as_str())?;
    t.raw_set("hour", s.hour as i64)?;
    t.raw_set("minute", s.minute as f64)?;
    hud.raw_set("state", t)
}

/// The local player's entity: `ingame::player::spawn_player` names it "player <name>".
fn find_player(world: &mut World, name: &str) -> Option<Entity> {
    let wanted = format!("player {name}");
    let mut q = world.query::<(Entity, &Name, &Transform)>();
    q.iter(world).find(|(_, n, _)| n.as_str() == wanted).map(|(e, _, _)| e)
}

/// Recompute the snapshot from the Bevy world.
pub fn refresh_snapshot(world: &mut World, bridge: &Shared) {
    let Some(spawn) = world.resource::<Player>().spawn.clone() else { return };
    let hour = world.get_resource::<aa_world::WorldClock>().map_or(12.0, |c| c.hour);
    // where the player is (game coordinates), from its entity's transform
    let known = bridge.borrow().player_entity;
    let entity = known.filter(|e| world.get::<Transform>(*e).is_some()).or_else(|| find_player(world, spawn.name()));
    let (pos, yaw) = match entity.and_then(|e| world.get::<Transform>(e)) {
        Some(t) => (aa_world::bevy_to_cry(t.translation), t.rotation.to_euler(EulerRot::YXZ).0),
        None => {
            let p = spawn.position();
            (Vec3::new(p.x, p.y, p.z), spawn.yaw())
        }
    };
    let zone_key = world
        .get_resource::<aa_world::stream::WorldInfo>()
        .and_then(|w| w.0.zone_at(pos))
        .map_or(spawn.zone_id(), u32::from);
    let mut data = bridge.borrow_mut();
    data.player_entity = entity;
    data.pos = pos;
    data.yaw = yaw;
    let character = spawn.character();
    let game = data.game.clone();
    let game = game.get();
    // the numbers the server keeps current (aa-net's PlayerStats), the spawn's values until it has them
    let stats = world.get_resource::<PlayerStats>().filter(|s| s.obj_id != 0);
    let level = stats.map_or(character.level, |s| s.level).max(1);
    let (exp_cur, exp_need) = stats.map_or((0, 1), |s| s.exp_progress());
    let exp_percent = if exp_need > 0 { 100.0 * exp_cur as f64 / exp_need as f64 } else { 100.0 };
    let faction_name = |g: &super::data::GameData, id: u32| g.factions.get(&id).cloned().filter(|f| !f.is_empty());
    let faction = game.and_then(|g| faction_name(g, character.faction_id)).unwrap_or_else(|| character.faction_name.clone());
    let zone = game.and_then(|g| g.zone(zone_key));
    let snapshot = Snapshot {
        unit_id: spawn.obj_id.to_string(),
        name: spawn.name().to_string(),
        level,
        race: character.race,
        gender: character.gender,
        hp: stats.map_or(character.hp, |s| s.hp),
        max_hp: stats.map_or(character.hp, |s| s.max_hp).max(1),
        mp: stats.map_or(character.mp, |s| s.mp),
        max_mp: stats.map_or(character.mp, |s| s.max_mp).max(1),
        dead: stats.is_some_and(|s| s.dead),
        in_combat: stats.is_some_and(|s| s.in_combat),
        exp_percent,
        exp_cur,
        exp_need,
        recoverable_exp: stats.map_or(0, |s| s.recoverable_exp),
        labor: stats.map_or(i32::from(character.labor_power), |s| s.labor),
        max_labor: LABOR_MAX,
        money: stats.map_or(character.money, |s| s.money),
        bm_point: stats.map_or(character.bm_point, |s| s.loyalty),
        honor: stats.map_or(0, |s| s.honor),
        vocation: stats.map_or(0, |s| s.vocation),
        zone: zone.map_or_else(String::new, |(_, g, _)| g.text.clone()),
        zone_group: zone.map_or(0, |(id, _, _)| id),
        zone_key_name: zone.map_or_else(String::new, |(_, g, _)| g.key.clone()),
        zone_rect: zone.map_or([0.0; 4], |(_, g, _)| g.rect),
        zone_faction: zone.map_or(0, |(_, _, f)| f),
        zone_faction_name: zone.and_then(|(_, _, f)| game.and_then(|g| faction_name(g, f))).unwrap_or_default(),
        sub_zone: String::new(),
        faction,
        hour: hour.floor() as u8 % 24,
        minute: (hour.fract() * 60.0).floor(),
    };
    data.snapshot = snapshot;
    let own_obj_id = spawn.obj_id;
    let own_faction = character.faction_id;
    let start = data.start;
    let game = data.game.clone();
    let (new_panels, target_pos) = panels::refresh(world, game.get(), own_faction, own_obj_id, start);
    data.panels = new_panels;
    data.target_pos = target_pos;
}

/// The events the client raises when the player has entered the world and the loading screen closed,
/// then the chat channels joined and the lines received so far.
pub fn enter_world(rt: &UiRuntime, bridge: &Shared, chat: &aa_net::state::Chat) {
    if let Err(e) = rt.exec("@enter_world", "HUD.enter_world()") {
        warn!("hud: enter world events: {e}");
    }
    let data = bridge.borrow();
    super::chat::replay(rt, chat, data.game.get());
}

/// Carry out what the scripts asked for.
pub fn handle_requests(bridge: NonSend<HudBridge>, mut net: MessageWriter<NetCommand>) {
    let requests = std::mem::take(&mut bridge.0.borrow_mut().requests);
    for request in requests {
        match request {
            Request::Chat { chat_type, message, target } => {
                net.write(NetCommand(Command::Chat { chat_type, target, message }));
            }
        }
    }
}

/// The stage was built from the current snapshot.
pub fn mark_pushed(bridge: &Shared) {
    let mut d = bridge.borrow_mut();
    d.pushed = Some(d.snapshot.clone());
    d.panels_pushed = Some(d.panels.clone());
}

/// Push what changed into the Lua state and raise the events the scripts listen for.
pub fn push_changes(rt: &UiRuntime, bridge: &Shared) {
    push_state(rt, bridge);
    push_panels(rt, bridge);
}

fn push_panels(rt: &UiRuntime, bridge: &Shared) {
    let (old, new) = {
        let mut d = bridge.borrow_mut();
        if d.panels_pushed.as_ref() == Some(&d.panels) {
            return;
        }
        let new = d.panels.clone();
        (d.panels_pushed.replace(new.clone()), new)
    };
    let Some(old) = old else { return };
    let lua = &rt.lua;
    let Ok(hud) = lua.globals().raw_get::<Table>("HUD") else { return };
    let changed = match panels::push(lua, &hud, &new, Some(&old)) {
        Ok(c) => c,
        Err(e) => {
            warn!("hud: panels: {e}");
            return;
        }
    };
    if changed.slots {
        if let Err(e) = rt.refresh_slots() {
            warn!("hud: refresh slots: {e}");
        }
    }
    let id = |t: &Option<panels::TargetSnap>| t.as_ref().map(|t| t.obj_id);
    if id(&old.target) != id(&new.target) {
        match &new.target {
            Some(t) => rt.fire_event("TARGET_CHANGED", (t.obj_id.to_string(), "npc")),
            None => rt.fire_event("TARGET_CHANGED", (mlua::Value::Nil, mlua::Value::Nil)),
        }
    } else if let (Some(o), Some(n)) = (&old.target, &new.target) {
        if (o.hp, o.max_hp) != (n.hp, n.max_hp) {
            rt.fire_event("TARGET_HEALTH_CHANGED", (n.hp as i64, n.max_hp as i64));
        }
        if (o.mp, o.max_mp) != (n.mp, n.max_mp) {
            rt.fire_event("TARGET_MANA_CHANGED", (n.mp as i64, n.max_mp as i64));
        }
    }
}

fn push_state(rt: &UiRuntime, bridge: &Shared) {    let (old, new) = {
        let mut d = bridge.borrow_mut();
        if d.pushed.as_ref() == Some(&d.snapshot) {
            return;
        }
        let new = d.snapshot.clone();
        (d.pushed.replace(new.clone()), new)
    };
    let Some(old) = old else { return };
    let lua = &rt.lua;
    let Ok(hud) = lua.globals().raw_get::<Table>("HUD") else { return };
    if let Err(e) = push_snapshot(lua, &hud, &new) {
        warn!("hud: state: {e}");
        return;
    }
    let id = new.unit_id.as_str();
    if (old.hp, old.max_hp) != (new.hp, new.max_hp) {
        rt.fire_event("PLAYER_HEALTH_CHANGED", (new.hp as i64, new.max_hp as i64));
    }
    if (old.mp, old.max_mp) != (new.mp, new.max_mp) {
        rt.fire_event("PLAYER_MANA_CHANGED", (new.mp as i64, new.max_mp as i64));
    }
    if old.level != new.level {
        rt.fire_event("LEVEL_CHANGED", (new.level as i64, id));
    }
    if old.exp_cur != new.exp_cur || old.exp_need != new.exp_need {
        rt.fire_event("EXP_CHANGED", (id, new.exp_cur, new.exp_cur.to_string()));
    }
    if old.labor != new.labor {
        rt.fire_event("LABORPOWER_CHANGED", ((new.labor - old.labor) as i64, new.labor as i64));
    }
    if old.in_combat != new.in_combat {
        rt.fire_event("UNIT_COMBAT_STATE_CHANGED", (new.in_combat, id));
    }
    if old.recoverable_exp != new.recoverable_exp {
        rt.fire_event("RECOVERABLE_EXP", (id, new.recoverable_exp as i64));
    }
    if old.money != new.money {
        rt.fire_event("PLAYER_MONEY", (new.money.to_string(), (new.money - old.money).to_string()));
    }
}
