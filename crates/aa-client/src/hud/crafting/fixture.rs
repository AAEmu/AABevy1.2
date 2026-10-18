//! Original recipe book on read-only definitions and an explicit received inventory.
use super::super::api;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub(crate) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, api::Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path, inventory) = super::super::expansion::fixture::runtime(width, height)?;
    let mut world = World::new();
    world.insert_resource(inventory);
    let mut received = aa_net::Actabilities::default();
    received.expanded_expert = Some(0);
    received.apply(&aa_net::Event::Actability(aa_net::ActabilityEvent::Snapshot { last: true, entries: vec![] }));
    world.insert_resource(received);
    super::refresh(&world, &mut bridge.borrow_mut());
    let id = aa_ui::lua::value_id(&rt.lua, &mlua::Value::String(rt.lua.create_string("craftFrame")?)).ok_or("original craft frame missing")?;
    rt.lua.globals().set(
        "CRAFTING_FRAME",
        aa_ui::lua::object_table(&rt.lua, id)?.ok_or("original craft frame table missing")?,
    )?;
    rt.lua.load("ToggleRecipeBook(true)").exec()?;
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    if std::env::args().any(|a| a == "--bench") {
        workbench(&rt, &bridge).unwrap();
    } else if std::env::args().any(|a| a == "--details") {
        select_first_recipe(&rt).unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original recipe book"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(api::HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

/// Explicit received fixtures: Alchemy Table, materials, labor and proficiency.
pub(crate) fn workbench(rt: &UiRuntime, bridge: &api::Shared) -> Result<u32, Box<dyn std::error::Error>> {
    let recipe;
    {
        let mut data = bridge.borrow_mut();
        assert_eq!(super::bench::open(&mut data, 300, 568), Some(71));
        let categories = super::bench::categories(&data, 1);
        recipe = super::bench::recipes(&data, 1, categories[0])[0];
        let game = data.game.get().ok_or("catalog not ready")?;
        let definition = &game.crafting.recipes[&recipe];
        let group = definition.actability_group;
        let points = definition.actability_limit;
        let bag = definition
            .materials
            .iter()
            .enumerate()
            .map(|(i, m)| {
                Some(aa_net::model::Item {
                    id: i as u64 + 9000,
                    template_id: m.item,
                    count: m.amount * 3,
                    grade: if m.require_grade < 0 { 0 } else { m.require_grade as u8 },
                    ..default()
                })
            })
            .collect::<Vec<_>>();
        let inventory = data.crafting.inventory.as_mut().unwrap();
        inventory.bag.fill(None);
        for (slot, item) in bag.into_iter().enumerate() {
            inventory.bag[slot] = item;
        }
        data.crafting.actabilities.apply(&aa_net::Event::Actability(aa_net::ActabilityEvent::Snapshot {
            last: true,
            entries: vec![aa_net::actability::ActabilityValue {
                id: group,
                point: points,
                step: 0,
            }],
        }));
        data.snapshot.labor = 1000;
        data.snapshot.labor_log.push((1000, 1000));
        data.snapshot.labor_seq += 1;
        let mut world = World::new();
        world.insert_resource(data.crafting.inventory.clone().unwrap());
        data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    api::push_changes(rt, bridge);
    rt.fire_event("BAG_UPDATE", (-1, -1));
    rt.fire_event("CRAFTING_START", ("300", 71));
    click_row(rt, 0)?;
    click_row(rt, 1)?;
    rt.lua
        .load("assert(CRAFTING_FRAME.detailPage.buttonFrame:IsVisible()); assert(CRAFTING_FRAME.detailPage.buttonFrame.craftBtn:IsEnabled())")
        .exec()?;
    Ok(recipe)
}

pub(crate) fn doodad() -> aa_net::Doodad {
    aa_net::Doodad {
        obj_id: 300,
        template_id: 568,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 0,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    }
}

pub(crate) fn select_first_recipe(rt: &UiRuntime) -> Result<(), Box<dyn std::error::Error>> {
    super::super::expansion::fixture::click(rt, "CRAFTING_FRAME.abilityGroupPage.buttons[1]")?;
    click_row(rt, 0)?;
    click_row(rt, 1)?;
    rt.lua
        .load("assert(CRAFTING_FRAME.detailPage.productInfoFrame.item.info and CRAFTING_FRAME.detailPage.productInfoFrame.item.info.itemType>0)")
        .exec()?;
    Ok(())
}

fn click_row(rt: &UiRuntime, row: usize) -> Result<(), Box<dyn std::error::Error>> {
    let object: mlua::Value = rt.lua.load("CRAFTING_FRAME.detailPage.categoryList.content").eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("original recipe list missing")?;
    let height: f32 = rt.lua.load("return CRAFTING_FRAME.detailPage.categoryList.content:GetHeight()").eval()?;
    let (rect, inset) = {
        let u = aa_ui::lua::ui(&rt.lua);
        (u.tree.rect(id), u.tree.widget(id).and_then(|w| w.text.as_ref()).map_or([0.0; 4], |t| t.inset))
    };
    let point = (rect.x + inset[0] + 35.0, rect.y + inset[1] + height * (row as f32 + 0.5));
    assert_eq!(
        aa_ui::input::pick(&rt.lua, point.0, point.1),
        Some(id),
        "original recipe row must receive the physical click"
    );
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), true, false, false, 0.0);
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, true, false, 0.0);
    Ok(())
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
