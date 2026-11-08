use super::*;

use bevy::prelude::*;

use aa_net::model::{Item, slot};
use aa_net::shop::{BuyEntry, ShopCommand};
use aa_net::state::{Doodads, Inventory, Player, Units};
use aa_net::{Command, Event, NetCommand};

use crate::hud::api::{HudBridge, HudData, Shared};
use crate::hud::data::{GameData, SkillInfo};
use crate::hud::doodad::{
    CRAFT_ACTION, Focus, PermEnv, Tables, Views, execute, find_focus, world_doodad,
};
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
    let mut app = base_app(game, group, template, action, inventory);
    app.add_systems(
        Update,
        (
            execute,
            crate::hud::store::update,
            crate::hud::auction::sync,
        ),
    );
    app
}

/// [`app`] without the HUD systems, for tests that run only the ones they exercise.
fn base_app(game: GameData, group: u32, template: u32, action: u32, inventory: Inventory) -> App {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).expect("the UI runtime");
    let mut views = Views::default();
    views.fixture_doodad(2075, Entity::PLACEHOLDER);
    let generation = views.doodad_generation(2075).unwrap();
    let bridge = Shared::default();
    {
        let mut d = bridge.borrow_mut();
        assert!(d.game.0.set(game).is_ok());
        d.snapshot.labor = 100;
        d.snapshot.money = 100_000_000;
        d.pos = Vec3::new(10.0, 10.0, 100.0);
        d.character_id = 42;
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
        .insert_resource(Player {
            spawn: Some(crate::hud::fixture_spawn()),
        })
        .init_resource::<aa_net::shop::Shop>()
        .add_message::<aa_net::NetEvent>()
        .add_message::<NetCommand>()
        .add_message::<crate::hud::UseFailed>()
        .add_message::<crate::hud::ShowError>()
        .init_resource::<crate::ingame::DevOptions>();
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
        t.local(MAILBOX_GROUP, MAILBOX_ACTION)
            .map(|l| (l.action, l.perm)),
        Some((LocalAction::Mailbox, 0))
    );
    assert_eq!(
        t.uses(FARM_GROUP).map(|u| u.skills.clone()),
        Some(vec![16_968]),
        "farm info board"
    );
    assert_eq!(
        t.local(FARM_GROUP, 16_968).map(|l| (l.action, l.perm)),
        Some((LocalAction::FarmInfo(1), 0))
    );
    assert_eq!(
        t.local(STORE_GROUP, 12_087).map(|l| (l.action, l.perm)),
        Some((LocalAction::Store(274), 0))
    );
    assert_eq!(
        t.local(15_916, 12_087).map(|l| (l.action, l.perm)),
        Some((LocalAction::Store(164), 6)),
        "Farmer's Workbench store is perm 6"
    );
    assert_eq!(
        t.local(BANK_GROUP, 13_238).map(|l| (l.action, l.perm)),
        Some((LocalAction::Bank, 0))
    );
    assert_eq!(
        t.insert_counter(COUNTER_GROUP, COUNTER_SKILL)
            .map(|c| (c.item, c.count, c.perm)),
        Some((COUNTER_ITEM, 1, 0))
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
    if !aa_cry::pak_available() {
        return;
    }
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

/// The fixture tables the pak-free permission/dispatch tests read: a perm-6 `DoodadFuncStoreUi`
/// (pack 274) and a perm-0 `DoodadFuncInsertCounter` (item 27301, count 1).
fn fixture_tables() -> Tables {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE doodad_funcs (id INTEGER, doodad_func_group_id INTEGER, func_skill_id INTEGER, actual_func_type TEXT, actual_func_id INTEGER, perm_id INTEGER, next_phase INTEGER);
         CREATE TABLE doodad_func_groups (id INTEGER, phase_msg TEXT, is_msg_to_zone INTEGER);
         CREATE TABLE doodad_almighties (id INTEGER, use_target_highlight INTEGER, use_target_silhouette INTEGER, use_target_decal INTEGER, target_decal_size REAL, sim_radius REAL);
         CREATE TABLE localized_texts (tbl_name TEXT, tbl_column_name TEXT, idx INTEGER, en_us TEXT);
         CREATE TABLE doodad_func_store_uis (id INTEGER, merchant_pack_id INTEGER);
         CREATE TABLE doodad_func_open_farm_infos (id INTEGER, farm_id INTEGER);
         CREATE TABLE doodad_func_insert_counters (id INTEGER, item_id INTEGER, item_count INTEGER);
         CREATE TABLE doodad_func_uses (id INTEGER, skill_id INTEGER);
         CREATE TABLE doodad_func_fake_uses (id INTEGER, fake_skill_id INTEGER);
         CREATE TABLE skills (id INTEGER);
         INSERT INTO doodad_funcs VALUES (1, 1, 12087, 'DoodadFuncStoreUi', 1, 6, -1);
         INSERT INTO doodad_funcs VALUES (2, 2, 23780, 'DoodadFuncInsertCounter', 2, 0, -1);
         INSERT INTO doodad_func_groups VALUES (1, '', 0), (2, '', 0);
         INSERT INTO doodad_almighties VALUES (100, 1, 0, 0, 0, 0), (101, 1, 0, 0, 0, 0);
         INSERT INTO localized_texts VALUES ('doodad_almighties', 'name', 100, 'Store Doodad');
         INSERT INTO doodad_func_store_uis VALUES (1, 274);
         INSERT INTO doodad_func_insert_counters VALUES (2, 27301, 1);",
    )
    .unwrap();
    Tables::load(&conn).unwrap()
}

/// `find_focus` lists a local action only when its func permission passes (`FUN_393bd290` /
/// `FUN_393b3300`). Runs without the pak: the tables are an in-memory fixture.
#[test]
fn a_local_action_is_listed_only_with_its_permission() {
    let mut game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    game.skills.insert(
        12_087,
        SkillInfo {
            max_range: 4,
            ..Default::default()
        },
    );
    let mut data = HudData::default();
    assert!(data.game.0.set(game).is_ok());
    data.pos = Vec3::new(10.0, 10.0, 100.0);
    data.character_id = 42;
    let mut doodads = Doodads::default();
    let mut store = world_doodad(1, 100, 1, Vec3::new(10.0, 10.0, 100.0));
    store.owner_id = 42;
    doodads.by_id.insert(1, store);
    let focus = find_focus(&data, &doodads, &Units::default(), PermEnv::default())
        .expect("the owned store is in reach");
    assert_eq!(focus.skills, vec![12_087]);

    doodads.by_id.get_mut(&1).unwrap().owner_id = 0;
    assert_eq!(
        find_focus(&data, &doodads, &Units::default(), PermEnv::default()),
        None,
        "perm 6 refuses a doodad owned by somebody else"
    );
}

/// A use of a local action rechecks `FUN_393b3300` before it opens: error 2 `NO_PERM`, no packet.
/// Runs without the pak.
#[test]
fn a_local_action_use_checks_its_permission() {
    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut app = dispatch_app(game, 1, 100, 12087, 0, Inventory::default(), None);
    app.update();
    let errors: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(errors, [NO_PERM]);
    assert!(sent(&mut app).is_empty());

    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut app = dispatch_app(game, 1, 100, 12087, 42, Inventory::default(), None);
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<crate::hud::ShowError>>()
            .drain()
            .next()
            .is_none()
    );
    assert!(sent(&mut app).is_empty());
}

/// A local action on a doodad that cannot be measured is not offered (the listing filter drops
/// it too): the use is skipped silently, not refused with `NO_PERM`. Runs without the pak.
#[test]
fn a_local_action_without_a_world_position_is_skipped_silently() {
    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut app = dispatch_app(game, 1, 100, 12_087, 42, Inventory::default(), None);
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .get_mut(&2075)
        .unwrap()
        .parent_obj_id = 999;
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<crate::hud::ShowError>>()
            .drain()
            .next()
            .is_none()
    );
    assert!(sent(&mut app).is_empty());
}

