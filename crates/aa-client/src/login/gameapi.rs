//! The native game API of the login screens: `X2World`, `X2LoginCharacter`, ... for the scripts of
//! world select, character select and loading, backed by aa-net's state.
//!
//! The scripts call `X2World:GetWorldInfo()`, `X2LoginCharacter:GetLoginCharacterName(i)`, ...
//! (see `gameapi.lua`); those are one-liners over the `NET` table registered here. The data lives in
//! a [`Bridge`] (a copy of aa-net's resources, refreshed by [`Bridge::sync`]) shared by every
//! stage's Lua state; what the scripts ask for ("enter this world", "start the game") is queued as
//! [`Request`]s that the systems of [`super`] carry out.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aa_net::housing::HouseSnapshot;
use aa_net::login::{AccountInfo, WorldList, WorldQueue};
use aa_net::model::{Character, gender, race};
use aa_ui::UiRuntime;
use aa_ui::lua::install_game_stub;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

/// What a script asked the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `Console:ExecuteString("quit")`
    Quit,
    EnterWorld { world_id: u8 },
    RefreshWorlds,
    CancelQueue,
    /// `X2LoginCharacter:ConnectToWorld()`
    Connect,
    /// `X2World:LeaveWorld(EXIT_TO_WORLD_LIST)`
    ToWorldSelect,
    RefreshCharacters,
    /// `ShowSelectedCharacter(index)`, 1-based
    ShowCharacter(usize),
    /// `SelectCharacter(index)`, 1-based: Start
    StartGame(usize),
    /// An empty slot: the create screens
    CreateCharacter,
    /// `DeleteCharacterCheckName(index, typed name)`
    DeleteCharacter { index: usize, name: String },
    CancelDelete(usize),
    /// `RequestLpManageCharacter(index)`, 1-based
    LpManage(usize),
}

/// aa-net's state as the scripts see it, and the requests they queue.
#[derive(Default)]
pub struct Bridge {
    pub worlds: WorldList,
    pub account: Option<AccountInfo>,
    pub queue: Option<WorldQueue>,
    pub characters: Vec<Character>,
    pub race_congestion: [u8; 9],
    /// The character whose labor power the account manages (`SCLpManaged`).
    pub lp_managed: Option<u32>,
    /// The houses of the account's characters (`SCLoginCharInfoHouse`, sent after the list).
    pub houses: Vec<HouseSnapshot>,
    /// Name of the connected world.
    pub world_name: String,
    pub requests: Vec<Request>,
    /// The character the list selects when it is built: the one just created (the client's "last played"
    /// before it ever played; the server's list has no play time for it). Cleared by Start.
    pub prefer: Option<u32>,
    prefer_since: Option<std::time::Instant>,
    names: Names,
}

pub type Shared = Rc<RefCell<Bridge>>;

pub fn new_shared() -> Shared {
    Rc::new(RefCell::new(Bridge::default()))
}

impl Bridge {
    /// Take over aa-net's resources.
    pub fn sync(&mut self, worlds: &aa_net::state::Worlds, characters: &aa_net::state::Characters) {
        self.worlds = worlds.list.clone();
        self.account = worlds.account;
        self.queue = worlds.queue;
        self.characters = characters.list.clone();
        self.race_congestion = characters.race_congestion;
        self.lp_managed = characters.lp_managed;
    }

    /// A house of the character list (`SCLoginCharInfoHouse`); a new list drops the old ones first.
    pub fn add_house(&mut self, house: &HouseSnapshot) {
        self.houses.retain(|h| h.db_id != house.db_id);
        self.houses.push(house.clone());
    }

    /// The world the character list belongs to.
    pub fn set_world(&mut self, id: u8) {
        self.world_name = self.worlds.world(id).map(|w| w.name.clone()).unwrap_or_default();
    }

    /// The character just created: the list selects it.
    pub fn set_prefer(&mut self, id: u32) {
        self.prefer = Some(id);
        self.prefer_since = Some(std::time::Instant::now());
    }

    /// The created character is not in the list yet (the refresh is on its way): the list is not built from a stale one.
    pub fn waiting_for_created(&self) -> bool {
        let missing = self.prefer.is_some_and(|id| !self.characters.iter().any(|c| c.id == id));
        missing && self.prefer_since.is_some_and(|t| t.elapsed().as_secs_f32() < 4.0)
    }

    /// Index (0-based) of the character the list selects first.
    pub fn initial_selection(&self) -> Option<usize> {
        let created = self.prefer.and_then(|id| self.characters.iter().position(|c| c.id == id));
        created.or_else(|| {
            let played = self.characters.iter().enumerate().filter(|(_, c)| c.leave_time > 0);
            played.max_by_key(|(_, c)| c.leave_time).map(|(i, _)| i)
        })
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }
}

