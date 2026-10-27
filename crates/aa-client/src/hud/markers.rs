//! The icons of the roadmap (top right) and of the big map (M): quest "!" and "?", the bank, the auction
//! house, stables, mailboxes, crafting stations, housing province signs, portals, the gravesite and the
//! dots of the units in view. The world map turned to another zone shows that zone's service icons too
//! (`HUD.map_atlas`, placed once from every zone's map files).
//!
//! The client's own data says where everything is. `worlds/main_world/map_data/npc_map/<zone>.dat` and
//! `doodad_map/<zone>.dat` (16 byte records: template id, x, y, z as f32 in game metres) list every NPC
//! and every map-worthy doodad of a zone group, so a bank or mailbox far outside the range the server
//! shows units in still has its icon. The quest marks are the other way round: they belong to the units
//! the client has, as the 3D marks over NPCs do ([`super::marks`]); `reference/ingame/spawn` shows the
//! two quest givers within 120 m of the player and `reference/ingame/worldmap_zone` none at Desireen,
//! 170 m away. What an NPC or doodad is drawn as comes from the game database: the NPC's service flags
//! (`banker`, `auctioneer`, `stabler` ...), the doodad's group (`doodad_groups`).
//!
//! What this module hands over is a list of icons in game coordinates, `HUD.map_icons` (rebuilt when
//! something changes: the zone, the player's quests, the dots twice a second), and `markers.lua` does
//! the rest: it keeps the filters of the world map window (`X2Map:SetNpcFilter` ...), places the icons
//! on the zone or city map as the roadmap and the big map show them and gives them to
//! `AAUI.RoadMap().markers` and `AAUI.WorldMap().markers` (aa-ui draws them).
//!
//! Checked against `reference/ingame/worldmap_zone` (the bank, the mailbox and the stables of Desireen,
//! the pumpkin farm zone sign west of it): the weapon, armor and general merchants and the priestess
//! have no icon there, so they get none here. The memory tomes do (the blue book, mostly hidden behind
//! the bank chest).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use aa_net::model::UnitKind;
use aa_net::state::{Buffs, Inventory, Player, PlayerStats, Quests, Skills, Units};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::{Connection, OpenFlags};

use super::api::HudBridge;
use super::data::{GameData, any_flag, any_int};
use super::quests::{Mark, PlayerFacts};
use crate::Screen;

/// Frames between two looks at what the icons are (the quests and the units in view change slowly).
const REFRESH_FRAMES: u32 = 30;
/// How far from the player the dots of the units in view reach, metres (the roadmap shows about 2400).
const DOT_RANGE: f32 = 250.0;
/// A unit this close to the NPC of the map file is that NPC (metres).
const SAME_SPOT: f32 = 20.0;
/// The map's "NPC" check (`roadmap:SetRoadMapNpc`) is on: the dots are worth the work (set by `HUD.map_dots`).
static DOTS: AtomicBool = AtomicBool::new(false);

/// One icon of the map, in game coordinates.
#[derive(Clone, Debug, PartialEq)]
struct Icon {
    x: f32,
    y: f32,
    /// A symbol of the client's map icon sheet (`quest_zone_gives`, `npc_store_bank` ...).
    kind: &'static str,
    /// What the tooltip says; empty: the symbol's own text.
    tip: String,
    /// The city (sub zone with a map of its own) the icon is in, 0 for none.
    city: u32,
}

/// A service icon of another zone than the player's, placed once on the maps it shows on: the world map shows the
/// banks, mailboxes, crafting stations and portals of every zone it is turned to, while the units, the quests and
/// the party exist only around the player (they stay in [`Icon`]).
#[derive(Clone, Debug, PartialEq)]
struct AtlasIcon {
    /// `zone_groups.id` of the zone map, and the place on it (fractions of its map rectangle, v down).
    group: u32,
    u: f32,
    v: f32,
    /// The city map it is on too (0 for none), and the place on that.
    city: u32,
    cu: f32,
    cv: f32,
    kind: &'static str,
    tip: String,
}

/// A place in a map file: template id and position.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    id: u32,
    x: f32,
    y: f32,
    city: u32,
}