/// `FUN_393bc260` case 0x54: no local player -> handled, no send; with a player, the item must be
/// in the bag or in the 28 equipment slots (`FUN_393b8f00`). Runs without the pak.
#[test]
fn the_insert_counter_needs_the_item_in_the_bag_or_equipment() {
    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut no_player = dispatch_app(game, 2, 101, COUNTER_SKILL, 0, Inventory::default(), None);
    no_player.update();
    assert!(
        !no_player
            .world_mut()
            .resource_mut::<Messages<crate::hud::ShowError>>()
            .drain()
            .any(|error| error.0 == NOT_ENOUGH_ITEM)
    );
    assert!(sent(&mut no_player).is_empty(), "no player sent the skill");

    let player = Player {
        spawn: Some(crate::hud::fixture_spawn()),
    };
    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut without = dispatch_app(
        game,
        2,
        101,
        COUNTER_SKILL,
        0,
        Inventory::default(),
        Some(player.clone()),
    );
    without.update();
    let errors: Vec<_> = without
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(errors, [NOT_ENOUGH_ITEM]);
    assert!(sent(&mut without).is_empty());

    let mut equipped = Inventory::default();
    equipped.equipment.slots[slot::BACKPACK] = Some(Item {
        template_id: COUNTER_ITEM,
        count: 1,
        ..Default::default()
    });
    let game = GameData {
        doodad: fixture_tables(),
        ..Default::default()
    };
    let mut with = dispatch_app(game, 2, 101, COUNTER_SKILL, 0, equipped, Some(player));
    with.update();
    assert!(matches!(
        sent(&mut with).as_slice(),
        [Command::UseDoodadSkill {
            skill_id: COUNTER_SKILL,
            ..
        }]
    ));
}

/// A store/bank window closes with `NPC_INTERACTION_END` (0x6c) then `INTERACTION_END` (0xd2)
/// (`FUN_393b7aa0`), on walk-away and when the doodad is gone.
#[test]
fn the_store_and_bank_close_with_the_native_end_events() {
    if !aa_cry::pak_available() {
        return;
    }
    for (group, template, action, activated) in [
        (STORE_GROUP, STORE_TEMPLATE, 12_087, "IsActivatedStore()"),
        (BANK_GROUP, BANK_TEMPLATE, 13_238, "IsActivatedBank()"),
    ] {
        let Some(game_data) = game() else { return };
        let mut app = app(game_data, group, template, action, Inventory::default());
        lua(
            &app,
            "STORE_ENDS=0; INTER_ENDS=0;              local a=UIParent:CreateWidget('emptywidget','storeEndProbe','UIParent');              a:RegisterEvent('NPC_INTERACTION_END');              a:SetHandler('OnEvent',function() STORE_ENDS=STORE_ENDS+1 end);              local b=UIParent:CreateWidget('emptywidget','interEndProbe','UIParent');              b:RegisterEvent('INTERACTION_END');              b:SetHandler('OnEvent',function() INTER_ENDS=INTER_ENDS+1 end)",
        )
        .unwrap();
        app.update();
        assert_eq!(
            lua(&app, &format!("return {activated}"))
                .unwrap()
                .as_boolean(),
            Some(true),
            "the window did not open"
        );
        app.world()
            .get_non_send::<HudBridge>()
            .unwrap()
            .0
            .borrow_mut()
            .pos = Vec3::new(1000.0, 1000.0, 100.0);
        app.update();
        assert_eq!(
            lua(&app, &format!("return {activated}"))
                .unwrap()
                .as_boolean(),
            Some(false),
            "walking away did not close the window"
        );
        assert_eq!(
            lua(&app, "return STORE_ENDS").unwrap().as_integer(),
            Some(1),
            "NPC_INTERACTION_END after the walk-away"
        );
        assert_eq!(
            lua(&app, "return INTER_ENDS").unwrap().as_integer(),
            Some(1),
            "INTERACTION_END after the walk-away"
        );

        // reopen, then despawn the doodad
        {
            let bridge = app.world().get_non_send::<HudBridge>().unwrap();
            let mut d = bridge.0.borrow_mut();
            d.pos = Vec3::new(10.0, 10.0, 100.0);
            d.dynamic_actions.request(1);
        }
        app.update();
        assert_eq!(
            lua(&app, &format!("return {activated}"))
                .unwrap()
                .as_boolean(),
            Some(true),
            "the window did not reopen"
        );
        app.world_mut()
            .resource_mut::<Doodads>()
            .by_id
            .remove(&2075);
        app.update();
        assert_eq!(
            lua(&app, "return STORE_ENDS").unwrap().as_integer(),
            Some(2),
            "NPC_INTERACTION_END after the despawn"
        );
        assert_eq!(
            lua(&app, "return INTER_ENDS").unwrap().as_integer(),
            Some(2),
            "INTERACTION_END after the despawn"
        );
    }
}

/// Opening a local window ends the interaction that is active (`FUN_393b7b10` ->
/// `FUN_393b7aa0`): the open store window gets `NPC_INTERACTION_END` then `INTERACTION_END`
/// before the mailbox window opens.
#[test]
fn a_local_window_ends_the_previous_store() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game_data) = game() else { return };
    let mut app = app(
        game_data,
        STORE_GROUP,
        STORE_TEMPLATE,
        12_087,
        Inventory::default(),
    );
    lua(
        &app,
        "STORE_ENDS=0; INTER_ENDS=0;          local a=UIParent:CreateWidget('emptywidget','storeEndProbe3','UIParent');          a:RegisterEvent('NPC_INTERACTION_END');          a:SetHandler('OnEvent',function() STORE_ENDS=STORE_ENDS+1 end);          local b=UIParent:CreateWidget('emptywidget','interEndProbe3','UIParent');          b:RegisterEvent('INTERACTION_END');          b:SetHandler('OnEvent',function() INTER_ENDS=INTER_ENDS+1 end)",
    )
    .unwrap();
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true)
    );

    // The same doodad becomes a mailbox; opening it must end the store first.
    app.world_mut().resource_mut::<Doodads>().by_id.insert(
        2075,
        world_doodad(
            2075,
            MAILBOX_TEMPLATE,
            MAILBOX_GROUP,
            Vec3::new(10.0, 10.0, 100.0),
        ),
    );
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![MAILBOX_ACTION],
            craft_icon: None,
            key: -3.0,
            dist: 3.0,
        });
        let generation = d.dynamic_actions.shown().and_then(|s| match s.source {
            Source::Doodad { generation, .. } => Some(generation),
            _ => None,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: MAILBOX_GROUP,
                generation: generation.unwrap_or_default(),
            },
            skills: vec![MAILBOX_ACTION],
        }));
        d.dynamic_actions.request(1);
    }
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(false),
        "the store window stayed open"
    );
    assert_eq!(
        lua(&app, "return IsActivatedMailBox()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the mailbox window did not open"
    );
    assert_eq!(
        lua(&app, "return STORE_ENDS").unwrap().as_integer(),
        Some(1),
        "NPC_INTERACTION_END before the new window"
    );
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1),
        "INTERACTION_END before the new window"
    );
}

/// A store opened at a fallback gap (no drawn bounds, 3 m) must not close on the next frame.
#[test]
fn a_fallback_gap_store_stays_open() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game_data) = game() else { return };
    let mut app = app(
        game_data,
        STORE_GROUP,
        STORE_TEMPLATE,
        12_087,
        Inventory::default(),
    );
    lua(
        &app,
        "STORE_ENDS=0; local a=UIParent:CreateWidget('emptywidget','storeEndProbe2','UIParent');          a:RegisterEvent('NPC_INTERACTION_END');          a:SetHandler('OnEvent',function() STORE_ENDS=STORE_ENDS+1 end)",
    )
    .unwrap();
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .get_mut(&2075)
        .unwrap()
        .pos = aa_net::Pos::new(10.0, 13.0, 100.0);
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true),
        "the fallback-gap store did not open"
    );
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true),
        "the fallback-gap store closed on the next frame"
    );
    assert_eq!(
        lua(&app, "return STORE_ENDS").unwrap().as_integer(),
        Some(0)
    );
}

