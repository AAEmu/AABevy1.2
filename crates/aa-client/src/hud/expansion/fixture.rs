//! Original expansion dialog on explicit synthetic inventory, with no connection.
use std::path::PathBuf;

use aa_net::state::Inventory;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::super::{api, panels};

pub(super) fn spawn() -> aa_net::PlayerSpawn {
    aa_net::PlayerSpawn {
        obj_id: 100,
        state: aa_net::model::CharacterState {
            character: aa_net::Character {
                id: 47,
                level: 30,
                ..default()
            },
            ..default()
        },
        unit: aa_net::Unit {
            obj_id: 100,
            name: "Expansion fixture".into(),
            kind: aa_net::model::UnitKind::Character { char_id: 47 },
            owner_name: String::new(),
            pos: default(),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 30,
            model_id: 0,
            equipment: default(),
            model_params: default(),
            hp: 100,
            mp: 100,
            max_hp: 100,
            max_mp: 100,
            dead: false,
            race_gender: 1,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            skills: vec![],
            passive_buffs: vec![],
            ability_exp: vec![],
            active_abilities: vec![],
            buffs: vec![],
            anim_action: 0,
            attached_to: None,
        },
    }
}

pub(crate) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, api::Shared, PathBuf, Inventory), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    api::mark_pushed(&bridge);
    let mut inventory = Inventory::default();
    inventory.bag_slots = 50;
    inventory.bank_slots = 50;
    inventory.bag.resize(50, None);
    inventory.bag[4] = Some(aa_net::model::Item {
        id: 9_007_199_254_740_993,
        template_id: 8000025,
        count: 2,
        ..default()
    });
    {
        let mut data = bridge.borrow_mut();
        data.snapshot.money = 1000;
        data.character_id = 47;
        data.expansion.owner = 47;
        data.expansion.received = Some(inventory.clone());
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        data.panels = panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    api::push_changes(&rt, &bridge);
    // Match drag::bag_events after the synthetic received inventory snapshot is pushed.
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    rt.lua
        .load(
            r#"
        local original=X2DialogManager.RequestTaskDialog
        X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
          local wnd=original(self,task,info,owner)
          if task==DLG_TASK_EXPAND_INVENTORY then EXPANSION_DIALOG=wnd; EXPANSION_INFO=info end
          return wnd
        end
        X2Bag:SwitchTab(1)
        ADDON:ShowContent(UIC_BAG,true)
        assert(newbag:IsVisible())
        assert(HUD.bag[5].id==8000025 and X2Bag:CountItems()==1 and X2Bag:ItemStack(5)==2)
        assert(X2Bag:ExpansionMaxSlotCount()==150 and X2Bank:ExpansionMaxSlotCount()==130)
    "#,
        )
        .exec()?;
    Ok((rt, bridge, path, inventory))
}

pub(crate) fn click(rt: &UiRuntime, widget: &str) -> Result<(), Box<dyn std::error::Error>> {
    let object: mlua::Value = rt.lua.load(widget).eval()?;
    let id = aa_ui::lua::value_id(&rt.lua, &object).ok_or("fixture widget missing")?;
    let rect = aa_ui::lua::ui(&rt.lua).tree.rect(id);
    let point = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    assert_eq!(
        aa_ui::input::pick(&rt.lua, point.0, point.1),
        Some(id),
        "{widget} must receive the real mouse hit"
    );
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), true, false, false, 0.0);
    aa_ui::input::update(&rt.lua, &mut mouse, Some(point), false, true, false, 0.0);
    Ok(())
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, _) = runtime(width as f32, height as f32).unwrap();
    click(&rt, "newbag.expandBtn").unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original inventory expansion"), aa_dev::DevPlugin))
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
