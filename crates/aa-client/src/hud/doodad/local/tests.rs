use super::*;

use bevy::prelude::*;

use aa_net::model::Item;
use aa_net::state::{Doodads, Inventory, Units};
use aa_net::{Command, Event, NetCommand};

use crate::hud::api::{HudBridge, Shared};
use crate::hud::data::GameData;
use crate::hud::doodad::{execute, world_doodad, Focus, Tables, Views};
use crate::hud::dynamic_actions::{Shown, Source};

const MAILBOX_GROUP: u32 = 107;
const MAILBOX_TEMPLATE: u32 = 320;
const FARM_GROUP: u32 = 10_669;
const FARM_TEMPLATE: u32 = 4357;
const STORE_GROUP: u32 = 21_823;
const STORE_TEMPLATE: u32 = 7911;
const BANK_GROUP: u32 = 22_065;
const BANK_TEMPLATE: u32 = 7983;
const COUNTER_GROUP: u32 = 19_645;
const COUNTER_SKILL: u32 = 23_780;
const COUNTER_TEMPLATE: u32 = 7229;
const COUNTER_ITEM: u32 = 27_301;

fn real_db() -> Option<rusqlite::Connection> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    std::path::Path::new(&path).exists().then(|| {
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
    })?
}

fn game() -> Option<GameData> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    std::path::Path::new(&path)
        .exists()
        .then(|| GameData::load(&path).ok())
        .flatten()
}

fn app(game: GameData, group: u32, template: u32, action: u32, inventory: Inventory) -> App {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).expect("the UI runtime");
    let mut views = Views::default();
    views.fixture_doodad(2075, Entity::PLACEHOLDER);
    let generation = views.doodad_generation(2075).unwrap();
    let bridge = Shared::default();
    {
        let mut d = bridge.borrow_mut();
        assert!(d.game.0.set(game).is_ok());
        d.snapshot.labor = 100;
        d.pos = Vec3::new(10.0, 10.0, 100.0);
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![action],
            craft_icon: None,
            key: -3.0,
            dist: 3.0,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: group,
                generation,
            },
            skills: vec![action],
        }));
        d.dynamic_actions.request(1);
    }
    crate::hud::api::install(&mut rt, &bridge).expect("the HUD API");
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    let mut doodads = Doodads::default();
    assert!(doodads.apply(&Event::DoodadsSpawned(vec![world_doodad(
        2075,
        template,
        group,
        Vec3::new(10.0, 10.0, 100.0)
    )])));
    let mut app = App::new();
    app.insert_non_send(HudBridge(bridge))
        .insert_non_send(rt)
        .insert_resource(doodads)
        .insert_resource(Units::default())
        .insert_resource(inventory)
        .insert_resource(views)
        .add_message::<NetCommand>()
        .add_message::<crate::hud::UseFailed>()
        .add_message::<crate::hud::ShowError>()
        .init_resource::<crate::ingame::DevOptions>()
        .add_systems(Update, execute);
    app
}

fn sent(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect()
}

fn lua(app: &App, script: &str) -> mlua::Result<mlua::Value> {
    app.world()
        .get_non_send::<UiRuntime>()
        .unwrap()
        .lua
        .load(script)
        .eval()
}

fn tick(app: &App) {
    aa_ui::lua::tick(&app.world().get_non_send::<UiRuntime>().unwrap().lua, 16.0);
}

fn bag_with(template: u32) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.bag = vec![None; 50];
    inventory.bag[0] = Some(Item {
        template_id: template,
        count: 1,
        ..Default::default()
    });
    inventory
}

/// The four tables the bar reads from the real 1.2 database: the mailbox maps its func to skill
/// 11400, and the farm/store funcs keep their `func_skill_id` and detail row.
#[test]
fn the_real_tables_map_the_local_funcs() {
    let Some(conn) = real_db() else { return };
    let t = Tables::load(&conn).unwrap();
    assert_eq!(
        t.uses(MAILBOX_GROUP).map(|u| u.skills.clone()),
        Some(vec![MAILBOX_ACTION]),
        "mailbox func has no func_skill_id; native maps the type to 11400"
    );
    assert_eq!(
        t.local(MAILBOX_GROUP, MAILBOX_ACTION),
        Some(LocalAction::Mailbox)
    );
    assert_eq!(
        t.uses(FARM_GROUP).map(|u| u.skills.clone()),
        Some(vec![16_968]),
        "farm info board"
    );
    assert_eq!(t.local(FARM_GROUP, 16_968), Some(LocalAction::FarmInfo(1)));
    assert_eq!(t.local(STORE_GROUP, 12_087), Some(LocalAction::Store(274)));
    assert_eq!(t.local(BANK_GROUP, 13_238), Some(LocalAction::Bank));
    assert_eq!(
        t.insert_counter(COUNTER_GROUP, COUNTER_SKILL),
        Some((COUNTER_ITEM, 1))
    );
}