/// `end_interaction` closes the bench, the coffer and a shown interaction list, and raises
/// `INTERACTION_END` once. Only a list that was shown counts (like `other_interaction`).
#[test]
fn end_interaction_closes_the_bench_coffer_and_shown_list() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game_data) = game() else { return };
    let mut app = app(
        game_data,
        STORE_GROUP,
        STORE_TEMPLATE,
        12_087,
        Inventory::default(),
    );
    lua(
        &app,
        "NPC_ENDS=0; INTER_ENDS=0;          local a=UIParent:CreateWidget('emptywidget','benchEndProbe','UIParent');          a:RegisterEvent('NPC_INTERACTION_END');          a:SetHandler('OnEvent',function() NPC_ENDS=NPC_ENDS+1 end);          local b=UIParent:CreateWidget('emptywidget','interEndProbe4','UIParent');          b:RegisterEvent('INTERACTION_END');          b:SetHandler('OnEvent',function() INTER_ENDS=INTER_ENDS+1 end)",
    )
    .unwrap();
    app.world_mut().resource_mut::<Inventory>().coffer.obj_id = 42;
    let inventory = app.world().resource::<Inventory>().clone();
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        assert!(crate::hud::crafting::open_bench(&mut d, 300, 0).is_some());
        d.doodad.menu = Some((2075, Vec::new()));
        d.doodad.menu_shown = true;
        d.doodad.local_state = Some(crate::hud::doodad::LocalState {
            state: 8,
            obj_id: 2075,
        });
    }
    {
        let rt = app.world().get_non_send::<UiRuntime>().unwrap();
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        assert_eq!(
            crate::hud::doodad::end_interaction(Some(rt), &bridge.0, &inventory)
                .map(|state| state.state),
            Some(8)
        );
        let d = bridge.0.borrow();
        assert_eq!(crate::hud::crafting::active_bench(&d), None);
        assert!(d.doodad.coffer_close);
        assert!(!d.doodad.menu_shown && d.doodad.menu.is_none());
        assert!(d.doodad.local_state.is_none());
    }
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));

    // A list kept for the immediate-use path (not shown) is not an active interaction.
    app.world_mut().resource_mut::<Inventory>().coffer.close();
    let inventory = app.world().resource::<Inventory>().clone();
    {
        let rt = app.world().get_non_send::<UiRuntime>().unwrap();
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        bridge.0.borrow_mut().doodad.menu = Some((2075, Vec::new()));
        assert!(crate::hud::doodad::end_interaction(Some(rt), &bridge.0, &inventory).is_none());
        assert!(bridge.0.borrow().doodad.menu.is_some());
    }
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
}

/// A doodad that exists but has no composed world position has no measurable interaction:
/// `left_interaction` reports it ended, like a failed `FUN_39093800`.
#[test]
fn a_doodad_without_a_world_position_has_left() {
    let mut doodad = world_doodad(1, 100, 1, Vec3::new(10.0, 10.0, 100.0));
    doodad.parent_obj_id = 999;
    let mut doodads = Doodads::default();
    doodads.by_id.insert(1, doodad);
    assert!(crate::hud::doodad::left_interaction(
        &HudData::default(),
        &doodads,
        &Units::default(),
        1
    ));
}

/// An `execute` app without a UI runtime: the pak-free dispatch/permission tests.
fn dispatch_app(
    game: GameData,
    group: u32,
    template: u32,
    action: u32,
    owner: u32,
    inventory: Inventory,
    player: Option<Player>,
) -> App {
    let mut views = Views::default();
    views.fixture_doodad(2075, Entity::PLACEHOLDER);
    let generation = views.doodad_generation(2075).unwrap();
    let bridge = Shared::default();
    {
        let mut d = bridge.borrow_mut();
        assert!(d.game.0.set(game).is_ok());
        d.snapshot.labor = 100;
        d.pos = Vec3::new(10.0, 10.0, 100.0);
        d.character_id = 42;
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
    let mut doodads = Doodads::default();
    let mut doodad = world_doodad(2075, template, group, Vec3::new(10.0, 10.0, 100.0));
    doodad.owner_id = owner;
    doodads.by_id.insert(2075, doodad);
    let mut app = App::new();
    app.insert_non_send(HudBridge(bridge))
        .insert_resource(doodads)
        .insert_resource(Units::default())
        .insert_resource(inventory)
        .insert_resource(views)
        .add_message::<NetCommand>()
        .add_message::<crate::hud::UseFailed>()
        .add_message::<crate::hud::ShowError>()
        .init_resource::<crate::ingame::DevOptions>()
        .add_systems(Update, execute);
    if let Some(player) = player {
        app.insert_resource(player);
    }
    app
}

fn event_probe(app: &App) {
    lua(
        app,
        "EVENTS={}; local w=UIParent:CreateWidget('emptywidget','doodadFuncProbe','UIParent');\
         w:SetHandler('OnEvent',function(self,event,...) EVENTS[#EVENTS+1]={event,...} end);\
         for _,e in ipairs({'OPEN_PAPER','CHAT_MSG_QUEST','CHAT_MSG_DOODAD','RENEW_ITEM_WND_SHOW','OPEN_EMBLEM_UPLOAD_UI',\
         'COFFER_INTERACTION_START','REPORT_CRIME','NPC_INTERACTION_START','SYSMSG'}) do w:RegisterEvent(e) end",
    )
    .unwrap();
}

fn event_arg(app: &App, event: &str, n: usize) -> Option<mlua::Value> {
    lua(
        app,
        &format!("for _,e in ipairs(EVENTS) do if e[1]=='{event}' then return e[{n}] end end"),
    )
    .ok()
    .filter(|v| !v.is_nil())
}

fn bag_count(template: u32, count: i32) -> Inventory {
    let mut inventory = Inventory::default();
    inventory.bag = vec![None; 50];
    inventory.bag[0] = Some(Item {
        template_id: template,
        count,
        ..Default::default()
    });
    inventory
}

/// `bubble_chats` (not `bubble_effects`) holds a doodad bubble's lines: bubble 2 starts with the
/// bound skull's (doodad 2999) "S-save me..." and goes on to NPC 4212's warning along `next`.
#[test]
fn the_bubble_conversation_follows_its_start_line() {
    let Some(game) = game() else { return };
    let chain = game.speech.bubble_chain(2);
    let lines: Vec<_> = chain.iter().map(|c| (c.id, c.doodad, c.npc)).collect();
    assert_eq!(lines, [(3, 2999, 0), (4, 0, 4212)]);
    assert!(chain[0].text.starts_with("S-save me"), "{}", chain[0].text);
    // bubble 5: the speakerless START line (the player's) "Should I get this?", then the doll
    assert_eq!(
        game.speech
            .bubble_chain(5)
            .first()
            .map(|c| (c.id, c.text.as_str())),
        Some((12, "Should I get this?"))
    );
    assert!(game.speech.bubble_chain(0).is_empty());
}

/// `DoodadFuncBubble` (`FUN_393b8240`): the `bubbles` conversation (bubble 27: the doll's own
/// "Where've you been?", `CHAT_MSG_DOODAD` over the doodad, `FUN_39374960`) and the native phase
/// change. `bubble_effects` 27 is an unrelated skill bubble and is not said.
#[test]
fn the_bubble_plays_its_speech_and_changes_the_phase() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 18_327, 6807, 22_584, Inventory::default());
    event_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        event_arg(&app, "CHAT_MSG_DOODAD", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("Where've you been?")
    );
    assert_eq!(
        event_arg(&app, "CHAT_MSG_DOODAD", 4).and_then(|v| v.as_f64()),
        Some(2075.0),
        "the doodad says it"
    );
    assert!(event_arg(&app, "CHAT_MSG_QUEST", 2).is_none());
    assert!(matches!(
        sent(&mut app).as_slice(),
        [Command::ChangeDoodadPhase {
            obj_id: 2075,
            skill_id: 22_584,
            next_phase: 18_328,
            func_id: 14_751
        }]
    ));
}

