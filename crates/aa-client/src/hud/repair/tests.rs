use super::*;

#[test]
fn repair_quote_matches_original_x86_float_oracle_including_lane_rounding_differences() {
    // Independent397cea70 execution, data/extract/aa-ui/repair-cost-oracle.csv.
    let cases = [
        (1000, 100, 25.0, 4, 10, 150),
        (1000, 100, 25.0, 10, 10, 0),
        (1_137_792, 150, 75.0, 97, 151, 457_755),
        (1_132_972, 120, 75.0, 4, 105, 980_830),
        (862_846, 300, 75.0, 28, 216, 1_689_741),
        (1_778_503, 300, 33.0, 125, 184, 564_578),
        (536_593, 120, 100.0, 40, 92, 363_950),
        (1_348_289, 200, 33.0, 81, 163, 447_666),
        (635_902, 120, 100.0, 89, 163, 346_430),
        (1_744_815, 150, 75.0, 96, 163, 806_844),
    ];
    for (price, refund, factor, current, maximum, expected) in cases {
        assert_eq!(cost(price, refund, factor, current, maximum), Some(expected));
    }
    assert_eq!(cost(1000, 100, 25.0, 1, 0), None);
}

#[test]
fn quotes_bind_received_items_and_all_repairs_exclude_the_bag() -> Result<(), Box<dyn std::error::Error>> {
    let game = GameData::load(&std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.into()))?;
    let (template, maximum) = game
        .items
        .iter()
        .find_map(|(id, info)| {
            let (maximum, repairable) = game.tips.items.durability(*id, 0)?;
            (info.price > 0 && maximum >= 8 && repairable).then_some((*id, maximum))
        })
        .ok_or("missing repair quote fixture")?;
    let mut item = Item {
        id: 100,
        template_id: template,
        grade: 0,
        detail: ItemDetail::Equip(aa_net::model::EquipDetail {
            durability: maximum / 2,
            ..default()
        }),
        ..default()
    };
    let mut inventory = Inventory::default();
    inventory.bag_slots = 50;
    inventory.bag.resize(50, None);
    inventory.equipment.slots[0] = Some(item.clone());
    item.id = 101;
    inventory.bag[7] = Some(item.clone());
    let mut data = HudData::default();
    data.character_id = 47;
    let mut world = World::new();
    world.insert_resource(inventory.clone());
    data.panels = super::super::panels::refresh(&world, Some(&game), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
    assert!(data.game.0.set(game).is_ok());
    request(&mut data, 1, None, true);
    let quote = data.repair.requests.pop().ok_or("no all-repair quote")?;
    assert_eq!(quote.targets.len(), 1, "repair all scans only equipment, never damaged bag items");
    assert_eq!(quote.targets[0].uid, 100);
    let game = data.game.get().unwrap();
    assert_eq!(
        command(&quote, 47, None, game, &inventory, i64::MAX)?,
        RepairCommand::All { auto_use_aa_point: true }
    );
    assert!(command(&quote, 48, None, game, &inventory, i64::MAX).is_err());
    assert!(command(&quote, 47, Some((200, 1)), game, &inventory, i64::MAX).is_err());
    assert!(command(&quote, 47, None, game, &inventory, i64::from(quote.targets[0].cost) - 1).is_err());
    let mut changed = inventory.clone();
    changed.equipment.slots[0].as_mut().unwrap().id = 999;
    assert!(command(&quote, 47, None, game, &changed, i64::MAX).is_err());
    changed = inventory.clone();
    changed.equipment.slots[1] = Some(item.clone());
    assert!(
        command(&quote, 47, None, game, &changed, i64::MAX).is_err(),
        "the entire all-repair candidate set is rechecked"
    );
    request(&mut data, 2, Some(7), false);
    let quote = data.repair.requests.pop().unwrap();
    assert_eq!(
        command(&quote, 47, None, data.game.get().unwrap(), &inventory, i64::MAX)?,
        RepairCommand::Single {
            container: 2,
            slot: 7,
            auto_use_aa_point: false
        }
    );
    let ItemDetail::Equip(detail) = &inventory.bag[7].as_ref().unwrap().detail else {
        panic!()
    };
    assert_eq!(detail.durability, maximum / 2, "quoting/sending never repairs locally");
    data.panels.equipment.clear();
    request(&mut data, 1, None, false);
    assert!(data.repair.requests.is_empty(), "zero candidates sends no repair-all request");
    Ok(())
}

