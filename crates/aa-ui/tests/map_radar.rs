//! Original map event handlers consume independently injected radar channels.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/radar.rs"]
mod radar_fixture;

use aa_ui::{
    UiRuntime,
    lua::{self, radar},
    tree::{Id, Rect},
};

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(
        &ui.lua,
        &ui.lua.globals().raw_get::<mlua::Value>(name).unwrap(),
    )
    .unwrap()
}

fn icons(ui: &UiRuntime, map: Id) -> Vec<([f32; 4], Rect)> {
    let u = lua::ui(&ui.lua);
    u.tree
        .widget(map)
        .unwrap()
        .drawables
        .iter()
        .filter_map(|&dr| {
            let o = u.tree.get(dr);
            let d = o.drawable()?;
            (u.tree.visible(dr) && d.texture.as_deref() == Some("ui/map/icon/npc_icon.dds"))
                .then(|| (d.coords.unwrap(), u.tree.rect(dr)))
        })
        .collect()
}

#[test]
fn original_events_project_both_maps_and_remove_clear_only_their_own_channels() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    let mut source = radar_fixture::install(&mut ui);
    let (world, road) = (id(&ui, "worldmap"), id(&ui, "roadmap"));
    for map in [world, road] {
        let drawn = icons(&ui, map);
        assert_eq!(drawn.len(), 8, "{drawn:?}");
        let ship = drawn
            .iter()
            .find(|(c, _)| *c == [144., 0., 24., 24.])
            .unwrap()
            .1;
        let r = lua::ui(&ui.lua).tree.rect(map);
        assert!(
            (ship.x + 12. - (r.x + r.w * 0.32)).abs() < 0.01,
            "{ship:?} {r:?}"
        );
        assert!(
            (ship.y + 12. - (r.y + r.h * 0.44)).abs() < 0.01,
            "{ship:?} {r:?}"
        );
    }
    let before = icons(&ui, world);
    std::sync::Arc::make_mut(&mut source.ships)[0].position[0] += 200.;
    radar::set_snapshot(&ui.lua, source);
    lua::tick(&ui.lua, 16.);
    assert_eq!(
        icons(&ui, world),
        before,
        "injection alone must not publish cached map rows"
    );
    lua::fire_ui_event(&ui.lua, "UPDATE_TELESCOPE_INFO", vec![]);
    assert_ne!(icons(&ui, world), before);
    lua::fire_ui_event(
        &ui.lua,
        "REMOVE_TELESCOPE_INFO",
        vec![mlua::Value::Integer(1_234_567_891)],
    );
    for map in [world, road] {
        assert_eq!(icons(&ui, map).len(), 7);
    }
    lua::fire_ui_event(&ui.lua, "CLEAR_TRANSFER_TELESCOPE_INFO", vec![]);
    for map in [world, road] {
        assert_eq!(icons(&ui, map).len(), 4);
    }
    lua::fire_ui_event(&ui.lua, "CLEAR_FISH_SCHOOL_INFO", vec![]);
    for map in [world, road] {
        assert_eq!(icons(&ui, map).len(), 3);
    }
    lua::fire_ui_event(&ui.lua, "CLEAR_TELESCOPE_INFO", vec![]);
    for map in [world, road] {
        assert!(icons(&ui, map).is_empty());
    }
}

#[test]
fn native_radar_symbols_hide_on_world_and_reproject_for_continent_and_resize() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    radar_fixture::install(&mut ui);
    let world = id(&ui, "worldmap");
    assert!(lua::worldmap::show_map(&ui.lua, world, 0, 2));
    lua::tick(&ui.lua, 16.);
    assert!(icons(&ui, world).is_empty());
    assert!(lua::worldmap::show_map(&ui.lua, world, 1, 3));
    lua::tick(&ui.lua, 16.);
    assert_eq!(icons(&ui, world).len(), 8);
    let old = icons(&ui, world);
    ui.exec("@resize", "worldmap:SetExtent(1392,834)").unwrap();
    lua::tick(&ui.lua, 16.);
    let resized = icons(&ui, world);
    assert_ne!(old, resized);
    assert!(resized.iter().all(|(_, r)| r.w == 24. && r.h == 24.));
}

#[test]
fn original_ship_tooltip_receives_numeric_identity_and_live_health_metadata() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    radar_fixture::install(&mut ui);
    ui.exec(
        "@capture_radar_tooltip",
        r#"
        UIParent:SetEventHandler('SHOW_WORLDMAP_TOOLTIP',function(rows,count)
            assert(count==1);radarTooltip=rows[1]
        end)
    "#,
    )
    .unwrap();
    let r = lua::ui(&ui.lua).tree.rect(id(&ui, "worldmap"));
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((r.x + r.w * 0.32, r.y + r.h * 0.44)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 16.);
    assert!(eval::<bool>(
        &ui,
        r#"
        radarTooltip and radarTooltip.tooltipType=='slave' and radarTooltip.id==201
        and radarTooltip.name=='Windfinder' and radarTooltip.owner=='Radar fixture'
        and radarTooltip.hp==720 and radarTooltip.maxHp==1000 and radarTooltip.enemy==false
        and mapTooltip:IsVisible() and mapTooltip.targetId==201
    "#
    ));
    lua::fire_ui_event(
        &ui.lua,
        "REMOVE_TELESCOPE_INFO",
        vec![mlua::Value::Integer(201)],
    );
    assert!(!eval::<bool>(&ui, "mapTooltip:IsVisible()"));
}