/// `DoodadFuncOpenPaper` (`FUN_393b8450`): a page opens the page window; a book opens the book
/// window through `X2Book.GetBookInfo`; both send the phase change when the row names one.
#[test]
fn the_open_paper_opens_its_page_or_book() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game1) = game() else { return };
    let mut page = app(game1, 16_297, 6167, 20_743, Inventory::default());
    event_probe(&page);
    tick(&page);
    page.update();
    tick(&page);
    assert_eq!(
        event_arg(&page, "OPEN_PAPER", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("page")
    );
    assert_eq!(
        event_arg(&page, "OPEN_PAPER", 3).and_then(|v| v.as_u32()),
        Some(453)
    );
    assert!(matches!(
        sent(&mut page).as_slice(),
        [Command::ChangeDoodadPhase {
            obj_id: 2075,
            skill_id: 20_743,
            next_phase: 16_295,
            func_id: 13_413
        }]
    ));

    let Some(game2) = game() else { return };
    let mut book = app(game2, 17_230, 6497, 21_814, Inventory::default());
    event_probe(&book);
    tick(&book);
    book.update();
    tick(&book);
    assert_eq!(
        event_arg(&book, "OPEN_PAPER", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("book")
    );
    assert_eq!(
        event_arg(&book, "OPEN_PAPER", 3).and_then(|v| v.as_u32()),
        Some(8)
    );
    let rt = book.world().get_non_send::<UiRuntime>().unwrap();
    let info = rt
        .lua
        .load("return X2Book:GetBookInfo(8)")
        .eval::<Option<mlua::Table>>()
        .ok()
        .flatten()
        .expect("X2Book.GetBookInfo(8)");
    assert!(info.get::<String>("name").is_ok());
    assert!(
        info.get::<mlua::Table>("pages")
            .map(|p| p.raw_len() > 0)
            .unwrap_or(false)
    );
    assert!(sent(&mut book).is_empty(), "no phase when next_phase < 0");
}

/// `DoodadFuncStampMaker` (`FUN_393b82a0`): the object id goes to `ucc.alb` as a string.
#[test]
fn the_stamp_maker_opens_the_ucc_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 2260, 1470, 14_435, Inventory::default());
    event_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        event_arg(&app, "OPEN_EMBLEM_UPLOAD_UI", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("2075")
    );
    assert_eq!(
        lua(
            &app,
            "return ucc ~= nil and ucc.window ~= nil and ucc.window:IsVisible()"
        )
        .unwrap()
        .as_boolean(),
        Some(true),
        "the UCC window did not open"
    );
    assert!(sent(&mut app).is_empty());
}

/// `DoodadFuncAuctionUi` (`FUN_393b8b80`): `NPC_INTERACTION_START("auction")` opens the original
/// auction window.
#[test]
fn the_auction_doodad_opens_the_auction_window() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 22_065, 7983, 12_083, Inventory::default());
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the auction window did not open"
    );
    assert!(sent(&mut app).is_empty());
}

/// Turns the fixture doodad into the mailbox and asks the bar for its action.
fn request_mailbox(app: &mut App) {
    app.world_mut().resource_mut::<Doodads>().by_id.insert(
        2075,
        world_doodad(
            2075,
            MAILBOX_TEMPLATE,
            MAILBOX_GROUP,
            Vec3::new(10.0, 10.0, 100.0),
        ),
    );
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![MAILBOX_ACTION],
            craft_icon: None,
            key: -3.0,
            dist: 3.0,
        });
        let generation = d.dynamic_actions.shown().and_then(|s| match s.source {
            Source::Doodad { generation, .. } => Some(generation),
            _ => None,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: MAILBOX_GROUP,
                generation: generation.unwrap_or_default(),
            },
            skills: vec![MAILBOX_ACTION],
        }));
        d.dynamic_actions.request(1);
    }
}

/// Turns the fixture doodad into a workbench and asks the bar for its craft action.
fn request_bench(app: &mut App, template: u32, group: u32) {
    app.world_mut().resource_mut::<Doodads>().by_id.insert(
        2075,
        world_doodad(2075, template, group, Vec3::new(10.0, 10.0, 100.0)),
    );
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![CRAFT_ACTION],
            craft_icon: None,
            key: -3.0,
            dist: 3.0,
        });
        let generation = d.dynamic_actions.shown().and_then(|s| match s.source {
            Source::Doodad { generation, .. } => Some(generation),
            _ => None,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: group,
                generation: generation.unwrap_or_default(),
            },
            skills: vec![CRAFT_ACTION],
        }));
        d.dynamic_actions.request(1);
    }
}

fn auction_end_probe(app: &App) {
    lua(
        app,
        "NPC_ENDS=0; INTER_ENDS=0; ORDER='';\
         local a=UIParent:CreateWidget('emptywidget','auctionNpcEnd','UIParent');\
         a:RegisterEvent('NPC_INTERACTION_END');\
         a:SetHandler('OnEvent',function() NPC_ENDS=NPC_ENDS+1; ORDER=ORDER..'N;' end);\
         local b=UIParent:CreateWidget('emptywidget','auctionInterEnd','UIParent');\
         b:RegisterEvent('INTERACTION_END');\
         b:SetHandler('OnEvent',function() INTER_ENDS=INTER_ENDS+1; ORDER=ORDER..'I;' end)",
    )
    .unwrap();
}

/// `FUN_393b7aa0` raises `NPC_INTERACTION_END` (0x6c) for the auction state 0xe before
/// `INTERACTION_END` (0xd2): the next interaction (a mailbox) takes over.
#[test]
fn the_auction_interaction_ends_before_a_mailbox() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 22_065, 7983, 12_083, Inventory::default());
    auction_end_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the auction window did not open"
    );
    request_mailbox(&mut app);
    app.update();
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(1));
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
    assert_eq!(
        lua(&app, "return ORDER")
            .unwrap()
            .as_string()
            .and_then(|s| s.to_str().ok().map(|s| s.to_string()))
            .as_deref(),
        Some("N;I;"),
        "NPC_INTERACTION_END must come first"
    );
    assert_eq!(
        lua(&app, "return IsActivatedMailBox()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the mailbox did not open"
    );
}

/// Closing the auction window leaves state 0xe set: `FUN_394ce540` only clears it when the
/// doodad is gone or out of reach. The next interaction (a mailbox) still takes it over and
/// raises `NPC_INTERACTION_END` then `INTERACTION_END` once.
#[test]
fn a_closed_auction_still_ends_at_the_next_interaction() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 22_065, 7983, 12_083, Inventory::default());
    auction_end_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the auction window did not open"
    );
    lua(&app, "auctionWindow:Show(false)").unwrap();
    tick(&app);
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the auction window did not close"
    );
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(0)
    );
    assert_eq!(
        app.world()
            .get_non_send::<HudBridge>()
            .unwrap()
            .0
            .borrow()
            .doodad
            .local_state
            .map(|state| state.state),
        Some(0xe),
        "closing the window cleared the auction state"
    );

    request_mailbox(&mut app);
    app.update();
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(1));
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
    assert_eq!(
        lua(&app, "return ORDER")
            .unwrap()
            .as_string()
            .and_then(|s| s.to_str().ok().map(|s| s.to_string()))
            .as_deref(),
        Some("N;I;"),
        "NPC_INTERACTION_END must come first"
    );
    assert_eq!(
        lua(&app, "return IsActivatedMailBox()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the mailbox did not open"
    );
}

/// An NPC store opens while a doodad auction is active: `npc_interaction.alb` hides the auction
/// window on `NPC_INTERACTION_START`, but that alone must not clear state 0xe or the auction would
/// close the store it just opened (`store::end_interaction`).
#[test]
fn a_closed_auction_does_not_end_an_npc_store() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = base_app(game, 22_065, 7983, 12_083, Inventory::default());
    app.add_systems(Update, (execute, crate::hud::auction::sync));
    auction_end_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the auction window did not open"
    );
    // `store::start_selected` installs the store context, then raises the event.
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.store.npc = Some((4242, 0));
        d.store.doodad = false;
    }
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    rt.fire_event("NPC_INTERACTION_START", ("store", "", "4242"));
    tick(&app);
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true),
        "the store window did not open"
    );
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the auction window stayed open under the store"
    );
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    assert_eq!(bridge.0.borrow().store.npc, Some((4242, 0)));
    assert_eq!(
        bridge.0.borrow().doodad.local_state.map(|state| state.state),
        Some(0xe)
    );
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(0)
    );
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true),
        "the store closed on the next frame"
    );
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));
}