/// Zone and faction names from the game database (the scripts show names, the server sends ids).
#[derive(Default)]
struct Names {
    conn: Option<rusqlite::Connection>,
    opened: bool,
    zones: HashMap<u32, String>,
    factions: HashMap<u32, String>,
}

impl Names {
    fn conn(&mut self) -> Option<&rusqlite::Connection> {
        if !self.opened {
            self.opened = true;
            let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
            match rusqlite::Connection::open_with_flags(&path, flags) {
                Ok(c) => self.conn = Some(c),
                Err(e) => warn!("login: cannot open {path}: {e}"),
            }
        }
        self.conn.as_ref()
    }

    /// Display name of the zone group of a zone (`zones.zone_key` is what the character list sends,
    /// the name is `zone_groups.display_text`, English).
    fn zone(&mut self, key: u32) -> String {
        if let Some(n) = self.zones.get(&key) {
            return n.clone();
        }
        let name = self
            .conn()
            .and_then(|c| {
                c.query_row(
                    "SELECT COALESCE(l.en_us, g.display_text) FROM zones z JOIN zone_groups g ON g.id = z.group_id                      LEFT JOIN localized_texts l ON l.tbl_name = 'zone_groups' AND l.tbl_column_name = 'display_text' AND l.idx = g.id                      WHERE z.zone_key = ?1",
                    [key],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
            })
            .unwrap_or_default();
        self.zones.insert(key, name.clone());
        name
    }

    /// The premium grade of a character's premium points (x2game 397a7d10): the highest
    /// `premium_grades.grade_id` whose `point` it reaches, at least 1.
    fn premium_grade(&mut self, point: i32) -> i64 {
        let grade = self.conn().and_then(|c| {
            c.query_row("SELECT MAX(grade_id) FROM premium_grades WHERE point <= ?1", [point], |r| r.get::<_, Option<i64>>(0)).ok().flatten()
        });
        grade.unwrap_or(1).max(1)
    }

    /// `housings.category_id` of a house template (0 when unknown).
    fn housing_category(&mut self, template_id: u32) -> u32 {
        self.conn()
            .and_then(|c| c.query_row("SELECT category_id FROM housings WHERE id = ?1", [template_id], |r| r.get::<_, i64>(0)).ok())
            .unwrap_or(0) as u32
    }

    /// Name of a faction (`system_factions.name`, English).
    fn faction(&mut self, id: u32) -> String {
        if let Some(n) = self.factions.get(&id) {
            return n.clone();
        }
        let name = self
            .conn()
            .and_then(|c| {
                c.query_row(
                    "SELECT COALESCE(l.en_us, f.name) FROM system_factions f \
                     LEFT JOIN localized_texts l ON l.tbl_name = 'system_factions' AND l.tbl_column_name = 'name' AND l.idx = f.id \
                     WHERE f.id = ?1",
                    [id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
            })
            .unwrap_or_default();
        self.factions.insert(id, name.clone());
        name
    }
}

/// `X2LoginCharacter:GetLoginCharacterInfoHouse(char, n)` (x2game 39485c20): owner and house name,
/// position in metres, build steps and the kind the map icon picks (`house`, `farm`, `fishfarm`).
fn house_table(lua: &Lua, h: &HouseSnapshot, category: u32) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("owner", h.owner_name.as_str())?;
    t.set("name", h.name.as_str())?;
    t.set("x", aa_net::wire::long_to_coord(h.x) as i64)?;
    t.set("y", aa_net::wire::long_to_coord(h.y) as i64)?;
    t.set("curstep", h.current_actions)?;
    t.set("allstep", h.all_actions)?;
    // `housings.category_id`: 7 the aquafarm, 16 / 17 the scarecrow gardens and farms
    t.set("type", match category {
        7 => "fishfarm",
        16 | 17 => "farm",
        _ => "house",
    })?;
    Ok(t)
}

/// `X2Unit:GetRaceStr` names by race id.
pub fn race_name(id: u8) -> &'static str {
    match id {
        race::NUIAN => "nuian",
        race::FAIRY => "fairy",
        race::DWARF => "dwarf",
        race::ELF => "elf",
        race::HARIHARAN => "hariharan",
        race::FERRE => "ferre",
        race::RETURNED => "returned",
        _ => "warborn",
    }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Every game API object the scripts of any screen touch: each one exists (a call returns nothing)
/// and the app replaces what it implements.
pub const GAME_OBJECTS: &[&str] = &[
    "X2", "X2Ability", "X2Achievement", "X2Action", "X2Auction", "X2Bag", "X2Bank", "X2BattleField", "X2Book", "X2Bot", "X2Camera",
    "X2Chat", "X2Coffer", "X2Craft", "X2Cursor", "X2Customizer", "X2Debug", "X2Decal", "X2Demo", "X2DialogManager", "X2Dominion",
    "X2Equipment", "X2Faction", "X2Family", "X2Friend", "X2Gm", "X2GoodsMail", "X2Helper", "X2Hotkey", "X2House", "X2InGameShop",
    "X2Input", "X2Interaction", "X2Item", "X2ItemEnchant", "X2ItemLookConverter", "X2Locale", "X2LoginCharacter", "X2Loot", "X2Mail",
    "X2Map", "X2Mate", "X2MiaMail", "X2NameTag", "X2Nation", "X2Option", "X2Player", "X2PremiumService", "X2Priest", "X2Quest",
    "X2Racing", "X2Rank", "X2Security", "X2SiegeWeapon", "X2Skill", "X2Sound", "X2Store", "X2Team", "X2Time", "X2Trade", "X2Trial",
    "X2Ucc", "X2Unit", "X2UserMusic", "X2Util", "X2Warp", "X2World", "Console", "Movie", "DockingManager",
];

/// Install the common game API into a fresh runtime, before any script of the stage runs:
/// the `X2*` objects, the native constants and the `NET` functions with `gameapi.lua`.
pub fn install(rt: &UiRuntime, shared: &Shared) -> mlua::Result<()> {
    let lua = &rt.lua;
    for name in GAME_OBJECTS {
        install_game_stub(lua, name)?;
    }
    let g = lua.globals();
    // Original Lua deadlines and DelayCall share the toolkit's monotonic UI frame clock.
    // Network receipt timestamps are translated into this clock's stage-local epoch.
    let time: Table = g.raw_get("X2Time")?;
    time.set("GetUiMsec", lua.create_function(|lua, _: Table| Ok(aa_ui::lua::ui(lua).time_ms))?)?;
    // `ADDON` is the same native table as `X2`.
    let x2: Table = g.raw_get("X2")?;
    g.raw_set("ADDON", x2)?;
    // aa-ui installs the client's real constants; any other ALL_CAPS global the UI reads but never
    // assigns gets a unique value.
    let mut next = 1000i64;
    for name in include_str!("native_constants.txt").lines().map(str::trim).filter(|l| !l.is_empty()) {
        if g.raw_get::<Value>(name)?.is_nil() {
            g.raw_set(name, next)?;
            next += 1;
        }
    }
    g.raw_set("NET", net_table(lua, shared)?)?;
    // sounds: `AAUI.PlaySound`, `PlayMusic`, `WidgetSound` ... (the HUD installs this first, too)
    crate::sound::bind(lua);
    crate::settings::install_options(lua)?;
    lua.load(include_str!("gameapi.lua")).set_name("@gameapi.lua").exec()?;
    super::name_policy::install(lua)
}

fn int(args: &MultiValue, i: usize) -> Option<i64> {
    match args.iter().nth(i)? {
        Value::Integer(n) => Some(*n),
        Value::Number(n) => Some(*n as i64),
        _ => None,
    }
}

fn string(args: &MultiValue, i: usize) -> String {
    match args.iter().nth(i) {
        Some(Value::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

/// The `NET` table: data getters and `request`.
fn net_table(lua: &Lua, shared: &Shared) -> mlua::Result<Table> {
    let net = lua.create_table()?;
    crate::settings::install(lua, &net)?;

    let s = shared.clone();
    net.set(
        "worlds",
        lua.create_function(move |lua, ()| {
            let b = s.borrow();
            let out = lua.create_table()?;
            for (i, w) in b.worlds.worlds.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", w.id as i64)?;
                t.set("name", w.name.as_str())?;
                t.set("available", w.online)?;
                t.set("congestion", w.congestion as i64)?;
                t.set("characters", b.worlds.character_count(w.id) as i64)?;
                let rc = lua.create_table()?;
                for r in 1..=8u8 {
                    rc.set(race_name(r), w.race_congestion.get(r as usize).copied().unwrap_or(0) as i64)?;
                }
                t.set("raceCongestions", rc)?;
                out.raw_set(i + 1, t)?;
            }
            Ok(out)
        })?,
    )?;

    let s = shared.clone();
    net.set(
        "account",
        lua.create_function(move |lua, ()| {
            let b = s.borrow();
            let t = lua.create_table()?;
            let a = b.account.unwrap_or(AccountInfo { max_characters: 6, ..Default::default() });
            t.set("max_characters", a.max_characters as i64)?;
            t.set("extra_characters", a.extra_characters_per_world as i64)?;
            t.set("pre_select", a.pre_select_period)?;
            t.set("total_characters", b.worlds.characters.len() as i64)?;
            Ok(t)
        })?,
    )?;

    let s = shared.clone();
    net.set(
        "queue",
        lua.create_function(move |lua, ()| {
            let b = s.borrow();
            let Some(q) = b.queue else { return Ok(Value::Nil) };
            let t = lua.create_table()?;
            t.set("position", q.my_turn as i64)?;
            t.set("normal", q.normal_length as i64)?;
            t.set("premium_length", q.premium_length as i64)?;
            t.set("premium", q.premium)?;
            Ok(Value::Table(t))
        })?,
    )?;

    let s = shared.clone();
    net.set(
        "chars",
        lua.create_function(move |lua, ()| {
            let mut b = s.borrow_mut();
            let now = unix_now();
            let out = lua.create_table()?;
            let list = b.characters.clone();
            let all_houses = b.houses.clone();
            for (i, c) in list.iter().enumerate() {
                let t = lua.create_table()?;
                t.set("id", c.id as i64)?;
                t.set("name", c.name.as_str())?;
                t.set("level", c.level as i64)?;
                t.set("race", race_name(c.race))?;
                t.set("gender", if c.gender == gender::FEMALE { "female" } else { "male" })?;
                t.set("abilities", lua.create_sequence_from(c.abilities.iter().map(|&a| a as i64))?)?;
                t.set("zone", b.names.zone(c.zone_id))?;
                t.set("faction_id", c.faction_id as i64)?;
                let faction = if c.faction_name.is_empty() { b.names.faction(c.faction_id) } else { c.faction_name.clone() };
                t.set("faction", faction)?;
                t.set("money", c.money.to_string())?;
                t.set("bm_point", c.bm_point)?;
                t.set("labor", c.labor_power as i64)?;
                t.set("x", c.pos.x as f64)?;
                t.set("y", c.pos.y as f64)?;
                t.set("transfer", c.transfer_request_time != 0)?;
                t.set("premium_grade", b.names.premium_grade(c.point))?;
                let houses = lua.create_table()?;
                for (n, h) in all_houses.iter().filter(|h| h.owner_id == c.id).enumerate() {
                    houses.raw_set(n + 1, house_table(lua, h, b.names.housing_category(h.template_id))?)?;
                }
                t.set("houses", houses)?;
                if c.delete_request_time != 0 {
                    t.set("delete_remain", (c.delete_time - now).max(0))?;
                }
                out.raw_set(i + 1, t)?;
            }
            Ok(out)
        })?,
    )?;

    // Index (1-based) of the character the list selects first: the one just created, else the one that played
    // last; nil when there is none (the scripts take the first then).
    let s = shared.clone();
    net.set("last_played", lua.create_function(move |_, ()| Ok(s.borrow().initial_selection().map(|i| i as i64 + 1)))?)?;

    // Index (1-based) of the character whose labor power the account manages, 0 for none.
    let s = shared.clone();
    net.set(
        "lp_managed",
        lua.create_function(move |_, ()| {
            let b = s.borrow();
            Ok(b.lp_managed.and_then(|id| b.characters.iter().position(|c| c.id == id)).map_or(0, |i| i as i64 + 1))
        })?,
    )?;

    let s = shared.clone();
    net.set("world_name", lua.create_function(move |_, ()| Ok(s.borrow().world_name.clone()))?)?;
    net.set("now", lua.create_function(|_, ()| Ok(unix_now()))?)?;

    let s = shared.clone();
    net.set(
        "race_congestions",
        lua.create_function(move |lua, ()| {
            let c = s.borrow().race_congestion;
            lua.create_sequence_from([race::NUIAN, race::ELF, race::FERRE, race::HARIHARAN].map(|r| c[r as usize] as i64))
        })?,
    )?;

    let s = shared.clone();
    net.set(
        "request",
        lua.create_function(move |_, args: MultiValue| {
            let kind = string(&args, 0);
            let index = |i: usize| int(&args, i).map(|n| n.max(0) as usize).unwrap_or(0);
            let req = match kind.as_str() {
                "quit" => Request::Quit,
                "enter_world" => Request::EnterWorld { world_id: int(&args, 1).unwrap_or(0) as u8 },
                "refresh_worlds" => Request::RefreshWorlds,
                "cancel_queue" => Request::CancelQueue,
                "connect" => Request::Connect,
                "to_world_select" => Request::ToWorldSelect,
                "refresh_characters" => Request::RefreshCharacters,
                "show_character" => Request::ShowCharacter(index(1)),
                "start_game" => Request::StartGame(index(1)),
                "create" => Request::CreateCharacter,
                "delete" => Request::DeleteCharacter { index: index(1), name: string(&args, 2) },
                "cancel_delete" => Request::CancelDelete(index(1)),
                "lp_manage" => Request::LpManage(index(1)),
                other => {
                    warn!("login: unknown request {other:?}");
                    return Ok(());
                }
            };
            s.borrow_mut().requests.push(req);
            Ok(())
        })?,
    )?;
    Ok(net)
}
