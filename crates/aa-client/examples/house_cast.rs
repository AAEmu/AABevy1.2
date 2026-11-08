//! A right click on a house under construction casts its stage skill, from the 2026-10-31 capture; no socket.
//!
//! The HUD's own `hud::housing::sync` runs against a synthetic house and bag: the `HUD.house_interact` dev hook is
//! the right click. One click with the stage material in the bag sends exactly one `Command::UseSkill { 14575, 39109 }`
//! whose `CSStartSkill` is the captured 13-byte body (target kind 0 + the house object id). Without the material the
//! click sends no cast and shows the captured text "Not enough Stone Pack." (the client's local check looks at the
//! bag only; the same click still casts when a bundle is equipped in the backpack slot, as in the capture).
//!
//! `cargo run -p aa-client --example house_cast -- --mute` exits 0 when every check passes and 1 when one fails.
//! Needs `AA_DB` for the housing steps and the English UI texts; without it it says so and exits 0.
#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"] mod sound;
#[path = "../src/task.rs"]
mod task;
use bevy::prelude::*;
use lua_value::lua_int;
#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

use std::cell::RefCell;
use std::rc::Rc;

use aa_net::housing::{House, HouseSnapshot};
use aa_net::model::{slot, Item, Unit, UnitKind};
use aa_net::state::{Doodads, Inventory, Units};
use aa_net::wire::Pos;
use aa_net::{Command, Houses, NetCommand, NetEvent};
use bevy::ecs::system::SystemParam;
use hud::api::{self, HudBridge, HudData};
use hud::data::{GameData, ItemInfo, Named};
use hud::housing::sync;
use ingame::housing::data::{self as house_data, HousingData};
use ingame::housing::placement::{Builder, BuilderSignal};

const HOUSE_OBJ: u32 = 39_109;
const SKILL: u32 = 14_575;
const STONE_PACK: u32 = 17_684;
const CAPTURE_CAST: [u8; 13] = [0xef, 0x38, 0, 0, 0, 0xa7, 0x21, 0, 0, 0xc5, 0x98, 0, 0];
const NOTICE: &str = "Not enough Stone Pack.";

fn main() -> AppExit {
    let Ok(db) = std::env::var("AA_DB") else {
        eprintln!("house_cast: skipped, no game database (set AA_DB)");
        return AppExit::Success;
    };
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        eprintln!("house_cast: skipped, cannot open {db}");
        return AppExit::Success;
    };
    let Ok(tables) = HousingData::load_tables(&conn) else {
        eprintln!("house_cast: skipped, cannot read the housing tables");
        return AppExit::Success;
    };
    if tables.designs.get(&171).is_none_or(|d| d.steps.is_empty()) {
        eprintln!("house_cast: skipped, design 171 is missing");
        return AppExit::Success;
    }
    assert!(house_data::set(tables), "housing data already loaded");

    let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0).expect("ui runtime");
    let bridge = Rc::new(RefCell::new(HudData::default()));
    let mut game = GameData::default();
    game.items.insert(
        STONE_PACK,
        ItemInfo { named: Named { name: "Stone Pack".into(), ..default() }, ..default() },
    );
    assert!(bridge.borrow().game.0.set(game).is_ok());
    bridge.borrow_mut().character_id = 42;
    api::install(&mut rt, &bridge).expect("hud api");
    rt.load_ui_texts(std::path::Path::new(&db)).expect("ui texts");
    rt.lua
        .load(
            r#"SYSMSG_LAST = nil
            local w = UIParent:CreateWidget('emptywidget', 'houseCastProbe', 'UIParent')
            w:RegisterEvent('SYSMSG')
            w:SetHandler('OnEvent', function(_, _, text) SYSMSG_LAST = text end)"#,
        )
        .exec()
        .expect("sysmsg probe");

    App::new()
        .add_plugins(MinimalPlugins)
        .add_message::<NetCommand>()
        .add_message::<NetEvent>()
        .add_message::<BuilderSignal>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<hud::HudPointerOverUi>()
        .insert_resource(synthetic_units())
        .insert_resource(synthetic_houses())
        .init_resource::<Doodads>()
        .insert_resource(synthetic_inventory())
        .init_resource::<aa_net::Shipyards>()
        .init_resource::<Builder>()
        .insert_non_send(HudBridge(bridge))
        .insert_non_send(rt)
        .init_resource::<Test>()
        .add_systems(Update, (script, sync, check).chain())
        .run()
}