/// `DoodadFuncCofferPerm` (`FUN_393bc260` case 0x51): `FUN_3983b3b0` finds the template's
/// positive `DoodadFuncCoffer`, then `COFFER_INTERACTION_START`. Without one the skill is sent.
#[test]
fn the_coffer_permission_needs_a_coffer_on_the_template() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 17_762, 6692, 22_691, Inventory::default());
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .get_mut(&2075)
        .unwrap()
        .owner_id = 42;
    event_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        event_arg(&app, "COFFER_INTERACTION_START", 1)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("COFFER_INTERACTION_START")
    );
    assert!(sent(&mut app).is_empty());

    // A fixture phase with the same action id but a template without a coffer func: the native
    // returns 0 and the skill is sent.
    let game = GameData {
        doodad: {
            let mut tables = Tables::default();
            tables.add_full(
                1,
                22_691,
                "DoodadFuncCofferPerm",
                0,
                1,
                1,
                -1,
                &LocalFuncs::default(),
            );
            tables
        },
        ..Default::default()
    };
    let mut plain = dispatch_app(game, 1, 101, 22_691, 0, Inventory::default(), None);
    plain.update();
    assert!(matches!(
        sent(&mut plain).as_slice(),
        [Command::UseDoodadSkill {
            skill_id: 22_691,
            ..
        }]
    ));
}

/// `DoodadFuncRenewItem` (`FUN_393b8610`): `RENEW_ITEM_WND_SHOW(skill, objId)`. The shipped row
/// has no `func_skill_id` (the native does not map type 0x47 either), so the bar never offers it;
/// the fixture names the action so the dispatch path is still driven.
#[test]
fn the_renew_item_raises_its_window_event() {
    if !aa_cry::pak_available() {
        return;
    }
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE doodad_func_renew_items (id INTEGER, skill_id INTEGER);
         INSERT INTO doodad_func_renew_items VALUES (2, 15446);",
    )
    .unwrap();
    let funcs = LocalFuncs::load(&conn);
    let Some(mut game) = game() else { return };
    game.doodad = {
        let mut tables = Tables::default();
        tables.add_full(1, 99, "DoodadFuncRenewItem", 0, 2, 2, -1, &funcs);
        tables
    };
    let mut app = app(game, 1, 101, 99, Inventory::default());
    event_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        event_arg(&app, "RENEW_ITEM_WND_SHOW", 2).and_then(|v| v.as_u32()),
        Some(15_446)
    );
    assert_eq!(
        event_arg(&app, "RENEW_ITEM_WND_SHOW", 3).and_then(|v| v.as_u32()),
        Some(2075)
    );
    assert!(sent(&mut app).is_empty(), "a local renew sends nothing");
}

/// `DoodadFuncParentInfo` (`FUN_393b8400`): the parent house's info opens (`FUN_393befd0`).
#[test]
fn the_parent_info_opens_the_parent_house() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 11_179, 4570, 15_212, Inventory::default());
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .get_mut(&2075)
        .unwrap()
        .owner_db_id = 99;
    let mut houses = aa_net::Houses::default();
    houses.by_tl_id.insert(
        1,
        aa_net::House {
            snapshot: aa_net::HouseSnapshot {
                tl_id: 1,
                db_id: 99,
                obj_id: 55,
                template_id: 0,
                model_id: 0,
                co_owner_id: 0,
                owner_id: 0,
                owner_name: String::new(),
                account_id: 0,
                permission: 0,
                all_actions: 0,
                current_actions: 0,
                tax: 0,
                x: 0,
                y: 0,
                z: 0.0,
                name: String::new(),
                allow_recover: false,
                sell_price: 0,
                sell_to_player_id: 0,
                sell_to_name: String::new(),
            },
            demolished: false,
            build_pay: None,
            tax: None,
            sale: None,
        },
    );
    app.insert_resource(houses);
    app.update();
    let rt = app.world().get_non_send::<UiRuntime>().unwrap();
    assert_eq!(
        crate::hud::housing::interaction_target(rt),
        Some(55),
        "the parent house was not opened"
    );
    assert!(sent(&mut app).is_empty());
}

/// `DoodadFuncEvidenceItemLoot` (`FUN_393bb090`): the report text and `REPORT_CRIME`; the action
/// id the native shows is the row's `skill_id` (`FUN_393b7eb0` case 0x34).
#[test]
fn the_evidence_loot_opens_the_crime_report() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 15_249, 5911, 11_672, Inventory::default());
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.snapshot.sub_zone = "Two Crowns".into();
        d.snapshot.zone = "Marianople".into();
    }
    event_probe(&app);
    tick(&app);
    app.update();
    tick(&app);
    assert!(
        event_arg(&app, "REPORT_CRIME", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .is_some()
    );
    let text = event_arg(&app, "REPORT_CRIME", 3)
        .and_then(|v| {
            v.as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    // FUN_393bb090: the player's sub-zone and zone entry, not the target or the crime kind
    assert_eq!(text, "Two Crowns>>Marianople");
    assert_eq!(
        lua(
            &app,
            "return userTrial ~= nil and userTrial.reportCrimeWnd ~= nil and userTrial.reportCrimeWnd:IsVisible()"
        )
        .unwrap()
        .as_boolean(),
        Some(true),
        "the crime report window did not open"
    );
    assert!(sent(&mut app).is_empty(), "the report use sends nothing");

    // The report button (`report_crime.alb`) calls X2Trial.ReportCrime(text); FUN_3907bfd0 sends
    // the context FUN_393bb090 stored with opcode 0x76.
    lua(&app, "X2Trial:ReportCrime('blood on the ground')").unwrap();
    app.update();
    assert!(matches!(
        sent(&mut app).as_slice(),
        [Command::ReportCrime {
            obj_id: 2075,
            skill_id: 11_672,
            next_phase: 15_251,
            func_id: 12_578,
            msg,
        }] if msg == "blood on the ground"
    ));
}

/// `DoodadFuncPurchase` (`FUN_393b80d0`): task 10 for a money row, task 11 for a coin row; the
/// confirmation sends `CSBuyItemsPacket` / `CSBuyCoinItemPacket`.
#[test]
fn the_purchase_dialog_confirms_with_the_native_packets() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game1) = game() else { return };
    let mut money = app(game1, 11_355, 4601, 17_537, Inventory::default());
    lua(
        &money,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;\
         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)\
           local wnd=original(self,task,info,owner);\
           PURCHASE={task=task,itemCount=info.itemCount,money=info.moneyString,item=info.itemInfo,wnd=wnd}; return wnd end",
    )
    .unwrap();
    tick(&money);
    money.update();
    tick(&money);
    assert_eq!(
        lua(&money, "return PURCHASE.task").unwrap().as_integer(),
        Some(10)
    );
    assert_eq!(
        lua(&money, "return PURCHASE.itemCount")
            .unwrap()
            .as_integer(),
        Some(1)
    );
    assert!(
        lua(&money, "return PURCHASE.money ~= nil")
            .unwrap()
            .as_boolean()
            .unwrap_or(false)
    );
    // `FUN_393b9080`: the full item info (`tips::item_tip`), not a name-only stub
    assert_eq!(
        lua(&money, "return PURCHASE.item.itemType").unwrap().as_integer(),
        Some(23_614)
    );
    assert_eq!(
        lua(&money, "return PURCHASE.item.isPetOnly ~= nil")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the item info is the stub"
    );
    lua(&money, "PURCHASE.wnd.OkProc()").unwrap();
    money.update();
    let sent_buy = sent(&mut money);
    assert!(matches!(
        sent_buy.as_slice(),
        [Command::Shop(ShopCommand::Buy { npc_obj_id: 0, doodad_obj_id: 2075, func_id: 10_117, buy, buy_back, use_aa_point: false })]
            if buy.as_slice() == [BuyEntry { item_id: 23_614, grade: 0, count: 1, currency: 0 }]
                && buy_back.is_empty()
    ));

    let Some(game2) = game() else { return };
    let mut coin = app(game2, 11_691, 4729, 14_569, Inventory::default());
    event_probe(&coin);
    lua(
        &coin,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;\
         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)\
           local wnd=original(self,task,info,owner);\
           PURCHASE={task=task,itemCount=info.itemCount,coin=info.coinCount,coinName=info.coinName,wnd=wnd}; return wnd end",
    )
    .unwrap();
    tick(&coin);
    coin.update();
    tick(&coin);
    assert_eq!(
        lua(&coin, "return PURCHASE.task").unwrap().as_integer(),
        Some(11)
    );
    assert_eq!(
        lua(&coin, "return PURCHASE.coin").unwrap().as_integer(),
        Some(15)
    );
    assert!(
        lua(&coin, "return PURCHASE.coinName ~= nil")
            .unwrap()
            .as_boolean()
            .unwrap_or(false)
    );
    // without the coins the native does not send the buy; FUN_393bac20 shows message 0x1e3
    // (`not_enough_coin`) with the coin name and count.
    lua(&coin, "PURCHASE.wnd.OkProc()").unwrap();
    coin.update();
    assert!(sent(&mut coin).is_empty(), "sent without the coin item");
    assert_eq!(
        event_arg(&coin, "SYSMSG", 2)
            .and_then(|v| v
                .as_string()
                .and_then(|s| s.to_str().ok())
                .map(|s| s.to_string()))
            .as_deref(),
        Some("not_enough_coin")
    );

    let Some(game3) = game() else { return };
    let mut paid = app(game3, 11_691, 4729, 14_569, bag_count(23_633, 15));
    lua(
        &paid,
        "PURCHASE=true; local original=X2DialogManager.RequestTaskDialog;\
         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)\
           local wnd=original(self,task,info,owner); PURCHASE=wnd; return wnd end",
    )
    .unwrap();
    tick(&paid);
    paid.update();
    tick(&paid);
    lua(&paid, "PURCHASE.OkProc()").unwrap();
    paid.update();
    assert!(matches!(
        sent(&mut paid).as_slice(),
        [Command::Shop(ShopCommand::BuyCoin {
            obj_id: 2075,
            func_id: 10_262
        })]
    ));
}

