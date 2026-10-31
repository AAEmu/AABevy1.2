//! S14 self-check of the action bar's "Use Glider": a press of skill 14901 goes through the real HUD slot
//! (`hud::panels::use_slot`) and the shared skill-use route (`ingame::ground_target::route`), which applies the 1.2
//! linked-item rule (`FUN_393925c0`).
//!
//! It asserts the outcomes from the decompile, using the real game database (`AA_DB`):
//! * glider 14677 equipped in the back slot -> one `Command::UseItem` with skill 12373 (`items.use_skill_id`), the
//!   glider's item id 16777259 and template 14677 as the Item caster, target self;
//! * no glider -> no command and the local error 326 `MUST_EQUIP_PROPER_ITEM`;
//! * a type-3 item whose use skill is 0 (23605) -> no command, no 326, and the slot's `URK_EQUIP_ITEM` refusal.
//!
//! `cargo run -p aa-client --example glider_skill_press -- --mute`

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
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;

use aa_net::items::{ACTION_SLOTS, ActionKind, ActionSlot};
use aa_net::model::Item;
use aa_net::state::{ActionBar, Inventory, Target, Units};
use aa_net::{Command, NetCommand};
use bevy::prelude::*;
use lua_value::lua_int;

use hud::ShowError;
use hud::api::HudData;
use hud::panels;
use hud::usable::{Env, InFlight};
use ingame::data::Db;
use ingame::ground_target::{self, GroundSkills, Script, SkillPressed};

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

const GLIDER_SKILL: u32 = 14_901;
const GLIDER_ITEM: u32 = 14_677;
const GLIDER_ITEM_ID: u64 = 16_777_259;
const GLIDER_USE_SKILL: u32 = 12_373;
const TRADE_PACK_SKILL: u32 = 14_902;
const TRADE_PACK_ITEM: u32 = 23_605;
const TRADE_PACK_ITEM_ID: u64 = 0x0100_0001;

fn bar(skill: u32) -> ActionBar {
    let mut slots = vec![ActionSlot::default(); ACTION_SLOTS];
    slots[1] = ActionSlot {
        kind: ActionKind::Spell,
        id: u64::from(skill),
    };
    ActionBar {
        slots,
        ..Default::default()
    }
}

fn inventory(glider: bool) -> Inventory {
    let mut inventory = Inventory::default();
    if glider {
        inventory.equipment.slots[aa_net::model::slot::BACKPACK] = Some(Item {
            id: GLIDER_ITEM_ID,
            template_id: GLIDER_ITEM,
            ..Default::default()
        });
    }
    inventory
}

fn trade_pack() -> Inventory {
    let mut inventory = Inventory::default();
    inventory.equipment.slots[aa_net::model::slot::BACKPACK] = Some(Item {
        id: TRADE_PACK_ITEM_ID,
        template_id: TRADE_PACK_ITEM,
        ..Default::default()
    });
    inventory
}

/// The bar's press: `use_slot` sees the `Spell` slot and sends the same `SkillPressed` a click or hotkey does.
fn press_bar(
    bar: Res<ActionBar>,
    inventory: Res<Inventory>,
    mut pressed: MessageWriter<SkillPressed>,
    mut failed: MessageWriter<hud::UseFailed>,
) {
    let data = HudData::default();
    let env = Env::of(&data, None, &[], Some(&inventory));
    panels::use_slot(
        1,
        &env,
        &InFlight::default(),
        &bar,
        &inventory,
        &Target::default(),
        &mut pressed,
        &mut failed,
    );
}

fn run(db: Db, skill: u32, inventory: Inventory) -> (Vec<Command>, Vec<i16>, Vec<hud::UseFailed>) {
    let mut app = App::new();
    app.add_message::<SkillPressed>()
        .add_message::<hud::PortalBookUsed>()
        .add_message::<hud::MusicItemUsed>()
        .add_message::<hud::UseFailed>()
        .add_message::<ShowError>()
        .add_message::<NetCommand>()
        .init_resource::<GroundSkills>()
        .init_resource::<ingame::ground_target::Aiming>()
        .init_resource::<hud::HudPointerOverUi>()
        .init_resource::<InFlight>()
        .insert_resource(Script::default())
        .init_resource::<Target>()
        .init_resource::<Units>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(db)
        .insert_resource(bar(skill))
        .insert_resource(inventory)
        .add_systems(Update, (press_bar, ground_target::route).chain());
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|NetCommand(c)| c)
        .collect();
    let alerts = app
        .world_mut()
        .resource_mut::<Messages<ShowError>>()
        .drain()
        .map(|ShowError(code)| code)
        .collect();
    let failed = app
        .world_mut()
        .resource_mut::<Messages<hud::UseFailed>>()
        .drain()
        .collect();
    (sent, alerts, failed)
}

fn main() -> AppExit {
    let Some(db) = Db::open() else {
        eprintln!(
            "glider_skill_press: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    };
    let (sent, alerts, _) = run(db.clone(), GLIDER_SKILL, inventory(true));
    assert!(
        matches!(
            &sent[..],
            [Command::UseItem {
                item_id: GLIDER_ITEM_ID,
                template_id: GLIDER_ITEM,
                skill_id: GLIDER_USE_SKILL,
                target_obj_id: 0
            }]
        ),
        "glider_skill_press: FAILED, the bar press did not become the glider item's use skill: {sent:?}"
    );
    assert!(
        alerts.is_empty(),
        "glider_skill_press: FAILED, an equipped glider raised {alerts:?}"
    );
    println!(
        "glider_skill_press: PASS glider 14677 -> CSStartSkill {GLIDER_USE_SKILL} as item {GLIDER_ITEM_ID}/{GLIDER_ITEM}"
    );

    let (sent, alerts, _) = run(db.clone(), GLIDER_SKILL, inventory(false));
    assert!(
        sent.is_empty(),
        "glider_skill_press: FAILED, a press without a glider sent {sent:?}"
    );
    assert!(
        matches!(&alerts[..], [326]),
        "glider_skill_press: FAILED, expected the local 326, got {alerts:?}"
    );
    println!("glider_skill_press: PASS no glider -> local 326 MUST_EQUIP_PROPER_ITEM, no packet");

    let (sent, alerts, failed) = run(db, TRADE_PACK_SKILL, trade_pack());
    assert!(
        sent.is_empty(),
        "glider_skill_press: FAILED, a use skill of 0 sent {sent:?}"
    );
    assert!(
        alerts.is_empty(),
        "glider_skill_press: FAILED, a use skill of 0 raised the 326 {alerts:?}"
    );
    assert!(
        matches!(
            &failed[..],
            [f] if f.skill == TRADE_PACK_SKILL
                && f.result == hud::usable::code::URK_EQUIP_ITEM
                && !f.server
        ),
        "glider_skill_press: FAILED, expected the slot's URK_EQUIP_ITEM, got {failed:?}"
    );
    println!(
        "glider_skill_press: PASS use skill 0 -> {TRADE_PACK_ITEM} dark, URK_EQUIP_ITEM, no packet"
    );

    AppExit::Success
}
