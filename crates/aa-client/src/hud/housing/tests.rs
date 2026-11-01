use super::*;
use aa_net::{Event, ShipyardEvent, ShipyardState};

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

fn setup() -> (UiRuntime, State) {
    let (rt, state, _) = setup_shared();
    (rt, state)
}

fn setup_with(game: super::super::data::GameData) -> (UiRuntime, State) {
    let (rt, state, _) = setup_with_bridge(game);
    (rt, state)
}

fn setup_shared() -> (UiRuntime, State, Shared) {
    setup_with_bridge(super::super::data::GameData::default())
}

fn setup_with_bridge(game: super::super::data::GameData) -> (UiRuntime, State, Shared) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(game).is_ok());
    bridge.borrow_mut().character_id = 42;
    super::super::api::install(&mut rt, &bridge).unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    (rt, state, bridge)
}

fn design(id: u32, always_public: bool) -> super::super::data::housing::HousingTemplate {
    super::super::data::housing::HousingTemplate {
        id,
        category_id: 18,
        absolute_deco_limit: 51,
        deco_limit: 40,
        housing_deco_limit_id: 10,
        binding_doodads: 2,
        always_public,
        ..default()
    }
}

fn open(state: &State, owner: u32, all: i32, current: i32) {
    let mut v = state.0.borrow_mut();
    v.me = 42;
    v.money = 50_000;
    v.current = Some(Current {
        obj_id: 900,
        tl_id: 7,
        design: 171,
        snapshot: Some(snapshot(owner, all, current)),
        name: "Farm".into(),
        mine: owner == 42,
        tax: None,
        decorations: 2,
        furniture: Vec::new(),
        view_type: Some("house"),
        always_public: false,
    });
}

