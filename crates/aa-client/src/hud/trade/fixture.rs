//! Explicit synthetic trade instances on the original UI and real item definitions; no network.
use std::cell::RefCell;
use std::rc::Rc;

use aa_net::model::{EquipDetail, Equipment, ModelParams, Unit, UnitKind};
use bevy::window::PrimaryWindow;

use super::*;

pub(super) fn other_item() -> Item {
    Item {
        id: 900_002,
        template_id: 14698,
        count: 2,
        grade: 2,
        ..default()
    }
}

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, Inventory, Units), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let game = super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?;
    let potion = *game
        .items
        .iter()
        .filter(|(_, i)| i.named.name == "Heart's Beat")
        .map(|(id, _)| id)
        .min()
        .unwrap();
    let mut inventory = Inventory::default();
    inventory.bag_slots = 50;
    inventory.bag.resize(50, None);
    inventory.bag[4] = Some(Item {
        id: 9_007_199_254_740_993,
        template_id: potion,
        count: 4,
        ..default()
    });
    inventory.bag[6] = Some(Item {
        id: 900_001,
        template_id: 24488,
        count: 1,
        grade: 3,
        detail: ItemDetail::Equip(EquipDetail { durability: 37, ..default() }),
        ..default()
    });
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Trade fixture".into();
        d.snapshot.level = 30;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        d.snapshot.money = 20_000;
        d.panels.actions_known = true;
        d.panels.bag_slots = inventory.bag_slots;
        d.panels.bag = inventory
            .bag
            .iter()
            .map(|item| {
                item.as_ref().map(|item| {
                    let info = &game.items[&item.template_id];
                    super::super::panels::SlotSnap {
                        kind: "item",
                        id: item.template_id,
                        uid: item.id,
                        name: info.named.name.clone(),
                        icon: info.named.icon.clone(),
                        count: i64::from(item.count),
                        stack: info.max_stack > 1,
                        grade: super::super::store::ui_grade(item.grade),
                        desc: info.desc.clone(),
                        level: info.level,
                        group: info.group,
                        flags: Some(item.flags),
                        ..default()
                    }
                })
            })
            .collect();
        d.panels.target = Some(super::super::panels::TargetSnap {
            obj_id: 200,
            character_id: 2,
            name: "Fixture peer".into(),
            unit_type: "character",
            detail: "character",
            relation: "friendly",
            ..default()
        });
        assert!(d.game.0.set(game).is_ok());
    }
    super::super::api::install(&mut rt, &bridge)?;
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    super::super::api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    let mut units = Units::default();
    units.by_id.insert(
        200,
        Unit {
            obj_id: 200,
            name: "Fixture peer".into(),
            kind: UnitKind::Character { char_id: 2 },
            owner_name: String::new(),
            pos: default(),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 30,
            model_id: 0,
            equipment: Equipment::default(),
            model_params: ModelParams::default(),
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
            house_posture: None,
        },
    );
    Ok((rt, bridge, inventory, units))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, inventory, units) = runtime(width as f32, height as f32).unwrap();
    let mut trade = Trade::default();
    trade.apply(&Event::Trade(TradeEvent::Started { obj_id: 200 }));
    trade.apply(&Event::Trade(TradeEvent::ItemOffered {
        container: 2,
        slot: 4,
        amount: 0,
    }));
    trade.mine[0].item = inventory.bag[4].clone();
    trade.apply(&Event::Trade(TradeEvent::MoneyOffered(105)));
    trade.apply(&Event::Trade(TradeEvent::OtherItemOffered(other_item())));
    trade.apply(&Event::Trade(TradeEvent::OtherMoneyOffered(25)));
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original trade"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(inventory)
        .insert_resource(units)
        .insert_resource(trade)
        .insert_resource(PlayerStats { money: 20_000, ..default() })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, sync.before(aa_ui::UiFrame))
        .run();
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