/// `FUN_393bc4e0`: a missing item template refuses with 0x57 and sends nothing; a money total
/// below the price shows `NOT_ENOUGH_MONEY` (0x1a, `FUN_394f0eb0`) and still sends `CSBuyItems`.
#[test]
fn the_money_purchase_checks_the_item_and_the_price() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(mut game1) = game() else { return };
    game1.items.remove(&23_614);
    let mut missing = app(game1, 11_355, 4601, 17_537, Inventory::default());
    lua(
        &missing,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)           PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&missing);
    missing.update();
    tick(&missing);
    lua(&missing, "PURCHASE.OkProc()").unwrap();
    missing.update();
    let refused: Vec<_> = missing
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(refused, [STORE_INVALID_ITEM]);
    assert!(sent(&mut missing).is_empty());

    let Some(game2) = game() else { return };
    let mut poor = app(game2, 11_355, 4601, 17_537, Inventory::default());
    lua(
        &poor,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)           PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&poor);
    poor.update();
    tick(&poor);
    poor.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .snapshot
        .money = 0;
    lua(&poor, "PURCHASE.OkProc()").unwrap();
    poor.update();
    let refused: Vec<_> = poor
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(refused, [NOT_ENOUGH_MONEY]);
    assert!(matches!(
        sent(&mut poor).as_slice(),
        [Command::Shop(ShopCommand::Buy { doodad_obj_id: 2075, func_id: 10_117, .. })]
    ));
}

/// A purchase dialog closes when the next local interaction starts (`FUN_393b7b10` ->
/// `FUN_393b7aa0` -> `INTERACTION_END`); its old OK must not send.
#[test]
fn a_purchase_dialog_cannot_confirm_after_the_next_interaction() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 11_355, 4601, 17_537, Inventory::default());
    lua(
        &app,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)           PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return PURCHASE ~= nil and PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(true)
    );

    // The doodad becomes a mailbox; opening it ends the purchase interaction.
    app.world_mut().resource_mut::<Doodads>().by_id.insert(
        2075,
        world_doodad(
            2075,
            MAILBOX_TEMPLATE,
            MAILBOX_GROUP,
            Vec3::new(10.0, 10.0, 100.0),
        ),
    );
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.doodad.focus = Some(Focus {
            obj_id: 2075,
            skills: vec![MAILBOX_ACTION],
            craft_icon: None,
            key: -3.0,
            dist: 3.0,
        });
        let generation = d.dynamic_actions.shown().and_then(|s| match s.source {
            Source::Doodad { generation, .. } => Some(generation),
            _ => None,
        });
        d.dynamic_actions.replace(Some(Shown {
            source: Source::Doodad {
                obj_id: 2075,
                phase: MAILBOX_GROUP,
                generation: generation.unwrap_or_default(),
            },
            skills: vec![MAILBOX_ACTION],
        }));
        d.dynamic_actions.request(1);
    }
    app.update();
    assert_eq!(
        lua(&app, "return PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the purchase dialog stayed open"
    );
    // The closed dialog's OK is inert.
    lua(&app, "PURCHASE.OkProc()").unwrap();
    app.update();
    assert!(sent(&mut app).is_empty(), "the old purchase still sent");
}

/// Native `FUN_393b7b10` runs `FUN_393b7aa0` before the bench (`FUN_394dc360`): the open
/// purchase dialog (state 8) is an active interaction and is cancelled; its OK cannot send.
#[test]
fn a_purchase_dialog_cannot_confirm_after_a_bench_opens() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 11_355, 4601, 17_537, Inventory::default());
    auction_end_probe(&app);
    lua(
        &app,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;\
         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)\
         PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return PURCHASE ~= nil and PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the purchase dialog did not open"
    );
    request_bench(&mut app, 274, 10);
    app.update();
    assert_eq!(
        lua(&app, "return PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the purchase dialog stayed open under the bench"
    );
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));
    assert!(
        app.world()
            .get_non_send::<HudBridge>()
            .unwrap()
            .0
            .borrow()
            .doodad
            .local_state
            .is_none(),
        "the bench left the purchase state behind"
    );
    // The cancelled dialog's OK is inert.
    lua(&app, "PURCHASE.OkProc()").unwrap();
    app.update();
    assert!(sent(&mut app).is_empty(), "the old purchase still sent");
}

/// Native `FUN_393b7b10(5, ...)` (the coffer open) also runs `FUN_393b7aa0` first: the open
/// purchase dialog is cancelled before the coffer request leaves.
#[test]
fn a_purchase_dialog_cannot_confirm_after_a_coffer_opens() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 11_355, 4601, 17_537, Inventory::default());
    auction_end_probe(&app);
    lua(
        &app,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;\
         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)\
         PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return PURCHASE ~= nil and PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the purchase dialog did not open"
    );
    {
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        let mut d = bridge.0.borrow_mut();
        d.doodad.menu = Some((2075, Vec::new()));
        d.doodad
            .chosen
            .push(crate::hud::doodad::NAVI_ID + crate::hud::doodad::Navi::Coffer as i64);
    }
    app.update();
    assert_eq!(
        lua(&app, "return PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the purchase dialog stayed open under the coffer"
    );
    assert_eq!(
        lua(&app, "return INTER_ENDS").unwrap().as_integer(),
        Some(1)
    );
    assert!(
        matches!(
            sent(&mut app).as_slice(),
            [Command::Coffer(aa_net::CofferCommand::Interact {
                obj_id: 2075,
                open: true
            })]
        ),
        "the coffer open was not sent"
    );
}

