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

use aa_net::state::Player;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table};

use super::data::{SharedGameData, Snapshot};
use crate::login::gameapi;

/// What a script asked the app to do (queued by `HUD.request(kind, ...)`).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {}

/// The state shared between the Bevy systems and the Lua closures (single threaded).
#[derive(Default)]
pub struct HudData {
    pub snapshot: Snapshot,
    /// What the Lua state has seen last.
    pub pushed: Option<Snapshot>,
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
        lua.create_function(move |_, _args: MultiValue| {
            let _ = &b;
            Ok(())
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
    let prev = &data.snapshot;
    let level = character.level.max(1);
    let (exp_cur, exp_need) = game.map_or((0, 1), |g| g.level_progress(level, spawn.state.experience as i64));
    let faction_name = |g: &super::data::GameData, id: u32| g.factions.get(&id).cloned().filter(|f| !f.is_empty());
    let faction = game.and_then(|g| faction_name(g, character.faction_id)).unwrap_or_else(|| character.faction_name.clone());
    let zone = game.and_then(|g| g.zone(zone_key));
    // the maxima are not on the wire yet: the highest seen so far, from the spawn values up
    let max_hp = prev.max_hp.max(character.hp).max(1);
    let max_mp = prev.max_mp.max(character.mp).max(1);
    let snapshot = Snapshot {
        unit_id: spawn.obj_id.to_string(),
        name: spawn.name().to_string(),
        level,
        race: character.race,
        gender: character.gender,
        hp: character.hp,
        max_hp,
        mp: character.mp,
        max_mp,
        exp_percent: 100.0 * exp_cur as f64 / exp_need as f64,
        exp_cur,
        exp_need,
        labor: character.labor_power as i32,
        max_labor: 5000,
        money: character.money,
        bm_point: character.bm_point,
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
}

/// The events the client raises when the player has entered the world and the loading screen closed.
pub fn enter_world(rt: &UiRuntime, bridge: &Shared) {
    if let Err(e) = rt.exec("@enter_world", "HUD.enter_world()") {
        warn!("hud: enter world events: {e}");
    }
    let _ = bridge;
}

/// The stage was built from the current snapshot.
pub fn mark_pushed(bridge: &Shared) {
    let mut d = bridge.borrow_mut();
    d.pushed = Some(d.snapshot.clone());
}

/// Push a changed snapshot into the Lua state and raise the events the scripts listen for.
pub fn push_changes(rt: &UiRuntime, bridge: &Shared) {
    let (old, new) = {
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
    if old.money != new.money {
        rt.fire_event("PLAYER_MONEY", (new.money.to_string(), (new.money - old.money).to_string()));
    }
}
