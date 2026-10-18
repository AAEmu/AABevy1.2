//! Explicit received proficiency on the original skill book and confirmation dialog.
use super::*;
use aa_ui::UiRuntime;
use bevy::window::PrimaryWindow;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = crate::hud::breath::fixture::runtime(width, height)?;
    {
        let mut d = bridge.borrow_mut();
        d.character_id = 47;
        d.snapshot.name = "Proficiency fixture".into();
        d.snapshot.vocation = 1000;
        let offer = d.game.get().unwrap().crafting.expansions.get(&1).unwrap();
        let mut inventory = aa_net::state::Inventory::default();
        inventory.bag_slots = 50;
        inventory.bag.resize(50, None);
        if offer.item != 0 && offer.count > 0 {
            inventory.bag[0] = Some(aa_net::model::Item {
                template_id: offer.item,
                id: 901,
                count: offer.count,
                ..default()
            });
        }
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        d.panels = crate::hud::panels::refresh(&world, d.game.get(), 0, 100, d.start, &d.snapshot, Vec3::ZERO).0;
        d.panels.actions_known = true;
        d.crafting.inventory = Some(inventory);
        d.crafting.actabilities.expanded_expert = Some(0);
        d.crafting.actabilities.apply(&Event::Actability(ActabilityEvent::Snapshot {
            last: true,
            entries: vec![aa_net::ActabilityValue { id: 1, point: 10000, step: 0 }],
        }));
    }
    crate::hud::api::push_changes(&rt, &bridge);
    let id = aa_ui::lua::value_id(&rt.lua, &Value::String(rt.lua.create_string("skillWnd")?)).ok_or("original skill window missing")?;
    rt.lua
        .globals()
        .set("EXPERT_SKILL", aa_ui::lua::object_table(&rt.lua, id)?.ok_or("skill window table missing")?)?;
    rt.lua
        .load(
            r#"
        ADDON:ShowContent(UIC_SKILL,true)
        EXPERT_SKILL.tab:SelectTab(4)
        actabilityTab:UpdateAll()
        for _,row in ipairs(actabilityTab.actabilityItem) do if row.itemInfo.type==1 then EXPERT_ROW=row;break end end
        assert(EXPERT_ROW and EXPERT_ROW.itemInfo.point==10000)
        EXPERT_ROW:Update(X2Ability:GetMyActabilityInfo(1))
        assert(EXPERT_ROW.upgradeButton:IsVisible() and EXPERT_ROW.upgradeButton:IsEnabled())
        local request=X2DialogManager.RequestDefaultDialog
        X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
            local window=request(self,handler,owner); EXPERT_DIALOG=window;return window
        end
    "#,
        )
        .exec()?;
    aa_ui::lua::tick(&rt.lua, 500.);
    Ok((rt, bridge, path))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path) = runtime(width as f32, height as f32).unwrap();
    if std::env::args().any(|a| a == "--confirm") {
        crate::hud::expansion::fixture::click(&rt, "EXPERT_ROW.upgradeButton").unwrap();
    } else if std::env::args().any(|a| a == "--expand") {
        crate::hud::expansion::fixture::click(&rt, "actabilityTab.expandBtn").unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original proficiency controls"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(crate::hud::api::HudBridge(bridge))
        .add_systems(Startup, physical_pixels)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.));
        window.resolution.set_physical_resolution(width, height);
    }
}