#[test]
#[ignore = "loads original world-stage assets; run in an isolated fixture process"]
fn original_repair_buttons_physical_slots_costs_and_received_wear() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, inventory) = fixture::runtime(1920.0, 1080.0)?;
    rt.lua
        .load(
            r#"
      assert(not X2Item:IsInRepairMode())
      local d=X2Bag:ItemDurability(8,false); assert(d and d.current>0 and d.current<d.max)
      assert(X2Bag:ItemDurability(1)==nil and X2Equipment:ItemDurability(0)==nil)
      local types,grades,slots=X2Equipment:DameagedItems(5,false)
      assert(#types==1 and #grades==1 and #slots==1 and slots[1]==ES_MAINHAND)
      assert(X2Equipment:DameagedItems()==nil)
      local t,g,s=X2Equipment:DameagedItems(-1,false); assert(#t==0 and #g==0 and #s==0)
      local info=X2Bag:GetBagItemInfo(1,8); assert(info and info.repairCost and info.repairCost>0)
    "#,
        )
        .exec()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .insert_resource(inventory)
        .insert_resource(Player {
            spawn: Some(super::super::expansion::fixture::spawn()),
        })
        .insert_resource(PlayerStats {
            obj_id: 100,
            character_id: 47,
            money: 1_000_000,
            ..default()
        })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, update);
    let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
    super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "repairWindow.partRepairButton")?;
    app.world().non_send::<UiRuntime>().lua.load("assert(X2Item:IsInRepairMode())").exec()?;
    super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "newbag.slots.btns[8]")?;
    assert!(!app.world().non_send::<UiRuntime>().holding());
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::Repair(RepairCommand::Single {
            container: 2,
            slot: 7,
            auto_use_aa_point: false
        })]
    ));
    // A normal bag click in repair mode consumes both healthy and non-equipment items.
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Bag:HandleClick(5); assert(X2Cursor:GetHeld()==nil); X2Equipment:ItemRepair(0)")
        .exec()?;
    app.update();
    assert!(drain(&mut app).is_empty());
    app.world().non_send::<UiRuntime>().lua.load("for i,slot in ipairs(playerEquipSlots) do if slot==ES_MAINHAND then REPAIR_EQUIPMENT_BUTTON=equippedItem.slot[i] end end; assert(REPAIR_EQUIPMENT_BUTTON)").exec()?;
    super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "REPAIR_EQUIPMENT_BUTTON")?;
    app.update();
    assert!(
        matches!(drain(&mut app).as_slice(), [Command::Repair(RepairCommand::Single { container: 1, slot, auto_use_aa_point: false })] if usize::from(*slot)==aa_net::model::slot::MAINHAND)
    );
    assert!(escape(app.world().non_send::<UiRuntime>()));
    assert!(!escape(app.world().non_send::<UiRuntime>()));
    super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "repairWindow.allRepairButton")?;
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::Repair(RepairCommand::All { auto_use_aa_point: false })]
    ));
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(not X2Item:IsInRepairMode()); X2Item:EnterRepairMode(); repairWindow:Show(false); assert(X2Item:IsInRepairMode())")
        .exec()?;
    app.world().non_send::<UiRuntime>().fire_event("NPC_INTERACTION_END", ());
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(not X2Item:IsInRepairMode())").exec()?;
    assert_eq!(app.world().resource::<PlayerStats>().money, 1_000_000);
    let item = app.world().resource::<Inventory>().bag[7].as_ref().unwrap().clone();
    let maximum = bridge
        .borrow()
        .game
        .get()
        .unwrap()
        .tips
        .items
        .durability(item.template_id, item.grade)
        .unwrap()
        .0;
    app.world().non_send::<UiRuntime>().lua.load("wear_events={}; UIParent:SetEventHandler('UPDATE_DURABILITY_STATUS',function(added,removed) wear_events[#wear_events+1]={added,removed,X2Bag:ItemDurability(8).current} end)").exec()?;
    let update_detail = |durability| aa_net::items::ItemTask::Update {
        container: 2,
        slot: 7,
        item_id: item.id,
        detail_payload: None,
        raw_detail_block: None,
        detail: ItemDetail::Equip(aa_net::model::EquipDetail { durability, ..default() }),
    };
    let ack = Event::ItemsChanged(aa_net::items::ItemTasks {
        reason: aa_net::repair::REPAIR_TASK,
        tasks: vec![update_detail(0), update_detail(maximum)],
        ..default()
    });
    app.world_mut().resource_mut::<Inventory>().apply(&ack);
    app.world_mut().write_message(NetEvent(ack));
    let inventory = app.world().resource::<Inventory>();
    {
        let mut data = bridge.borrow_mut();
        data.panels = super::super::panels::refresh(app.world(), data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
    }
    assert!(matches!(&inventory.bag[7].as_ref().unwrap().detail, ItemDetail::Equip(d) if d.durability==maximum));
    super::super::api::push_changes(app.world().non_send::<UiRuntime>(), &bridge);
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    rt.fire_event("BAG_UPDATE", (-1i64, -1i64));
    rt.lua
        .load("local d=X2Bag:ItemDurability(8); assert(d.current==d.max); local i=X2Bag:GetBagItemInfo(1,8); assert(i.repairCost==nil); assert(#wear_events==1 and wear_events[1][1] and wear_events[1][2] and wear_events[1][3]==d.max)")
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