/// Native `FUN_393b7b10(5, obj)` runs `FUN_393b7aa0` first, which closes the open coffer
/// (`FUN_394d08c0`) before the new `CSCofferInteraction` open leaves, including when the same
/// coffer is picked again.
#[test]
fn a_coffer_open_closes_the_open_coffer_first() {
    if !aa_cry::pak_available() {
        return;
    }
    for open in [2075_u32, 42] {
        let Some(game) = game() else { return };
        let mut inventory = Inventory::default();
        inventory.coffer.obj_id = open;
        inventory.coffer.slots = vec![None; 10];
        let mut app = base_app(game, 22_065, 7983, 12_083, inventory);
        app.add_systems(Update, execute);
        {
            let bridge = app.world().get_non_send::<HudBridge>().unwrap();
            let mut d = bridge.0.borrow_mut();
            d.doodad.focus = None;
            d.dynamic_actions.replace(None);
            d.doodad.menu = Some((2075, Vec::new()));
            d.doodad
                .chosen
                .push(crate::hud::doodad::NAVI_ID + crate::hud::doodad::Navi::Coffer as i64);
        }
        app.update();
        let sent = sent(&mut app);
        assert_eq!(sent.len(), 2, "the coffer {open} close and open");
        assert!(
            matches!(
                &sent[0],
                Command::Coffer(aa_net::CofferCommand::Interact {
                    obj_id,
                    open: false
                }) if *obj_id == open
            ),
            "the coffer {open} close must precede the open"
        );
        assert!(matches!(
            &sent[1],
            Command::Coffer(aa_net::CofferCommand::Interact {
                obj_id: 2075,
                open: true
            })
        ));
        assert!(!app.world().resource::<Inventory>().coffer.is_open());
    }
}

/// Points the bar of an [`app`] at doodad 2075 as `template` in phase `group`, offering `action`,
/// and requests it once (the next `execute` runs it).
fn retarget(app: &mut App, template: u32, group: u32, action: u32) {
    app.world_mut().resource_mut::<Doodads>().by_id.insert(
        2075,
        world_doodad(2075, template, group, Vec3::new(10.0, 10.0, 100.0)),
    );
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    let mut d = bridge.0.borrow_mut();
    d.doodad.focus = Some(Focus {
        obj_id: 2075,
        skills: vec![action],
        craft_icon: None,
        key: -3.0,
        dist: 3.0,
    });
    let generation = d.dynamic_actions.shown().and_then(|s| match s.source {
        Source::Doodad { generation, .. } => Some(generation),
        _ => None,
    });
    d.dynamic_actions.replace(Some(Shown {
        source: Source::Doodad {
            obj_id: 2075,
            phase: group,
            generation: generation.unwrap_or_default(),
        },
        skills: vec![action],
    }));
    d.dynamic_actions.request(1);
}

fn set_pos(app: &App, pos: Vec3) {
    app.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .pos = pos;
}

fn mail_open(app: &App) -> bool {
    lua(app, "return IsActivatedMailBox()")
        .unwrap()
        .as_boolean()
        .unwrap_or(false)
}

/// The mailbox [`app`] with the window opened by its bar action.
fn open_mailbox() -> Option<App> {
    if !aa_cry::pak_available() {
        return None;
    }
    let mut app = app(
        game()?,
        MAILBOX_GROUP,
        MAILBOX_TEMPLATE,
        MAILBOX_ACTION,
        Inventory::default(),
    );
    tick(&app);
    app.update();
    tick(&app);
    assert!(mail_open(&app), "the mailbox window did not open");
    Some(app)
}

/// The mailbox is interaction state 4 (`FUN_393b1040`): `FUN_393b7e40` ends it when the player
/// walks off, and `mailbox.lua` hides the window on `INTERACTION_END`.
#[test]
fn walking_away_from_the_mailbox_closes_the_mail_window() {
    let Some(mut app) = open_mailbox() else { return };
    set_pos(&app, Vec3::new(1000.0, 1000.0, 100.0));
    app.update();
    assert!(!mail_open(&app), "the mail window stayed open after the walk-away");
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    assert_eq!(bridge.0.borrow().doodad.local_state, None);
}

/// A store opened after the mailbox ends the mailbox interaction first (`FUN_393b7b10`).
#[test]
fn a_store_after_the_mailbox_closes_the_mail_window() {
    let Some(mut app) = open_mailbox() else { return };
    retarget(&mut app, STORE_TEMPLATE, STORE_GROUP, 12_087);
    app.update();
    tick(&app);
    assert!(!mail_open(&app), "the mail window stayed open beside the store");
    assert_eq!(
        lua(&app, "return IsActivatedStore()").unwrap().as_boolean(),
        Some(true),
        "the store window did not open"
    );
}

/// Using the mailbox again while its window is open: the native ends state 4 first
/// (`INTERACTION_END` hides the window), then `TOGGLE_MAIL` opens it again, so it stays visible.
#[test]
fn a_second_mailbox_use_keeps_the_mail_window_open() {
    let Some(mut app) = open_mailbox() else { return };
    retarget(&mut app, MAILBOX_TEMPLATE, MAILBOX_GROUP, MAILBOX_ACTION);
    app.update();
    tick(&app);
    assert!(mail_open(&app), "the second use toggled the mail window closed");
    assert!(sent(&mut app).is_empty());
}

/// A farm board opened from the doodad is interaction state 10 (`FUN_393b8700`): it closes at the
/// native 2.1 m measured gap (`FUN_393b7e40`), not at the packet path's 10 m `FARM_LEAVE`.
#[test]
fn a_doodad_farm_board_closes_at_the_interaction_reach() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, FARM_GROUP, FARM_TEMPLATE, 16_968, Inventory::default());
    app.world()
        .get_non_send::<HudBridge>()
        .unwrap()
        .0
        .borrow_mut()
        .doodad
        .bounds
        .insert(2075, (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)));
    let board_open = |app: &App| {
        let rt = app.world().get_non_send::<UiRuntime>().unwrap();
        let ui = aa_ui::lua::ui(&rt.lua);
        ui.tree.find("farm_info").is_some_and(|w| ui.tree.visible(w))
    };
    tick(&app);
    app.update();
    tick(&app);
    assert!(board_open(&app), "the farm board did not open");
    // about 0.5 m from the model: still in reach
    set_pos(&app, Vec3::new(10.0, 11.0, 100.0));
    app.update();
    assert!(board_open(&app), "the farm board closed in reach");
    // about 2.5 m from the model: past the 2.1 m reach, well inside 10 m
    set_pos(&app, Vec3::new(10.0, 13.0, 100.0));
    app.update();
    assert!(!board_open(&app), "the farm board stayed open past the interaction reach");
}

/// A bench opened while a doodad bank is open ends the bank first (`FUN_394dc360` ->
/// `FUN_393b7b10` -> `FUN_393b7aa0`): `NPC_INTERACTION_END` and `INTERACTION_END` once each,
/// before `CRAFTING_START`.
#[test]
fn a_bench_after_a_doodad_bank_ends_the_bank() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game_data) = game() else { return };
    let mut app = app(
        game_data,
        BANK_GROUP,
        BANK_TEMPLATE,
        13_238,
        Inventory::default(),
    );
    lua(
        &app,
        "NPC_ENDS=0; INTER_ENDS=0; ENDS_AT_CRAFT=-1;          local a=UIParent:CreateWidget('emptywidget','bankBenchProbe','UIParent');          a:RegisterEvent('NPC_INTERACTION_END'); a:RegisterEvent('INTERACTION_END'); a:RegisterEvent('CRAFTING_START');          a:SetHandler('OnEvent',function(_,event) if event == 'NPC_INTERACTION_END' then NPC_ENDS=NPC_ENDS+1 elseif event == 'INTERACTION_END' then INTER_ENDS=INTER_ENDS+1 else ENDS_AT_CRAFT=NPC_ENDS+INTER_ENDS end end)",
    )
    .unwrap();
    app.update();
    assert_eq!(
        lua(&app, "return IsActivatedBank()").unwrap().as_boolean(),
        Some(true),
        "the bank window did not open"
    );
    retarget(
        &mut app,
        BANK_TEMPLATE,
        BANK_GROUP,
        crate::hud::doodad::CRAFT_ACTION,
    );
    app.update();
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(1));
    assert_eq!(lua(&app, "return INTER_ENDS").unwrap().as_integer(), Some(1));
    assert_eq!(
        lua(&app, "return ENDS_AT_CRAFT").unwrap().as_integer(),
        Some(2),
        "the bank ended after CRAFTING_START"
    );
    assert_eq!(
        lua(&app, "return IsActivatedBank()").unwrap().as_boolean(),
        Some(false),
        "the bank window stayed open beside the bench"
    );
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    assert!(bridge.0.borrow().store.npc.is_none());
}