/// The records of an `npc_map` / `doodad_map` file.
fn parse_spots(bytes: &[u8]) -> Vec<Spot> {
    bytes
        .chunks_exact(16)
        .map(|r| {
            let f = |i: usize| f32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
            Spot { id: u32::from_le_bytes([r[0], r[1], r[2], r[3]]), x: f(4), y: f(8), city: 0 }
        })
        .collect()
}

/// The icon of a doodad group (`doodad_almighties.group_id`); `None`: the map has none for it.
fn doodad_icon(group: u32) -> Option<&'static str> {
    Some(match group {
        6 => "doodad_mail",
        7 => "doodad_alchemy",
        9 => "doodad_cook",
        10 => "doodad_craft",
        16 => "doodad_machinery",
        17 => "doodad_metal",
        18 => "doodad_coatofarms",
        19 => "doodad_stone",
        20 => "doodad_fabric",
        21 => "doodad_leather",
        22 => "doodad_weapon",
        23 => "doodad_timber",
        // the portal of Mirage Isle (the teal swirl beside the bank of Lacton in the capture), the world gates, the dungeon doors
        44 => "doodad_portal_archemall",
        43 => "doodad_portal",
        45 | 47 => "doodad_portal_dungeon",
        46 => "doodad_special_product",
        48 => "doodad_archium",
        49 => "doodad_light_armor",
        50 => "doodad_leather_armor",
        51 => "doodad_woodwork",
        52 => "doodad_heavy_armor",
        // the memory tomes: the blue book half hidden behind the bank chest at Desireen and Lacton in the capture
        53 => "doodad_inn",
        60 => "house_normal_house",
        61 => "house_sea_farm_house",
        62 => "house_high_house",
        63 => "house_pumpkin_house",
        64 => "house_farm_house",
        66 => "doodad_paper",
        67 => "doodad_print",
        71 => "doodad_fish_stand",
        78 => "house_benny_house",
        79 => "house_bungalow_house",
        83 => "doodad_art_work",
        _ => return None,
    })
}

/// The icon of an NPC's service, from its flags in `npcs` (`trader`: the trade goods and relic sellers).
fn service_icon(banker: bool, auctioneer: bool, stabler: bool, expedition: bool, trader: bool, ability_changer: bool, priest: bool) -> Option<&'static str> {
    if banker {
        Some("npc_store_bank")
    } else if auctioneer {
        Some("npc_store_auction_house")
    } else if stabler {
        Some("npc_stabler")
    } else if expedition {
        Some("npc_store_expedition")
    } else if trader {
        Some("npc_store_trade")
    } else if ability_changer && !priest {
        Some("npc_ability_changer")
    } else {
        None
    }
}

/// What the map's icons come from in the game database, read once in the background.
#[derive(Default)]
struct MapTables {
    /// NPC template -> the icon of the service it gives.
    npcs: HashMap<u32, &'static str>,
    /// Doodad template -> its icon and English name.
    doodads: HashMap<u32, (&'static str, String)>,
}

impl MapTables {
    fn load(path: &str) -> rusqlite::Result<MapTables> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut t = MapTables::default();
        {
            let mut q = conn.prepare("SELECT id, banker, auctioneer, stabler, expedition, trader, ability_changer, priest, specialty, npc_nickname_id FROM npcs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let flag = |i: usize| any_flag(r, i);
                if let Some(icon) = service_icon(flag(1)?, flag(2)?, flag(3)?, flag(4)?, flag(5)? || flag(8)?, flag(6)?, flag(7)?).or_else(|| super::map_services::nickname_icon(any_int(r, 9).unwrap_or(0))) {
                    t.npcs.insert(any_int(r, 0)? as u32, icon);
                }
            }
        }
        let mut names: HashMap<u32, String> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'doodad_almighties' AND tbl_column_name = 'name' AND en_us IS NOT NULL AND en_us <> ''")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                names.insert(any_int(r, 0)? as u32, r.get(1)?);
            }
        }
        {
            let mut q = conn.prepare("SELECT id, group_id FROM doodad_almighties")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)? as u32;
                if let Some(icon) = doodad_icon(any_int(r, 1)? as u32) {
                    t.doodads.insert(id, (icon, names.remove(&id).unwrap_or_default()));
                }
            }
        }
        Ok(t)
    }
}

/// The map files of the zone group the player is in.
#[derive(Default)]
struct ZoneSpots {
    key: String,
    npcs: Vec<Spot>,
    doodads: Vec<Spot>,
}