fn spawn(owner: u32, family: u32, expedition: u32) -> Box<aa_net::PlayerSpawn> {
    Box::new(aa_net::PlayerSpawn {
        obj_id: 100,
        state: aa_net::model::CharacterState {
            character: aa_net::Character { id: owner, family, expedition_id: expedition, ..default() },
            ..default()
        },
        unit: aa_net::Unit {
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            obj_id: 100,
            name: "Self".into(),
            kind: aa_net::model::UnitKind::Character { char_id: owner },
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
            expedition_id: expedition,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            skills: vec![],
            passive_buffs: vec![],
            ability_exp: vec![],
            active_abilities: vec![],
            buffs: vec![],
            anim_action: 0,
            house_posture: None,
            turret: None,
            state_attachments: None,
            attached_to: None,
            doodad_bond: None,
        },
    })
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
        .init_resource::<aa_net::Shipyards>()
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


#[test]
fn the_interaction_view_type_follows_the_design_category() {
    assert_eq!(
        [7, 8, 16, 17, 9, 10, 11, 12, 15, 18, 1].map(view_type),
        [Some("seafarm"), Some("farm"), Some("farm"), Some("farm"), Some("house"), Some("house"), Some("house"), Some("house"), Some("house"), Some("house"), None]
    );
}

#[test]
fn deco_counts_exclude_the_built_in_furniture_and_use_the_absolute_limit() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);
    state.0.borrow_mut().current.as_mut().unwrap().decorations = 5;
    rt.lua
        .load(
            r#"
            local count, limit = X2House:GetHouseDecoCount()
            assert(count == 3, tostring(count))
            assert(limit == 49, tostring(limit))
            "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn the_actability_getters_read_the_house_groups_and_the_limit_table() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    game.housing.deco_actability_groups.insert(1, "Specialty furniture".into());
    game.housing.deco_actability_groups.insert(5, "Storage".into());
    game.housing.deco_limit_elems.push(super::super::data::housing::DecoLimitElem {
        id: 8,
        housing_deco_limit_id: 10,
        deco_actability_group_id: 1,
        count: 3,
    });
    game.housing.deco_limit_elems.push(super::super::data::housing::DecoLimitElem {
        id: 15,
        housing_deco_limit_id: 10,
        deco_actability_group_id: 5,
        count: 3,
    });
    game.items.insert(
        107,
        super::super::data::ItemInfo { named: super::super::data::Named { name: "Pot".into(), icon: String::new() }, ..default() },
    );
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);
    state.0.borrow_mut().current.as_mut().unwrap().furniture = vec![
        Furniture { item: 107, actability_group: 1, points: 100, deco_group: 1 },
        Furniture { item: 107, actability_group: 1, points: 50, deco_group: 1 },
        Furniture { item: 108, actability_group: 5, points: 30, deco_group: 5 },
        Furniture { item: 0, actability_group: 0, points: 0, deco_group: 0 },
    ];
    rt.lua
        .load(
            r#"
            assert(X2House:GetActabilityDecoCount() == 2)
            local first = X2House:GetDecoActabilityPoint(1)
            assert(first.decoPoint == 150, tostring(first.decoPoint))
            local second = X2House:GetDecoActabilityPoint(2)
            assert(second.decoPoint == 30, tostring(second.decoPoint))
            assert(X2House:GetDecoActabilityPoint(3) == nil)
            local totals, items = X2House:GetDecoActabilityByType(1)
            assert(totals.type == 1 and totals.totalDecoPoint == 150 and totals.totalPoint == 0)
            assert(#items == 2 and items[1].name == 'Pot' and items[1].decoPoint == 100)
            local groups = X2House:GetDecoActabilityLimitGroup()
            assert(#groups == 2)
            assert(groups[1].name == 'Specialty furniture' and groups[1].arrangeCount == 2 and groups[1].maxCount == 3)
            assert(groups[2].name == 'Storage' and groups[2].arrangeCount == 1 and groups[2].maxCount == 3)
            "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn actability_counts_need_a_limit_list_and_a_deco_group() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    let mut no_list = design(172, false);
    no_list.housing_deco_limit_id = 0;
    game.housing.housings.insert(172, no_list);
    game.housing
        .deco_limit_elems
        .push(super::super::data::housing::DecoLimitElem {
            id: 8,
            housing_deco_limit_id: 10,
            deco_actability_group_id: 1,
            count: 3,
        });
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);
    state.0.borrow_mut().current.as_mut().unwrap().furniture = vec![
        Furniture {
            item: 107,
            actability_group: 1,
            points: 100,
            deco_group: 1,
        },
        // an actability group alone does not count: `FUN_3932FD00` wants a deco group too
        Furniture {
            item: 108,
            actability_group: 5,
            points: 30,
            deco_group: 0,
        },
    ];
    rt.lua
        .load(
            r#"
            assert(X2House:GetActabilityDecoCount() == 1, tostring(X2House:GetActabilityDecoCount()))
            assert(X2House:GetDecoActabilityPoint(1).decoPoint == 100)
            assert(X2House:GetDecoActabilityPoint(2) == nil)
            "#,
        )
        .exec()?;
    // a design with a NULL `housing_deco_limit_id` has no actability list at all
    state.0.borrow_mut().current.as_mut().unwrap().design = 172;
    rt.lua
        .load(
            r#"
            assert(X2House:GetActabilityDecoCount() == 0, tostring(X2House:GetActabilityDecoCount()))
            assert(X2House:GetDecoActabilityPoint(1) == nil)
            assert(X2House:GetDecoActabilityLimitGroup() == nil, 'no limit list must be nil')
            "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn clicking_the_radio_already_set_sends_nothing() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);
    // the received permission is 0 (private), which the window shows as radio 1
    rt.lua.load("X2House:SetHousePermission(1)").exec()?;
    assert!(
        state.0.borrow().commands.is_empty(),
        "the already-set radio sends nothing"
    );
    assert!(state.0.borrow().notices.is_empty());
    Ok(())
}

#[test]
fn a_non_owner_on_an_always_public_design_hits_the_always_public_gate_first() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, true));
    let (rt, state) = setup_with(game);
    // owner 5, me 42: not mine. The 0x80 gate comes before the owner test.
    open(&state, 5, 0, 0);
    rt.lua.load("X2House:SetHousePermission(3)").exec()?;
    assert!(state.0.borrow().commands.is_empty());
    assert_eq!(
        state.0.borrow().notices,
        vec![Notice {
            name: "HOUSE_CANNOT_CHANGE_PERMISSION",
            arg: None
        }],
        "the always-public refusal, not a silent owner-only return"
    );
    Ok(())
}

#[derive(Resource, Default)]
struct ActabilitySpied(u32);

fn read_actability_spy(rt: NonSend<UiRuntime>, mut spied: ResMut<ActabilitySpied>) {
    spied.0 = rt
        .lua
        .load("return ACTABILITY_SPY or 0")
        .eval()
        .unwrap_or(0);
}

