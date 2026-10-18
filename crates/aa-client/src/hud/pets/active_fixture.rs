//! Original single-pet frame/bar over explicit received metadata and an actual correlated unit.
use super::*;
use aa_net::mate::{MateEvent, MateSpawn, Mates};
use bevy::window::PrimaryWindow;

pub(super) fn received() -> (Player, Units, Mates) {
    let player = Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    let mut unit = player.spawn.as_ref().unwrap().unit.clone();
    unit.obj_id = 201;
    unit.name = "Stormmane".into();
    unit.owner_name = "Pet fixture".into();
    unit.kind = aa_net::model::UnitKind::Mate {
        tl_id: 12,
        template_id: 504,
        owner_id: 47,
    };
    unit.model_id = 139;
    unit.level = 30;
    unit.hp = 720;
    unit.max_hp = 1000;
    unit.mp = 260;
    unit.max_mp = 400;
    let mut units = Units::default();
    units.by_id.insert(100, player.spawn.as_ref().unwrap().unit.clone());
    units.by_id.insert(201, unit.clone());
    let mut mates = Mates::default();
    mates.apply(&Event::PlayerSpawn(Box::new(player.spawn.as_ref().unwrap().clone())));
    mates.apply(&Event::Mate(MateEvent::Spawned(MateSpawn {
        tl_id: 12,
        id: 71,
        item_id: 9001,
        user_state: 2,
        experience: 426_750,
        skills: [5, 1, 3, 4, 6, 2, 8, 0, 0, 0],
        ..default()
    })));
    mates.apply(&Event::UnitSpawned(Box::new(unit)));
    (player, units, mates)
}

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf, Player, Units, Mates), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    let (player, units, mates) = received();
    {
        let mut world = World::new();
        world.insert_resource(player.clone());
        world.insert_resource(units.clone());
        world.insert_resource(mates.clone());
        let mut data = bridge.borrow_mut();
        data.character_id = 47;
        super::refresh(&world, &mut data);
    }
    super::super::api::push_changes(&rt, &bridge);
    rt.lua
        .load(
            r#"
        assert(X2Mate:IsPlayerPetExists()==true)
        local joined,capacity,corps=X2BattleField:GetJoinedInfos()
        assert(joined==0 and capacity==0 and corps==-1 and not X2BattleField:IsInInstantGame())
        assert(battlefield.entranceAlarm==nil,'no queue must not create an entrance alarm')
        assert(X2Unit:GetUnitId('playerpet')=='201' and X2Unit:UnitName('201')=='Stormmane')
        assert(X2Unit:GetUnitType('playerpet')=='mate' and X2Unit:GetUnitInfoById('201').owner_name=='Pet fixture')
        assert(X2Unit:UnitHealth('playerpet')==720 and X2Unit:UnitMaxHealth('playerpet')==1000)
        assert(petFrame and petFrame:IsVisible(),'original pet frame missing: '..tostring(petFrame)..'; bar='..tostring(petBar))
        assert(petBar and petBar:IsVisible(),'original pet bar missing: '..tostring(petBar))
        assert(X2Mate:PetState()=='protective' and X2Mate:GetNumMountSkills()==5)
        assert(X2Mate:IsMountablePet() and X2Mate:IsHavePassengerSeat() and not X2Mate:IsAttackablePet())
        assert(petBar:GetWidth()==250 and petBar:GetHeight()==105,'original spawn handler must finish sizing the bar')
        petFrame:UpdateAll()
        ADDON:ShowContent(UIC_BAG,false)
    "#,
        )
        .set_name("@active_pet_fixture")
        .exec()?;
    Ok((rt, bridge, path, player, units, mates))
}

pub(super) fn native(rt: &UiRuntime, bridge: &Shared, player: &Player, units: &mut Units, mates: &mut Mates) -> aa_net::state::Buffs {
    let unit = units.by_id.get_mut(&201).unwrap();
    unit.kind = aa_net::model::UnitKind::Mate { tl_id: 12, template_id: 11554, owner_id: 47 };
    unit.model_id = 503;
    unit.level = 5;
    unit.name = "Native pool fixture".into();
    mates.own.as_mut().unwrap().spawn.experience = 1500;
    let mut buffs = aa_net::state::Buffs::default();
    buffs.native.set_catalog(bridge.borrow().game.get().unwrap().native_buffs.clone());
    buffs.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
    let mut world = World::new();
    world.insert_resource(player.clone());
    world.insert_resource(units.clone());
    world.insert_resource(mates.clone());
    world.insert_resource(buffs.clone());
    super::refresh(&world, &mut bridge.borrow_mut());
    super::push(rt, bridge);
    buffs
}

