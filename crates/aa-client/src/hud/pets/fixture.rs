//! Original stablehand confirmation over explicit received dead bag/bank summon items.
use super::super::api;
use super::*;
use bevy::window::PrimaryWindow;

pub(crate) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf, Inventory, Units, u32), Box<dyn std::error::Error>> {
    let (rt, bridge, path, mut inventory) = super::super::expansion::fixture::runtime(width, height)?;
    let (item, npc) = {
        let data = bridge.borrow();
        let game = data.game.get().ok_or("fixture catalog missing")?;
        let item = game
            .pets
            .summons
            .iter()
            .filter(|id| game.items.get(id).is_some_and(|i| i.impl_id == 11))
            .copied()
            .min()
            .ok_or("summon item missing")?;
        let npc = game.pets.stables.iter().copied().min().ok_or("stablehand missing")?;
        (item, npc)
    };
    inventory.bag.fill(None);
    inventory.bank.resize(50, None);
    inventory.bag[4] = Some(aa_net::model::Item {
        id: 9001,
        template_id: item,
        count: 1,
        detail: ItemDetail::Mate { exp: 0, dead: 1, level: 30 },
        ..default()
    });
    inventory.bank[7] = Some(aa_net::model::Item {
        id: 9002,
        template_id: item,
        count: 99,
        detail: ItemDetail::Mate { exp: 0, dead: 2, level: 144 },
        ..default()
    });
    let mut unit = super::super::expansion::fixture::spawn().unit;
    unit.obj_id = 200;
    unit.name = "Stablehand fixture".into();
    unit.kind = aa_net::model::UnitKind::Npc { template_id: npc, owner_id: 0 };
    let mut units = Units::default();
    units.by_id.insert(200, unit);
    {
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        let mut data = bridge.borrow_mut();
        data.dialog.npc = Some((200, npc));
        data.panels = super::super::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
        super::refresh(&world, &mut data);
    }
    api::push_changes(&rt, &bridge);
    rt.fire_event("BAG_UPDATE", (-1, -1));
    rt.lua
        .load(
            r#"
      local original=X2DialogManager.RequestDefaultDialog
      X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
        local window=original(self,handler,owner)
        if owner==GetStablerMessageBoxHandleWindow():GetId() then REVIVE_DIALOG=window end
        return window
      end
      local notice=X2DialogManager.RequestNoticeDialog
      X2DialogManager.RequestNoticeDialog=function(self,handler,owner)
        local window=notice(self,handler,owner)
        if owner==GetStablerMessageBoxHandleWindow():GetId() then REVIVE_NOTICE=window end
        return window
      end
      assert(X2:GetTotalRepairsForPetItems()==94678)
    "#,
        )
        .exec()?;
    Ok((rt, bridge, path, inventory, units, npc))
}

pub(crate) fn service_event() -> Event {
    Event::Interaction(InteractEvent::SkillList {
        npc_obj_id: 200,
        obj_id: 100,
        extra_info: 0,
        pick_id: 0,
        mouse_button: 2,
        skills: vec![12195],
        interactable: true,
        modifier_keys: 0,
    })
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, inventory, units, _) = runtime(width as f32, height as f32).unwrap();
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("aa-client: original stablehand confirmation"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(inventory)
        .insert_resource(units)
        .insert_resource(Player {
            spawn: Some(super::super::expansion::fixture::spawn()),
        })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, super::update);
    app.world_mut().write_message(NetEvent(service_event()));
    app.run();
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
