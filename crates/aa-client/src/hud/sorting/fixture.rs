//! The original sort button and display mapping, using explicit fixture inventory.
use super::super::{api, expansion};
use aa_net::state::Inventory;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, api::Shared, PathBuf, Inventory), Box<dyn std::error::Error>> {
    let (rt, bridge, path, mut inventory) = expansion::fixture::runtime(width, height)?;
    inventory.bank.resize(50, None);
    inventory.bank[7] = Some(aa_net::model::Item {
        id: 9_007_199_254_740_995,
        template_id: 8000025,
        count: 1,
        ..default()
    });
    {
        let mut data = bridge.borrow_mut();
        data.snapshot.name = "Inventory fixture".into();
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    api::push_changes(&rt, &bridge);
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    Ok((rt, bridge, path, inventory))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, mut inventory) = runtime(width as f32, height as f32).unwrap();
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|a| a == "--max-bag") {
        inventory.apply(&aa_net::Event::InventoryExpanded { container: 2, slots: 150 });
        inventory.bag[146] = Some(aa_net::model::Item {
            id: 9_007_199_254_740_997,
            template_id: 8000025,
            count: 1,
            ..default()
        });
        {
            let mut data = bridge.borrow_mut();
            let mut world = World::new();
            world.insert_resource(inventory);
            data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
            data.panels.actions_known = true;
        }
        api::push_changes(&rt, &bridge);
        rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
        rt.fire_event("BAG_EXPANDED", ());
        rt.lua
            .load("assert(#newbag.slots.btns==150 and newbag.slots.btns[150]:IsVisible() and X2Bag:SlotByIdx(147)==147)")
            .exec()
            .unwrap();
    }
    if !args.iter().any(|a| a == "--unsorted") {
        expansion::fixture::click(&rt, "newbag.sortBtn").unwrap();
        rt.lua
            .load("assert(X2Bag:SlotByIdx(1)==5 and HUD.inventory_dest(2,X2Bag:CountItems()+1)==1)")
            .exec()
            .unwrap();
    }
    show_bank(&rt).unwrap();
    expansion::fixture::click(&rt, "GetBankInventory().sortBtn").unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original inventory sorting"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(api::HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    std::fs::remove_file(path).unwrap();
}

pub(super) fn show_bank(rt: &UiRuntime) -> mlua::Result<()> {
    rt.lua
        .load(
            r#"
        npcInteraction.interactionWindows[1]=npcInteraction.interactionWindows[1] or {}
        ToggleBankInventory(true,1)
        assert(IsActivatedBank())
        local bank=GetBankInventory()
        bank:RemoveAllAnchors(); bank:AddAnchor('TOPLEFT','UIParent',30,320)
        assert(X2Bank:SlotByIdx(8)==8)
    "#,
        )
        .exec()
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
