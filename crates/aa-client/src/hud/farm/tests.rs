use super::*;
use std::sync::Arc;

use aa_net::Pos;
use aa_net::farm::CommonFarmEntry;

use crate::hud::api::HudBridge;
use crate::hud::data::housing::{CommonFarm, FarmGroup, FarmGroupDoodad, FarmGrowthStep, Tables};
use crate::hud::data::{ItemInfo, Named, ZoneGroup};

fn game() -> GameData {
    let mut game = GameData::default();
    let mut housing = Tables::default();
    for (id, name, count, description) in [(1, "Farm", 10, "crops"), (2, "Nursery", 5, "trees")] {
        housing.farm_groups.insert(
            id,
            FarmGroup {
                id,
                name: name.into(),
                count,
                description: description.into(),
            },
        );
        housing.farm_group_ids.push(id);
    }
    housing.farm_group_doodads = vec![
        FarmGroupDoodad {
            id: 13,
            farm_group_id: 1,
            doodad_id: 380,
            item_id: 13725,
        },
        FarmGroupDoodad {
            id: 14,
            farm_group_id: 1,
            doodad_id: 381,
            item_id: 13726,
        },
        FarmGroupDoodad {
            id: 15,
            farm_group_id: 2,
            doodad_id: 413,
            item_id: 4865,
        },
    ];
    housing.common_farms = vec![
        CommonFarm {
            id: 10,
            name: "Farm".into(),
            guard_time: 86_400_000,
            farm_group_id: 1,
            comments: "zone (33)".into(),
        },
        CommonFarm {
            id: 12,
            name: "Nursery".into(),
            guard_time: 0,
            farm_group_id: 2,
            comments: String::new(),
        },
    ];
    game.housing = housing;
    game.items.insert(
        13725,
        ItemInfo {
            named: Named {
                name: "Parsley".into(),
                icon: String::new(),
            },
            ..Default::default()
        },
    );
    game.housing
        .farm_doodad_names
        .insert(380, "Parsley Doodad".into());
    game.housing.farm_phase_names.insert(77, "Growing".into());
    game.housing.farm_phase_names.insert(78, "Next".into());
    game.housing.farm_phase_names.insert(79, "Ripe".into());
    game.housing.farm_growth_steps.insert(
        77,
        FarmGrowthStep {
            delay_ms: 5000,
            next_phase: 78,
        },
    );
    game.housing.farm_growth_steps.insert(
        78,
        FarmGrowthStep {
            delay_ms: 1000,
            next_phase: 0,
        },
    );
    game.housing.farm_climates.insert(380, 3);
    // `DAT_39ac318c` = 1 ("all"): this template always scales.
    game.housing.farm_climates.insert(381, 1);
    game.zones.insert(129, (5, 0));
    game.zone_groups.insert(
        5,
        ZoneGroup {
            text: "Gweonid Forest".into(),
            key: "w_gweonid_forest".into(),
            rect: [0.0; 4],
        },
    );
    game.zone_climate.insert(129, 8);
    game.zone_climate_elems.insert(8, vec![3]);
    game
}

fn zone_world(zone: u16) -> aa_world::stream::WorldInfo {
    let mut world = aa_world::World::default();
    world.cells.insert((0, 0), vec![zone]);
    aa_world::stream::WorldInfo(Arc::new(world))
}

fn setup(game: GameData) -> (UiRuntime, Shared) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(game).is_ok());
    super::super::api::install(&mut rt, &bridge).unwrap();
    (rt, bridge)
}

fn entry(farm_type: u32, func_group_id: u32, time_left: u32, plant_time: i64) -> CommonFarmEntry {
    CommonFarmEntry {
        farm_type,
        template_id: 380,
        time_left,
        func_group_id,
        pos: Pos::new(100.0, 200.0, 30.0),
        plant_time,
    }
}

fn list(entries: Vec<CommonFarmEntry>) -> CommonFarmList {
    CommonFarmList {
        max_count: entries.len() as i32,
        entries,
    }
}