impl ZoneSpots {
    fn load(key: &str, game: &GameData) -> ZoneSpots {
        let pak = aa_pak::Pak::global();
        let read = |dir: &str| {
            let mut spots = pak.read(&format!("worlds/main_world/map_data/{dir}/{key}.dat")).map(|b| parse_spots(&b)).unwrap_or_default();
            for s in &mut spots {
                s.city = city_at(game, s.x, s.y);
            }
            spots
        };
        ZoneSpots { key: key.to_string(), npcs: read("npc_map"), doodads: read("doodad_map") }
    }
}

/// The service icons of every zone group with a map, from its map files (`worlds/main_world/map_data/*/<key>.dat`).
fn build_atlas(tables: &MapTables, game: &GameData) -> Vec<AtlasIcon> {
    let pak = aa_pak::Pak::global();
    let read = |dir: &str, key: &str| pak.read(&format!("worlds/main_world/map_data/{dir}/{key}.dat")).map(|b| parse_spots(&b)).unwrap_or_default();
    let fraction = |r: &[f32; 4], x: f32, y: f32| {
        let (u, v) = ((x - r[0]) / r[2], 1.0 - (y - r[1]) / r[3]);
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
    };
    let mut groups: Vec<_> = game.zone_groups.iter().filter(|(_, g)| g.rect[2] > 0.0 && g.rect[3] > 0.0).collect();
    groups.sort_by_key(|(id, _)| **id);
    let mut out = Vec::new();
    for (&group, g) in groups {
        // the doodads, then the NPCs on top (as `build_icons` orders them)
        let doodads = read("doodad_map", &g.key).into_iter().filter_map(|s| tables.doodads.get(&s.id).map(|(k, n)| (s, *k, n.clone())));
        let npcs = read("npc_map", &g.key).into_iter().filter_map(|s| {
            let name = game.npcs.get(&s.id).map(|n| n.name.clone()).unwrap_or_default();
            tables.npcs.get(&s.id).map(|k| (s, *k, name))
        });
        // the resurrection points have no template: the places of `graveyards.tsv` inside this map
        let graves = super::map_services::graveyards_in(g.rect).map(|gy| (Spot { id: 0, x: gy.x, y: gy.y, city: 0 }, "npc_graveyard", gy.name.to_string()));
        for (s, kind, tip) in doodads.collect::<Vec<_>>().into_iter().chain(npcs).chain(graves) {
            let Some((u, v)) = fraction(&g.rect, s.x, s.y) else { continue };
            let city = city_at(game, s.x, s.y);
            let (city, cu, cv) = match game.sub_zone_rects.get(&city).and_then(|r| fraction(r, s.x, s.y)) {
                Some((cu, cv)) => (city, cu, cv),
                None => (0, 0.0, 0.0),
            };
            out.push(AtlasIcon { group, u, v, city, cu, cv, kind, tip });
        }
    }
    out
}

/// `HUD.map_atlas` (`group`, `u`, `v`, `kind`, `tip`, and `city`, `cu`, `cv` when the icon is in a city) and
/// `HUD.map_atlas_version`.
fn push_atlas(lua: &Lua, atlas: &[AtlasIcon], version: u32) -> mlua::Result<()> {
    let hud: Table = lua.globals().raw_get("HUD")?;
    let list = lua.create_table_with_capacity(atlas.len(), 0)?;
    for (i, ic) in atlas.iter().enumerate() {
        let t = lua.create_table_with_capacity(0, 8)?;
        t.raw_set("group", ic.group)?;
        t.raw_set("u", f64::from(ic.u))?;
        t.raw_set("v", f64::from(ic.v))?;
        t.raw_set("kind", ic.kind)?;
        t.raw_set("tip", ic.tip.as_str())?;
        if ic.city != 0 {
            t.raw_set("city", ic.city)?;
            t.raw_set("cu", f64::from(ic.cu))?;
            t.raw_set("cv", f64::from(ic.cv))?;
        }
        list.raw_set(i + 1, t)?;
    }
    hud.raw_set("map_atlas", list)?;
    hud.raw_set("map_atlas_version", version)
}

/// The city (a sub zone with a map of its own: the road map of a city, a cave) around a place, 0 for none.
fn city_at(game: &GameData, x: f32, y: f32) -> u32 {
    game.sub_zone_root_at(x, y).filter(|id| game.sub_zone_rects.contains_key(id)).unwrap_or(0)
}

