use super::*;
use aa_net::Pos;
use aa_net::farm::CommonFarmEntry;

use crate::hud::data::housing::{CommonFarm, FarmGroup, FarmGroupDoodad, Tables};

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
    game
}

fn setup(game: GameData) -> (UiRuntime, Shared) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(game).is_ok());
    super::super::api::install(&mut rt, &bridge).unwrap();
    (rt, bridge)
}

fn entry() -> CommonFarmEntry {
    CommonFarmEntry {
        farm_type: 1,
        template_id: 380,
        time_left: 5000,
        func_group_id: 77,
        pos: Pos::new(100.0, 200.0, 30.0),
        plant_time: super::unix_ms() / 1000,
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
fn a_list_event_updates_the_runtime_getters_and_raises_common_farm_updated() {
    let (rt, bridge) = setup(game());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
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
        .write_message(NetEvent(Event::Farm(FarmEvent::List(list(vec![entry()])))));
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
            assert(crop.farmType == 1 and crop.itemType == 13725)
            assert(crop.name == '' and crop.doodadName == '')
            assert(crop.vPosX == 100 and crop.vPosY == 200)
            assert(crop.leftGrowthTime > 4000 and crop.leftGrowthTime <= 5000)
            assert(crop.leftProtectTime > 86000000)
            assert(crop.growUp == true)
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
    let (rt, bridge) = setup(game());
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    let drain = |app: &mut App| {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|v| v.0)
            .collect::<Vec<_>>()
    };
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::RequestCommonFarmList]
    ));
    // A second request inside 2 s only refreshes the window.
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    app.update();
    assert!(drain(&mut app).is_empty());
    // An instant zone refuses the request (0x242) and still refreshes.
    bridge.borrow_mut().snapshot.instance_id = 9;
    let state = app
        .world()
        .non_send::<UiRuntime>()
        .lua
        .app_data_ref::<State>()
        .unwrap()
        .clone();
    state.0.borrow_mut().last_request = None;
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2:RequestRuntimeCommonFarmDoodadInfo()")
        .exec()
        .unwrap();
    app.update();
    assert!(drain(&mut app).is_empty());
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
    if let Err(e) = aa_lua::run_alb(&rt.lua, aa_lua::stage::INIT_SCRIPT) {
        eprintln!("skip: base locale: {e}");
        return;
    }
    for toc in [
        "scriptsbin/x2ui/baselib/toc.g",
        "scriptsbin/x2ui/commonfarm/toc.g",
    ] {
        let errors = aa_lua::run_toc_collect(&rt.lua, toc).unwrap();
        if let Some((script, error)) = errors.first() {
            eprintln!("skip: {script}: {error}");
            return;
        }
    }
    if let Err(e) = aa_lua::run_alb(
        &rt.lua,
        "scriptsbin/x2ui/hud/main_menu_bar/main_menu/table.alb",
    ) {
        eprintln!("skip: main menu table: {e}");
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