#[test]
fn static_getters_match_the_native_shapes() {
    let (rt, _) = setup(game());
    rt.lua
        .load(
            r#"
            local ids = X2:GetFarmGroups()
            assert(#ids == 2 and ids[1].type == 1 and ids[2].type == 2, 'farm groups')
            assert(ids[1].name == 'Farm' and ids[2].name == 'Nursery', 'farm group names')
            local name, count, description = X2:GetFarmGorupInfo(1)
            assert(name == 'Farm' and count == 10 and description == 'crops', 'group info')
            assert(X2:GetFarmGorupInfo(99) == nil, 'unknown group')
            local doodads = X2:GetFarmGroupDoodadList(1)
            assert(#doodads == 2, 'doodad list')
            assert(X2:GetFarmGroupDoodadList(99) == nil, 'unknown group list')
            local farm, guard, group = X2:GetCommonFarmInfo(10)
            assert(farm == 'Farm' and guard == 86400000 and group == 1, 'common farm')
            assert(X2:GetCommonFarmInfo(99) == nil, 'unknown common farm')
        "#,
        )
        .exec()
        .unwrap();
}

#[test]
fn runtime_entries_use_seconds_the_common_farm_and_the_crop_zone() {
    let game = game();
    let now = entry(10, 77, 5000, 0).plant_time + 10;
    let world = zone_world(129);
    let crops = build_runtime(
        Some(&game),
        Some(world.0.as_ref()),
        &list(vec![
            entry(10, 77, 5000, now - 10),
            entry(10, 79, 0, now - 10),
        ]),
        now,
    );
    assert_eq!(crops.len(), 2);
    let growing = &crops[0];
    assert_eq!(growing.farm_group_id, 1);
    assert_eq!(growing.farm_type, 10, "the packet id is the common farm id");
    assert_eq!(growing.name, "Parsley");
    assert_eq!(growing.doodad_name, "Parsley Doodad");
    assert_eq!(growing.current_phase_name, "Growing");
    assert_eq!(growing.item_type, 13725);
    assert_eq!(
        growing.left_growth, 5,
        "5000 ms + 1000 ms * 0.7, in seconds"
    );
    assert_eq!(growing.left_protect, 86_390);
    assert_eq!(growing.location, "Gweonid Forest");
    assert_eq!(growing.vpos, (100.0, 200.0));
    assert!(!growing.grow_up);
    let grown = &crops[1];
    assert_eq!(grown.left_growth, 0);
    assert!(grown.grow_up, "no further growth phase");

    // The climate flag scales the following phases: 1.0 normally, 0.7 with the zone climate.
    assert_eq!(growth_remaining_ms(&game, 77, 5000, false), 6000);
    assert_eq!(growth_remaining_ms(&game, 77, 5000, true), 5700);
    assert!(climate_reduces_growth(&game, Some(129), 380), "zone list");
    assert!(
        climate_reduces_growth(&game, None, 381),
        "climate 1 is all-zones"
    );
    assert!(!climate_reduces_growth(&game, None, 380), "3 is not all");

    // Unknown common farms, farm groups, doodads and func groups are skipped.
    assert!(
        build_runtime(
            Some(&game),
            None,
            &list(vec![entry(99, 77, 5000, now), entry(10, 9999, 5000, now),]),
            now
        )
        .is_empty()
    );
}

#[test]
fn a_list_event_updates_the_runtime_getters_and_raises_common_farm_updated() {
    let (rt, bridge) = setup(game());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(zone_world(129))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
            UPDATED = 0
            local probe = UIParent:CreateWidget('emptywidget', 'farmUpdatedProbe', 'UIParent')
            probe:RegisterEvent('COMMON_FARM_UPDATED')
            probe:SetHandler('OnEvent', function() UPDATED = UPDATED + 1 end)
        "#,
        )
        .exec()
        .unwrap();
    app.world_mut()
        .write_message(NetEvent(Event::Farm(FarmEvent::List(list(vec![entry(
            10,
            77,
            5000,
            unix_secs() - 10,
        )])))));
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    assert_eq!(rt.lua.load("return UPDATED").eval::<i64>().unwrap(), 1);
    rt.lua
        .load(
            r#"
            assert(X2:GetRuntimeCommonFarmDoodadCount(1) == 1)
            assert(X2:GetRuntimeCommonFarmDoodadCount(2) == 0)
            assert(X2:GetRuntimeCommonFarmDoodadsInfo(2) == nil)
            local crops = X2:GetRuntimeCommonFarmDoodadsInfo(1)
            assert(#crops == 1)
            local crop = crops[1]
            assert(crop.farmType == 10 and crop.itemType == 13725)
            assert(crop.name == 'Parsley' and crop.doodadName == 'Parsley Doodad')
            assert(crop.currentPhaseName == 'Growing')
            assert(crop.location == 'Gweonid Forest')
            assert(crop.vPosX == 100 and crop.vPosY == 200)
            assert(crop.leftGrowthTime <= 5 and crop.leftGrowthTime >= 4)
            assert(crop.leftProtectTime > 86000 and crop.leftProtectTime <= 86400)
            assert(crop.growUp == false)
            assert(type(crop.leftProtectTime) == 'number')
        "#,
        )
        .exec()
        .unwrap();
}

#[test]
fn show_raises_open_common_farm_info_and_walking_away_ends_it() {
    let (rt, bridge) = setup(game());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
            FARM_OPENED = 0
            FARM_ENDED = 0
            local open = UIParent:CreateWidget('emptywidget', 'farmOpenProbe', 'UIParent')
            open:RegisterEvent('OPEN_COMMON_FARM_INFO')
            open:SetHandler('OnEvent', function(self, event, kind) FARM_OPENED = kind end)
            local ended = UIParent:CreateWidget('emptywidget', 'farmEndProbe', 'UIParent')
            ended:RegisterEvent('INTERACTION_END')
            ended:SetHandler('OnEvent', function() FARM_ENDED = FARM_ENDED + 1 end)
        "#,
        )
        .exec()
        .unwrap();
    bridge.borrow_mut().pos = aa_world::cry_to_bevy(Vec3::new(100.0, 200.0, 30.0));
    app.world_mut()
        .write_message(NetEvent(Event::Farm(FarmEvent::Show(
            aa_net::ShowCommonFarm {
                farm_type: 10,
                count: 1,
                x: aa_net::wire::coord_to_long(100.0),
                y: aa_net::wire::coord_to_long(200.0),
                z: 30.0,
            },
        ))));
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    assert_eq!(rt.lua.load("return FARM_OPENED").eval::<i64>().unwrap(), 10);
    assert_eq!(rt.lua.load("return FARM_ENDED").eval::<i64>().unwrap(), 0);
    bridge.borrow_mut().pos = aa_world::cry_to_bevy(Vec3::new(200.0, 200.0, 30.0));
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    assert_eq!(rt.lua.load("return FARM_ENDED").eval::<i64>().unwrap(), 1);
}

#[test]
fn the_request_gate_sends_once_and_fires_the_update_even_when_refused() {
    let Some(db) = std::env::var("AA_DB")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
    else {
        return;
    };
    let (rt, bridge) = setup(game());
    rt.load_ui_texts(std::path::Path::new(&db)).unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
            UPDATED, MESSAGES, MESSAGE = 0, 0, nil
            local updated = UIParent:CreateWidget('emptywidget', 'farmRequestUpdated', 'UIParent')
            updated:RegisterEvent('COMMON_FARM_UPDATED')
            updated:SetHandler('OnEvent', function() UPDATED = UPDATED + 1 end)
            local messages = UIParent:CreateWidget('emptywidget', 'farmRequestMessage', 'UIParent')
            messages:RegisterEvent('SYSMSG')
            messages:SetHandler('OnEvent', function(self, event, text) MESSAGES = MESSAGES + 1; MESSAGE = text end)
        "#,
        )
        .exec()
        .unwrap();
    let drain = |app: &mut App| {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|v| v.0)
            .collect::<Vec<_>>()
    };
    let state = app
        .world()
        .non_send::<UiRuntime>()
        .lua
        .app_data_ref::<State>()
        .unwrap()
        .clone();
    state.0.borrow_mut().list = Some(Received {
        entries: Vec::new(),
        at: Instant::now(),
    });
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    assert!(
        state.0.borrow().list.is_none(),
        "a sent request clears the list"
    );
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::RequestCommonFarmList]
    ));
    // A second request inside 2 s only refreshes the window and keeps the list.
    state.0.borrow_mut().list = Some(Received {
        entries: Vec::new(),
        at: Instant::now(),
    });
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    app.update();
    assert!(drain(&mut app).is_empty());
    assert!(state.0.borrow().list.is_some());
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    assert_eq!(rt.lua.load("return UPDATED").eval::<i64>().unwrap(), 1);
    // An instant zone refuses (0x242), clears the list and still refreshes.
    bridge.borrow_mut().snapshot.instance_id = 9;
    state.0.borrow_mut().list = Some(Received {
        entries: Vec::new(),
        at: Instant::now(),
    });
    state.0.borrow_mut().last_request = None;
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    app.update();
    assert!(drain(&mut app).is_empty());
    assert!(state.0.borrow().list.is_none());
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    assert_eq!(rt.lua.load("return UPDATED").eval::<i64>().unwrap(), 2);
    assert_eq!(rt.lua.load("return MESSAGES").eval::<i64>().unwrap(), 1);
    rt.lua
        .load(
            "local want = X2Locale:LocalizeUiText(ERROR_MSG, 'CANNOT_REQUEST_COMMONFARM_INFO_IN_INSTANT_ZONE') assert(MESSAGE == want) assert(MESSAGE ~= 'CANNOT_REQUEST_COMMONFARM_INFO_IN_INSTANT_ZONE')",
        )
        .exec()
        .unwrap();
}