#[derive(Resource, Default)]
pub struct MapIcons {
    tables: Arc<OnceLock<MapTables>>,
    loading: bool,
    zone: ZoneSpots,
    /// Where the player died while exp can still be won back (the gravesite), and whether they were dead a frame ago.
    corpse: Option<Vec2>,
    was_dead: bool,
    refresh_in: u32,
    icons: Vec<Icon>,
    /// The service icons of all zones (built once the tables are read; never changes after).
    atlas: Option<Vec<AtlasIcon>>,
    /// Bumped with every new list: what the Lua state has seen last.
    version: u32,
}

/// `HUD.map_dots(on)`: the "NPC" check of the roadmap options (a result of the scripts' option data).
pub fn install(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    hud.raw_set("map_dots", lua.create_function(|_, on: Option<bool>| Ok(DOTS.store(on.unwrap_or(false), Ordering::Relaxed)))?)
}

/// `markers.lua`: the filters, the places and the hooks of the toolkit (after `ingame.lua`).
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("markers.lua")).set_name("@markers.lua").exec()
}

pub fn plugin(app: &mut App) {
    app.init_resource::<MapIcons>()
        .add_systems(OnExit(Screen::InGame), |mut icons: ResMut<MapIcons>| {
            let (tables, atlas) = (icons.tables.clone(), icons.atlas.take());
            *icons = MapIcons { tables, atlas, loading: icons.loading, ..default() };
        })
        .add_systems(Update, update_icons.after(super::sync_hud).run_if(super::hud_up).run_if(in_state(Screen::InGame)));
}

/// Quest icon of an NPC for the player: the kind of the quests it offers, takes in or waits for.
fn quest_icon(game: &GameData, facts: &PlayerFacts, quests: &Quests, npc: u32, mark: Mark) -> &'static str {
    let defs = |ids: &[u32]| ids.iter().filter_map(|q| game.quests.def(*q)).collect::<Vec<_>>();
    match mark {
        Mark::Available => {
            let offered = defs(&game.quests.offers(npc, facts));
            if offered.iter().any(|d| d.is_main()) {
                "quest_main_gives"
            } else if offered.iter().any(|d| !d.is_daily() && !d.is_livelihood()) {
                "quest_zone_gives"
            } else if offered.iter().any(|d| d.is_livelihood()) {
                "quest_livelihood_gives"
            } else {
                "quest_repeat"
            }
        }
        Mark::Ready => {
            let done = defs(&game.quests.completes(npc, facts));
            if done.iter().any(|d| d.is_main()) {
                "quest_main_completes"
            } else if done.iter().any(|d| d.is_livelihood()) {
                "quest_livelihood_complete"
            } else {
                "quest_zone_completes"
            }
        }
        Mark::InProgress => {
            let waiting: Vec<u32> = quests.active.iter().map(|q| q.template_id).filter(|q| game.quests.report_npc(*q) == Some(npc)).collect();
            if defs(&waiting).iter().any(|d| d.is_main()) { "quest_main_progress" } else { "quest_zone_progress" }
        }
    }
}

