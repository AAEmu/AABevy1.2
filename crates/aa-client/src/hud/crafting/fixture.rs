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
    if std::env::args().any(|a| a == "--details") {
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
