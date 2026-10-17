//! The game API objects every login-stage script reads: permissive stand-ins for the objects the
//! client's `x2game.dll` registers (`X2*`), the constants it defines, and the Lua glue that makes
//! the ones the create screens use real.

use aa_ui::UiRuntime;

use super::api::{self, SharedRef};

/// Every game API object of the client (all `X2*` plus a few CryEngine ones). Each exists with
/// every method returning nothing, until a script file below replaces the ones that matter.
const GAME_OBJECTS: &[&str] = &[
    "X2",
    "X2Ability",
    "X2Achievement",
    "X2Action",
    "X2Auction",
    "X2Bag",
    "X2Bank",
    "X2BattleField",
    "X2Book",
    "X2Bot",
    "X2Camera",
    "X2Chat",
    "X2Coffer",
    "X2Craft",
    "X2Cursor",
    "X2Customizer",
    "X2Debug",
    "X2Decal",
    "X2Demo",
    "X2DialogManager",
    "X2Dominion",
    "X2Equipment",
    "X2Faction",
    "X2Family",
    "X2Friend",
    "X2Gm",
    "X2GoodsMail",
    "X2Helper",
    "X2Hotkey",
    "X2House",
    "X2InGameShop",
    "X2Input",
    "X2Interaction",
    "X2Item",
    "X2ItemEnchant",
    "X2ItemLookConverter",
    "X2Locale",
    "X2LoginCharacter",
    "X2Loot",
    "X2Mail",
    "X2Map",
    "X2Mate",
    "X2MiaMail",
    "X2NameTag",
    "X2Nation",
    "X2Option",
    "X2Player",
    "X2PremiumService",
    "X2Priest",
    "X2Quest",
    "X2Racing",
    "X2Rank",
    "X2Security",
    "X2SiegeWeapon",
    "X2Skill",
    "X2Sound",
    "X2Store",
    "X2Team",
    "X2Time",
    "X2Trade",
    "X2Trial",
    "X2Ucc",
    "X2Unit",
    "X2UserMusic",
    "X2Util",
    "X2Warp",
    "X2World",
    "Console",
    "Movie",
    "DockingManager",
];

/// Names of the global constants the client defines natively and the scripts read (the real values
/// are in aa-ui's constants; any other gets a unique number so `x == CONST` tests stay false).
const NATIVE_CONSTANTS: &str = include_str!("native_constants.txt");

/// Installs the game API into a fresh UI runtime, before any script of the stage runs.
pub fn install(ui: &mut UiRuntime, shared: &SharedRef) -> anyhow::Result<()> {
    let lua = &ui.lua;
    for name in GAME_OBJECTS {
        aa_ui::lua::install_game_stub(lua, name).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let g = lua.globals();
    // `ADDON` is the same native table as `X2`.
    let x2: mlua::Table = g.raw_get("X2").map_err(|e| anyhow::anyhow!("{e}"))?;
    g.raw_set("ADDON", x2).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut next = 1000i64;
    for name in NATIVE_CONSTANTS.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if g.raw_get::<mlua::Value>(name).map_err(|e| anyhow::anyhow!("{e}"))?.is_nil() {
            g.raw_set(name, next).map_err(|e| anyhow::anyhow!("{e}"))?;
            next += 1;
        }
    }
    crate::sound::bind(lua);
    api::install(lua, shared).map_err(|e| anyhow::anyhow!("{e}"))?;
    ui.exec("@game_base.lua", include_str!("game_base.lua")).map_err(|e| anyhow::anyhow!("game_base.lua: {e}"))?;
    ui.exec("@game_create.lua", include_str!("game_create.lua")).map_err(|e| anyhow::anyhow!("game_create.lua: {e}"))?;
    Ok(())
}
