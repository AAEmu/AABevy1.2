//! Original blacksmith window with explicit synthetic equipment and no connection.
use super::super::api::{self, Shared};
use aa_net::model::{EquipDetail, Item, ItemDetail};
use aa_net::state::Inventory;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub(crate) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf, Inventory), Box<dyn std::error::Error>> {
    let (rt, bridge, path, mut inventory) = super::super::expansion::fixture::runtime(width, height)?;
    let (template, grade, maximum) = {
        let data = bridge.borrow();
        let game = data.game.get().ok_or("fixture database not loaded")?;
        // Use an existing repairable weapon and its real price/max durability, never a fake
        // definition. Sort IDs so repeated runs choose the same template.
        let mut templates = game
            .items
            .iter()
            .filter(|(_, info)| info.impl_id == 1 && info.price > 0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        templates.sort_unstable();
        templates
            .into_iter()
            .find_map(|id| {
                let (maximum, repairable) = game.tips.items.durability(id, 0)?;
                (repairable && maximum >= 8).then_some((id, 0, maximum))
            })
            .ok_or("no repairable weapon fixture")?
    };
    let item = |id, current| Item {
        id,
        template_id: template,
        grade,
        detail: ItemDetail::Equip(EquipDetail {
            durability: current,
            ..default()
        }),
        ..default()
    };
    inventory.equipment.slots[aa_net::model::slot::MAINHAND] = Some(item(9_007_199_254_740_994, maximum / 4));
    inventory.bag[7] = Some(item(9_007_199_254_740_995, maximum / 2));
    {
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        let mut data = bridge.borrow_mut();
        data.snapshot.money = 1_000_000;
        data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
        super::refresh(&world, &mut data);
    }
    api::push_changes(&rt, &bridge);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    rt.fire_event("UNIT_EQUIPMENT_CHANGED", (aa_net::model::slot::MAINHAND + 1) as i64);
    rt.lua.load("ToggleRepairWindow(true, BLACK_SMITH); assert(repairWindow and repairWindow:IsVisible()); assert(type(X2Item:RepairAllCost())=='number' and X2Item:RepairAllCost()>0)").exec()?;
    Ok((rt, bridge, path, inventory))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, _) = runtime(width as f32, height as f32).unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original repair controls"), aa_dev::DevPlugin))
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

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