/// The mailbox's bar action opens the original mail window and sends nothing (`FUN_393b1040`).
#[test]
fn the_mailbox_action_opens_the_original_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        MAILBOX_GROUP,
        MAILBOX_TEMPLATE,
        MAILBOX_ACTION,
        Inventory::default(),
    );
    tick(&app);
    app.update();
    assert!(sent(&mut app).is_empty(), "the mailbox use sent a packet");
    tick(&app);
    assert!(
        lua(&app, "return IsActivatedMailBox()")
            .unwrap()
            .as_boolean()
            .unwrap_or(false),
        "the mailbox window did not open"
    );
}

/// `DoodadFuncOpenFarmInfo`: the client raises `OPEN_COMMON_FARM_INFO(farm_id)` itself; the
/// original `info.alb` opens.
#[test]
fn the_farm_board_opens_the_original_info_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        FARM_GROUP,
        FARM_TEMPLATE,
        16_968,
        Inventory::default(),
    );
    tick(&app);
    app.update();
    assert!(sent(&mut app).is_empty(), "the farm info use sent a packet");
    tick(&app);
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    let info = aa_ui::lua::ui(&rt.lua).tree.find("farm_info");
    assert!(info.is_some_and(|w| aa_ui::lua::ui(&rt.lua).tree.visible(w)));
}

/// `DoodadFuncStoreUi`: the store window opens with the func's merchant pack, not an NPC catalog.
#[test]
fn the_store_doodad_opens_the_original_store_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        STORE_GROUP,
        STORE_TEMPLATE,
        12_087,
        Inventory::default(),
    );
    tick(&app);
    app.update();
    assert!(sent(&mut app).is_empty(), "the store use sent a packet");
    tick(&app);
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    assert!(
        rt.lua
            .load("return IsActivatedStore()")
            .eval::<bool>()
            .unwrap_or(false),
        "the store window did not open"
    );
    let goods = rt
        .lua
        .load("return #HUD.store_goods()")
        .eval::<usize>()
        .unwrap_or(0);
    assert!(goods > 0, "the doodad's merchant pack has no goods");
}

/// `DoodadFuncBankUi`: the bank window opens locally, no packet.
#[test]
fn the_bank_doodad_opens_the_original_bank_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(
        game,
        BANK_GROUP,
        BANK_TEMPLATE,
        13_238,
        Inventory::default(),
    );
    tick(&app);
    app.update();
    assert!(sent(&mut app).is_empty(), "the bank use sent a packet");
    tick(&app);
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    assert!(
        rt.lua
            .load("return IsActivatedBank()")
            .eval::<bool>()
            .unwrap_or(false),
        "the bank window did not open"
    );
}

/// `DoodadFuncInsertCounter`: without the detail row's item the client shows `NOT_ENOUGH_ITEM`
/// (0x1b) and sends nothing; with it the skill is sent unchanged.
#[test]
fn the_insert_counter_checks_the_item_before_sending() {
    let Some(game_data) = game() else { return };
    let mut without = app(
        game_data,
        COUNTER_GROUP,
        COUNTER_TEMPLATE,
        COUNTER_SKILL,
        Inventory::default(),
    );
    without.update();
    let errors: Vec<_> = without
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(errors, [NOT_ENOUGH_ITEM]);
    assert!(
        sent(&mut without).is_empty(),
        "sent without the counter item"
    );

    let Some(game_data) = game() else { return };
    let mut with = app(
        game_data,
        COUNTER_GROUP,
        COUNTER_TEMPLATE,
        COUNTER_SKILL,
        bag_with(COUNTER_ITEM),
    );
    with.update();
    assert!(matches!(
        sent(&mut with).as_slice(),
        [Command::UseDoodadSkill {
            skill_id: COUNTER_SKILL,
            ..
        }]
    ));
}
