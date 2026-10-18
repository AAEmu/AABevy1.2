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
    let settings = lua.create_table().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::settings::install(lua, &settings).map_err(|e| anyhow::anyhow!("{e}"))?;
    g.raw_set("NET", settings).map_err(|e| anyhow::anyhow!("{e}"))?;
    api::install(lua, shared).map_err(|e| anyhow::anyhow!("{e}"))?;
    ui.exec("@game_base.lua", include_str!("game_base.lua")).map_err(|e| anyhow::anyhow!("game_base.lua: {e}"))?;
    crate::settings::install_options(lua).map_err(|e| anyhow::anyhow!("{e}"))?;
    ui.exec("@game_create.lua", include_str!("game_create.lua")).map_err(|e| anyhow::anyhow!("game_create.lua: {e}"))?;
    crate::login::name_policy::install(lua).map_err(|e| anyhow::anyhow!("name policy: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_character_name_field_normalizes_punctuation_and_uses_native_26_character_limit() -> anyhow::Result<()> {
        use super::super::api::{Request, Shared};
        use super::super::look::Customizer;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Shared {
            cz: Customizer::open(aa_anim::Race::Nuian, aa_anim::Gender::Male)?,
            step: 3,
            requests: Vec::new(),
            race_congestion: [0; 9],
            zoom: 0.,
            ability: 1,
            create_pending: false,
            locale: "en_us".into(),
        }));
        let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
        rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
        install(&mut rt, &shared)?;
        rt.run_stage(aa_ui::Stage::CharacterCreate);
        rt.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
        rt.lua.load("characterCreate.OnNextStep()").exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        rt.fire_event("SHOW_CHARACTER_CREATE_WINDOW", false);
        rt.fire_event("SHOW_CHARACTER_CUSTOMIZE_WINDOW", true);
        rt.lua.load("characterCreate.OnNextStep()").exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        rt.fire_event("SHOW_CHARACTER_CUSTOMIZE_WINDOW", false);
        rt.fire_event("SHOW_CHARACTER_ABILITY_WINDOW", true);
        aa_ui::lua::tick(&rt.lua, 1000.);
        rt.lua.load(r#"
            assert(characterCreate.state=='Step3');SelectAbility(1);characterCreate.createRequested=false
            local edit=characterCreate.step3.content.nameWnd.characterName
            local ok=characterCreate.bgWindow.stepCompleteButton
            edit:SetText('a_1B');assert(edit:GetText()=='A_1b' and ok:IsEnabled())
            assert(edit:MaxTextLength()==26)
            edit:SetText('a'..string.rep('B',30));assert(edit:GetText()=='A'..string.rep('b',30) and not ok:IsEnabled())
            edit:SetText('!aBc');assert(edit:GetText()=='!aBc' and not ok:IsEnabled())
            edit:SetText('aBc DEF');assert(edit:GetText()=='Abc def' and not ok:IsEnabled())
            edit:SetText('a-b.c');assert(edit:GetText()=='A-b.c' and ok:IsEnabled())
            characterCreate.createRequested=true;UpdateCreateCharacterBtnState();assert(not ok:IsEnabled())
            characterCreate.createRequested=false;EnableEditBox(false);edit:SetText('');edit:SetFocus()
        "#).exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(rt.type_text(&format!("a{}", "B".repeat(30))), "original visible field must receive typed input");
        rt.lua.load("local edit=characterCreate.step3.content.nameWnd.characterName;assert(edit:GetText()=='A'..string.rep('b',25) and characterCreate.bgWindow.stepCompleteButton:IsEnabled(),edit:GetText());edit:SetText('');edit:SetFocus()").exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        let window = bevy::prelude::Entity::PLACEHOLDER;
        assert!(rt.ime_event(&bevy::window::Ime::Enabled { window }));
        assert!(rt.ime_event(&bevy::window::Ime::Preedit { window, value: "éCOLE".into(), cursor: Some((6, 6)) }));
        rt.lua.load("local edit=characterCreate.step3.content.nameWnd.characterName;assert(edit:IsComposingText() and edit:GetText()=='éCOLE')").exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(rt.ime_event(&bevy::window::Ime::Commit { window, value: "éCOLE".into() }));
        rt.lua.load("local edit=characterCreate.step3.content.nameWnd.characterName;assert(not edit:IsComposingText() and edit:GetText()=='École' and characterCreate.bgWindow.stepCompleteButton:IsEnabled())").exec().map_err(|e| anyhow::anyhow!("{e}"))?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        assert!(!super::super::api::lock(&shared).requests.iter().any(|r| matches!(r, Request::Create { .. })));
        Ok(())
    }
}