/// Everything the map shows now.
#[allow(clippy::too_many_arguments)]
fn build_icons(
    tables: &MapTables,
    zone: &ZoneSpots,
    game: &GameData,
    facts: &PlayerFacts,
    quests: &Quests,
    units: &Units,
    me: Vec3,
    own_faction: u32,
    corpse: Option<Vec2>,
) -> Vec<Icon> {
    let mut out = Vec::new();
    let npc_name = |id: u32| game.npcs.get(&id).map(|n| n.name.clone()).unwrap_or_default();
    // the doodads, then the NPCs on top, then the quests
    for s in &zone.doodads {
        if let Some((kind, name)) = tables.doodads.get(&s.id) {
            out.push(Icon { x: s.x, y: s.y, kind, tip: name.clone(), city: s.city });
        }
    }
    // The quest marks belong to the units the client has (as the 3D marks do): the real big map shows no "!" at
    // Desireen, 170 m from where the capture stands, while it shows its bank, mailbox and stables.
    let mut quest_icons = Vec::new();
    let mut quest_givers: Vec<(u32, f32, f32)> = Vec::new();
    let mut by_template: HashMap<u32, Option<&'static str>> = HashMap::new();
    for u in units.iter() {
        let UnitKind::Npc { template_id, .. } = u.kind else { continue };
        let quest = *by_template.entry(template_id).or_insert_with(|| game.quests.mark(template_id, facts).map(|m| quest_icon(game, facts, quests, template_id, m)));
        let Some(kind) = quest else { continue };
        let tip = Some(npc_name(template_id)).filter(|n| !n.is_empty()).unwrap_or_else(|| u.name.clone());
        quest_icons.push(Icon { x: u.pos.x, y: u.pos.y, kind, tip, city: city_at(game, u.pos.x, u.pos.y) });
        quest_givers.push((template_id, u.pos.x, u.pos.y));
    }
    // the NPCs of the map file: their service icon, unless the NPC is in view with a quest icon over it
    for s in &zone.npcs {
        let Some(kind) = tables.npcs.get(&s.id) else { continue };
        if quest_givers.iter().any(|(id, x, y)| *id == s.id && (x - s.x).hypot(y - s.y) < SAME_SPOT) {
            continue;
        }
        out.push(Icon { x: s.x, y: s.y, kind, tip: npc_name(s.id), city: s.city });
    }
    if let Some(g) = game.zone_groups.values().find(|g| g.key == zone.key) {
        for gy in super::map_services::graveyards_in(g.rect) {
            out.push(Icon { x: gy.x, y: gy.y, kind: "npc_graveyard", tip: gy.name.to_string(), city: city_at(game, gy.x, gy.y) });
        }
    }
    if DOTS.load(Ordering::Relaxed) {
        for u in units.iter() {
            let UnitKind::Npc { .. } = u.kind else { continue };
            let here = Vec3::new(u.pos.x, u.pos.y, u.pos.z);
            if u.dead || me.distance(here) > DOT_RANGE {
                continue;
            }
            let kind = match game.relation(own_faction, u.faction_id) {
                "hostile" => "hostile",
                "friendly" => "friendly",
                _ => continue,
            };
            out.push(Icon { x: here.x, y: here.y, kind, tip: u.name.clone(), city: city_at(game, here.x, here.y) });
        }
    }
    out.append(&mut quest_icons);
    if let Some(c) = corpse {
        out.push(Icon { x: c.x, y: c.y, kind: "corpse_pos", tip: String::new(), city: city_at(game, c.x, c.y) });
    }
    out
}

/// The list as the Lua state reads it: `HUD.map_icons` (`x`, `y`, `kind`, `tip`, `city`), the rectangles of
/// the cities in it (`HUD.map_city_rects[id] = { west, south, width, height }`) and `HUD.map_icons_version`.
fn push(lua: &Lua, game: &GameData, icons: &[Icon], version: u32) -> mlua::Result<()> {
    let hud: Table = lua.globals().raw_get("HUD")?;
    let list = lua.create_table_with_capacity(icons.len(), 0)?;
    let cities = lua.create_table()?;
    for (i, ic) in icons.iter().enumerate() {
        let t = lua.create_table_with_capacity(0, 5)?;
        t.raw_set("x", f64::from(ic.x))?;
        t.raw_set("y", f64::from(ic.y))?;
        t.raw_set("kind", ic.kind)?;
        t.raw_set("tip", ic.tip.as_str())?;
        if ic.city != 0 {
            t.raw_set("city", ic.city)?;
            if let Some(r) = game.sub_zone_rects.get(&ic.city) {
                cities.raw_set(ic.city, lua.create_sequence_from(r.iter().map(|v| f64::from(*v)))?)?;
            }
        }
        list.raw_set(i + 1, t)?;
    }
    hud.raw_set("map_icons", list)?;
    hud.raw_set("map_city_rects", cities)?;
    hud.raw_set("map_icons_version", version)
}