fn load_commonfarm(rt: &UiRuntime) -> bool {
    if let Err(e) = aa_lua::run_alb(&rt.lua, aa_lua::stage::INIT_SCRIPT) {
        eprintln!("skip: base locale: {e}");
        return false;
    }
    for toc in [
        "scriptsbin/x2ui/baselib/toc.g",
        "scriptsbin/x2ui/commonfarm/toc.g",
    ] {
        let errors = aa_lua::run_toc_collect(&rt.lua, toc).unwrap();
        if let Some((script, error)) = errors.first() {
            eprintln!("skip: {script}: {error}");
            return false;
        }
    }
    if let Err(e) = aa_lua::run_alb(
        &rt.lua,
        "scriptsbin/x2ui/hud/main_menu_bar/main_menu/table.alb",
    ) {
        eprintln!("skip: main menu table: {e}");
        return false;
    }
    true
}

#[test]
fn the_original_commonfarm_scripts_open_the_info_and_my_farm_windows() {
    if !std::env::var("AA_PAK").is_ok_and(|p| std::path::Path::new(&p).exists()) {
        return;
    }
    let Some(db) = std::env::var("AA_DB")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
    else {
        return;
    };
    let game = GameData::load(&db).unwrap();
    // `doodad_func_open_farm_infos.farm_id` is 1..4 in 1.2: those ids are also the matching
    // `farm_groups` rows (the scripts pass the same value to `GetCommonFarmInfo` and
    // `GetFarmGorupInfo`).
    let farm = game
        .housing
        .common_farms
        .iter()
        .find(|f| f.id == f.farm_group_id)
        .map_or(1, |f| f.id);
    let (rt, _) = setup(game);
    if !load_commonfarm(&rt) {
        return;
    }
    rt.lua
        .load(
            r#"
            local found = false
            for _, entry in pairs(MAIN_MENU_INFO) do
                if type(entry) == 'table' and entry.hotkey == 'toggle_common_farm_info' then
                    found = true
                end
            end
            assert(found, 'main menu entry')
        "#,
        )
        .exec()
        .unwrap();
    rt.lua
        .load(format!(
            "local n = X2:GetCommonFarmInfo({farm}) assert(n ~= nil, 'common farm {farm}')"
        ))
        .exec()
        .unwrap();
    let info = aa_ui::lua::ui(&rt.lua)
        .tree
        .find("farm_info")
        .expect("info window");
    assert!(!aa_ui::lua::ui(&rt.lua).tree.visible(info));
    rt.fire_event("OPEN_COMMON_FARM_INFO", (i64::from(farm),));
    assert!(aa_ui::lua::ui(&rt.lua).tree.visible(info));
    rt.fire_event("INTERACTION_END", ());
    assert!(!aa_ui::lua::ui(&rt.lua).tree.visible(info));
    let my_farm = aa_ui::lua::ui(&rt.lua)
        .tree
        .find("myFarminfo")
        .expect("my farm window");
    assert!(!aa_ui::lua::ui(&rt.lua).tree.visible(my_farm));
    assert!(aa_ui::content::hotkey(&rt.lua, "toggle_common_farm_info").unwrap());
    assert!(aa_ui::lua::ui(&rt.lua).tree.visible(my_farm));
}