#[test]
fn a_change_of_the_received_actability_points_raises_the_spy_event() -> mlua::Result<()> {
    let (rt, _state, bridge) = setup_shared();
    rt.lua
        .load(
            r#"
            local w = UIParent:CreateWidget('emptywidget', 'actabilitySpy', 'UIParent')
            w:RegisterEvent('CHANGE_ACTABILITY_DECO_NUM')
            ACTABILITY_SPY = 0
            w:SetHandler('OnEvent', function(self, event)
                if event == 'CHANGE_ACTABILITY_DECO_NUM' then ACTABILITY_SPY = ACTABILITY_SPY + 1 end
            end)
            "#,
        )
        .exec()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_message::<BuilderSignal>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<HudPointerOverUi>()
        .init_resource::<ActabilitySpied>()
        .insert_resource(Units::default())
        .insert_resource(Houses::default())
        .init_resource::<aa_net::Shipyards>()
        .insert_resource(Doodads::default())
        .insert_resource(Inventory::default())
        .init_resource::<Builder>()
        .insert_non_send(HudBridge(bridge.clone()))
        .insert_non_send(rt)
        .add_systems(Update, (sync, read_actability_spy).chain());
    app.update();
    assert_eq!(
        app.world().resource::<ActabilitySpied>().0,
        0,
        "the first frame only records the points"
    );
    crate::hud::crafting::apply_actability(
        &mut bridge.borrow_mut(),
        &Event::Actability(aa_net::ActabilityEvent::Snapshot {
            last: true,
            entries: vec![aa_net::ActabilityValue {
                id: 1,
                point: 100,
                step: 0,
            }],
        }),
    );
    app.update();
    assert_eq!(
        app.world().resource::<ActabilitySpied>().0,
        1,
        "the spy sees CHANGE_ACTABILITY_DECO_NUM"
    );
    app.update();
    assert_eq!(
        app.world().resource::<ActabilitySpied>().0,
        1,
        "an unchanged signature raises nothing"
    );
    Ok(())
}

#[test]
fn permission_clicks_pass_the_native_gates_and_go_out_once_as_the_server_enum() -> mlua::Result<()>
{
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);

    // `SetHousePermission`'s 2 is the server's guild enum (1) and 4 the family enum (3).
    rt.lua.load("X2House:SetHousePermission(2) X2House:SetHousePermission(4)").exec()?;
    assert!(state.0.borrow().commands.is_empty(), "no membership: nothing goes out");
    assert_eq!(
        state.0.borrow().notices,
        vec![
            Notice { name: "CHAT_NOT_IN_EXPEDITION", arg: None },
            Notice { name: "FAMILY_NOT_EXIST", arg: None },
        ]
    );

    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, false));
    let (rt, state, bridge) = setup_with_bridge(game);
    let spawn = spawn(42, 12, 900);
    super::super::family::dispatch(None, &bridge, &Event::PlayerSpawn(spawn.clone()));
    super::super::expedition::dispatch(None, &bridge, &Event::PlayerSpawn(spawn));
    open(&state, 42, 0, 0);
    rt.lua
        .load("X2House:SetHousePermission(2) X2House:SetHousePermission(3) X2House:SetHousePermission(4)")
        .exec()?;
    let commands = std::mem::take(&mut state.0.borrow_mut().commands);
    assert_eq!(
        format!("{commands:?}"),
        format!("{:?}", vec![
            Command::House(HouseCommand::ChangePermission { tl: 7, permission: 1 }),
            Command::House(HouseCommand::ChangePermission { tl: 7, permission: 2 }),
            Command::House(HouseCommand::ChangePermission { tl: 7, permission: 3 }),
        ]),
        "one packet per changed radio, carrying the server enum"
    );
    assert!(state.0.borrow().notices.is_empty());
    Ok(())
}

#[test]
fn an_always_public_design_refuses_every_permission_click() -> mlua::Result<()> {
    let mut game = super::super::data::GameData::default();
    game.housing.housings.insert(171, design(171, true));
    let (rt, state) = setup_with(game);
    open(&state, 42, 0, 0);
    rt.lua.load("X2House:SetHousePermission(3)").exec()?;
    assert!(state.0.borrow().commands.is_empty());
    assert_eq!(
        state.0.borrow().notices,
        vec![Notice { name: "HOUSE_CANNOT_CHANGE_PERMISSION", arg: None }]
    );
    Ok(())
}