#[allow(clippy::too_many_arguments)]
fn update_icons(
    mut state: ResMut<MapIcons>,
    bridge: NonSend<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    units: Res<Units>,
    player: Res<Player>,
    quests: Res<Quests>,
    inventory: Res<Inventory>,
    skills: Res<Skills>,
    stats: Res<PlayerStats>,
    buffs: Res<Buffs>,
    houses: Res<aa_net::Houses>,
    slaves: Res<aa_net::Slaves>,
) {
    let (Some(rt), Some(spawn)) = (rt, player.spawn.as_ref()) else { return };
    let data = bridge.0.borrow();
    let Some(game) = data.game.get() else { return };
    if !state.loading {
        state.loading = true;
        let slot = state.tables.clone();
        crate::task::spawn("hud-map-db", move || {
                let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
                match MapTables::load(&path) {
                    Ok(t) => {
                        let _ = slot.set(t);
                    }
                    Err(e) => {
                        warn!("hud: map icons from {path}: {e}");
                        let _ = slot.set(MapTables::default());
                    }
                }
            });
    }
    let tables = state.tables.clone();
    let Some(tables) = tables.get() else { return };
    if state.atlas.is_none() {
        let start = std::time::Instant::now();
        let atlas = build_atlas(tables, game);
        debug!("hud: map atlas: {} icons in {:.1} ms", atlas.len(), start.elapsed().as_secs_f64() * 1000.0);
        state.atlas = Some(atlas);
    }
    // every stage's Lua state gets the atlas once (version 1: it never changes)
    let atlas_seen = rt.lua.globals().raw_get::<Table>("HUD").and_then(|h| h.raw_get::<Option<u32>>("map_atlas_version")).ok().flatten();
    if atlas_seen.is_none()
        && let Err(e) = push_atlas(&rt.lua, state.atlas.as_deref().unwrap_or_default(), 1)
    {
        warn!("hud: map atlas: {e}");
    }
    // where the player died stays a gravesite while the exp lost can still be won back
    let dead = stats.dead;
    if dead && !state.was_dead {
        state.corpse = Some(Vec2::new(data.pos.x, data.pos.y));
    } else if !dead && stats.recoverable_exp <= 0 && state.corpse.is_some() {
        state.corpse = None;
        state.refresh_in = 0;
    }
    state.was_dead = dead;
    let zone_key = data.snapshot.zone_key_name.as_str();
    if zone_key != state.zone.key {
        state.zone = ZoneSpots::load(zone_key, game);
        state.refresh_in = 0;
        debug!("hud: map {zone_key}: {} npcs, {} doodads", state.zone.npcs.len(), state.zone.doodads.len());
    }
    // what the stage's Lua state has seen (a stage built anew starts without any list)
    let seen = rt.lua.globals().raw_get::<Table>("HUD").and_then(|h| h.raw_get::<Option<u32>>("map_icons_version")).ok().flatten();
    if state.refresh_in > 0 && seen.is_some() {
        state.refresh_in -= 1;
        return;
    }
    state.refresh_in = REFRESH_FRAMES;
    let facts = PlayerFacts::of(spawn, &stats, &inventory, &skills, &buffs, &quests);
    let mut icons = build_icons(tables, &state.zone, game, &facts, &quests, &units, data.pos, spawn.character().faction_id, state.corpse);
    let own_instance = spawn.state.instance_id;
    for m in &data.party.members {
        if m.id == spawn.character().id || m.instance_id != own_instance { continue; }
        // Before the first remote status, only nearby units have a trustworthy position.
        if !m.has_status && units.get(m.obj_id).is_none() { continue; }
        if m.has_status && game.zones.get(&m.zone_id).map(|z| z.0) != Some(data.snapshot.zone_group) { continue; }
        let kind = if !m.online { "offline_party" } else if m.id == data.party.owner_id && !data.party.is_party { "raidteam_owner" }
            else if data.party.is_party { "party" } else { "raidteam" };
        icons.push(Icon { x: m.pos.x, y: m.pos.y, kind, tip: m.name.clone(), city: city_at(game, m.pos.x, m.pos.y) });
    }
    for (x, y, kind, tip) in super::housing::own_house_icons(&houses, spawn.character().id) {
        icons.push(Icon { x, y, kind, tip, city: city_at(game, x, y) });
    }
    if let Some((x, y, kind, tip)) = super::map_slaves::own_slave_icon(&slaves) {
        icons.push(Icon { x, y, kind, tip, city: city_at(game, x, y) });
    }
    if icons != state.icons {
        state.version += 1;
        state.icons = icons;
    } else if seen == Some(state.version) {
        return;
    }
    if let Err(e) = push(&rt.lua, game, &state.icons, state.version) {
        warn!("hud: map icons: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_are_sixteen_byte_records() {
        let mut bytes = Vec::new();
        for (id, x, y, z) in [(3597u32, 15562.45f32, 15354.32f32, 127.96f32), (320, 100.0, 200.0, 3.0)] {
            bytes.extend_from_slice(&id.to_le_bytes());
            for v in [x, y, z] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[1, 2, 3]); // a torn record is dropped
        let spots = parse_spots(&bytes);
        assert_eq!(spots.len(), 2);
        assert_eq!((spots[0].id, spots[0].x, spots[0].y), (3597, 15562.45, 15354.32));
        assert_eq!(spots[1].id, 320);
    }

    #[test]
    fn what_the_map_draws() {
        // a bank, the stables, the mailbox and the farm zone sign of the Nuian spawn; not the priestess
        assert_eq!(service_icon(false, false, false, false, false, true, true), None);
        assert_eq!(service_icon(true, false, false, false, false, false, false), Some("npc_store_bank"));
        assert_eq!(service_icon(false, false, true, false, false, false, false), Some("npc_stabler"));
        assert_eq!(doodad_icon(6), Some("doodad_mail"));
        assert_eq!(doodad_icon(63), Some("house_pumpkin_house"));
        assert_eq!(doodad_icon(53), Some("doodad_inn"), "the memory tome");
        assert_eq!(doodad_icon(34), None, "a test doodad");
    }

    /// The real tables and the real map file of Solzreed Peninsula, when the game database is there.
    #[test]
    fn solzreed_has_a_bank_and_a_mailbox() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(t) = MapTables::load(&path) else { return };
        let Ok(bytes) = aa_pak::Pak::global().read("worlds/main_world/map_data/npc_map/w_solzreed.dat") else { return };
        let spots = parse_spots(&bytes);
        assert!(spots.iter().any(|s| t.npcs.get(&s.id) == Some(&"npc_store_bank")));
        assert!(spots.iter().any(|s| t.npcs.get(&s.id) == Some(&"npc_stabler")));
        assert_eq!(t.doodads.get(&320).map(|d| d.0), Some("doodad_mail"));
        assert_eq!(t.doodads.get(&320).map(|d| d.1.as_str()), Some("Mailbox"));
    }

    /// The world map turned to another zone shows that zone's banks and mailboxes (real data, when it is there).
    #[test]
    fn atlas_covers_other_zones() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let (Ok(t), Ok(game)) = (MapTables::load(&path), GameData::load(&path)) else { return };
        let start = std::time::Instant::now();
        let atlas = build_atlas(&t, &game);
        eprintln!("atlas: {} icons in {:?}", atlas.len(), start.elapsed());
        let in_group = |g: u32, kind: &str| atlas.iter().any(|i| i.group == g && i.kind == kind);
        // Solzreed (5), Two Crowns (8), Mahadevi (9): a bank and a mailbox each
        for g in [5, 8, 9] {
            assert!(in_group(g, "npc_store_bank"), "bank in zone group {g}");
            assert!(in_group(g, "doodad_mail"), "mailbox in zone group {g}");
        }
        assert!(atlas.iter().all(|i| (0.0..=1.0).contains(&i.u) && (0.0..=1.0).contains(&i.v)));
        assert!(atlas.iter().any(|i| i.city != 0), "some icons are in a city with a map of its own");
    }

    /// Stations, gravesites and public farms exist in the world atlas (real data, when it is there).
    #[test]
    fn atlas_has_stations_graveyards_and_farms() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let (Ok(t), Ok(game)) = (MapTables::load(&path), GameData::load(&path)) else { return };
        let atlas = build_atlas(&t, &game);
        let count = |kind: &str| atlas.iter().filter(|i| i.kind == kind).count();
        eprintln!("npc_station {}, npc_graveyard {}, common_farm {}", count("npc_station"), count("npc_graveyard"), count("common_farm"));
        assert!(count("npc_graveyard") > 100, "the resurrection points of the zone maps");
        // Desireen's graveyard (Solzreed, zone group 5)
        assert!(atlas.iter().any(|i| i.group == 5 && i.kind == "npc_graveyard" && i.tip == "Desireen"));
        if aa_pak::Pak::global().read("worlds/main_world/map_data/npc_map/w_solzreed.dat").is_ok() {
            assert!(count("npc_station") > 10, "carriage and airship conductors");
            assert!(count("common_farm") > 0, "public farm managers");
        }
    }
}
