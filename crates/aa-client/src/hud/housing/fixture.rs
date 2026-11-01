//! Self-check for the owner menu: the original `housing_manager.alb` and `maintain_window.alb` over synthetic
//! `SCMyHouse` / `SCHouseTaxInfo` events, no socket. Run headless:
//! `cargo run -p aa-client --example house_owner -- --headless --mute --state paid --exit-after 2`.
use aa_net::housing::{House, HouseCommand, HouseSale, HouseSnapshot, HouseTaxInfo, HousingEvent};
use aa_net::model::{Equipment, ModelParams, UnitKind};
use aa_net::state::{Doodads, Inventory, Units, WorldState};
use aa_net::wire::Writer;
use aa_net::{Command, Event, Houses, NetCommand, NetEvent, Pos};
use aa_ui::UiRuntime;
use bevy::window::PrimaryWindow;

use super::*;
use crate::hud::api::{self, HudBridge};
use crate::hud::data::housing::HousingDecoration;
use crate::hud::data::{GameData, ItemInfo, Named};

const HOUSE_OBJ: u32 = 900;
const OPEN_HOUSE_OBJ: u32 = 901;
const HOUSE_TL: u16 = 7;
const OPEN_HOUSE_TL: u16 = 8;
const HOUSE_DESIGN: u32 = 171;
const OPEN_HOUSE_DESIGN: u32 = 273;
const FURNITURE_ITEM: u32 = 107;
const STORAGE_ITEM: u32 = 108;

#[derive(Resource, Default)]
struct Step(u32);

