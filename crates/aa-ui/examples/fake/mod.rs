//! Fake game API for the examples: permissive `X2*` objects, native constants, UI texts from the
//! game database, and a small scenario (3 servers). The real app registers real objects.

use std::collections::HashMap;

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// Startup system: make the window size physical pixels (`--size 1920x1080` = the client's
/// 1920x1080 framebuffer even on a scaled desktop), so screenshots compare 1:1 with `reference/`.
pub fn physical_pixels(mut q: Query<&mut Window, With<PrimaryWindow>>) {
    let (pw, ph) = aa_dev::DevArgs::from_env().size;
    for mut w in &mut q {
        w.resolution.set_scale_factor_override(Some(1.0));
        w.resolution.set_physical_resolution(pw, ph);
    }
}

/// Game API objects the UI scripts use (all `X2*` plus a few CryEngine ones).
pub const GAME_OBJECTS: &[&str] = &[
    "X2", "X2Ability", "X2Achievement", "X2Action", "X2Auction", "X2Bag", "X2Bank", "X2BattleField", "X2Book", "X2Bot",
    "X2Camera", "X2Chat", "X2Coffer", "X2Craft", "X2Cursor", "X2Customizer", "X2Debug", "X2Decal", "X2Demo",
    "X2DialogManager", "X2Dominion", "X2Equipment", "X2Faction", "X2Family", "X2Friend", "X2Gm", "X2GoodsMail",
    "X2Helper", "X2Hotkey", "X2House", "X2InGameShop", "X2Input", "X2Interaction", "X2Item", "X2ItemEnchant",
    "X2ItemLookConverter", "X2Locale", "X2LoginCharacter", "X2Loot", "X2Mail", "X2Map", "X2Mate", "X2MiaMail",
    "X2NameTag", "X2Nation", "X2Option", "X2Player", "X2PremiumService", "X2Priest", "X2Quest", "X2Racing", "X2Rank",
    "X2Security", "X2SiegeWeapon", "X2Skill", "X2Sound", "X2Store", "X2Team", "X2Time", "X2Trade", "X2Trial", "X2Ucc",
    "X2Unit", "X2UserMusic", "X2Util", "X2Warp", "X2World", "Console", "Movie", "DockingManager",
];

pub struct FakeWorld {
    pub id: i64,
    pub name: &'static str,
    pub available: bool,
    pub congestion: i64,
    pub characters: i64,
}

pub fn default_worlds() -> Vec<FakeWorld> {
    vec![
        FakeWorld { id: 1, name: "Lane-bevy", available: true, congestion: 0, characters: 2 },
        FakeWorld { id: 2, name: "Kyprosa", available: true, congestion: 1, characters: 0 },
        FakeWorld { id: 3, name: "Melisara", available: false, congestion: 2, characters: 1 },
    ]
}

/// `--worlds ref0|ref1` reproduces the reference captures (one server `Lane-bevy` with 0 / 1
/// characters); default: [`default_worlds`].
pub fn worlds_from_args() -> Vec<FakeWorld> {
    let args: Vec<String> = std::env::args().collect();
    let v = args.iter().position(|a| a == "--worlds").and_then(|i| args.get(i + 1)).map(String::as_str);
    match v {
        Some("ref0") => vec![FakeWorld { id: 1, name: "Lane-bevy", available: true, congestion: 0, characters: 0 }],
        Some("ref1") => vec![FakeWorld { id: 1, name: "Lane-bevy", available: true, congestion: 0, characters: 1 }],
        _ => default_worlds(),
    }
}

/// `(category, key) -> text` for a locale, from the server's copy of the game database.
fn load_ui_texts(locale: &str) -> HashMap<(i64, String), String> {
    let mut out = HashMap::new();
    let path = "D:/aaemu-live/rs-game/Data/compact.sqlite3";
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(db) = rusqlite::Connection::open_with_flags(path, flags) else {
        eprintln!("fake: cannot open {path}; UI texts will show their keys");
        return out;
    };
    let col = match locale {
        "ko" | "en_us" | "zh_cn" | "ja" | "ru" | "de" | "fr" => locale,
        _ => "en_us",
    };
    let sql = format!(
        "SELECT u.category_id, u.key, COALESCE(l.{col}, u.text) FROM ui_texts u \
         LEFT JOIN localized_texts l ON l.tbl_name = 'ui_texts' AND l.tbl_column_name = 'text' AND l.idx = u.id"
    );
    if let Ok(mut st) = db.prepare(&sql) {
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)));
        if let Ok(rows) = rows {
            for (cat, key, text) in rows.flatten() {
                if let Some(t) = text {
                    out.insert((cat, key), t);
                }
            }
        }
    }
    out
}

/// Install the fake game layer into a fresh runtime (before any script runs).
pub fn install(rt: &mut UiRuntime, locale: &str, worlds: &[FakeWorld]) {
    let lua = &rt.lua;
    for name in GAME_OBJECTS {
        aa_ui::lua::install_game_stub(lua, name).unwrap();
    }
    // `ADDON` is the same native table as `X2`.
    let g = lua.globals();
    let x2: mlua::Table = g.raw_get("X2").unwrap();
    g.raw_set("ADDON", x2).unwrap();
    // aa-ui installs the client's real constants (`SERVER_TEXT`, `TRION`, ...); any other ALL_CAPS
    // global the UI reads but never assigns gets a unique value.
    let mut next = 1000i64;
    for name in include_str!("native_constants.txt").lines().map(str::trim).filter(|l| !l.is_empty()) {
        if g.raw_get::<mlua::Value>(name).unwrap().is_nil() {
            g.raw_set(name, next).unwrap();
            next += 1;
        }
    }
    // Scenario data.
    let fake = lua.create_table().unwrap();
    fake.raw_set("locale", locale).unwrap();
    let texts = load_ui_texts(locale);
    eprintln!("fake: {} UI texts", texts.len());
    let ui_text = lua
        .create_function(move |_, (cat, key): (i64, String)| Ok(texts.get(&(cat, key)).cloned()))
        .unwrap();
    fake.raw_set("ui_text", ui_text).unwrap();
    let list = lua.create_table().unwrap();
    for (i, w) in worlds.iter().enumerate() {
        let t = lua.create_table().unwrap();
        t.raw_set("id", w.id).unwrap();
        t.raw_set("name", w.name).unwrap();
        t.raw_set("available", w.available).unwrap();
        t.raw_set("congestion", w.congestion).unwrap();
        t.raw_set("characters", w.characters).unwrap();
        list.raw_set(i + 1, t).unwrap();
    }
    fake.raw_set("worlds", list).unwrap();
    g.raw_set("FAKE", fake).unwrap();
    if let Err(e) = rt.exec("@fake_game.lua", include_str!("fake_game.lua")) {
        eprintln!("fake: fake_game.lua: {e}");
    }
}

/// `--dump <file.json>`: write the UI tree (reference `ui.json` format) once, `--dump-after`
/// seconds (default 3) after start.
pub fn dump_tree(rt: Option<NonSend<UiRuntime>>, time: Res<Time>, mut done: Local<bool>) {
    let Some(rt) = rt else { return };
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let Some(path) = value("--dump") else { return };
    let after: f32 = value("--dump-after").and_then(|s| s.parse().ok()).unwrap_or(3.0);
    if *done || time.elapsed_secs() < after {
        return;
    }
    *done = true;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, rt.dump_json()) {
        Ok(()) => info!("fake: UI tree -> {path}"),
        Err(e) => warn!("fake: cannot write {path}: {e}"),
    }
}