#[test]
fn the_original_my_farm_list_shows_growing_and_grown_crops() {
    if !std::env::var("AA_PAK").is_ok_and(|p| std::path::Path::new(&p).exists()) {
        return;
    }
    let Some(db) = std::env::var("AA_DB")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
    else {
        return;
    };
    let game = GameData::load(&db).unwrap();
    let expected_zone = game
        .zone(129)
        .map(|(_, zone, _)| zone.text.clone())
        .unwrap_or_else(|| "unknown".into());
    let (rt, bridge) = setup(game);
    if !load_commonfarm(&rt) {
        return;
    }
    rt.lua
        .globals()
        .set("EXPECTED_ZONE", expected_zone)
        .unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(zone_world(129))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    // Opening the window runs its `ShowProc`, which requests a fresh list and clears the old one.
    {
        let rt = app.world().non_send::<UiRuntime>();
        assert!(aa_ui::content::hotkey(&rt.lua, "toggle_common_farm_info").unwrap());
    }
    app.update();
    let now = unix_secs();
    app.world_mut()
        .write_message(NetEvent(Event::Farm(FarmEvent::List(list(vec![
            // 380/1779 grows into 1780; 380/1785 is final. Both are planted in common farm 1.
            entry(1, 1779, 5000, now - 10),
            entry(1, 1785, 0, now - 10),
        ])))));
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    aa_ui::lua::tick(&rt.lua, 16.0);
    let window = aa_ui::lua::ui(&rt.lua)
        .tree
        .find("myFarminfo")
        .expect("my farm window");
    let w = aa_ui::lua::object_table(&rt.lua, window)
        .unwrap()
        .expect("window object");
    rt.lua.globals().set("MY_FARM", w).unwrap();
    rt.lua
        .load(
            r#"
            MY_FARM:RefreshCommonFarmList(1)
            local crops = X2:GetRuntimeCommonFarmDoodadsInfo(1)
            assert(#crops == 2 and crops[2].growUp == true, 'growing and grown crops')
            local growing = MY_FARM.tab.listCtrl.items[1]
            local grown = MY_FARM.tab.listCtrl.items[2]
                        local time = growing.subItems[2].textBox:GetText()
            local expected = MakeTimeString(crops[1].leftGrowthTime)
            assert(time:sub(1, #expected) == expected, time .. ' / ' .. expected)
            assert(crops[1].leftGrowthTime > 10 and crops[1].leftGrowthTime < 100000, 'leftGrowthTime is seconds')
            assert(grown.subItems[2].textBox:GetText():find(locale.commonFarm.growUp, 1, true) ~= nil)
            local location = growing.subItems[3].label:GetText()
            assert(location:find(EXPECTED_ZONE, 1, true) ~= nil, location)
        "#,
        )
        .exec()
        .unwrap();
}