pub(crate) fn run() {
    let args = aa_dev::DevArgs::from_env();
    let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    if !std::path::Path::new(&pak).exists() || !aa_anim::db::GameDb::available() {
        eprintln!("house_owner: needs game_pak (AA_PAK) and compact.sqlite3 (AA_DB); skipped");
        return;
    }
    let (rt, bridge, path) = runtime(args.size.0 as f32, args.size.1 as f32).unwrap();
    let (units, houses, doodads, inventory) = world();
    App::new()
        .add_plugins((
            aa_dev::default_plugins(
                "aa-client: original housing owner menu — synthetic SCMyHouse/SCHouseTaxInfo",
            ),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(units)
        .insert_resource(houses)
        .init_resource::<aa_net::Shipyards>()
        .insert_resource(doodads)
        .insert_resource(inventory)
        .insert_resource(Step::default())
        .init_resource::<crate::ingame::housing::placement::Builder>()
        .init_resource::<crate::hud::HudPointerOverUi>()
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_message::<crate::ingame::housing::placement::BuilderSignal>()
        .add_systems(Startup, physical_pixels)
        .add_systems(
            Update,
            (
                drive.before(super::sync),
                super::sync,
                verify.after(super::sync),
                advance.after(verify),
            ),
        )
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

fn runtime(
    width: f32,
    height: f32,
) -> Result<(UiRuntime, api::Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    rt.load_ui_texts(std::path::Path::new(&db))?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(api::HudData::default()));
    {
        let mut d = bridge.borrow_mut();
        d.character_id = 42;
        d.pos = Vec3::new(15338.0, 14632.0, 100.0);
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Owner".into();
        d.snapshot.level = 30;
        d.snapshot.money = 500_000;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.snapshot.mp = 100;
        d.snapshot.max_mp = 100;
        d.panels.bag_slots = 50;
        assert!(d.game.0.set(game(&db)).is_ok(), "fresh game data");
    }
    api::install(&mut rt, &bridge)?;
    let path = crate::settings::isolated_bindings(&rt)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    // The spy is a real widget handler: it sees exactly the tuples the original scripts see.
    rt.lua
        .load(
            r#"
            local w = UIParent:CreateWidget('emptywidget', 'houseOwnerSpy', 'UIParent')
            for _, name in ipairs({'HOUSE_INTERACTION_START', 'HOUSE_TAX_INFO', 'CHANGE_ACTABILITY_DECO_NUM'}) do
                w:RegisterEvent(name)
            end
            SPY = {}
            w:SetHandler('OnEvent', function(self, event, ...) SPY[#SPY + 1] = {event, ...} end)
            "#,
        )
        .exec()?;
    api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    Ok((rt, bridge, path))
}

/// The real tables, with design 171 given two built-in doodads and two known furniture designs.
fn game(db: &str) -> GameData {
    let mut game = GameData::load(db).expect("game data");
    if let Some(design) = game.housing.housings.get_mut(&HOUSE_DESIGN) {
        design.binding_doodads = 2;
    }
    for (id, item, actability, points, deco_group) in [
        (900_010, FURNITURE_ITEM, 1, 100, 1),
        (900_011, STORAGE_ITEM, 5, 30, 5),
    ] {
        game.housing.decorations.insert(
            id,
            HousingDecoration {
                id,
                name: format!("Furniture {item}"),
                doodad_id: 5000 + id,
                actability_group_id: actability,
                actability_up: points,
                deco_actability_group_id: deco_group,
                ..default()
            },
        );
        game.housing.decoration_design_by_item.insert(item, id);
    }
    for (item, name) in [
        (FURNITURE_ITEM, "Furniture"),
        (STORAGE_ITEM, "Storage chest"),
    ] {
        game.items.insert(
            item,
            ItemInfo {
                named: Named {
                    name: name.into(),
                    icon: String::new(),
                },
                ..default()
            },
        );
    }
    game
}

fn snapshot(tl_id: u16, obj_id: u32, design: u32) -> HouseSnapshot {
    HouseSnapshot {
        tl_id,
        db_id: u32::from(tl_id),
        obj_id,
        template_id: design,
        model_id: 1083,
        co_owner_id: 42,
        owner_id: 42,
        owner_name: "Owner".into(),
        account_id: 17,
        permission: 0,
        all_actions: 0,
        current_actions: 0,
        tax: 150_000,
        x: aa_net::wire::coord_to_long(15340.0),
        y: aa_net::wire::coord_to_long(14633.0),
        z: 100.0,
        name: "Thatched farm".into(),
        allow_recover: true,
        sell_price: 0,
        sell_to_player_id: 0,
        sell_to_name: String::new(),
    }
}

fn housing_unit(obj_id: u32, tl_id: u16, design: u32, name: &str) -> aa_net::Unit {
    aa_net::Unit {
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        obj_id,
        name: name.into(),
        kind: UnitKind::Housing {
            tl_id,
            template_id: design,
            build_step: -1,
        },
        owner_name: "Owner".into(),
        pos: Pos::new(15340.0, 14633.0, 100.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 1,
        model_id: 1083,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 100,
        mp: 0,
        max_hp: 100,
        max_mp: 0,
        dead: false,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
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
    }
}

fn doodad(obj_id: u32, house: u32, doodad_template: u32, item: u32) -> aa_net::Doodad {
    aa_net::Doodad {
        obj_id,
        template_id: doodad_template,
        owner_obj_id: 0,
        parent_obj_id: house,
        attach_point: 1,
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 0,
        owner_id: 42,
        ucc_id: 0,
        item_template_id: item,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 3,
        owner_db_id: u32::from(HOUSE_TL),
        data: 0,
    }
}

fn world() -> (Units, Houses, Doodads, Inventory) {
    let mut units = Units::default();
    units.by_id.insert(
        HOUSE_OBJ,
        housing_unit(HOUSE_OBJ, HOUSE_TL, HOUSE_DESIGN, "Thatched farm"),
    );
    units.by_id.insert(
        OPEN_HOUSE_OBJ,
        housing_unit(OPEN_HOUSE_OBJ, OPEN_HOUSE_TL, OPEN_HOUSE_DESIGN, "Hall"),
    );
    let mut doodads = Doodads::default();
    for (i, (doodad_template, item)) in [
        (4566, 0),
        (4278, 0),
        (5010, FURNITURE_ITEM),
        (5011, STORAGE_ITEM),
    ]
    .into_iter()
    .enumerate()
    {
        let id = 0x100 + i as u32;
        doodads
            .by_id
            .insert(id, doodad(id, HOUSE_OBJ, doodad_template, item));
    }
    (units, Houses::default(), doodads, Inventory::default())
}

fn tax(weeks: i32, paid: bool) -> HouseTaxInfo {
    HouseTaxInfo {
        tl_id: HOUSE_TL,
        dominion_tax_rate: 17,
        money_amount: 150_000,
        money_amount2: 450_000,
        due: 1_700_000_000,
        is_already_paid: paid,
        weeks_without_pay: weeks,
        is_heavy_tax_house: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    step: Res<Step>,
    mut world: Local<Option<WorldState>>,
    rt: NonSend<UiRuntime>,
    bridge: NonSend<HudBridge>,
    mut units: ResMut<Units>,
    mut houses: ResMut<Houses>,
    mut doodads: ResMut<Doodads>,
    mut inventory: ResMut<Inventory>,
    mut events: MessageWriter<NetEvent>,
) {
    let world = world.get_or_insert_with(|| WorldState {
        units: units.clone(),
        doodads: doodads.clone(),
        inventory: inventory.clone(),
        ..default()
    });
    let lua = |code: &str| {
        rt.lua
            .load(code)
            .exec()
            .unwrap_or_else(|e| panic!("house_owner lua: {e}\n{code}"));
    };
    let send = |event: Event, world: &mut WorldState, events: &mut MessageWriter<NetEvent>| {
        world.apply(&event);
        events.write(NetEvent(event));
    };
    match step.0 {
        0 => {
            for (tl, obj, design) in [
                (HOUSE_TL, HOUSE_OBJ, HOUSE_DESIGN),
                (OPEN_HOUSE_TL, OPEN_HOUSE_OBJ, OPEN_HOUSE_DESIGN),
            ] {
                send(
                    Event::Housing(HousingEvent::MyHouse(snapshot(tl, obj, design))),
                    world,
                    &mut events,
                );
            }
            lua("HUD.house_interact(900)");
        }
        1 => send(
            Event::Housing(HousingEvent::TaxInfo(tax(-1, true))),
            world,
            &mut events,
        ),
        2 => send(
            Event::Housing(HousingEvent::TaxInfo(tax(0, false))),
            world,
            &mut events,
        ),
        3 => send(
            Event::Housing(HousingEvent::TaxInfo(tax(1, false))),
            world,
            &mut events,
        ),
        4 => lua("X2House:SetHousePermission(3)"),
        5 => {
            // The always-public design's menu: the same click must send nothing.
            send(
                Event::Housing(HousingEvent::MyHouse(snapshot(
                    OPEN_HOUSE_TL,
                    OPEN_HOUSE_OBJ,
                    OPEN_HOUSE_DESIGN,
                ))),
                world,
                &mut events,
            );
            lua("HUD.house_interact(901)");
        }
        6 => lua("X2House:SetHousePermission(3)"),
        7 => lua("HUD.house_interact(900)"),
        8 => {
            // Leave the window in the state the screenshot names.
            let state = crate::ingame::enter::flag(&aa_dev::args(), "--state")
                .unwrap_or_else(|| "paid".into());
            let tax = match state.as_str() {
                "due" => tax(0, false),
                "overdue" => tax(1, false),
                _ => tax(-1, true),
            };
            send(
                Event::Housing(HousingEvent::TaxInfo(tax)),
                world,
                &mut events,
            );
            info!("house_owner: check done (final tax state: {state})");
        }
        9 => {
            crate::hud::crafting::apply_actability(
                &mut bridge.0.borrow_mut(),
                &Event::Actability(aa_net::ActabilityEvent::Snapshot {
                    last: true,
                    entries: vec![aa_net::ActabilityValue { id: 1, point: 100, step: 0 }],
                }),
            );
        }
        _ => {}
    }
    *houses = world.houses.clone();
    *units = world.units.clone();
    *doodads = world.doodads.clone();
    *inventory = world.inventory.clone();
}

fn advance(mut step: ResMut<Step>) {
    step.0 += 1;
}

fn verify(
    step: Res<Step>,
    rt: NonSend<UiRuntime>,
    mut commands: MessageReader<NetCommand>,
    mut seen: Local<Vec<aa_net::Command>>,
) {
    for NetCommand(command) in commands.read() {
        seen.push(command.clone());
    }
    let lua = |code: &str| {
        rt.lua
            .load(code)
            .exec()
            .unwrap_or_else(|e| panic!("house_owner lua: {e}\n{code}"))
    };
    let permissions: Vec<_> = seen
        .iter()
        .filter_map(|c| match c {
            aa_net::Command::House(HouseCommand::ChangePermission { tl, permission }) => {
                Some((*tl, *permission))
            }
            _ => None,
        })
        .collect();
    match step.0 {
        // Step 1 has read the paid tax event; the window follows it in the same frame.
        1 => {
            lua(r#"
                assert(X2House:IsMyHouse(), 'not mine: ' .. X2House:GetHouseName() .. ' owner=' .. X2House:GetHouseOwnerName())
                local start
                for i = #SPY, 1, -1 do
                    if SPY[i][1] == 'HOUSE_INTERACTION_START' then start = SPY[i] break end
                end
                assert(start, 'no HOUSE_INTERACTION_START')
                assert(start[2] == 'housing', tostring(start[2]))
                assert(start[3] == 'house', tostring(start[3]))
                assert(start[4] == false, tostring(start[4]))
                local tax
                for i = #SPY, 1, -1 do
                    if SPY[i][1] == 'HOUSE_TAX_INFO' then tax = SPY[i] break end
                end
                assert(tax, 'no HOUSE_TAX_INFO')
                assert(tax[2] == 17 and tax[3] == '450000' and type(tax[4]) == 'table', 'rate/total/due')
                assert(tax[5] == -1 and tax[6] == true and tax[7] == false, 'raw weeks after paid')
                assert(tax[8] == '150000', 'deposit')
                local count, limit = X2House:GetHouseDecoCount()
                assert(count == 2 and limit == 49, tostring(count) .. '/' .. tostring(limit))
                assert(X2House:GetActabilityDecoCount() == 2)
                local point = X2House:GetDecoActabilityPoint(1)
                assert(point.decoPoint == 100 and type(point.name) == 'string', tostring(point.decoPoint))
                local groups = X2House:GetDecoActabilityLimitGroup()
                assert(#groups == 2 and groups[1].maxCount == 3 and groups[1].arrangeCount == 1)
                local text = housing.maintainWindow.contentTaxInfo:GetText()
                assert(string.find(text, locale.housing.payState.full, 1, true), text)
                "#);
        }
        2 => lua(r#"
            local tax
            for i = #SPY, 1, -1 do if SPY[i][1] == 'HOUSE_TAX_INFO' then tax = SPY[i] break end end
            assert(tax[5] == 0 and tax[6] == false, 'raw weeks when due')
            local text = housing.maintainWindow.contentTaxInfo:GetText()
            assert(string.find(text, locale.housing.payState.unpaid, 1, true), text)
            "#),
        3 => lua(r#"
            local tax
            for i = #SPY, 1, -1 do if SPY[i][1] == 'HOUSE_TAX_INFO' then tax = SPY[i] break end end
            assert(tax[5] == 1 and tax[6] == false, 'raw weeks when overdue')
            local text = housing.maintainWindow.contentTaxInfo:GetText()
            assert(string.find(text, locale.housing.payState.overdue, 1, true), text)
            "#),
        // A click on the public radio sends one packet with the server enum (2).
        5 => {
            assert_eq!(
                permissions,
                vec![(HOUSE_TL, 2)],
                "one CSChangeHousePermission, the server enum"
            );
            let mut writer = aa_net::wire::Writer::new();
            HouseCommand::ChangePermission {
                tl: HOUSE_TL,
                permission: 2,
            }
            .write(&mut writer);
            assert_eq!(writer.as_bytes(), &[7, 0, 2], "0x5a body: tl, enum");
        }
        // The always-public design refuses the same click: still only the one packet.
        6 => {
            assert_eq!(
                permissions,
                vec![(HOUSE_TL, 2)],
                "an always-public design sends nothing"
            );
            lua(r#"
                local _, alwaysPublic = X2House:GetHousePermission()
                assert(alwaysPublic == true)
                "#);
        }
        // The received actability points changed: the spy sees the native event.
        9 => lua(r#"
            local seen = false
            for i = #SPY, 1, -1 do
                if SPY[i][1] == 'CHANGE_ACTABILITY_DECO_NUM' then seen = true break end
            end
            assert(seen, 'the spy saw no CHANGE_ACTABILITY_DECO_NUM')
            assert(X2House:GetActabilityDecoCount() == 2, tostring(X2House:GetActabilityDecoCount()))
            "#),
        _ => {}
    }
}

/// Self-check for slice 6b: two sessions (seller 42, buyer 7) over synthetic sale receipts, no socket.
/// The seller's and the buyer's `sync` run the real `X2House` sale methods; every click must send exactly one
/// `CSSellHouse` / `CSBuyHouse` / `CSSellHouseCancel` with its wire bytes, a refused gate none and its
/// `ERROR_MSG` notice, and the receipts raise the events in the native order.
#[allow(dead_code)]
pub(crate) fn run_sale() {
    let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    if !std::path::Path::new(&pak).exists() || !aa_anim::db::GameDb::available() {
        eprintln!("house_sale: needs game_pak (AA_PAK) and compact.sqlite3 (AA_DB); skipped");
        return;
    }
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let mut seller = Session::new(42, "Seller", 50_000, &db, 2);
    let mut buyer = Session::new(7, "Buyer", 20_000, &db, 0);
    seller.lua("HUD.house_interact(900)");
    seller.step();
    buyer.lua("HUD.house_interact(900)");
    buyer.step();
    seller.commands();
    buyer.commands();
    // The original sell form over our `GetHouseSaleInfo`/`GetHouseSaleItemInfo`: the register button is off with
    // 0/0/0, on for 1 gold with the seller's two certificates, and off for 201 gold (3 certificates needed).
    seller.lua(
        r#"
        local w = CreateSellRegisterWindow('sellTest', UIParent)
        housing.sellRegisterWindow = w
        w:Show(true)
        w.moneyEditsWindow.goldEdit:SetText('0')
        w.moneyEditsWindow.silverEdit:SetText('0')
        w.moneyEditsWindow.copperEdit:SetText('0')
        assert(not w.registerButton:IsEnabled(), '0/0/0 must disable the register button')
        w.moneyEditsWindow.goldEdit:SetText('1')
        assert(w.registerButton:IsEnabled(), '1 gold with 2 certificates must enable it')
        w.moneyEditsWindow.goldEdit:SetText('201')
        assert(not w.registerButton:IsEnabled(), '201 gold needs 3 certificates, the seller has 2')
        w:Show(false)
        print('SELLFORM checks ok')
    "#,
    );

    // Gates: not the owner, no sale to cancel, nothing for sale to buy. Each shows its ERROR_MSG text and
    // sends no packet.
    buyer.lua("X2House:SetHouseForSale('1', '0', '0', '')");
    buyer.step();
    assert!(
        buyer.commands().is_empty(),
        "a stranger's sell sent a packet"
    );
    let expected = buyer.localized("HOUSE_CANNOT_SELL_AS_NOT_OWNER");
    assert!(
        !expected.is_empty(),
        "no localized HOUSE_CANNOT_SELL_AS_NOT_OWNER"
    );
    assert_eq!(buyer.last_sysmsg().as_ref(), Some(&expected));

    seller.lua("X2House:CancelHouseForSale()");
    seller.step();
    assert!(
        seller.commands().is_empty(),
        "a cancel without a sale sent a packet"
    );
    let expected = seller.localized("HOUSE_CANNOT_CANCEL_SELL_AS_NOT_FOR_SALE");
    assert_eq!(seller.last_sysmsg().as_ref(), Some(&expected));

    buyer.lua("X2House:BuyHouse()");
    buyer.step();
    assert!(
        buyer.commands().is_empty(),
        "a buy without a sale sent a packet"
    );
    let expected = buyer.localized("HOUSE_CANNOT_BUY_AS_NOT_FOR_SALE");
    assert_eq!(buyer.last_sysmsg().as_ref(), Some(&expected));

    // The owner puts it up for sale: one CSSellHouse, 1 gold in copper, no designated buyer.
    seller.lua("X2House:SetHouseForSale('1', '0', '0', '')");
    seller.step();
    let commands = seller.commands();
    assert_eq!(commands.len(), 1, "{commands:?}");
    let Command::House(command) = &commands[0] else {
        panic!("not a house command: {commands:?}")
    };
    assert_eq!(
        format!("{command:?}"),
        format!(
            "{:?}",
            HouseCommand::Sell {
                tl: HOUSE_TL,
                money_amount: 10_000,
                sell_to: String::new()
            }
        )
    );
    assert_eq!(
        body(command),
        [7, 0, 0x10, 0x27, 0, 0, 0, 0],
        "CSSellHouse: tl, money, empty sellTo"
    );
    let set_for_sale = Event::Housing(HousingEvent::SetForSale(HouseSale {
        tl_id: HOUSE_TL,
        money_amount: 10_000,
        sell_to_player_id: 0,
        sell_to_name: String::new(),
        house_name: "Thatched farm".into(),
    }));
    seller.feed(set_for_sale.clone());
    buyer.feed(set_for_sale);
    seller.step();
    buyer.step();
    assert_eq!(
        seller.events(),
        vec!["HOUSE_SET_SELL_SUCCESS", "HOUSE_INFO_UPDATED"],
        "the seller's sale receipt"
    );
    assert_eq!(seller.event_arg(1), "Thatched farm");
    assert_eq!(
        buyer.events(),
        vec!["HOUSE_INFO_UPDATED"],
        "the open house refreshed for the buyer too"
    );

    // The owner cancels: one CSSellHouseCancel, then HOUSE_CANCEL_SELL_SUCCESS and HOUSE_INFO_UPDATED.
    seller.lua("X2House:CancelHouseForSale()");
    seller.step();
    let commands = seller.commands();
    assert_eq!(commands.len(), 1, "{commands:?}");
    let Command::House(command) = &commands[0] else {
        panic!("not a house command: {commands:?}")
    };
    assert_eq!(
        format!("{command:?}"),
        format!("{:?}", HouseCommand::CancelSell { tl: HOUSE_TL })
    );
    assert_eq!(body(command), [7, 0], "CSSellHouseCancel: tl");
    let reset = Event::Housing(HousingEvent::ResetForSale {
        tl_id: HOUSE_TL,
        house_name: "Thatched farm".into(),
    });
    seller.feed(reset.clone());
    buyer.feed(reset);
    seller.step();
    buyer.step();
    assert_eq!(
        seller.events(),
        vec![
            "HOUSE_SET_SELL_SUCCESS",
            "HOUSE_INFO_UPDATED",
            "HOUSE_CANCEL_SELL_SUCCESS",
            "HOUSE_INFO_UPDATED"
        ]
    );
    assert_eq!(
        buyer.events(),
        vec!["HOUSE_INFO_UPDATED", "HOUSE_INFO_UPDATED"],
        "the buyer's open house refreshes on both receipts"
    );

    // Put it up again, then the buyer cannot pay: no packet, HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY.
    seller.lua("X2House:SetHouseForSale('1', '0', '0', '')");
    seller.step();
    assert_eq!(seller.commands().len(), 1, "one CSSellHouse");
    let set_for_sale = Event::Housing(HousingEvent::SetForSale(HouseSale {
        tl_id: HOUSE_TL,
        money_amount: 10_000,
        sell_to_player_id: 0,
        sell_to_name: String::new(),
        house_name: "Thatched farm".into(),
    }));
    seller.feed(set_for_sale.clone());
    buyer.feed(set_for_sale);
    seller.step();
    buyer.step();
    assert_eq!(
        seller.events().last().map(String::as_str),
        Some("HOUSE_INFO_UPDATED")
    );
    assert_eq!(
        buyer.events().last().map(String::as_str),
        Some("HOUSE_INFO_UPDATED")
    );

    buyer.set_money(0);
    buyer.step();
    buyer.lua("X2House:BuyHouse()");
    buyer.step();
    assert!(
        buyer.commands().is_empty(),
        "a buy without money sent a packet"
    );
    let expected = buyer.localized("HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY");
    assert_eq!(buyer.last_sysmsg().as_ref(), Some(&expected));

    // With the money, one CSBuyHouse with the sale price.
    buyer.set_money(20_000);
    buyer.step();
    buyer.lua("X2House:BuyHouse()");
    buyer.step();
    let commands = buyer.commands();
    assert_eq!(commands.len(), 1, "{commands:?}");
    let Command::House(command) = &commands[0] else {
        panic!("not a house command: {commands:?}")
    };
    assert_eq!(
        format!("{command:?}"),
        format!(
            "{:?}",
            HouseCommand::Buy {
                tl: HOUSE_TL,
                money_amount: 10_000
            }
        )
    );
    assert_eq!(
        body(command),
        [7, 0, 0x10, 0x27, 0, 0],
        "CSBuyHouse: tl, money"
    );

    // The sale completes: the open house refreshes, then HOUSE_SALE_SUCCESS for the seller and
    // HOUSE_BUY_SUCCESS for the buyer.
    let sold = Event::Housing(HousingEvent::Sold {
        tl_id: HOUSE_TL,
        previous_owner_id: 42,
        new_owner_id: 7,
        new_owner_account_id: 17,
        owner_name: "Buyer".into(),
        house_name: "Thatched farm".into(),
    });
    seller.feed(sold.clone());
    buyer.feed(sold);
    seller.step();
    buyer.step();
    assert_eq!(
        seller.events()[6..],
        [
            "HOUSE_INFO_UPDATED".to_owned(),
            "HOUSE_SALE_SUCCESS".to_owned()
        ]
    );
    assert_eq!(seller.event_arg(8), "Thatched farm");
    assert_eq!(
        buyer.events()[3..],
        [
            "HOUSE_INFO_UPDATED".to_owned(),
            "HOUSE_BUY_SUCCESS".to_owned()
        ]
    );
    assert_eq!(buyer.event_arg(5), "Thatched farm");
    assert!(!seller.lua_bool("return X2House:IsMyHouse()"));
    assert!(buyer.lua_bool("return X2House:IsMyHouse()"));
    // `FUN_3932B8A0` re-requests the tax of the open house when the receipt arrives.
    assert!(matches!(
        buyer.commands().as_slice(),
        [Command::House(HouseCommand::RequestTax { tl: HOUSE_TL })]
    ));
    seller.commands();

    // The new owner demolishes it with the 1.2 client's skill (12945, `DAT_39ac99e0`); the house leaves
    // the scene and is no longer mine.
    buyer.lua("X2House:Demolish()");
    buyer.step();
    let commands = buyer.commands();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(
        format!("{:?}", commands[0]),
        format!(
            "{:?}",
            Command::UseSkill {
                skill_id: 12_945,
                target_obj_id: HOUSE_OBJ
            }
        )
    );
    let demolished = Event::Housing(HousingEvent::Demolished { tl_id: HOUSE_TL });
    seller.feed(demolished.clone());
    buyer.feed(demolished);
    seller.step();
    buyer.step();
    assert!(!buyer.lua_bool("return X2House:IsMyHouse()"));
    assert!(buyer.demolished(), "SCHouseDemolished marks the house");
    println!("house_sale: check done: gates, sell, cancel, buy, sold and demolish");
}

struct Session {
    app: App,
    state: State,
    bridge: Shared,
    path: std::path::PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Session {
    fn new(me: u32, name: &str, money: i64, db: &str, appraisals: i32) -> Session {
        let (rt, bridge, path) =
            runtime(1280.0, 720.0).unwrap_or_else(|e| panic!("the {name} UI runtime: {db}: {e}"));
        {
            let mut d = bridge.borrow_mut();
            d.character_id = me;
            d.snapshot.name = name.into();
            d.snapshot.money = money;
        }
        rt.lua
            .load(
                r#"
                local w = UIParent:CreateWidget('emptywidget', 'saleSpy', 'UIParent')
                for _, name in ipairs({'HOUSE_SET_SELL_SUCCESS', 'HOUSE_SET_SELL_FAIL',
                    'HOUSE_CANCEL_SELL_SUCCESS', 'HOUSE_CANCEL_SELL_FAIL', 'HOUSE_BUY_SUCCESS',
                    'HOUSE_BUY_FAIL', 'HOUSE_SALE_SUCCESS', 'HOUSE_INFO_UPDATED', 'SYSMSG'}) do
                    w:RegisterEvent(name)
                end
                EVENTS = {}
                SYSTEM = {}
                SYSCOUNT = 0
                w:SetHandler('OnEvent', function(self, event, ...)
                    if event == 'SYSMSG' then
                        SYSTEM[#SYSTEM + 1] = select(1, ...)
                        SYSCOUNT = SYSCOUNT + 1
                    else EVENTS[#EVENTS + 1] = {event, ...} end
                end)
                "#,
            )
            .exec()
            .expect("the sale spy");
        let state = rt
            .lua
            .app_data_ref::<State>()
            .expect("the housing state")
            .clone();
        let (units, mut houses, doodads, mut inventory) = world();
        houses.by_tl_id.insert(
            HOUSE_TL,
            House {
                snapshot: snapshot(HOUSE_TL, HOUSE_OBJ, HOUSE_DESIGN),
                demolished: false,
                build_pay: None,
                tax: None,
                sale: None,
            },
        );
        if appraisals > 0 {
            inventory.bag.push(Some(aa_net::model::Item {
                template_id: APPRAISAL_CERTIFICATE,
                count: appraisals,
                ..default()
            }));
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<NetCommand>()
            .add_message::<NetEvent>()
            .add_message::<crate::ingame::housing::placement::BuilderSignal>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<crate::hud::HudPointerOverUi>()
            .insert_resource(units)
            .insert_resource(houses)
            .insert_resource(doodads)
            .insert_resource(inventory)
            .init_resource::<aa_net::Shipyards>()
            .init_resource::<crate::ingame::housing::placement::Builder>()
            .insert_non_send(HudBridge(bridge.clone()))
            .insert_non_send(rt)
            .add_systems(Update, sync);
        Session {
            app,
            state,
            bridge,
            path,
        }
    }

    fn step(&mut self) {
        self.app.update();
    }

    fn lua(&self, code: &str) {
        self.app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load(code)
            .exec()
            .unwrap_or_else(|e| panic!("house_sale lua: {e}\n{code}"));
    }

    fn lua_bool(&self, code: &str) -> bool {
        self.app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load(code)
            .eval()
            .unwrap_or(false)
    }

    fn localized(&self, key: &str) -> String {
        self.app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load(format!(
                "return X2Locale:LocalizeUiText(ERROR_MSG, '{key}') or ''"
            ))
            .eval()
            .unwrap_or_default()
    }

    fn last_sysmsg(&self) -> Option<String> {
        self.app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load("return SYSTEM[#SYSTEM]")
            .eval()
            .ok()
            .flatten()
    }

    fn events(&self) -> Vec<String> {
        let text: String = self
            .app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load("local t = {} for i = 1, #EVENTS do t[i] = EVENTS[i][1] end return table.concat(t, ',')")
            .eval()
            .unwrap_or_default();
        text.split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn event_arg(&self, index: usize) -> String {
        self.app
            .world()
            .get_non_send::<UiRuntime>()
            .expect("the UI runtime")
            .lua
            .load(format!("return tostring(EVENTS[{index}][2] or '')"))
            .eval()
            .unwrap_or_default()
    }

    fn commands(&mut self) -> Vec<Command> {
        self.app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|NetCommand(c)| c)
            .collect()
    }

    fn feed(&mut self, event: Event) {
        self.app.world_mut().resource_mut::<Houses>().apply(&event);
        self.app
            .world_mut()
            .resource_mut::<Messages<NetEvent>>()
            .write(NetEvent(event));
    }

    fn set_money(&mut self, money: i64) {
        self.bridge.borrow_mut().snapshot.money = money;
    }

    fn demolished(&self) -> bool {
        self.app
            .world()
            .resource::<Houses>()
            .by_tl_id
            .get(&HOUSE_TL)
            .is_some_and(|h| h.demolished)
    }
}

fn body(command: &HouseCommand) -> Vec<u8> {
    let mut w = Writer::new();
    command.write(&mut w);
    w.as_bytes().to_vec()
}