#[test]
fn the_tax_event_keeps_the_native_order_and_raw_weeks() -> mlua::Result<()> {
    let (rt, state) = setup();
    rt.lua
        .load(
            r#"
            local w = UIParent:CreateWidget('emptywidget', 'taxSpy', 'UIParent')
            w:RegisterEvent('HOUSE_TAX_INFO')
            w:SetHandler('OnEvent', function(self, event, ...) TAX = {...} end)
            "#,
        )
        .exec()?;
    open(&state, 42, 0, 0);
    let fire_tax = |weeks: i32, paid: bool| {
        state.0.borrow_mut().current.as_mut().unwrap().tax = Some(HouseTaxInfo {
            tl_id: 7,
            dominion_tax_rate: 17,
            money_amount: 150_000,
            money_amount2: 450_000,
            due: 1_700_000_000,
            is_already_paid: paid,
            weeks_without_pay: weeks,
            is_heavy_tax_house: true,
        });
        fire(&rt, &state, Signal::Tax);
    };
    fire_tax(1, false);
    rt.lua
        .load(
            r#"
            assert(TAX[1] == 17, tostring(TAX[1]))
            assert(TAX[2] == '450000', tostring(TAX[2]))
            assert(type(TAX[3]) == 'table')
            assert(TAX[4] == 1, tostring(TAX[4]))
            assert(TAX[5] == false)
            assert(TAX[6] == true)
            assert(TAX[7] == '150000', tostring(TAX[7]))
            "#,
        )
        .exec()?;
    fire_tax(-1, true);
    rt.lua.load("assert(TAX[4] == -1, tostring(TAX[4])) assert(TAX[5] == true)").exec()?;
    Ok(())
}

