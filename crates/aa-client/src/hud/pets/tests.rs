use super::*;

#[test]
fn revival_quote_uses_native_pow_rounding_and_physical_eligibility() {
    for (level, expected) in [
        (0, 0),
        (1, 2),
        (30, 2512),
        (50, 8109),
        (144, 92166),
        (164, 124287),
        (219, 241667),
        (226, 259799),
        (244, 309861),
        (255, 342929),
    ] {
        assert_eq!(revival_cost(level), expected);
    }
    let mut game = super::super::data::GameData::default();
    game.pets.summons.insert(10);
    game.items.insert(
        10,
        super::super::data::ItemInfo {
            impl_id: 11,
            ..default()
        },
    );
    game.items.insert(
        11,
        super::super::data::ItemInfo {
            impl_id: 11,
            ..default()
        },
    );
    let item = |template, dead, level, count| {
        Some(aa_net::model::Item {
            template_id: template,
            count,
            detail: ItemDetail::Mate {
                exp: 0,
                dead,
                level,
            },
            ..default()
        })
    };
    let mut inventory = Inventory::default();
    inventory.bag_slots = 5;
    inventory.bank_slots = 2;
    inventory.bag = vec![
        item(10, 1, 30, 99),
        item(10, 0, 30, 1),
        item(11, 1, 30, 1),
        item(10, 1, 30, 1),
        None,
        item(10, 1, 30, 1),
    ];
    inventory.bank = vec![None, item(10, 2, 144, 0), item(10, 1, 30, 1)];
    inventory.mark_item_slot_pending(2, 3);
    let mut data = HudData::default();
    assert!(data.game.0.set(game).is_ok());
    data.pets.inventory = Some(inventory);
    assert_eq!(
        quote(&data),
        94678.0,
        "one charge per dead mapped item, not stack count; pending/unmapped/live/out-of-capacity skipped"
    );
    data.pets
        .inventory
        .as_mut()
        .unwrap()
        .clear_item_slot_pending(2, 3);
    assert_eq!(quote(&data), 97190.0);
}

#[test]
#[ignore = "isolated original stablehand dialog and physical confirmation"]
fn original_stablehand_quote_cancel_confirm_and_received_revival()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, inventory, units, npc) = fixture::runtime(1920.0, 1080.0)?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge.clone()))
        .insert_resource(inventory)
        .insert_resource(units)
        .insert_resource(Player {
            spawn: Some(super::super::expansion::fixture::spawn()),
        })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, update);
    let drain = |app: &mut App| {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|v| v.0)
            .collect::<Vec<_>>()
    };
    let click = |app: &App, widget: &str| {
        super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), widget)
    };
    fixture::open_service(
        app.world().non_send::<UiRuntime>(),
        &bridge,
        app.world().resource::<Units>(),
    );
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(REVIVE_DIALOG and REVIVE_DIALOG:IsVisible())")
        .exec()?;
    assert!(
        drain(&mut app).is_empty(),
        "opening the quote is local presentation"
    );
    click(&app, "REVIVE_DIALOG.btnCancel")?;
    app.update();
    assert!(drain(&mut app).is_empty());
    fixture::open_service(
        app.world().non_send::<UiRuntime>(),
        &bridge,
        app.world().resource::<Units>(),
    );
    app.update();
    click(&app, "REVIVE_DIALOG.btnOk")?;
    app.update();
    assert!(matches!(
        drain(&mut app).as_slice(),
        [Command::Mate(MateCommand::ReviveItems { npc_obj_id: 200 })]
    ));
    assert_eq!(
        quote(&bridge.borrow()),
        94678.0,
        "sending cannot clear dead flags"
    );
    fixture::open_service(
        app.world().non_send::<UiRuntime>(),
        &bridge,
        app.world().resource::<Units>(),
    );
    app.update();
    click(&app, "REVIVE_DIALOG.btnOk")?;
    bridge.borrow_mut().dialog.npc = None;
    app.update();
    assert!(
        drain(&mut app).is_empty(),
        "a changed interaction invalidates a queued confirmation"
    );
    let ack = Event::ItemsChanged(aa_net::items::ItemTasks {
        reason: aa_net::mate::REPAIR_PETS_TASK,
        tasks: vec![
            aa_net::items::ItemTask::Update {
                container: 2,
                slot: 4,
                item_id: 9001,
                detail: ItemDetail::Mate {
                    exp: 0,
                    dead: 0,
                    level: 30,
                },
                detail_payload: None,
                raw_detail_block: None,
            },
            aa_net::items::ItemTask::Update {
                container: 3,
                slot: 7,
                item_id: 9002,
                detail: ItemDetail::Mate {
                    exp: 0,
                    dead: 0,
                    level: 144,
                },
                detail_payload: None,
                raw_detail_block: None,
            },
        ],
        ..default()
    });
    app.world_mut().resource_mut::<Inventory>().apply(&ack);
    app.world_mut().write_message(NetEvent(ack));
    refresh(app.world(), &mut bridge.borrow_mut());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(X2:GetTotalRepairsForPetItems()==0)")
        .exec()?;
    bridge.borrow_mut().dialog.npc = Some((200, npc));
    fixture::open_service(
        app.world().non_send::<UiRuntime>(),
        &bridge,
        app.world().resource::<Units>(),
    );
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(REVIVE_NOTICE and REVIVE_NOTICE:IsVisible())")
        .exec()?;
    assert!(
        drain(&mut app).is_empty(),
        "zero quote opens the original notice without a revival request"
    );
    drop(app);
    drop(bridge);
    std::fs::remove_file(path)?;
    Ok(())
}
