use super::*;

fn snapshot(owner_id: u32, all: i32, current: i32) -> HouseSnapshot {
    HouseSnapshot {
        tl_id: 7,
        db_id: 70,
        obj_id: 900,
        template_id: 171,
        model_id: 1083,
        co_owner_id: owner_id,
        owner_id,
        owner_name: "Owner".into(),
        account_id: 17,
        permission: 0,
        all_actions: all,
        current_actions: current,
        tax: 0,
        x: 0,
        y: 0,
        z: 0.0,
        name: "Farm".into(),
        allow_recover: true,
        sell_price: 0,
        sell_to_player_id: 0,
        sell_to_name: String::new(),
    }
}

fn setup_shared() -> (UiRuntime, State, Shared) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(super::super::data::GameData::default()).is_ok());
    bridge.borrow_mut().character_id = 42;
    super::super::api::install(&mut rt, &bridge).unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    (rt, state, bridge)
}

fn setup() -> (UiRuntime, State) {
    let (rt, state, _) = setup_shared();
    (rt, state)
}

fn open(state: &State, owner: u32, all: i32, current: i32) {
    let mut v = state.0.borrow_mut();
    v.me = 42;
    v.money = 50_000;
    v.current = Some(Current { obj_id: 900, tl_id: 7, design: 171, snapshot: Some(snapshot(owner, all, current)), name: "Farm".into(), mine: owner == 42, tax: None, decorations: 2 });
}

#[test]
fn taxes_and_sales_cost_certificates() {
    // one tax certificate per gold (rounded up), one appraisal certificate per 100 gold (at least one)
    assert_eq!([certificates_for(0), certificates_for(1), certificates_for(10_000), certificates_for(10_001)], [0, 1, 1, 2]);
    assert_eq!([appraisals_for(0), appraisals_for(1_000_000), appraisals_for(1_000_001)], [1, 1, 2]);
    let (g, s, c) = (Value::Integer(1), Value::Number(2.0), Value::Integer(3));
    assert_eq!(copper(&g, &s, &c), 10_203);
    assert_eq!(amount(&Value::Nil), 0);
}

#[test]
fn the_getters_answer_for_the_house_interacted_with() -> mlua::Result<()> {
    let (rt, state) = setup();
    rt.lua.load("assert(not X2House:IsMyHouse()) assert(X2House:GetHouseName() == '')").exec()?;
    open(&state, 42, 20, 12);
    rt.lua
        .load(
            r#"
            assert(X2House:IsMyHouse())
            assert(X2House:IsHouseUnderConstruction())
            assert(X2House:GetHouseName() == "Farm" and X2House:GetHouseOwnerName() == "Owner")
            assert(X2House:GetHouseBaseProgress() == 20 and X2House:GetHouseRemainProgress() == 8)
            local permission, alwaysPublic = X2House:GetHousePermission()
            assert(permission == 1 and alwaysPublic == false, tostring(permission))
            assert(X2House:GetHouseAllowRecover())
            local count, limit = X2House:GetHouseDecoCount()
            assert(count == 2)
            assert(X2House:CountTaxItemForTax("25000") == 3)
            local sale = X2House:GetHouseSaleInfo()
            assert(sale.onSale == false and sale.canBuy == false)
            assert(not X2House:IsCastle())
        "#,
        )
        .exec()?;
    // a finished house sends no progress (0 of 0)
    open(&state, 42, 0, 0);
    rt.lua.load("assert(not X2House:IsHouseUnderConstruction())").exec()?;
    Ok(())
}