/// A shipyard unit at (1, 2, 3) with yaw 0 (the same origin as the real template 5 stage 0).
fn shipyard_unit() -> Unit {
    Unit {
        obj_id: 77,
        name: "Drydock".into(),
        kind: UnitKind::Shipyard { id: 9, template_id: 5 },
        owner_name: "Owner".into(),
        pos: aa_net::Pos::new(1.0, 2.0, 3.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id: 673,
        equipment: aa_net::model::Equipment::default(),
        model_params: aa_net::model::ModelParams::default(),
        hp: 300,
        mp: 0,
        max_hp: 300,
        max_mp: 0,
        dead: false,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        turret: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

fn shipyard_state() -> Shipyards {
    let mut shipyards = Shipyards::default();
    shipyards.apply(&Event::Shipyard(ShipyardEvent::State(ShipyardState {
        id: 9,
        template_id: 5,
        obj_id: 77,
        step: 1,
        actions: 2,
        owner_name: "Owner".into(),
        ..Default::default()
    })));
    shipyards
}

fn tax_info() -> HouseTaxInfo {
    HouseTaxInfo {
        tl_id: 7,
        dominion_tax_rate: 0,
        money_amount: 1,
        money_amount2: 2,
        due: 0,
        is_already_paid: false,
        weeks_without_pay: 0,
        is_heavy_tax_house: false,
    }
}

#[test]
fn picking_a_shipyard_ends_the_house_interaction() {
    let mut units = Units::default();
    units.apply(&Event::UnitSpawned(Box::new(shipyard_unit())));
    let shipyards = shipyard_state();
    let (houses, doodads, inventory) =
        (Houses::default(), Doodads::default(), Inventory::default());
    let mut view = View::default();
    view.current = Some(Current {
        obj_id: 900,
        tl_id: 7,
        design: 171,
        snapshot: Some(snapshot(42, 20, 12)),
        name: "Farm".into(),
        mine: true,
        tax: None,
        decorations: 2,
        ..Default::default()
    });
    let mut last_tax = Some(tax_info());
    interact_and_refresh(
        &mut view,
        Some(77),
        (&units, &houses, &doodads, &inventory),
        (Vec3::new(1.0, 2.0, 0.0), 42),
        &mut last_tax,
        &super::super::data::SharedGameData::default(),
        &shipyards,
        None,
        None,
    );
    assert!(view.current.is_none(), "the house window loses its house");
    assert!(last_tax.is_none(), "the house tax is stale");
    assert_eq!(view.signals, vec![Signal::End], "HOUSE_INTERACTION_END");
    let yard = view
        .shipyard
        .expect("the shipyard is the current interaction");
    assert_eq!(
        (yard.obj_id, yard.id, yard.owner_name.as_str()),
        (77, 9, "Owner")
    );
}

#[test]
fn a_shipyard_pick_without_a_house_raises_no_end() {
    let mut units = Units::default();
    units.apply(&Event::UnitSpawned(Box::new(shipyard_unit())));
    let shipyards = shipyard_state();
    let (houses, doodads, inventory) =
        (Houses::default(), Doodads::default(), Inventory::default());
    let mut view = View::default();
    let mut last_tax = Some(tax_info());
    interact_and_refresh(
        &mut view,
        Some(77),
        (&units, &houses, &doodads, &inventory),
        (Vec3::new(1.0, 2.0, 0.0), 42),
        &mut last_tax,
        &super::super::data::SharedGameData::default(),
        &shipyards,
        None,
        None,
    );
    assert!(view.signals.is_empty());
    assert!(last_tax.is_some());
}

#[test]
fn a_shipyard_pick_is_the_model_bounds_box_not_a_sphere() {
    let unit = shipyard_unit();
    let bounds = (Vec3::new(-6.0, -20.0, -10.0), Vec3::new(6.0, 20.0, -1.0));
    let centre = aa_world::cry_to_bevy(Vec3::new(1.0, 2.0, 3.0 - 5.0));
    let origin = centre + Vec3::new(0.0, 10.0, 30.0);
    let direction = Dir3::new(centre - origin).unwrap();
    let ray = Ray3d::new(origin, direction);
    assert!(
        shipyard_hit(&ray, &unit, bounds).is_some(),
        "the hull middle"
    );
    // 15 m along the keel (cry +y) still hits the 20 m half length
    let along = Ray3d::new(
        origin + aa_world::cry_to_bevy(Vec3::new(0.0, 15.0, 0.0)),
        direction,
    );
    assert!(shipyard_hit(&along, &unit, bounds).is_some(), "the bow");
    // 10 m to the side is inside a 20 m sphere around the middle, outside the 6 m half beam
    let beside = Ray3d::new(origin + Vec3::X * 10.0, direction);
    assert!(
        shipyard_hit(&beside, &unit, bounds).is_none(),
        "beside the hull"
    );
    // the least half-size keeps a small model pickable
    assert_eq!(
        half_extents((Vec3::splat(-1.0), Vec3::splat(1.0))),
        Vec2::splat(SHIPYARD_PICK_MIN)
    );
}

#[test]
fn the_pointer_system_ends_the_house_interaction_on_a_shipyard() {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(
        bridge
            .borrow()
            .game
            .0
            .set(super::super::data::GameData::default())
            .is_ok()
    );
    super::super::api::install(&mut rt, &bridge).unwrap();
    rt.lua
        .load(
            r#"
            shipyardEnded = 0
            local probe = UIParent:CreateWidget('emptywidget', 'shipyardProbe', 'UIParent')
            probe:RegisterEvent('HOUSE_INTERACTION_END')
            probe:SetHandler('OnEvent', function(self, event) shipyardEnded = shipyardEnded + 1 end)
        "#,
        )
        .exec()
        .unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    {
        let mut view = state.0.borrow_mut();
        view.current = Some(Current {
            obj_id: 900,
            tl_id: 7,
            design: 171,
            snapshot: Some(snapshot(42, 20, 12)),
            name: "Farm".into(),
            mine: true,
            tax: None,
            decorations: 2,
            ..Default::default()
        });
        view.last_tax = Some(tax_info());
        view.pointer_picked = Some(77);
    }
    let mut units = Units::default();
    units.apply(&Event::UnitSpawned(Box::new(shipyard_unit())));
    let mut app = App::new();
    app.insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(units)
        .init_resource::<Houses>()
        .init_resource::<Doodads>()
        .init_resource::<Inventory>()
        .insert_resource(shipyard_state())
        .add_message::<NetCommand>()
        .add_systems(Update, pointer_interaction);
    app.update();
    let ended: i64 = app
        .world()
        .get_non_send::<UiRuntime>()
        .unwrap()
        .lua
        .load("return shipyardEnded")
        .eval()
        .unwrap();
    assert_eq!(ended, 1, "HOUSE_INTERACTION_END");
    let view = state.0.borrow();
    assert!(view.current.is_none());
    assert!(view.last_tax.is_none());
}
