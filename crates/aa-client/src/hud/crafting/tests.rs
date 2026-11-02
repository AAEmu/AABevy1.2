use super::*;
use aa_net::state::{Doodads, Player, Units};
use aa_net::{Command, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;

fn real_pak() -> bool {
    let path = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    if !std::path::Path::new(&path).is_file() {
        eprintln!("skip: no game_pak at {path}");
        return false;
    }
    true
}

fn real_db() -> Option<rusqlite::Connection> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
    if conn.is_none() {
        eprintln!("skip: no client database at {path}");
    }
    conn
}

fn workbench_game() -> Option<super::super::data::GameData> {
    let conn = real_db()?;
    let mut game = super::super::data::GameData::default();
    game.crafting = Tables::load(&conn).ok()?;
    Some(game)
}

fn workbench_app(data: HudData, doodads: Doodads) -> (App, super::super::api::Shared) {
    let rt = UiRuntime::new("en_us", 800.0, 600.0).unwrap();
    rt.lua
        .load(
            "INTERACTION_END_COUNT=0;local w=UIParent:CreateWidget('emptywidget','craftEndProbe','UIParent');             w:RegisterEvent('INTERACTION_END');             w:SetHandler('OnEvent',function(_,event) INTERACTION_END_COUNT=INTERACTION_END_COUNT+1 end)",
        )
        .exec()
        .unwrap();
    let bridge = super::super::api::Shared::default();
    *bridge.borrow_mut() = data;
    let mut app = App::new();
    app.insert_non_send(rt)
        .insert_non_send(super::super::api::HudBridge(bridge.clone()))
        .insert_resource(Player {
            spawn: Some(super::super::expansion::fixture::spawn()),
        })
        .insert_resource(doodads)
        .insert_resource(Units::default())
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, batch::update);
    (app, bridge)
}

fn open_alchemy_bench(data: &mut HudData) -> u32 {
    assert_eq!(bench::open(data, 300, 568), Some(71));
    *data.game.get().unwrap().crafting.benches[&568].iter().next().unwrap()
}

#[test]
fn visible_grade_labor_matches_original_x86_oracle() {
    let path = std::path::Path::new(aa_anim::db::DEFAULT_DB);
    if !path.exists() {
        return;
    }
    let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tables = Tables::load(&conn).unwrap();
    assert_eq!(tables.grades, vec![1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(tables.experts.len(), 31);
    for (base, points, grade, expected) in [
        (100, 10000, 1, 100),
        (100, 19999, 1, 100),
        (100, 20000, 1, 95),
        (10, 20000, 1, 10),
        (0, 20000, 1, 1),
        (100, 30000, 2, 90),
    ] {
        assert_eq!(labor(&tables, 1, base, points, grade), expected);
    }
    assert_eq!(labor(&tables, 0, 100, 100000, 6), 0);
    assert_eq!(tables.recipes.len(), 7010);
    assert_eq!(tables.recipes.values().map(|r| r.materials.len()).max(), Some(8));
    assert!(tables.recipes.values().flat_map(|r| &r.materials).all(|m| m.item != 0));
    assert_eq!(tables.benches[&274].len(), 3, "CraftStart lists also contribute recipes");
    assert_eq!(tables.benches[&568].len(), 71);
    assert_eq!(tables.benches[&2241].len(), 153);
    assert!(tables.craft_skills.contains(&tables.recipes[&5557].skill));
}

#[test]
#[ignore = "isolated original world stage and physical input probe"]
fn original_recipe_book_groups_categories_and_recipe_selection() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = fixture::runtime(1920.0, 1080.0)?;
    rt.lua
        .load(
            r#"
      local groups=X2Craft:EnumActabilityGroupTypes(); assert(#groups==33)
      local found=false;for _,id in ipairs(groups) do assert(id~=20);if id==19 then found=true end end;assert(found)
      local categories=X2Craft:GetActabilityCategoryTypesGroupType(1); assert(#categories>0)
      assert(X2Craft:GetCraftBaseInfo(0)==nil and X2Craft:GetCraftMaterialInfo(0)==nil)
      assert(X2Craft:GetCraftProductInfo(0)==nil and X2Craft:GetExecutableCraftCount(0)==nil)
      assert(not X2Craft:IsWorkingCraft() and X2Craft:GetLeftBatchCount()==0)
      assert(X2Ability:GetMyActabilityInfo(1).point==0)
    "#,
        )
        .exec()?;
    fixture::select_first_recipe(&rt)?;
    rt.lua.load("assert(CRAFTING_FRAME.detailPage:IsVisible() and not CRAFTING_FRAME.abilityGroupPage:IsVisible()); assert(#CRAFTING_FRAME.detailPage.categoryList.content:GetViewItemsInfo()>0); assert(not CRAFTING_FRAME.detailPage.buttonFrame:IsVisible())").exec()?;
    aa_ui::lua::flush(&rt.lua);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    drop(bridge);
    drop(rt);
    std::fs::remove_file(path)?;
    Ok(())
}

/// A request from the window's Craft button goes out with the live bench object id, exactly the
/// three native `X2Craft:ExecuteBatchCraftByType` arguments (`FUN_394d1340` writes opcode 0xf8).
#[test]
fn a_craft_request_from_a_workbench_carries_the_bench_id() {
    if !real_pak() { return }
    let Some(game) = workbench_game() else { return };
    let mut data = HudData::default();
    data.character_id = 47;
    data.pos = Vec3::ZERO;
    data.player_radius = 0.0;
    assert!(data.game.0.set(game).is_ok());
    let recipe = open_alchemy_bench(&mut data);
    batch::queue(&mut data, recipe, 300, 3);
    let mut doodads = Doodads::default();
    doodads.by_id.insert(300, super::fixture::doodad());
    let (mut app, _bridge) = workbench_app(data, doodads);
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|message| message.0)
        .collect::<Vec<_>>();
    assert!(matches!(
        sent.as_slice(),
        [Command::ExecuteCraft {
            craft_id,
            doodad_obj_id: 300,
            count: 3
        }] if *craft_id == recipe
    ), "{sent:?}");
}

/// Native `393b7e40` closes the active interaction when the 2.1 m predicate (`FUN_39093800`) fails,
/// and `FUN_394d6dc0` refuses the request at the same distance. Walking away therefore ends the
/// bench and sends no `CSExecuteCraft`.
#[test]
fn walking_out_of_range_closes_the_bench_and_refuses_the_request() {
    if !real_pak() { return }
    let Some(game) = workbench_game() else { return };
    let mut data = HudData::default();
    data.character_id = 47;
    data.pos = Vec3::ZERO;
    data.player_radius = 0.0;
    assert!(data.game.0.set(game).is_ok());
    let recipe = open_alchemy_bench(&mut data);
    data.pos = Vec3::new(0.0, 10.0, 0.0);
    batch::queue(&mut data, recipe, 300, 3);
    let mut doodads = Doodads::default();
    doodads.by_id.insert(300, super::fixture::doodad());
    let (mut app, bridge) = workbench_app(data, doodads);
    app.update();
    assert!(active_bench(&bridge.borrow()).is_none(), "the bench closes out of range");
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|message| message.0)
        .collect::<Vec<_>>();
    assert!(sent.is_empty(), "a far bench sends no craft request: {sent:?}");
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(INTERACTION_END_COUNT==1)")
        .exec()
        .unwrap();
}