fn unit() -> Unit {
    Unit {
        obj_id: HOUSE_OBJ,
        name: "Thatched Farmhouse".into(),
        kind: UnitKind::Housing { tl_id: 13, template_id: 171, build_step: 0 },
        owner_name: String::new(),
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 1,
        model_id: 1083,
        equipment: Default::default(),
        model_params: Default::default(),
        hp: 1,
        mp: 0,
        max_hp: 1,
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

fn synthetic_units() -> Units {
    let mut units = Units::default();
    units.by_id.insert(HOUSE_OBJ, unit());
    units
}

fn snapshot() -> HouseSnapshot {
    HouseSnapshot {
        tl_id: 13,
        db_id: 13,
        obj_id: HOUSE_OBJ,
        template_id: 171,
        model_id: 1083,
        co_owner_id: 1,
        owner_id: 1,
        owner_name: "Recap".into(),
        account_id: 1,
        permission: 0,
        all_actions: 20,
        current_actions: 3,
        tax: 150_000,
        x: aa_net::wire::coord_to_long(0.0),
        y: aa_net::wire::coord_to_long(0.0),
        z: 0.0,
        name: "Thatched Farmhouse".into(),
        allow_recover: true,
        sell_price: 0,
        sell_to_player_id: 0,
        sell_to_name: String::new(),
    }
}

fn synthetic_houses() -> Houses {
    let mut houses = Houses::default();
    houses.by_tl_id.insert(
        13,
        House { snapshot: snapshot(), demolished: false, build_pay: None, tax: None, sale: None },
    );
    houses
}

fn synthetic_inventory() -> Inventory {
    let mut inventory = Inventory::default();
    inventory.bag.push(Some(Item { template_id: STONE_PACK, count: 1, ..default() }));
    inventory
}

#[derive(Resource, Default)]
struct Test {
    phase: u8,
    casts: u32,
    failed: Vec<String>,
    finished: bool,
}

#[derive(SystemParam)]
struct Script<'w> {
    rt: Option<NonSend<'w, aa_ui::UiRuntime>>,
    bridge: Option<NonSend<'w, HudBridge>>,
    inventory: ResMut<'w, Inventory>,
    test: ResMut<'w, Test>,
}

fn script(mut p: Script) {
    if p.test.finished {
        return;
    }
    let Some(rt) = p.rt.as_deref() else { return };
    let Some(bridge) = p.bridge.as_deref() else { return };
    match p.test.phase {
        0 => {
            bridge.0.borrow_mut().pos = Vec3::ZERO;
            click(rt);
            p.test.phase = 1;
        }
        2 => {
            p.inventory.bag.clear();
            rt.lua.load("SYSMSG_LAST = nil").exec().unwrap();
            click(rt);
            p.test.phase = 3;
        }
        4 => {
            p.inventory.equipment.slots[slot::BACKPACK] = Some(Item { template_id: STONE_PACK, count: 1, ..default() });
            rt.lua.load("SYSMSG_LAST = nil").exec().unwrap();
            click(rt);
            p.test.phase = 5;
        }
        _ => {}
    }
}

fn click(rt: &aa_ui::UiRuntime) {
    rt.lua.load("HUD.house_interact(39109)").exec().expect("house_interact");
}

#[derive(SystemParam)]
struct Check<'w, 's> {
    commands: MessageReader<'w, 's, NetCommand>,
    rt: Option<NonSend<'w, aa_ui::UiRuntime>>,
    test: ResMut<'w, Test>,
}

fn check(mut p: Check) {
    if p.test.finished {
        return;
    }
    for NetCommand(command) in p.commands.read() {
        let Command::UseSkill { skill_id, target_obj_id } = command else { continue };
        p.test.casts += 1;
        if (*skill_id, *target_obj_id) != (SKILL, HOUSE_OBJ) {
            fail(&mut p.test, format!("UseSkill {skill_id} at {target_obj_id}"));
        }
        let body = aa_net::session::use_skill_body(*skill_id, 8615, *target_obj_id);
        if body != CAPTURE_CAST {
            fail(&mut p.test, format!("CSStartSkill differs from the capture: {body:?}"));
        }
    }
    let Some(rt) = p.rt.as_deref() else { return };
    let sysmsg: Option<String> = rt.lua.globals().get("SYSMSG_LAST").ok().flatten();
    match p.test.phase {
        1 => {
            let casts = p.test.casts;
            if casts != 1 {
                fail(&mut p.test, format!("one click sent {casts} casts, expected 1"));
            }
            p.test.phase = 2;
        }
        3 => {
            let casts = p.test.casts;
            if casts != 1 {
                fail(&mut p.test, format!("the no-material click sent a cast ({casts} total)"));
            }
            if sysmsg.as_deref() != Some(NOTICE) {
                fail(&mut p.test, format!("no-material text {sysmsg:?}, expected {NOTICE:?}"));
            }
            p.test.phase = 4;
        }
        5 => {
            let casts = p.test.casts;
            if casts != 2 {
                fail(&mut p.test, format!("the equipped bundle click sent {casts} casts, expected 2"));
            }
            if sysmsg.as_deref() != Some(NOTICE) {
                fail(&mut p.test, format!("equipped-bundle text {sysmsg:?}, expected {NOTICE:?}"));
            }
            p.test.phase = 6;
        }
        6 => {
            p.test.finished = true;
            if p.test.failed.is_empty() {
                println!("house_cast: passed (one CSStartSkill per click, capture bytes, {NOTICE:?})");
                std::process::exit(0);
            }
            for message in &p.test.failed {
                eprintln!("house_cast: FAILED: {message}");
            }
            std::process::exit(1);
        }
        _ => {}
    }
}

fn fail(test: &mut Test, message: String) {
    test.failed.push(message);
}