#[test]
fn the_setters_send_what_changed_for_the_owner_only() -> mlua::Result<()> {
    let (rt, state) = setup();
    open(&state, 42, 0, 0);
    rt.lua
        .load(
            r#"
            X2House:SetHousePermission(1)         -- unchanged: private
            X2House:SetHousePermission(3)         -- public (the server's 2)
            X2House:SetHouseAllowRecover(true)    -- unchanged
            X2House:SetHouseAllowRecover(false)
            X2House:SetHouseName("Home")
            X2House:SetHouseForSale("1", "0", "0", "")
            X2House:Demolish()
        "#,
        )
        .exec()?;
    let commands = std::mem::take(&mut state.0.borrow_mut().commands);
    assert_eq!(
        format!("{commands:?}"),
        format!("{:?}", vec![
            Command::House(HouseCommand::ChangePermission { tl: 7, permission: 2 }),
            Command::House(HouseCommand::AllowRecover { tl: 7 }),
            Command::House(HouseCommand::ChangeName { tl: 7, name: "Home".into() }),
            Command::House(HouseCommand::Sell { tl: 7, money_amount: 10_000, sell_to: String::new() }),
            Command::UseSkill { skill_id: DEMOLISH_SKILL, target_obj_id: 900 },
        ])
    );
    // someone else's house: nothing goes out
    open(&state, 5, 0, 0);
    rt.lua.load("X2House:SetHousePermission(3) X2House:Demolish() X2House:SetHouseName('Mine')").exec()?;
    assert!(state.0.borrow().commands.is_empty());
    Ok(())
}

#[test]
fn a_house_for_sale_is_bought_at_its_price() -> mlua::Result<()> {
    let (rt, state) = setup();
    open(&state, 5, 0, 0);
    if let Some(s) = state.0.borrow_mut().current.as_mut().and_then(|c| c.snapshot.as_mut()) {
        s.sell_price = 30_000;
    }
    state.0.borrow_mut().money = 20_000;
    rt.lua.load("local s = X2House:GetHouseSaleInfo() assert(s.onSale and s.canBuy and s.gold == 3) X2House:BuyHouse()").exec()?;
    // 3 gold is more than the 2 gold the player has
    assert!(state.0.borrow().commands.is_empty());
    assert_eq!(
        state.0.borrow().notices,
        vec![Notice { name: "HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY", arg: None }]
    );
    state.0.borrow_mut().money = 30_000;
    rt.lua.load("X2House:BuyHouse()").exec()?;
    assert_eq!(format!("{:?}", state.0.borrow().commands), format!("{:?}", vec![Command::House(HouseCommand::Buy { tl: 7, money_amount: 30_000 })]));
    Ok(())
}

#[test]
fn the_build_window_buttons_reach_the_builder() -> mlua::Result<()> {
    let (rt, state) = setup();
    rt.lua.load("X2House:NextStepFromBuildCheck() X2House:PrevStepFromBuildCheck()").exec()?;
    let v = state.0.borrow();
    assert!(v.builder_next && v.builder_prev);
    Ok(())
}

/// The Start button (`NextStepFromBuildCheck`) through `sync` to the wire: the captured
/// `CSCreateHouse` at the snapped (15328, 14632), item 16777230, tax 450000.
#[test]
fn the_start_button_sends_the_captured_create_house() -> mlua::Result<()> {
    let (rt, _state, bridge) = setup_shared();
    rt.lua.load("X2House:NextStepFromBuildCheck()").exec()?;
    let mut builder = Builder::default();
    builder.build = Some(crate::ingame::housing::placement::Build::confirmed(
        171,
        16_777_230,
        21_165,
        Vec3::new(15328.0, 14632.0, 173.051_62),
        0.0,
        450_000,
    ));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_message::<BuilderSignal>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<HudPointerOverUi>()
        .insert_resource(Units::default())
        .insert_resource(Houses::default())
        .insert_resource(Doodads::default())
        .insert_resource(Inventory::default())
        .insert_resource(builder)
        .insert_non_send(HudBridge(bridge))
        .insert_non_send(rt)
        .add_systems(Update, sync);
    app.update();
    let commands: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
    assert_eq!(commands.len(), 1, "{commands:?}");
    let NetCommand(Command::House(HouseCommand::CreateHouse { design_id, x, y, z, z_rot, item_id, money_amount, ht, auto_use_aa_point })) = &commands[0] else {
        panic!("not a create house: {commands:?}");
    };
    assert_eq!(
        (*design_id, *x, *y, *z, *z_rot, *item_id, *money_amount, *ht, *auto_use_aa_point),
        (171, 15328.0, 14632.0, 173.051_62, 0.0, 16_777_230, 450_000, 0, false)
    );
    let mut w = aa_net::wire::Writer::new();
    aa_net::housing::HouseCommand::CreateHouse {
        design_id: *design_id,
        x: *x,
        y: *y,
        z: *z,
        z_rot: *z_rot,
        item_id: *item_id,
        money_amount: *money_amount,
        ht: *ht,
        auto_use_aa_point: *auto_use_aa_point,
    }
    .write(&mut w);
    assert_eq!(
        w.finish(),
        [
            0xab, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbe, 3, 0, 0, 0, 0, 0, 0x80, 0x92, 3, 0x37, 0x0d, 0x2d, 0x43,
            0, 0, 0, 0, 0x0e, 0, 0, 1, 0, 0, 0, 0, 0xd0, 0xdd, 6, 0, 0, 0, 0, 0, 0,
        ]
    );
    assert!(app.world().resource::<Builder>().build.is_none());
    Ok(())
}

#[test]
fn the_stage_material_decides_the_build_action() {
    use crate::ingame::housing::data::Step;
    let design = Design {
        id: 171,
        steps: vec![
            Step { model: 1083, skill: 14575, actions: 10, item: 17684, count: 1, labor: 25 },
            Step { model: 1084, skill: 14641, actions: 5, item: 17685, count: 1, labor: 25 },
        ],
        ..default()
    };
    let mut inventory = Inventory::default();
    assert_eq!(
        build_action(&design, 3, &inventory),
        Some(BuildAction { skill: 14575, material: Some(Material { item: 17684, in_bag: false, equipped: false }) })
    );
    inventory.bag.push(Some(aa_net::model::Item { template_id: 17685, count: 2, ..default() }));
    assert_eq!(
        build_action(&design, 10, &inventory),
        Some(BuildAction { skill: 14641, material: Some(Material { item: 17685, in_bag: true, equipped: false }) })
    );
    assert_eq!(build_action(&design, 15, &inventory), None);
    // the 2026-10-31 capture: a bundle equipped in the backpack slot is not seen by the bag check
    let mut inventory = Inventory::default();
    inventory.equipment.slots[aa_net::model::slot::BACKPACK] =
        Some(aa_net::model::Item { template_id: 17684, count: 1, ..default() });
    assert_eq!(
        build_action(&design, 3, &inventory),
        Some(BuildAction { skill: 14575, material: Some(Material { item: 17684, in_bag: false, equipped: true }) })
    );
}

#[test]
fn the_sync_system_has_no_conflicting_parameters() {
    let mut world = World::new();
    let mut system = IntoSystem::into_system(sync);
    system.initialize(&mut world);
}

#[test]
fn only_the_players_standing_houses_are_on_the_map() {
    let mut houses = Houses::default();
    let mut add = |tl: u16, owner: u32, demolished: bool| {
        let mut snapshot = snapshot(owner, 0, 0);
        snapshot.tl_id = tl;
        snapshot.x = aa_net::wire::coord_to_long(15339.5);
        snapshot.y = aa_net::wire::coord_to_long(14632.0);
        houses.by_tl_id.insert(tl, aa_net::housing::House { snapshot, demolished, build_pay: None, tax: None, sale: None });
    };
    add(1, 42, false);
    add(2, 5, false);
    add(3, 42, true);
    let icons = own_house_icons(&houses, 42);
    assert_eq!(icons.len(), 1);
    assert_eq!((icons[0].0, icons[0].1, icons[0].3.as_str()), (15339.5, 14632.0, "Farm"));
    assert!(own_house_icons(&houses, 0).is_empty());
    assert_eq!([map_icon_kind(7), map_icon_kind(18), map_icon_kind(1)], ["housing_fishfarm", "housing_farm", "housing_house"]);
}
