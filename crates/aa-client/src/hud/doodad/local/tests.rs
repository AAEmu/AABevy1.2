use super::*;

use bevy::prelude::*;

use aa_net::model::{Item, slot};
use aa_net::state::{Doodads, Inventory, Player, Units};
use aa_net::{Command, Event, NetCommand};

use crate::hud::api::{HudBridge, HudData, Shared};
use crate::hud::data::{GameData, SkillInfo};
use crate::hud::doodad::{Focus, PermEnv, Tables, Views, execute, find_focus, world_doodad};
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
        .init_resource::<crate::ingame::DevOptions>()
        .add_systems(Update, (execute, crate::hud::store::update));
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
        "CREATE TABLE doodad_funcs (id INTEGER, doodad_func_group_id INTEGER, func_skill_id INTEGER, actual_func_type TEXT, actual_func_id INTEGER, perm_id INTEGER);
         CREATE TABLE doodad_func_groups (id INTEGER, phase_msg TEXT, is_msg_to_zone INTEGER);
         CREATE TABLE doodad_almighties (id INTEGER, use_target_highlight INTEGER, use_target_silhouette INTEGER, use_target_decal INTEGER, target_decal_size REAL, sim_radius REAL);
         CREATE TABLE localized_texts (tbl_name TEXT, tbl_column_name TEXT, idx INTEGER, en_us TEXT);
         CREATE TABLE doodad_func_store_uis (id INTEGER, merchant_pack_id INTEGER);
         CREATE TABLE doodad_func_open_farm_infos (id INTEGER, farm_id INTEGER);
         CREATE TABLE doodad_func_insert_counters (id INTEGER, item_id INTEGER, item_count INTEGER);
         CREATE TABLE doodad_func_uses (id INTEGER, skill_id INTEGER);
         CREATE TABLE doodad_func_fake_uses (id INTEGER, fake_skill_id INTEGER);
         CREATE TABLE skills (id INTEGER);
         INSERT INTO doodad_funcs VALUES (1, 1, 12087, 'DoodadFuncStoreUi', 1, 6);
         INSERT INTO doodad_funcs VALUES (2, 2, 23780, 'DoodadFuncInsertCounter', 2, 0);
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
    }
    {
        let rt = app.world().get_non_send::<UiRuntime>().unwrap();
        let bridge = app.world().get_non_send::<HudBridge>().unwrap();
        assert!(crate::hud::doodad::end_interaction(
            Some(rt),
            &bridge.0,
            &inventory
        ));
        let d = bridge.0.borrow();
        assert_eq!(crate::hud::crafting::active_bench(&d), None);
        assert!(d.doodad.coffer_close);
        assert!(!d.doodad.menu_shown && d.doodad.menu.is_none());
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
        assert!(!crate::hud::doodad::end_interaction(
            Some(rt),
            &bridge.0,
            &inventory
        ));
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
    assert_eq!(bridge.0.borrow().doodad.local_active, None);
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