/// `FUN_393b3d50` runs before the local funcs (`FUN_393b3ec0`): a mounted player's use of the store
/// doodad (12087, `source_mount` and `source_mount_mate` off) is refused with 222 and opens nothing.
/// Runs without the pak.
#[test]
fn a_mounted_store_use_is_refused_before_the_local_window() {
    let mut tables = fixture_tables();
    tables.mount_flags.insert(12_087, (false, false));
    let game = GameData {
        doodad: tables,
        ..Default::default()
    };
    let mut app = dispatch_app(game, 1, 100, 12_087, 42, Inventory::default(), None);
    app.insert_resource(crate::ingame::LocalMount(Some(crate::ingame::LocalSeat {
        mount: 9,
        point: 1,
        drives: true,
        cart: false,
        ship: false,
        turret: false,
        speed: Default::default(),
        lift: 0.0,
    })));
    app.update();
    let errors: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<crate::hud::ShowError>>()
        .drain()
        .map(|error| error.0)
        .collect();
    assert_eq!(errors, [222]);
    assert!(sent(&mut app).is_empty());
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    assert!(bridge.0.borrow().store.npc.is_none());
}

/// `FUN_393b7e40` ends any local window state on walk-away, not only the auction: an open
/// purchase dialog (state 8) gets one `INTERACTION_END`, `dialog_handler.lua` cancels it, and its
/// OK no longer sends `CSBuyItems` for the far doodad.
#[test]
fn walking_away_from_a_purchase_cancels_the_dialog() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 11_355, 4601, 17_537, Inventory::default());
    auction_end_probe(&app);
    lua(
        &app,
        "PURCHASE=nil; local original=X2DialogManager.RequestTaskDialog;         X2DialogManager.RequestTaskDialog=function(self,task,info,owner)           PURCHASE=original(self,task,info,owner); return PURCHASE end",
    )
    .unwrap();
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return PURCHASE ~= nil and PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(true)
    );
    let state = |app: &App| {
        app.world()
            .get_non_send::<HudBridge>()
            .unwrap()
            .0
            .borrow()
            .doodad
            .local_state
            .map(|state| state.state)
    };
    assert_eq!(state(&app), Some(8));
    assert!(sent(&mut app).is_empty());

    set_pos(&app, Vec3::new(1000.0, 1000.0, 100.0));
    app.update();
    assert_eq!(state(&app), None, "the walk-away left the purchase state");
    assert_eq!(lua(&app, "return INTER_ENDS").unwrap().as_integer(), Some(1));
    assert_eq!(lua(&app, "return NPC_ENDS").unwrap().as_integer(), Some(0));
    assert_eq!(
        lua(&app, "return PURCHASE:IsVisible()")
            .unwrap()
            .as_boolean(),
        Some(false),
        "the purchase dialog stayed open after the walk-away"
    );
    lua(&app, "PURCHASE.OkProc()").unwrap();
    app.update();
    assert!(sent(&mut app).is_empty(), "the far purchase still sent");
    assert_eq!(lua(&app, "return INTER_ENDS").unwrap().as_integer(), Some(1));
}

/// `FUN_394ce5d0`: with no auctioneer NPC, the window's requests carry the state-0xe doodad
/// (`FUN_394ce540`) as `obj_id2`, and they are sent; out of reach, 388 ends the state instead.
#[test]
fn a_doodad_auction_sends_the_doodad_as_the_auctioneer() {
    if !aa_cry::pak_available() {
        return;
    }
    let Some(game) = game() else { return };
    let mut app = app(game, 22_065, 7983, 12_083, Inventory::default());
    tick(&app);
    app.update();
    tick(&app);
    assert_eq!(
        lua(&app, "return IsActivatedAuction()")
            .unwrap()
            .as_boolean(),
        Some(true),
        "the auction window did not open"
    );
    assert!(sent(&mut app).is_empty());
    lua(&app, "X2Auction:SearchAuctionArticle(3, 10, 20, 4, 10203, false, 'Sword')").unwrap();
    app.update();
    let searches: Vec<_> = sent(&mut app)
        .into_iter()
        .filter_map(|command| match command {
            Command::Auction(aa_net::AuctionCommand::Search { auctioneer, .. }) => Some(auctioneer),
            _ => None,
        })
        .collect();
    assert_eq!(
        searches,
        [aa_net::auction::AuctioneerIds {
            obj_id: 0,
            obj_id2: 2075
        }]
    );

    // Out of reach the request is not sent and the auction state ends.
    lua(&app, "X2Auction:SearchAuctionArticle(3, 10, 20, 4, 10203, false, 'Sword')").unwrap();
    set_pos(&app, Vec3::new(1000.0, 1000.0, 100.0));
    app.update();
    app.update();
    assert!(
        !sent(&mut app)
            .iter()
            .any(|command| matches!(command, Command::Auction(_))),
        "an auction request left for the far doodad"
    );
    let bridge = app.world().get_non_send::<HudBridge>().unwrap();
    assert_eq!(bridge.0.borrow().doodad.local_state, None);
}

/// `FUN_393bc4e0` / `FUN_394f0eb0`: a money total below price * count still sends `CSBuyItems`
/// (the caller shows `NOT_ENOUGH_MONEY`). `FUN_393bac20` counts the coins first: enough coins send
/// `CSBuyCoinItem` even without the coin template, which only the shortage message needs (27
/// without it). Runs without the pak.
#[test]
fn the_purchase_checks_follow_the_native_order() {
    let mut game = GameData::default();
    game.items.insert(
        23_614,
        crate::hud::data::ItemInfo {
            price: 50,
            ..Default::default()
        },
    );
    let buy = PurchaseOrder::Buy {
        obj_id: 2075,
        func_id: 10_117,
        item: 23_614,
        count: 2,
        currency: 0,
    };
    let inventory = Inventory::default();
    assert!(matches!(
        purchase_command(buy, &inventory, Some(&game), 99),
        Ok((Command::Shop(ShopCommand::Buy { doodad_obj_id: 2075, func_id: 10_117, .. }), true))
    ));
    assert!(matches!(purchase_command(buy, &inventory, Some(&game), 100), Ok((_, false))));
    assert_eq!(
        purchase_command(buy, &inventory, None, 100).err(),
        Some(PurchaseRefusal::InvalidItem)
    );

    let coin = PurchaseOrder::Coin {
        obj_id: 2075,
        func_id: 10_262,
        coin_item: 23_633,
        coin_count: 15,
    };
    assert!(matches!(
        purchase_command(coin, &bag_count(23_633, 15), None, 0),
        Ok((Command::Shop(ShopCommand::BuyCoin { obj_id: 2075, func_id: 10_262 }), false))
    ));
    assert_eq!(
        purchase_command(coin, &bag_count(23_633, 14), None, 0).err(),
        Some(PurchaseRefusal::UnknownCoin)
    );
    game.items.insert(23_633, crate::hud::data::ItemInfo::default());
    assert_eq!(
        purchase_command(coin, &bag_count(23_633, 14), Some(&game), 0).err(),
        Some(PurchaseRefusal::NotEnoughCoin {
            coin_item: 23_633,
            count: 15
        })
    );
}