pub(super) fn modified(rt: &UiRuntime, bridge: &Shared, player: &Player, units: &mut Units, mates: &Mates) -> aa_net::state::Buffs {
    let unit = units.by_id.get_mut(&201).unwrap();
    unit.buffs = [670,1426].into_iter().enumerate().map(|(index,buff_id)|aa_net::buff::Buff {
        index:index as u32+1,buff_id,ability_level:1,stacks:1,..default()
    }).collect();
    let mut buffs = aa_net::state::Buffs::default();
    buffs.native.set_catalog(bridge.borrow().game.get().unwrap().native_buffs.clone());
    buffs.apply(&aa_net::Event::UnitSpawned(Box::new(unit.clone())));
    let mut world = World::new();
    world.insert_resource(player.clone());world.insert_resource(units.clone());world.insert_resource(mates.clone());world.insert_resource(buffs.clone());
    super::refresh(&world,&mut bridge.borrow_mut());super::push(rt,bridge);
    buffs
}

pub(super) fn equipped(
    rt: &UiRuntime,
    bridge: &Shared,
    player: &Player,
    units: &mut Units,
    mates: &mut Mates,
    battle: bool,
) -> aa_net::state::Buffs {
    let _ = native(rt, bridge, player, units, mates);
    let unit = units.by_id.get_mut(&201).unwrap();
    let (template, model, name) = if battle {
        (10814, 1208, "Brown Wolfhound")
    } else {
        (10027, 1070, "Coalmane Snowlion")
    };
    unit.kind = aa_net::model::UnitKind::Mate {
        tl_id: 12,
        template_id: template,
        owner_id: 47,
    };
    unit.model_id = model;
    unit.name = name.into();
    unit.equipment = default();
    for (slot, template_id) in [
        (0, 23090),
        (
            if battle { 2 } else { 3 },
            if battle { 25303 } else { 23091 },
        ),
        (6, 23092),
    ] {
        unit.equipment.slots[slot] = Some(aa_net::model::Item {
            template_id,
            id: 0x200000000000001 + slot as u64,
            grade: 1,
            flags: 1,
            count: 1,
            detail: ItemDetail::Equip(aa_net::model::EquipDetail {
                durability: 42,
                temper_physical: 117,
                temper_magical: 100,
                ..default()
            }),
            ..default()
        });
    }
    // An explicit new received mate identity reconstructs the original slot-pack layout.
    mates.own.as_mut().unwrap().spawn.id += 1;
    let mut buffs = aa_net::state::Buffs::default();
    buffs
        .native
        .set_catalog(bridge.borrow().game.get().unwrap().native_buffs.clone());
    buffs.apply(&Event::UnitSpawned(Box::new(unit.clone())));
    let mut world = World::new();
    world.insert_resource(player.clone());
    world.insert_resource(units.clone());
    world.insert_resource(mates.clone());
    world.insert_resource(buffs.clone());
    super::refresh(&world, &mut bridge.borrow_mut());
    super::push(rt, bridge);
    buffs
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("aa-client: original owned pet HUD"), aa_dev::DevPlugin,
    ));
    let (rt, bridge, path, player, mut units, mut mates) = runtime(width as f32, height as f32).unwrap();
    let mut buffs = aa_net::state::Buffs::default();
    if std::env::args().any(|a| a == "--equipment" || a == "--battle-equipment") {
        buffs = equipped(
            &rt,
            &bridge,
            &player,
            &mut units,
            &mut mates,
            std::env::args().any(|a| a == "--battle-equipment"),
        );
    }
    if std::env::args().any(|a| a == "--native-pools" || a == "--info" || a == "--modified") {
        buffs = native(&rt, &bridge, &player, &mut units, &mut mates);
        rt.lua.load(r#"
            assert(X2Unit:UnitLevel('playerpet') == 5)
            assert(X2Unit:UnitMaxHealth('playerpet') == 1012 and X2Unit:UnitMaxMana('playerpet') == 335)
            assert(petFrame:IsVisible() and petBar:IsVisible())
        "#).exec().unwrap();
    }
    if std::env::args().any(|a| {
        a == "--info" || a == "--modified" || a == "--equipment" || a == "--battle-equipment"
    }) {
        super::super::expansion::fixture::click(&rt, "petBar.command.buttons[PET_CMD_EQUIP]").unwrap();
        rt.lua.load("assert(petBar.equip:IsVisible() and X2Unit:UnitInfo('playerpet'))").exec().unwrap();
    }
    if std::env::args().any(|a| a == "--modified") {
        buffs = modified(&rt,&bridge,&player,&mut units,&mates);
    }
    app.add_plugins(aa_ui::UiPlugin {
        config: Some(crate::login::host::ui_config()),
    })
    .insert_non_send(rt)
    .insert_non_send(HudBridge(bridge))
    .insert_resource(player)
    .insert_resource(units)
    .insert_resource(mates)
    .insert_resource(buffs)
    .add_message::<NetEvent>()
    .add_message::<NetCommand>()
    .add_systems(Startup, physical_pixels)
    .add_systems(Update, super::update)
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
