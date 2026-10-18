//! Actual original equipped-item buttons, bound to two real use-skill items and received timers.
use super::*;
use aa_net::model::{EquipDetail, Item, ItemDetail, slot};
use aa_net::state::ReceivedCooldown;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::path::PathBuf;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    {
        let mut data = bridge.borrow_mut();
        let mut equipment = vec![None; aa_net::model::slot::COUNT];
        for (index, template_id) in [(slot::FEET, 15875), (slot::BACK, 26743)] {
            let item = Item {
                template_id,
                id: 9000 + index as u64,
                grade: 1,
                count: 1,
                detail: ItemDetail::Equip(EquipDetail {
                    durability: 100,
                    ..Default::default()
                }),
                ..Default::default()
            };
            equipment[index] = Some(super::super::panels::item_slot(data.game.get(), template_id, 1, item.grade).with_instance(&item));
        }
        data.panels.equipment = equipment;
        data.cooldowns.obj_id = 100;
        let now = Instant::now();
        for (id, remaining_ms, total_ms) in [(655, 18_000, 30_000), (1004, 12_000, 300_000)] {
            data.cooldowns.received_tag_cooldowns.push(ReceivedCooldown {
                cooldown: aa_net::Cooldown { id, remaining_ms, total_ms },
                received_at: now,
                reset: false,
            });
        }
        let hud: Table = rt.lua.globals().get("HUD")?;
        super::super::panels::push(&rt.lua, &hud, &data.panels, None)?;
    }
    rt.lua
        .load(
            r#"
      ADDON:ShowContent(UIC_BAG,false)
      equippedItem:Update();equippedItem:Show(true);equippedItem:ShowProc()
      local feet=X2Equipment:GetEquippedItemTooltipText('player',ES_FEET)
      local back=X2Equipment:GetEquippedItemTooltipText('player',ES_BACK)
      assert(feet.itemType==15875 and feet.skillType==13419)
      assert(back.itemType==26743 and back.skillType==20676)
    "#,
        )
        .set_name("@equipment_cooldown_fixture")
        .exec()?;
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    if std::env::args().any(|arg| arg == "--ready") {
        for entry in &mut bridge.borrow_mut().cooldowns.received_tag_cooldowns {
            entry.reset = true;
        }
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original equipment cooldowns"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(super::super::api::HudBridge(bridge))
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
